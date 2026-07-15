// SPDX-License-Identifier: Apache-2.0

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use hyphae_core::DISK_FORMAT_VERSION;
use thiserror::Error;

use crate::{
    CommitReceipt, MAX_KEY_BYTES, MaterializedIndexError, index::MaterializedIndex,
    log::MAX_OPERATION_BYTES,
};

const MAGIC: [u8; 8] = *b"HYSNAP01";
const HEADER_LENGTH: usize = 112;
const HEADER_LENGTH_U64: u64 = 112;
const CHECKSUM_PREFIX_LENGTH: usize = 76;
const DIGEST_PREFIX_LENGTH: usize = 80;
const ENTRY_HEADER_LENGTH: usize = 12;
const ENTRY_HEADER_LENGTH_U64: u64 = 12;
const RECEIPT_LENGTH: usize = 88;
const RECEIPT_LENGTH_U64: u64 = 88;
const COPY_BUFFER_LENGTH: usize = 64 * 1024;
const COPY_BUFFER_LENGTH_U64: u64 = 64 * 1024;

/// Verified metadata for one logical snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotInfo {
    /// Snapshot file path.
    pub path: PathBuf,
    /// Materialized commit sequence captured by the snapshot.
    pub checkpoint_sequence: u64,
    /// Commit digest captured by the snapshot, absent for an empty log.
    pub checkpoint_digest: Option<[u8; 32]>,
    /// Number of sorted KV entries.
    pub entry_count: u64,
    /// Number of sorted durable idempotency receipts.
    pub receipt_count: u64,
    /// BLAKE3 digest of the canonical snapshot content.
    pub snapshot_digest: [u8; 32],
    /// Complete file length.
    pub file_bytes: u64,
}

/// Failure while creating or verifying a logical snapshot.
#[derive(Debug, Error)]
pub enum SnapshotError {
    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// The materialized index could not be read.
    #[error("materialized index failure during snapshot: {source}")]
    Index {
        /// Underlying index failure.
        #[source]
        source: Box<MaterializedIndexError>,
    },

    /// Snapshot bytes violate the canonical format.
    #[error("invalid snapshot: {reason}")]
    Invalid {
        /// Stable diagnostic reason.
        reason: &'static str,
    },

    /// The snapshot was produced by a future disk format.
    #[error("unsupported snapshot format {found}; supported format is {supported}")]
    UnsupportedVersion {
        /// Version found in the snapshot.
        found: u16,
        /// Version understood by this binary.
        supported: u16,
    },

    /// A same-sequence snapshot exists for a different commit.
    #[error("snapshot sequence {sequence} already exists for a different commit")]
    CheckpointConflict {
        /// Conflicting commit sequence.
        sequence: u64,
    },
}

impl From<MaterializedIndexError> for SnapshotError {
    fn from(source: MaterializedIndexError) -> Self {
        Self::Index {
            source: Box::new(source),
        }
    }
}

