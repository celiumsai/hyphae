// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use hyphae_core::{
    DISK_FORMAT_VERSION, MIN_DISK_FORMAT_VERSION, Q15Vector, VectorMetric, VectorSpaceDefinition,
    VectorSpaceName,
};
use hyphae_query::FieldPath;
use hyphae_retrieval::{
    LexicalField, LexicalIndexDefinition, MAX_LEXICAL_FIELDS, MAX_LEXICAL_PATH_SEGMENT_BYTES,
    MAX_LEXICAL_PATH_SEGMENTS,
};
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
const V2_COUNTS_LENGTH: usize = 24;
const V2_COUNTS_LENGTH_U64: u64 = 24;
const VECTOR_SPACE_FIXED_LENGTH_U64: u64 = 5;
const VECTOR_FIXED_LENGTH_U64: u64 = 7;
const COPY_BUFFER_LENGTH: usize = 64 * 1024;
const COPY_BUFFER_LENGTH_U64: u64 = 64 * 1024;

/// Verified metadata for one logical snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotInfo {
    /// Snapshot file path.
    pub path: PathBuf,
    /// Disk-format version governing the logical payload.
    pub disk_format_version: u16,
    /// Materialized commit sequence captured by the snapshot.
    pub checkpoint_sequence: u64,
    /// Commit digest captured by the snapshot, absent for an empty log.
    pub checkpoint_digest: Option<[u8; 32]>,
    /// Number of sorted KV entries.
    pub entry_count: u64,
    /// Number of sorted named vector-space definitions.
    pub vector_space_count: u64,
    /// Number of sorted durable vector records.
    pub vector_count: u64,
    /// Number of sorted immutable lexical-index definitions.
    pub lexical_index_count: u64,
    /// Number of sorted durable idempotency receipts.
    pub receipt_count: u64,
    /// BLAKE3 digest of the canonical snapshot content.
    pub snapshot_digest: [u8; 32],
    /// Complete file length.
    pub file_bytes: u64,
}

/// Resource limits for loading a verified logical snapshot as an offline
/// witness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotReadLimits {
    /// Maximum complete snapshot file length.
    pub file_bytes: u64,
    /// Maximum number of logical KV entries.
    pub entries: u64,
    /// Maximum aggregate decoded key and value bytes retained in memory.
    pub decoded_bytes: u64,
}

impl Default for SnapshotReadLimits {
    fn default() -> Self {
        Self {
            file_bytes: 2 * 1024 * 1024 * 1024,
            entries: 1_000_000,
            decoded_bytes: 1024 * 1024 * 1024,
        }
    }
}

/// One verified logical KV entry loaded from a canonical snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotEntry {
    /// Nonempty binary key.
    pub key: Vec<u8>,
    /// Opaque stored value bytes.
    pub value: Vec<u8>,
}

/// Fully verified logical snapshot contents for offline operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotContents {
    /// Verified snapshot metadata and checkpoint anchor.
    pub info: SnapshotInfo,
    /// Strictly key-ordered logical entries.
    pub entries: Vec<SnapshotEntry>,
    /// Strictly name-ordered vector-space definitions.
    pub vector_spaces: Vec<VectorSpaceDefinition>,
    /// Strictly `(space, key)`-ordered durable vector records.
    pub vectors: Vec<SnapshotVectorEntry>,
    /// Strictly name-ordered immutable lexical-index definitions.
    pub lexical_indexes: Vec<LexicalIndexDefinition>,
}

/// One verified durable vector loaded from a canonical snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotVectorEntry {
    /// Canonical vector-space name.
    pub space: VectorSpaceName,
    /// Nonempty binary object key.
    pub key: Vec<u8>,
    /// Canonical signed-Q15 vector.
    pub vector: Q15Vector,
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

    /// Snapshot file length exceeds caller policy.
    #[error("snapshot file length {actual} exceeds verification limit {maximum}")]
    FileLimitExceeded {
        /// Observed file length.
        actual: u64,
        /// Configured maximum.
        maximum: u64,
    },

    /// Snapshot entry count exceeds caller policy.
    #[error("snapshot entry count {actual} exceeds verification limit {maximum}")]
    EntryLimitExceeded {
        /// Observed entry count.
        actual: u64,
        /// Configured maximum.
        maximum: u64,
    },

    /// Aggregate decoded entry bytes exceed caller policy.
    #[error("snapshot decoded bytes exceed verification limit {maximum}")]
    DecodedBytesLimitExceeded {
        /// Configured maximum.
        maximum: u64,
    },
}