pub(crate) fn create_snapshot(
    index: &MaterializedIndex,
    snapshots_directory: &Path,
    temporary_directory: &Path,
) -> Result<SnapshotInfo, SnapshotError> {
    let checkpoint = index.checkpoint()?;
    if checkpoint.sequence == 0 && checkpoint.digest.is_some() {
        return Err(SnapshotError::Invalid {
            reason: "empty checkpoint has a digest",
        });
    }

    let measurements = measure_payload(index, checkpoint.sequence)?;
    let final_path =
        snapshots_directory.join(format!("snapshot-{:020}.hysnap", checkpoint.sequence));
    if final_path.exists() {
        let existing = verify_snapshot(&final_path)?;
        if existing.checkpoint_digest != checkpoint.digest {
            return Err(SnapshotError::CheckpointConflict {
                sequence: checkpoint.sequence,
            });
        }
        return Ok(existing);
    }

    let mut header = [0_u8; HEADER_LENGTH];
    header[0..8].copy_from_slice(&MAGIC);
    header[8..10].copy_from_slice(&DISK_FORMAT_VERSION.to_le_bytes());
    header[10..12].copy_from_slice(&0_u16.to_le_bytes());
    header[12..20].copy_from_slice(&checkpoint.sequence.to_le_bytes());
    header[20..52].copy_from_slice(&checkpoint.digest.unwrap_or([0; 32]));
    header[52..60].copy_from_slice(&measurements.entry_count.to_le_bytes());
    header[60..68].copy_from_slice(&measurements.receipt_count.to_le_bytes());
    header[68..76].copy_from_slice(&measurements.payload_length.to_le_bytes());

    let mut checksum = crc32c::crc32c(&header[..CHECKSUM_PREFIX_LENGTH]);
    let mut checksum_error = None;
    index.for_each_entry(|key, value| {
        if checksum_error.is_some() {
            return;
        }
        match encode_entry_header(key, value) {
            Ok(entry_header) => {
                checksum = crc32c::crc32c_append(checksum, &entry_header);
                checksum = crc32c::crc32c_append(checksum, key);
                checksum = crc32c::crc32c_append(checksum, value);
            }
            Err(source) => checksum_error = Some(source),
        }
    })?;
    if let Some(source) = checksum_error {
        return Err(source);
    }
    index.for_each_receipt(|receipt| {
        checksum = crc32c::crc32c_append(checksum, &encode_receipt(receipt));
    })?;
    header[76..80].copy_from_slice(&checksum.to_le_bytes());

    let temporary_path = temporary_directory.join(format!(
        "snapshot-{:020}-{}.tmp",
        checkpoint.sequence,
        uuid::Uuid::now_v7()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&temporary_path)?;
    file.write_all(&header)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&header[..DIGEST_PREFIX_LENGTH]);
    let mut write_error = None;
    index.for_each_entry(|key, value| {
        if write_error.is_none()
            && let Err(source) = write_entry(&mut file, &mut hasher, key, value)
        {
            write_error = Some(source);
        }
    })?;
    if let Some(source) = write_error {
        return Err(source);
    }
    let mut receipt_write_error = None;
    index.for_each_receipt(|receipt| {
        if receipt_write_error.is_none()
            && let Err(source) = write_receipt(&mut file, &mut hasher, receipt)
        {
            receipt_write_error = Some(source);
        }
    })?;
    if let Some(source) = receipt_write_error {
        return Err(source);
    }
    let snapshot_digest = *hasher.finalize().as_bytes();
    file.seek(SeekFrom::Start(80))?;
    file.write_all(&snapshot_digest)?;
    file.sync_all()?;
    drop(file);

    let temporary_info = verify_snapshot(&temporary_path)?;
    std::fs::rename(&temporary_path, &final_path)?;
    #[cfg(unix)]
    sync_directory(snapshots_directory)?;
    Ok(SnapshotInfo {
        path: final_path,
        ..temporary_info
    })
}

/// Streams and verifies a snapshot without opening a Hyphae data directory.
///
/// # Errors
///
/// Returns an error for I/O, future versions, length mismatches, unsorted or
/// duplicate keys, invalid checksums, or invalid digests.
pub fn verify_snapshot(path: impl AsRef<Path>) -> Result<SnapshotInfo, SnapshotError> {
    let path = path.as_ref();
    let mut file = File::open(path)?;
    let file_bytes = file.metadata()?.len();
    let mut header = [0_u8; HEADER_LENGTH];
    read_exact_or_invalid(&mut file, &mut header, "truncated header")?;
    let decoded = decode_header(&header, file_bytes)?;
    verify_payload(&mut file, &header, &decoded)?;

    Ok(SnapshotInfo {
        path: path.to_path_buf(),
        checkpoint_sequence: decoded.checkpoint_sequence,
        checkpoint_digest: decoded.checkpoint_digest,
        entry_count: decoded.entry_count,
        receipt_count: decoded.receipt_count,
        snapshot_digest: decoded.expected_digest,
        file_bytes,
    })
}

pub(crate) trait SnapshotRecordVisitor {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), SnapshotError>;
    fn receipt(&mut self, receipt: &CommitReceipt) -> Result<(), SnapshotError>;
}

pub(crate) fn read_snapshot_records(
    path: &Path,
    visitor: &mut impl SnapshotRecordVisitor,
) -> Result<SnapshotInfo, SnapshotError> {
    let before = verify_snapshot(path)?;
    let mut file = File::open(path)?;
    let file_bytes = file.metadata()?.len();
    let mut header = [0_u8; HEADER_LENGTH];
    read_exact_or_invalid(&mut file, &mut header, "truncated header")?;
    let decoded = decode_header(&header, file_bytes)?;
    let mut consumed = 0_u64;

    for _ in 0..decoded.entry_count {
        let mut entry_header = [0_u8; ENTRY_HEADER_LENGTH];
        read_payload_exact(
            &mut file,
            &mut entry_header,
            &mut consumed,
            decoded.payload_length,
        )?;
        let key_length = usize::try_from(u32::from_le_bytes(copy_array(&entry_header[..4])))
            .map_err(|_| SnapshotError::Invalid {
                reason: "key length overflow during restore",
            })?;
        let value_length = usize::try_from(u64::from_le_bytes(copy_array(&entry_header[4..12])))
            .map_err(|_| SnapshotError::Invalid {
                reason: "value length overflow during restore",
            })?;
        if key_length == 0 || key_length > MAX_KEY_BYTES || value_length > MAX_OPERATION_BYTES {
            return Err(SnapshotError::Invalid {
                reason: "record exceeds restore bounds",
            });
        }
        let mut key = vec![0_u8; key_length];
        let mut value = vec![0_u8; value_length];
        read_payload_exact(&mut file, &mut key, &mut consumed, decoded.payload_length)?;
        read_payload_exact(&mut file, &mut value, &mut consumed, decoded.payload_length)?;
        visitor.put(&key, &value)?;
    }
    for _ in 0..decoded.receipt_count {
        let mut encoded = [0_u8; RECEIPT_LENGTH];
        read_payload_exact(
            &mut file,
            &mut encoded,
            &mut consumed,
            decoded.payload_length,
        )?;
        visitor.receipt(&decode_snapshot_receipt(&encoded))?;
    }
    if consumed != decoded.payload_length {
        return Err(SnapshotError::Invalid {
            reason: "record counts do not consume payload during restore",
        });
    }

    let after = verify_snapshot(path)?;
    if before != after {
        return Err(SnapshotError::Invalid {
            reason: "snapshot changed during restore",
        });
    }
    Ok(after)
}

#[derive(Clone, Copy, Debug)]
struct DecodedHeader {
    checkpoint_sequence: u64,
    checkpoint_digest: Option<[u8; 32]>,
    entry_count: u64,
    receipt_count: u64,
    payload_length: u64,
    expected_checksum: u32,
    expected_digest: [u8; 32],
}

fn decode_header(
    header: &[u8; HEADER_LENGTH],
    file_bytes: u64,
) -> Result<DecodedHeader, SnapshotError> {
    if header[0..8] != MAGIC {
        return Err(SnapshotError::Invalid {
            reason: "bad magic",
        });
    }
    let version = u16::from_le_bytes(copy_array(&header[8..10]));
    if version != DISK_FORMAT_VERSION {
        return Err(SnapshotError::UnsupportedVersion {
            found: version,
            supported: DISK_FORMAT_VERSION,
        });
    }
    if u16::from_le_bytes(copy_array(&header[10..12])) != 0 {
        return Err(SnapshotError::Invalid {
            reason: "unsupported flags",
        });
    }

    let checkpoint_sequence = u64::from_le_bytes(copy_array(&header[12..20]));
    let raw_checkpoint_digest: [u8; 32] = copy_array(&header[20..52]);
    let checkpoint_digest = if checkpoint_sequence == 0 {
        if raw_checkpoint_digest != [0; 32] {
            return Err(SnapshotError::Invalid {
                reason: "empty checkpoint has a digest",
            });
        }
        None
    } else {
        Some(raw_checkpoint_digest)
    };
    let entry_count = u64::from_le_bytes(copy_array(&header[52..60]));
    let receipt_count = u64::from_le_bytes(copy_array(&header[60..68]));
    if checkpoint_sequence == 0 && receipt_count != 0 {
        return Err(SnapshotError::Invalid {
            reason: "empty checkpoint has idempotency receipts",
        });
    }
    let payload_length = u64::from_le_bytes(copy_array(&header[68..76]));
    let expected_file_bytes =
        HEADER_LENGTH_U64
            .checked_add(payload_length)
            .ok_or(SnapshotError::Invalid {
                reason: "file length overflow",
            })?;
    if file_bytes != expected_file_bytes {
        return Err(SnapshotError::Invalid {
            reason: "file length mismatch",
        });
    }

    Ok(DecodedHeader {
        checkpoint_sequence,
        checkpoint_digest,
        entry_count,
        receipt_count,
        payload_length,
        expected_checksum: u32::from_le_bytes(copy_array(&header[76..80])),
        expected_digest: copy_array(&header[80..112]),
    })
}