impl From<MaterializedIndexError> for SnapshotError {
    fn from(source: MaterializedIndexError) -> Self {
        Self::Index {
            source: Box::new(source),
        }
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn create_snapshot(
    index: &MaterializedIndex,
    snapshots_directory: &Path,
    temporary_directory: &Path,
    disk_format_version: u16,
) -> Result<SnapshotInfo, SnapshotError> {
    let checkpoint = index.checkpoint()?;
    if checkpoint.sequence == 0 && checkpoint.digest.is_some() {
        return Err(SnapshotError::Invalid {
            reason: "empty checkpoint has a digest",
        });
    }

    let measurements = measure_payload(index, checkpoint.sequence, disk_format_version)?;
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
    header[8..10].copy_from_slice(&disk_format_version.to_le_bytes());
    header[10..12].copy_from_slice(&0_u16.to_le_bytes());
    header[12..20].copy_from_slice(&checkpoint.sequence.to_le_bytes());
    header[20..52].copy_from_slice(&checkpoint.digest.unwrap_or([0; 32]));
    header[52..60].copy_from_slice(&measurements.entry_count.to_le_bytes());
    header[60..68].copy_from_slice(&measurements.receipt_count.to_le_bytes());
    header[68..76].copy_from_slice(&measurements.payload_length.to_le_bytes());

    let mut checksum = crc32c::crc32c(&header[..CHECKSUM_PREFIX_LENGTH]);
    if disk_format_version >= 2 {
        checksum = crc32c::crc32c_append(checksum, &measurements.v2_counts());
    }
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
    let mut vector_checksum_error = None;
    if disk_format_version >= 2 {
        index.for_each_vector_space(|definition| {
            if vector_checksum_error.is_none() {
                match encode_vector_space(definition) {
                    Ok(encoded) => checksum = crc32c::crc32c_append(checksum, &encoded),
                    Err(source) => vector_checksum_error = Some(source),
                }
            }
        })?;
        index.for_each_vector(|space, key, vector| {
            if vector_checksum_error.is_none() {
                match encode_vector(space, key, vector) {
                    Ok(encoded) => checksum = crc32c::crc32c_append(checksum, &encoded),
                    Err(source) => vector_checksum_error = Some(source),
                }
            }
        })?;
        index.for_each_lexical_index(|definition| {
            if vector_checksum_error.is_none() {
                match encode_lexical_index(definition) {
                    Ok(encoded) => checksum = crc32c::crc32c_append(checksum, &encoded),
                    Err(source) => vector_checksum_error = Some(source),
                }
            }
        })?;
    }
    if let Some(source) = vector_checksum_error {
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
    if disk_format_version >= 2 {
        let counts = measurements.v2_counts();
        file.write_all(&counts)?;
        hasher.update(&counts);
    }
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
    let mut vector_write_error = None;
    if disk_format_version >= 2 {
        index.for_each_vector_space(|definition| {
            if vector_write_error.is_none()
                && let Err(source) =
                    write_encoded(&mut file, &mut hasher, encode_vector_space(definition))
            {
                vector_write_error = Some(source);
            }
        })?;
        index.for_each_vector(|space, key, vector| {
            if vector_write_error.is_none()
                && let Err(source) =
                    write_encoded(&mut file, &mut hasher, encode_vector(space, key, vector))
            {
                vector_write_error = Some(source);
            }
        })?;
        index.for_each_lexical_index(|definition| {
            if vector_write_error.is_none()
                && let Err(source) =
                    write_encoded(&mut file, &mut hasher, encode_lexical_index(definition))
            {
                vector_write_error = Some(source);
            }
        })?;
    }
    if let Some(source) = vector_write_error {
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
    let (vector_space_count, vector_count, lexical_index_count) =
        verify_payload(&mut file, &header, &decoded)?;

    Ok(SnapshotInfo {
        path: path.to_path_buf(),
        disk_format_version: decoded.disk_format_version,
        checkpoint_sequence: decoded.checkpoint_sequence,
        checkpoint_digest: decoded.checkpoint_digest,
        entry_count: decoded.entry_count,
        vector_space_count,
        vector_count,
        lexical_index_count,
        receipt_count: decoded.receipt_count,
        snapshot_digest: decoded.expected_digest,
        file_bytes,
    })
}

/// Loads every logical KV entry from a verified snapshot under explicit
/// resource limits.
///
/// The snapshot is verified before and after streaming to reject mutation
/// during the read. Durable idempotency receipts are verified but are not
/// retained in the returned witness.
///
/// # Errors
///
/// Returns a canonical snapshot error, I/O error, concurrent-change error, or
/// resource-limit error.
pub fn load_snapshot(
    path: impl AsRef<Path>,
    limits: &SnapshotReadLimits,
) -> Result<SnapshotContents, SnapshotError> {
    let path = path.as_ref();
    let mut collector = SnapshotCollector {
        entries: Vec::new(),
        vector_spaces: Vec::new(),
        vectors: Vec::new(),
        lexical_indexes: Vec::new(),
        decoded_bytes: 0,
        limits,
    };
    let info = read_snapshot_records_with_limits(path, &mut collector, Some(limits))?;
    Ok(SnapshotContents {
        info,
        entries: collector.entries,
        vector_spaces: collector.vector_spaces,
        vectors: collector.vectors,
        lexical_indexes: collector.lexical_indexes,
    })
}

pub(crate) trait SnapshotRecordVisitor {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), SnapshotError>;
    fn vector_space(&mut self, _definition: &VectorSpaceDefinition) -> Result<(), SnapshotError> {
        Ok(())
    }
    fn vector(
        &mut self,
        _space: &VectorSpaceName,
        _key: &[u8],
        _vector: &Q15Vector,
    ) -> Result<(), SnapshotError> {
        Ok(())
    }
    fn lexical_index(&mut self, _definition: &LexicalIndexDefinition) -> Result<(), SnapshotError> {
        Ok(())
    }
    fn receipt(&mut self, receipt: &CommitReceipt) -> Result<(), SnapshotError>;
}

pub(crate) fn read_snapshot_records(
    path: &Path,
    visitor: &mut impl SnapshotRecordVisitor,
) -> Result<SnapshotInfo, SnapshotError> {
    read_snapshot_records_with_limits(path, visitor, None)
}

fn read_snapshot_records_with_limits(
    path: &Path,
    visitor: &mut impl SnapshotRecordVisitor,
    limits: Option<&SnapshotReadLimits>,
) -> Result<SnapshotInfo, SnapshotError> {
    let before = verify_snapshot(path)?;
    if let Some(limits) = limits {
        validate_read_limits(&before, limits)?;
    }
    let mut file = File::open(path)?;
    let file_bytes = file.metadata()?.len();
    let mut header = [0_u8; HEADER_LENGTH];
    read_exact_or_invalid(&mut file, &mut header, "truncated header")?;
    let decoded = decode_header(&header, file_bytes)?;
    let mut consumed = 0_u64;
    let (vector_space_count, vector_count, lexical_index_count) =
        read_v2_counts(&mut file, &decoded, &mut consumed)?;

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
    for _ in 0..vector_space_count {
        let definition = read_vector_space(&mut file, &decoded, &mut consumed)?;
        visitor.vector_space(&definition)?;
    }
    for _ in 0..vector_count {
        let (space, key, vector) = read_vector(&mut file, &decoded, &mut consumed)?;
        visitor.vector(&space, &key, &vector)?;
    }
    for _ in 0..lexical_index_count {
        let definition = read_lexical_index(&mut file, &decoded, &mut consumed)?;
        visitor.lexical_index(&definition)?;
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

fn validate_read_limits(
    info: &SnapshotInfo,
    limits: &SnapshotReadLimits,
) -> Result<(), SnapshotError> {
    if info.file_bytes > limits.file_bytes {
        return Err(SnapshotError::FileLimitExceeded {
            actual: info.file_bytes,
            maximum: limits.file_bytes,
        });
    }
    let logical_records = info
        .entry_count
        .checked_add(info.vector_space_count)
        .and_then(|count| count.checked_add(info.vector_count))
        .and_then(|count| count.checked_add(info.lexical_index_count))
        .ok_or(SnapshotError::EntryLimitExceeded {
            actual: u64::MAX,
            maximum: limits.entries,
        })?;
    if logical_records > limits.entries {
        return Err(SnapshotError::EntryLimitExceeded {
            actual: logical_records,
            maximum: limits.entries,
        });
    }
    Ok(())
}

struct SnapshotCollector<'limits> {
    entries: Vec<SnapshotEntry>,
    vector_spaces: Vec<VectorSpaceDefinition>,
    vectors: Vec<SnapshotVectorEntry>,
    lexical_indexes: Vec<LexicalIndexDefinition>,
    decoded_bytes: u64,
    limits: &'limits SnapshotReadLimits,
}

impl SnapshotRecordVisitor for SnapshotCollector<'_> {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), SnapshotError> {
        let next_entry_count = u64::try_from(self.entries.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(SnapshotError::EntryLimitExceeded {
                actual: u64::MAX,
                maximum: self.limits.entries,
            })?;
        if next_entry_count > self.limits.entries {
            return Err(SnapshotError::EntryLimitExceeded {
                actual: next_entry_count,
                maximum: self.limits.entries,
            });
        }
        let entry_bytes = u64::try_from(key.len())
            .ok()
            .and_then(|key_bytes| {
                u64::try_from(value.len())
                    .ok()
                    .and_then(|value_bytes| key_bytes.checked_add(value_bytes))
            })
            .ok_or(SnapshotError::DecodedBytesLimitExceeded {
                maximum: self.limits.decoded_bytes,
            })?;
        self.decoded_bytes = self.decoded_bytes.checked_add(entry_bytes).ok_or(
            SnapshotError::DecodedBytesLimitExceeded {
                maximum: self.limits.decoded_bytes,
            },
        )?;
        if self.decoded_bytes > self.limits.decoded_bytes {
            return Err(SnapshotError::DecodedBytesLimitExceeded {
                maximum: self.limits.decoded_bytes,
            });
        }
        self.entries.push(SnapshotEntry {
            key: key.to_vec(),
            value: value.to_vec(),
        });
        Ok(())
    }

    fn receipt(&mut self, _receipt: &CommitReceipt) -> Result<(), SnapshotError> {
        Ok(())
    }

    fn vector_space(&mut self, definition: &VectorSpaceDefinition) -> Result<(), SnapshotError> {
        self.add_decoded_bytes(definition.name.as_str().len())?;
        self.vector_spaces.push(definition.clone());
        Ok(())
    }

    fn vector(
        &mut self,
        space: &VectorSpaceName,
        key: &[u8],
        vector: &Q15Vector,
    ) -> Result<(), SnapshotError> {
        let vector_bytes = vector
            .as_slice()
            .len()
            .checked_mul(2)
            .and_then(|length| length.checked_add(space.as_str().len()))
            .and_then(|length| length.checked_add(key.len()))
            .ok_or(SnapshotError::DecodedBytesLimitExceeded {
                maximum: self.limits.decoded_bytes,
            })?;
        self.add_decoded_bytes(vector_bytes)?;
        self.vectors.push(SnapshotVectorEntry {
            space: space.clone(),
            key: key.to_vec(),
            vector: vector.clone(),
        });
        Ok(())
    }

    fn lexical_index(&mut self, definition: &LexicalIndexDefinition) -> Result<(), SnapshotError> {
        let encoded_length = encode_lexical_index(definition)?.len();
        self.add_decoded_bytes(encoded_length)?;
        self.lexical_indexes.push(definition.clone());
        Ok(())
    }
}

impl SnapshotCollector<'_> {
    fn add_decoded_bytes(&mut self, bytes: usize) -> Result<(), SnapshotError> {
        let bytes = u64::try_from(bytes).map_err(|_| SnapshotError::DecodedBytesLimitExceeded {
            maximum: self.limits.decoded_bytes,
        })?;
        self.decoded_bytes = self.decoded_bytes.checked_add(bytes).ok_or(
            SnapshotError::DecodedBytesLimitExceeded {
                maximum: self.limits.decoded_bytes,
            },
        )?;
        if self.decoded_bytes > self.limits.decoded_bytes {
            return Err(SnapshotError::DecodedBytesLimitExceeded {
                maximum: self.limits.decoded_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct DecodedHeader {
    disk_format_version: u16,
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
    if !(MIN_DISK_FORMAT_VERSION..=DISK_FORMAT_VERSION).contains(&version) {
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
        disk_format_version: version,
        checkpoint_sequence,
        checkpoint_digest,
        entry_count,
        receipt_count,
        payload_length,
        expected_checksum: u32::from_le_bytes(copy_array(&header[76..80])),
        expected_digest: copy_array(&header[80..112]),
    })
}

#[allow(clippy::too_many_lines)]
fn verify_payload(
    file: &mut File,
    header: &[u8; HEADER_LENGTH],
    decoded: &DecodedHeader,
) -> Result<(u64, u64, u64), SnapshotError> {
    let mut checksum = crc32c::crc32c(&header[..CHECKSUM_PREFIX_LENGTH]);
    let mut hasher = blake3::Hasher::new();
    hasher.update(&header[..DIGEST_PREFIX_LENGTH]);
    let mut consumed = 0_u64;
    let mut counts_bytes = [0_u8; V2_COUNTS_LENGTH];
    let (vector_space_count, vector_count, lexical_index_count) =
        if decoded.disk_format_version >= 2 {
            read_payload_exact(
                file,
                &mut counts_bytes,
                &mut consumed,
                decoded.payload_length,
            )?;
            checksum = crc32c::crc32c_append(checksum, &counts_bytes);
            hasher.update(&counts_bytes);
            (
                u64::from_le_bytes(copy_array(&counts_bytes[..8])),
                u64::from_le_bytes(copy_array(&counts_bytes[8..16])),
                u64::from_le_bytes(copy_array(&counts_bytes[16..24])),
            )
        } else {
            (0, 0, 0)
        };
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
    let mut definitions = BTreeMap::new();
    let mut previous_space: Option<VectorSpaceName> = None;
    for _ in 0..vector_space_count {
        let encoded = read_encoded_vector_space(file, decoded, &mut consumed)?;
        checksum = crc32c::crc32c_append(checksum, &encoded);
        hasher.update(&encoded);
        let definition = decode_vector_space(&encoded)?;
        if previous_space
            .as_ref()
            .is_some_and(|previous| previous >= &definition.name)
        {
            return Err(SnapshotError::Invalid {
                reason: "vector spaces are not strictly sorted",
            });
        }
        previous_space = Some(definition.name.clone());
        definitions.insert(definition.name.clone(), definition);
    }
    let mut previous_vector_identity: Option<(VectorSpaceName, Vec<u8>)> = None;
    for _ in 0..vector_count {
        let encoded = read_encoded_vector(file, decoded, &mut consumed)?;
        checksum = crc32c::crc32c_append(checksum, &encoded);
        hasher.update(&encoded);
        let (space, key, vector) = decode_vector(&encoded)?;
        if previous_vector_identity
            .as_ref()
            .is_some_and(|previous| previous >= &(space.clone(), key.clone()))
        {
            return Err(SnapshotError::Invalid {
                reason: "vectors are not strictly sorted",
            });
        }
        let definition = definitions.get(&space).ok_or(SnapshotError::Invalid {
            reason: "vector references an undefined space",
        })?;
        definition
            .validate_vector(&vector)
            .map_err(|_| SnapshotError::Invalid {
                reason: "vector dimension does not match its space",
            })?;
        previous_vector_identity = Some((space, key));
    }
    let mut previous_lexical_name: Option<VectorSpaceName> = None;
    for _ in 0..lexical_index_count {
        let encoded = read_encoded_lexical_index(file, decoded, &mut consumed)?;
        checksum = crc32c::crc32c_append(checksum, &encoded);
        hasher.update(&encoded);
        let definition = decode_lexical_index(&encoded)?;
        if previous_lexical_name
            .as_ref()
            .is_some_and(|previous| previous >= &definition.name)
        {
            return Err(SnapshotError::Invalid {
                reason: "lexical indexes are not strictly sorted",
            });
        }
        previous_lexical_name = Some(definition.name);
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
    Ok((vector_space_count, vector_count, lexical_index_count))
}

#[derive(Clone, Copy, Debug)]
struct Measurements {
    entry_count: u64,
    vector_space_count: u64,
    vector_count: u64,
    lexical_index_count: u64,
    receipt_count: u64,
    payload_length: u64,
}

impl Measurements {
    fn v2_counts(self) -> [u8; V2_COUNTS_LENGTH] {
        let mut encoded = [0_u8; V2_COUNTS_LENGTH];
        encoded[..8].copy_from_slice(&self.vector_space_count.to_le_bytes());
        encoded[8..16].copy_from_slice(&self.vector_count.to_le_bytes());
        encoded[16..24].copy_from_slice(&self.lexical_index_count.to_le_bytes());
        encoded
    }
}

#[allow(clippy::too_many_lines)]
fn measure_payload(
    index: &MaterializedIndex,
    checkpoint_sequence: u64,
    disk_format_version: u16,
) -> Result<Measurements, SnapshotError> {
    if !(MIN_DISK_FORMAT_VERSION..=DISK_FORMAT_VERSION).contains(&disk_format_version) {
        return Err(SnapshotError::UnsupportedVersion {
            found: disk_format_version,
            supported: DISK_FORMAT_VERSION,
        });
    }
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
    let mut vector_space_count = Some(0_u64);
    let mut vector_count = Some(0_u64);
    let mut lexical_index_count = Some(0_u64);
    if disk_format_version >= 2 {
        payload_length = payload_length.and_then(|length| length.checked_add(V2_COUNTS_LENGTH_U64));
        index.for_each_vector_space(|definition| {
            let Ok(name_length) = u64::try_from(definition.name.as_str().len()) else {
                valid = false;
                return;
            };
            vector_space_count = vector_space_count.and_then(|count| count.checked_add(1));
            payload_length = payload_length.and_then(|length| {
                length
                    .checked_add(VECTOR_SPACE_FIXED_LENGTH_U64)
                    .and_then(|length| length.checked_add(name_length))
            });
        })?;
        index.for_each_vector(|space, key, vector| {
            let Ok(name_length) = u64::try_from(space.as_str().len()) else {
                valid = false;
                return;
            };
            let Ok(key_length) = u64::try_from(key.len()) else {
                valid = false;
                return;
            };
            let Ok(vector_bytes) = u64::try_from(vector.as_slice().len().saturating_mul(2)) else {
                valid = false;
                return;
            };
            vector_count = vector_count.and_then(|count| count.checked_add(1));
            payload_length = payload_length.and_then(|length| {
                length
                    .checked_add(VECTOR_FIXED_LENGTH_U64)
                    .and_then(|length| length.checked_add(name_length))
                    .and_then(|length| length.checked_add(key_length))
                    .and_then(|length| length.checked_add(vector_bytes))
            });
        })?;
        index.for_each_lexical_index(|definition| {
            let Ok(encoded) = encode_lexical_index(definition) else {
                valid = false;
                return;
            };
            let Ok(record_bytes) = u64::try_from(encoded.len()) else {
                valid = false;
                return;
            };
            lexical_index_count = lexical_index_count.and_then(|count| count.checked_add(1));
            payload_length = payload_length.and_then(|length| length.checked_add(record_bytes));
        })?;
    }
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
    let Some(vector_space_count) = vector_space_count else {
        return Err(SnapshotError::Invalid {
            reason: "vector-space count overflow",
        });
    };
    let Some(vector_count) = vector_count else {
        return Err(SnapshotError::Invalid {
            reason: "vector count overflow",
        });
    };
    let Some(lexical_index_count) = lexical_index_count else {
        return Err(SnapshotError::Invalid {
            reason: "lexical-index count overflow",
        });
    };
    Ok(Measurements {
        entry_count,
        vector_space_count,
        vector_count,
        lexical_index_count,
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

fn encode_vector_space(definition: &VectorSpaceDefinition) -> Result<Vec<u8>, SnapshotError> {
    let name = definition.name.as_str().as_bytes();
    let name_length = u8::try_from(name.len()).map_err(|_| SnapshotError::Invalid {
        reason: "vector-space name length overflow",
    })?;
    let mut encoded = Vec::with_capacity(name.len() + 5);
    encoded.push(name_length);
    encoded.extend_from_slice(name);
    encoded.extend_from_slice(&definition.dimension.to_le_bytes());
    encoded.push(definition.metric as u8);
    encoded.push(1);
    Ok(encoded)
}

fn decode_vector_space(encoded: &[u8]) -> Result<VectorSpaceDefinition, SnapshotError> {
    let name_length = usize::from(*encoded.first().ok_or(SnapshotError::Invalid {
        reason: "truncated vector-space record",
    })?);
    let expected_length = name_length.checked_add(5).ok_or(SnapshotError::Invalid {
        reason: "vector-space record length overflow",
    })?;
    if encoded.len() != expected_length || name_length == 0 {
        return Err(SnapshotError::Invalid {
            reason: "invalid vector-space record length",
        });
    }
    let name =
        std::str::from_utf8(&encoded[1..=name_length]).map_err(|_| SnapshotError::Invalid {
            reason: "invalid vector-space name",
        })?;
    let name = VectorSpaceName::new(name.to_owned()).map_err(|_| SnapshotError::Invalid {
        reason: "invalid vector-space name",
    })?;
    let dimension = u16::from_le_bytes(copy_array(&encoded[1 + name_length..3 + name_length]));
    if encoded[3 + name_length] != VectorMetric::Cosine as u8 || encoded[4 + name_length] != 1 {
        return Err(SnapshotError::Invalid {
            reason: "unsupported vector-space tags",
        });
    }
    VectorSpaceDefinition::cosine(name, dimension).map_err(|_| SnapshotError::Invalid {
        reason: "invalid vector-space dimension",
    })
}

fn encode_vector(
    space: &VectorSpaceName,
    key: &[u8],
    vector: &Q15Vector,
) -> Result<Vec<u8>, SnapshotError> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(SnapshotError::Invalid {
            reason: "invalid vector object key",
        });
    }
    let space_name = space.as_str().as_bytes();
    let space_length = u8::try_from(space_name.len()).map_err(|_| SnapshotError::Invalid {
        reason: "vector-space name length overflow",
    })?;
    let key_length = u32::try_from(key.len()).map_err(|_| SnapshotError::Invalid {
        reason: "vector key length overflow",
    })?;
    let mut encoded =
        Vec::with_capacity(space_name.len() + key.len() + vector.as_slice().len() * 2 + 7);
    encoded.push(space_length);
    encoded.extend_from_slice(space_name);
    encoded.extend_from_slice(&key_length.to_le_bytes());
    encoded.extend_from_slice(key);
    encoded.extend_from_slice(&vector.dimension().to_le_bytes());
    for value in vector.as_slice() {
        encoded.extend_from_slice(&value.to_le_bytes());
    }
    Ok(encoded)
}

fn decode_vector(encoded: &[u8]) -> Result<(VectorSpaceName, Vec<u8>, Q15Vector), SnapshotError> {
    let space_length = usize::from(*encoded.first().ok_or(SnapshotError::Invalid {
        reason: "truncated vector record",
    })?);
    let key_length_offset = 1_usize
        .checked_add(space_length)
        .ok_or(SnapshotError::Invalid {
            reason: "vector record length overflow",
        })?;
    let key_length_end = key_length_offset
        .checked_add(4)
        .ok_or(SnapshotError::Invalid {
            reason: "vector record length overflow",
        })?;
    let key_length = usize::try_from(u32::from_le_bytes(copy_array(
        encoded
            .get(key_length_offset..key_length_end)
            .ok_or(SnapshotError::Invalid {
                reason: "truncated vector record",
            })?,
    )))
    .map_err(|_| SnapshotError::Invalid {
        reason: "vector key length overflow",
    })?;
    if space_length == 0 || key_length == 0 || key_length > MAX_KEY_BYTES {
        return Err(SnapshotError::Invalid {
            reason: "invalid vector identity",
        });
    }
    let key_end = key_length_end
        .checked_add(key_length)
        .ok_or(SnapshotError::Invalid {
            reason: "vector record length overflow",
        })?;
    let dimension_end = key_end.checked_add(2).ok_or(SnapshotError::Invalid {
        reason: "vector record length overflow",
    })?;
    let dimension = usize::from(u16::from_le_bytes(copy_array(
        encoded
            .get(key_end..dimension_end)
            .ok_or(SnapshotError::Invalid {
                reason: "truncated vector record",
            })?,
    )));
    let expected_length = dimension
        .checked_mul(2)
        .and_then(|length| length.checked_add(dimension_end))
        .ok_or(SnapshotError::Invalid {
            reason: "vector record length overflow",
        })?;
    if encoded.len() != expected_length {
        return Err(SnapshotError::Invalid {
            reason: "invalid vector record length",
        });
    }
    let space = std::str::from_utf8(&encoded[1..key_length_offset]).map_err(|_| {
        SnapshotError::Invalid {
            reason: "invalid vector-space name",
        }
    })?;
    let space = VectorSpaceName::new(space.to_owned()).map_err(|_| SnapshotError::Invalid {
        reason: "invalid vector-space name",
    })?;
    let key = encoded[key_length_end..key_end].to_vec();
    let values = encoded[dimension_end..]
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes(copy_array(chunk)))
        .collect::<Vec<_>>();
    let vector = Q15Vector::new(values).map_err(|_| SnapshotError::Invalid {
        reason: "invalid Q15 vector",
    })?;
    Ok((space, key, vector))
}

fn encode_lexical_index(definition: &LexicalIndexDefinition) -> Result<Vec<u8>, SnapshotError> {
    let name = definition.name.as_str().as_bytes();
    let name_length = u8::try_from(name.len()).map_err(|_| SnapshotError::Invalid {
        reason: "lexical-index name length overflow",
    })?;
    let field_count =
        u8::try_from(definition.fields.len()).map_err(|_| SnapshotError::Invalid {
            reason: "lexical-index field count overflow",
        })?;
    let mut encoded = Vec::new();
    encoded.push(name_length);
    encoded.extend_from_slice(name);
    encoded.push(1);
    encoded.push(field_count);
    for field in &definition.fields {
        let segment_count =
            u8::try_from(field.path.segments().len()).map_err(|_| SnapshotError::Invalid {
                reason: "lexical-index segment count overflow",
            })?;
        encoded.push(segment_count);
        for segment in field.path.segments() {
            let segment_length =
                u16::try_from(segment.len()).map_err(|_| SnapshotError::Invalid {
                    reason: "lexical-index segment length overflow",
                })?;
            encoded.extend_from_slice(&segment_length.to_le_bytes());
            encoded.extend_from_slice(segment.as_bytes());
        }
        encoded.extend_from_slice(&field.weight_micros.to_le_bytes());
    }
    Ok(encoded)
}

#[allow(clippy::too_many_lines)]
fn decode_lexical_index(encoded: &[u8]) -> Result<LexicalIndexDefinition, SnapshotError> {
    let name_length = usize::from(*encoded.first().ok_or(SnapshotError::Invalid {
        reason: "truncated lexical-index record",
    })?);
    let name_end = 1_usize
        .checked_add(name_length)
        .ok_or(SnapshotError::Invalid {
            reason: "lexical-index record length overflow",
        })?;
    if name_length == 0 || encoded.get(name_end) != Some(&1) {
        return Err(SnapshotError::Invalid {
            reason: "invalid lexical-index record prefix",
        });
    }
    let name = std::str::from_utf8(encoded.get(1..name_end).ok_or(SnapshotError::Invalid {
        reason: "truncated lexical-index name",
    })?)
    .map_err(|_| SnapshotError::Invalid {
        reason: "invalid lexical-index name",
    })?;
    let name = VectorSpaceName::new(name.to_owned()).map_err(|_| SnapshotError::Invalid {
        reason: "invalid lexical-index name",
    })?;
    let mut cursor = name_end.checked_add(1).ok_or(SnapshotError::Invalid {
        reason: "lexical-index record length overflow",
    })?;
    let field_count = usize::from(*encoded.get(cursor).ok_or(SnapshotError::Invalid {
        reason: "truncated lexical-index field count",
    })?);
    cursor = cursor.checked_add(1).ok_or(SnapshotError::Invalid {
        reason: "lexical-index record length overflow",
    })?;
    if field_count == 0 || field_count > MAX_LEXICAL_FIELDS {
        return Err(SnapshotError::Invalid {
            reason: "invalid lexical-index field count",
        });
    }
    let mut fields = Vec::with_capacity(field_count);
    for _ in 0..field_count {
        let segment_count = usize::from(*encoded.get(cursor).ok_or(SnapshotError::Invalid {
            reason: "truncated lexical-index path",
        })?);
        cursor = cursor.checked_add(1).ok_or(SnapshotError::Invalid {
            reason: "lexical-index record length overflow",
        })?;
        if segment_count == 0 || segment_count > MAX_LEXICAL_PATH_SEGMENTS {
            return Err(SnapshotError::Invalid {
                reason: "invalid lexical-index path",
            });
        }
        let mut segments = Vec::with_capacity(segment_count);
        for _ in 0..segment_count {
            let length_end = cursor.checked_add(2).ok_or(SnapshotError::Invalid {
                reason: "lexical-index record length overflow",
            })?;
            let length = usize::from(u16::from_le_bytes(copy_array(
                encoded
                    .get(cursor..length_end)
                    .ok_or(SnapshotError::Invalid {
                        reason: "truncated lexical-index segment length",
                    })?,
            )));
            cursor = length_end;
            if length == 0 || length > MAX_LEXICAL_PATH_SEGMENT_BYTES {
                return Err(SnapshotError::Invalid {
                    reason: "invalid lexical-index segment length",
                });
            }
            let segment_end = cursor.checked_add(length).ok_or(SnapshotError::Invalid {
                reason: "lexical-index record length overflow",
            })?;
            let segment = std::str::from_utf8(encoded.get(cursor..segment_end).ok_or(
                SnapshotError::Invalid {
                    reason: "truncated lexical-index segment",
                },
            )?)
            .map_err(|_| SnapshotError::Invalid {
                reason: "invalid lexical-index segment",
            })?
            .to_owned();
            cursor = segment_end;
            segments.push(segment);
        }
        let weight_end = cursor.checked_add(4).ok_or(SnapshotError::Invalid {
            reason: "lexical-index record length overflow",
        })?;
        let weight_micros = u32::from_le_bytes(copy_array(encoded.get(cursor..weight_end).ok_or(
            SnapshotError::Invalid {
                reason: "truncated lexical-index field weight",
            },
        )?));
        cursor = weight_end;
        fields.push(LexicalField {
            path: FieldPath::new(segments),
            weight_micros,
        });
    }
    if cursor != encoded.len() {
        return Err(SnapshotError::Invalid {
            reason: "invalid lexical-index record length",
        });
    }
    LexicalIndexDefinition::new(name, fields).map_err(|_| SnapshotError::Invalid {
        reason: "invalid lexical-index definition",
    })
}

fn write_encoded(
    writer: &mut impl Write,
    hasher: &mut blake3::Hasher,
    encoded: Result<Vec<u8>, SnapshotError>,
) -> Result<(), SnapshotError> {
    let encoded = encoded?;
    writer.write_all(&encoded)?;
    hasher.update(&encoded);
    Ok(())
}

fn read_v2_counts(
    reader: &mut impl Read,
    decoded: &DecodedHeader,
    consumed: &mut u64,
) -> Result<(u64, u64, u64), SnapshotError> {
    if decoded.disk_format_version < 2 {
        return Ok((0, 0, 0));
    }
    let mut encoded = [0_u8; V2_COUNTS_LENGTH];
    read_payload_exact(reader, &mut encoded, consumed, decoded.payload_length)?;
    Ok((
        u64::from_le_bytes(copy_array(&encoded[..8])),
        u64::from_le_bytes(copy_array(&encoded[8..16])),
        u64::from_le_bytes(copy_array(&encoded[16..24])),
    ))
}

fn read_encoded_vector_space(
    reader: &mut impl Read,
    decoded: &DecodedHeader,
    consumed: &mut u64,
) -> Result<Vec<u8>, SnapshotError> {
    let mut name_length = [0_u8; 1];
    read_payload_exact(reader, &mut name_length, consumed, decoded.payload_length)?;
    let remaining = usize::from(name_length[0])
        .checked_add(4)
        .ok_or(SnapshotError::Invalid {
            reason: "vector-space record length overflow",
        })?;
    let mut encoded = vec![name_length[0]];
    let mut tail = vec![0_u8; remaining];
    read_payload_exact(reader, &mut tail, consumed, decoded.payload_length)?;
    encoded.extend_from_slice(&tail);
    Ok(encoded)
}

fn read_vector_space(
    reader: &mut impl Read,
    decoded: &DecodedHeader,
    consumed: &mut u64,
) -> Result<VectorSpaceDefinition, SnapshotError> {
    decode_vector_space(&read_encoded_vector_space(reader, decoded, consumed)?)
}

fn read_encoded_vector(
    reader: &mut impl Read,
    decoded: &DecodedHeader,
    consumed: &mut u64,
) -> Result<Vec<u8>, SnapshotError> {
    let mut space_length = [0_u8; 1];
    read_payload_exact(reader, &mut space_length, consumed, decoded.payload_length)?;
    let space_length = usize::from(space_length[0]);
    let mut prefix_tail = vec![0_u8; space_length + 4];
    read_payload_exact(reader, &mut prefix_tail, consumed, decoded.payload_length)?;
    let key_length = usize::try_from(u32::from_le_bytes(copy_array(&prefix_tail[space_length..])))
        .map_err(|_| SnapshotError::Invalid {
            reason: "vector key length overflow",
        })?;
    if space_length == 0 || key_length == 0 || key_length > MAX_KEY_BYTES {
        return Err(SnapshotError::Invalid {
            reason: "invalid vector identity",
        });
    }
    let mut key_and_dimension = vec![0_u8; key_length + 2];
    read_payload_exact(
        reader,
        &mut key_and_dimension,
        consumed,
        decoded.payload_length,
    )?;
    let dimension = usize::from(u16::from_le_bytes(copy_array(
        &key_and_dimension[key_length..],
    )));
    let vector_bytes = dimension.checked_mul(2).ok_or(SnapshotError::Invalid {
        reason: "vector record length overflow",
    })?;
    let mut values = vec![0_u8; vector_bytes];
    read_payload_exact(reader, &mut values, consumed, decoded.payload_length)?;
    let mut encoded =
        Vec::with_capacity(1 + prefix_tail.len() + key_and_dimension.len() + values.len());
    encoded.push(
        u8::try_from(space_length).map_err(|_| SnapshotError::Invalid {
            reason: "vector-space name length overflow",
        })?,
    );
    encoded.extend_from_slice(&prefix_tail);
    encoded.extend_from_slice(&key_and_dimension);
    encoded.extend_from_slice(&values);
    Ok(encoded)
}

fn read_vector(
    reader: &mut impl Read,
    decoded: &DecodedHeader,
    consumed: &mut u64,
) -> Result<(VectorSpaceName, Vec<u8>, Q15Vector), SnapshotError> {
    decode_vector(&read_encoded_vector(reader, decoded, consumed)?)
}

fn read_encoded_lexical_index(
    reader: &mut impl Read,
    decoded: &DecodedHeader,
    consumed: &mut u64,
) -> Result<Vec<u8>, SnapshotError> {
    let mut name_length = [0_u8; 1];
    read_payload_exact(reader, &mut name_length, consumed, decoded.payload_length)?;
    let name_length_usize = usize::from(name_length[0]);
    if name_length_usize == 0 {
        return Err(SnapshotError::Invalid {
            reason: "invalid lexical-index name length",
        });
    }
    let mut name_and_counts = vec![0_u8; name_length_usize + 2];
    read_payload_exact(
        reader,
        &mut name_and_counts,
        consumed,
        decoded.payload_length,
    )?;
    if name_and_counts[name_length_usize] != 1 {
        return Err(SnapshotError::Invalid {
            reason: "unsupported lexical-index record version",
        });
    }
    let field_count = usize::from(name_and_counts[name_length_usize + 1]);
    if field_count == 0 || field_count > MAX_LEXICAL_FIELDS {
        return Err(SnapshotError::Invalid {
            reason: "invalid lexical-index field count",
        });
    }
    let mut encoded = Vec::new();
    encoded.push(name_length[0]);
    encoded.extend_from_slice(&name_and_counts);
    for _ in 0..field_count {
        let mut segment_count = [0_u8; 1];
        read_payload_exact(reader, &mut segment_count, consumed, decoded.payload_length)?;
        let segment_count_usize = usize::from(segment_count[0]);
        if segment_count_usize == 0 || segment_count_usize > MAX_LEXICAL_PATH_SEGMENTS {
            return Err(SnapshotError::Invalid {
                reason: "invalid lexical-index path",
            });
        }
        encoded.push(segment_count[0]);
        for _ in 0..segment_count_usize {
            let mut length = [0_u8; 2];
            read_payload_exact(reader, &mut length, consumed, decoded.payload_length)?;
            let length_usize = usize::from(u16::from_le_bytes(length));
            if length_usize == 0 || length_usize > MAX_LEXICAL_PATH_SEGMENT_BYTES {
                return Err(SnapshotError::Invalid {
                    reason: "invalid lexical-index segment length",
                });
            }
            let mut segment = vec![0_u8; length_usize];
            read_payload_exact(reader, &mut segment, consumed, decoded.payload_length)?;
            encoded.extend_from_slice(&length);
            encoded.extend_from_slice(&segment);
        }
        let mut weight = [0_u8; 4];
        read_payload_exact(reader, &mut weight, consumed, decoded.payload_length)?;
        encoded.extend_from_slice(&weight);
    }
    Ok(encoded)
}

fn read_lexical_index(
    reader: &mut impl Read,
    decoded: &DecodedHeader,
    consumed: &mut u64,
) -> Result<LexicalIndexDefinition, SnapshotError> {
    decode_lexical_index(&read_encoded_lexical_index(reader, decoded, consumed)?)
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