fn verify_payload(
    file: &mut File,
    header: &[u8; HEADER_LENGTH],
    decoded: &DecodedHeader,
) -> Result<(), SnapshotError> {
    let mut checksum = crc32c::crc32c(&header[..CHECKSUM_PREFIX_LENGTH]);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&header[..DIGEST_PREFIX_LENGTH]);
    let mut consumed = 0_u64;
    let mut previous_key: Option<Vec<u8>> = None;
    let mut buffer = vec![0_u8; COPY_BUFFER_LENGTH].into_boxed_slice();
    for _ in 0..decoded.entry_count {
        let mut entry_header = [0_u8; ENTRY_HEADER_LENGTH];
        read_payload_exact(
            file,
            &mut entry_header,
            &mut consumed,
            decoded.payload_length,
        )?;
        checksum = crc32c::crc32c_append(checksum, &entry_header);
        hasher.update(&entry_header);
        let key_length = usize::try_from(u32::from_le_bytes(copy_array(&entry_header[..4])))
            .map_err(|_| SnapshotError::Invalid {
                reason: "key length overflow",
            })?;
        let value_length = u64::from_le_bytes(copy_array(&entry_header[4..12]));
        if key_length == 0 || key_length > MAX_KEY_BYTES {
            return Err(SnapshotError::Invalid {
                reason: "invalid key length",
            });
        }

        let mut key = vec![0_u8; key_length];
        read_payload_exact(file, &mut key, &mut consumed, decoded.payload_length)?;
        checksum = crc32c::crc32c_append(checksum, &key);
        hasher.update(&key);
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(SnapshotError::Invalid {
                reason: "keys are not strictly sorted",
            });
        }
        previous_key = Some(key);

        let mut remaining = value_length;
        while remaining > 0 {
            let chunk_length =
                usize::try_from(remaining.min(COPY_BUFFER_LENGTH_U64)).map_err(|_| {
                    SnapshotError::Invalid {
                        reason: "value length overflow",
                    }
                })?;
            let chunk = &mut buffer[..chunk_length];
            read_payload_exact(file, chunk, &mut consumed, decoded.payload_length)?;
            checksum = crc32c::crc32c_append(checksum, chunk);
            hasher.update(chunk);
            remaining -= u64::try_from(chunk_length).map_err(|_| SnapshotError::Invalid {
                reason: "value length overflow",
            })?;
        }
    }
    let mut previous_transaction_id = None;
    for _ in 0..decoded.receipt_count {
        let mut encoded = [0_u8; RECEIPT_LENGTH];
        read_payload_exact(file, &mut encoded, &mut consumed, decoded.payload_length)?;
        checksum = crc32c::crc32c_append(checksum, &encoded);
        hasher.update(&encoded);

        let transaction_id: [u8; 16] = copy_array(&encoded[..16]);
        if previous_transaction_id
            .as_ref()
            .is_some_and(|previous| previous >= &transaction_id)
        {
            return Err(SnapshotError::Invalid {
                reason: "transaction identifiers are not strictly sorted",
            });
        }
        previous_transaction_id = Some(transaction_id);
        let commit_sequence = u64::from_le_bytes(copy_array(&encoded[16..24]));
        if commit_sequence == 0 || commit_sequence > decoded.checkpoint_sequence {
            return Err(SnapshotError::Invalid {
                reason: "idempotency receipt exceeds snapshot checkpoint",
            });
        }
    }
    if consumed != decoded.payload_length {
        return Err(SnapshotError::Invalid {
            reason: "record counts do not consume payload",
        });
    }
    if checksum != decoded.expected_checksum {
        return Err(SnapshotError::Invalid {
            reason: "CRC32C mismatch",
        });
    }
    let actual_digest = *hasher.finalize().as_bytes();
    if actual_digest != decoded.expected_digest {
        return Err(SnapshotError::Invalid {
            reason: "BLAKE3 digest mismatch",
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Measurements {
    entry_count: u64,
    receipt_count: u64,
    payload_length: u64,
}

fn measure_payload(
    index: &MaterializedIndex,
    checkpoint_sequence: u64,
) -> Result<Measurements, SnapshotError> {
    let mut entry_count = Some(0_u64);
    let mut payload_length = Some(0_u64);
    let mut valid = true;
    index.for_each_entry(|key, value| {
        if key.is_empty() || key.len() > MAX_KEY_BYTES {
            valid = false;
            return;
        }
        let Ok(key_length) = u64::try_from(key.len()) else {
            valid = false;
            return;
        };
        let Ok(value_length) = u64::try_from(value.len()) else {
            valid = false;
            return;
        };
        entry_count = entry_count.and_then(|count| count.checked_add(1));
        payload_length = payload_length.and_then(|length| {
            length
                .checked_add(ENTRY_HEADER_LENGTH_U64)
                .and_then(|length| length.checked_add(key_length))
                .and_then(|length| length.checked_add(value_length))
        });
    })?;
    let mut receipt_count = Some(0_u64);
    index.for_each_receipt(|receipt| {
        if receipt.commit_sequence == 0 || receipt.commit_sequence > checkpoint_sequence {
            valid = false;
            return;
        }
        receipt_count = receipt_count.and_then(|count| count.checked_add(1));
        payload_length = payload_length.and_then(|length| length.checked_add(RECEIPT_LENGTH_U64));
    })?;
    if !valid {
        return Err(SnapshotError::Invalid {
            reason: "index contains an invalid key or idempotency receipt",
        });
    }
    let Some(entry_count) = entry_count else {
        return Err(SnapshotError::Invalid {
            reason: "entry count overflow",
        });
    };
    let Some(payload_length) = payload_length else {
        return Err(SnapshotError::Invalid {
            reason: "payload length overflow",
        });
    };
    let Some(receipt_count) = receipt_count else {
        return Err(SnapshotError::Invalid {
            reason: "receipt count overflow",
        });
    };
    Ok(Measurements {
        entry_count,
        receipt_count,
        payload_length,
    })
}

fn encode_entry_header(
    key: &[u8],
    value: &[u8],
) -> Result<[u8; ENTRY_HEADER_LENGTH], SnapshotError> {
    let key_length = u32::try_from(key.len()).map_err(|_| SnapshotError::Invalid {
        reason: "key length overflow",
    })?;
    let value_length = u64::try_from(value.len()).map_err(|_| SnapshotError::Invalid {
        reason: "value length overflow",
    })?;
    let mut entry_header = [0_u8; ENTRY_HEADER_LENGTH];
    entry_header[..4].copy_from_slice(&key_length.to_le_bytes());
    entry_header[4..].copy_from_slice(&value_length.to_le_bytes());
    Ok(entry_header)
}

fn write_entry(
    writer: &mut impl Write,
    hasher: &mut blake3::Hasher,
    key: &[u8],
    value: &[u8],
) -> Result<(), SnapshotError> {
    let entry_header = encode_entry_header(key, value)?;
    for bytes in [&entry_header[..], key, value] {
        writer.write_all(bytes)?;
        hasher.update(bytes);
    }
    Ok(())
}

fn encode_receipt(receipt: &CommitReceipt) -> [u8; RECEIPT_LENGTH] {
    let mut encoded = [0_u8; RECEIPT_LENGTH];
    encoded[..16].copy_from_slice(receipt.transaction_id.as_bytes());
    encoded[16..24].copy_from_slice(&receipt.commit_sequence.to_le_bytes());
    encoded[24..56].copy_from_slice(&receipt.commit_digest);
    encoded[56..88].copy_from_slice(&receipt.transaction_digest);
    encoded
}

fn decode_snapshot_receipt(encoded: &[u8; RECEIPT_LENGTH]) -> CommitReceipt {
    CommitReceipt {
        transaction_id: uuid::Uuid::from_bytes(copy_array(&encoded[..16])),
        commit_sequence: u64::from_le_bytes(copy_array(&encoded[16..24])),
        commit_digest: copy_array(&encoded[24..56]),
        transaction_digest: copy_array(&encoded[56..88]),
    }
}

fn write_receipt(
    writer: &mut impl Write,
    hasher: &mut blake3::Hasher,
    receipt: &CommitReceipt,
) -> Result<(), SnapshotError> {
    let encoded = encode_receipt(receipt);
    writer.write_all(&encoded)?;
    hasher.update(&encoded);
    Ok(())
}

fn read_payload_exact(
    reader: &mut impl Read,
    buffer: &mut [u8],
    consumed: &mut u64,
    payload_length: u64,
) -> Result<(), SnapshotError> {
    let length = u64::try_from(buffer.len()).map_err(|_| SnapshotError::Invalid {
        reason: "payload length overflow",
    })?;
    let next = consumed.checked_add(length).ok_or(SnapshotError::Invalid {
        reason: "payload length overflow",
    })?;
    if next > payload_length {
        return Err(SnapshotError::Invalid {
            reason: "entry exceeds payload",
        });
    }
    read_exact_or_invalid(reader, buffer, "truncated payload")?;
    *consumed = next;
    Ok(())
}

fn read_exact_or_invalid(
    reader: &mut impl Read,
    buffer: &mut [u8],
    reason: &'static str,
) -> Result<(), SnapshotError> {
    reader.read_exact(buffer).map_err(|source| {
        if source.kind() == io::ErrorKind::UnexpectedEof {
            SnapshotError::Invalid { reason }
        } else {
            SnapshotError::Io(source)
        }
    })
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), SnapshotError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn copy_array<const N: usize>(source: &[u8]) -> [u8; N] {
    let mut output = [0_u8; N];
    output.copy_from_slice(source);
    output
}
