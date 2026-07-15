// SPDX-License-Identifier: Apache-2.0

mod frame;

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Seek, SeekFrom, Write},
    marker::PhantomData,
    path::Path,
};

use thiserror::Error;
use uuid::Uuid;

use self::frame::{
    Frame, FrameKind, HEADER_LENGTH, MAX_PAYLOAD_LENGTH, ReadStatus, payload_length,
    read_exact_or_tail,
};

const DESCRIPTOR_LENGTH: usize = 36;
const TRANSACTION_DOMAIN: &[u8] = b"hyphae-transaction-v1";
pub(crate) const MAX_OPERATION_BYTES: usize = MAX_PAYLOAD_LENGTH;

/// Failure while opening, validating, or appending to a durable log.
#[derive(Debug, Error)]
pub enum LogError {
    /// A filesystem operation failed.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// A frame does not start with the Hyphae magic bytes.
    #[error("invalid frame magic at byte offset {offset}")]
    BadMagic {
        /// Frame offset.
        offset: u64,
    },

    /// A frame uses a disk version this binary cannot decode.
    #[error(
        "unsupported log version {found} at byte offset {offset}; supported version is {supported}"
    )]
    UnsupportedVersion {
        /// Frame offset.
        offset: u64,
        /// Version found on disk.
        found: u16,
        /// Version understood by this binary.
        supported: u16,
    },

    /// A frame kind is not part of this format version.
    #[error("unknown frame kind {kind} at byte offset {offset}")]
    UnknownFrameKind {
        /// Frame offset.
        offset: u64,
        /// Raw kind byte.
        kind: u8,
    },

    /// Reserved frame flags are nonzero.
    #[error("unsupported frame flags {flags:#04x} at byte offset {offset}")]
    UnsupportedFlags {
        /// Frame offset.
        offset: u64,
        /// Raw flag byte.
        flags: u8,
    },

    /// A payload exceeds the per-frame allocation limit.
    #[error("frame payload is {length} bytes; maximum is {maximum}")]
    PayloadTooLarge {
        /// Requested or decoded length.
        length: usize,
        /// Configured maximum.
        maximum: usize,
    },

    /// A frame sequence is not exactly the previous sequence plus one.
    #[error("invalid sequence at byte offset {offset}: expected {expected}, found {found}")]
    InvalidSequence {
        /// Frame offset.
        offset: u64,
        /// Expected sequence.
        expected: u64,
        /// Sequence found.
        found: u64,
    },

    /// The digest chain does not connect to the prior frame.
    #[error("previous-frame digest mismatch at sequence {sequence}")]
    PreviousDigestMismatch {
        /// Invalid frame sequence.
        sequence: u64,
    },

    /// The CRC32C integrity check failed.
    #[error("CRC32C mismatch at sequence {sequence}")]
    ChecksumMismatch {
        /// Invalid frame sequence.
        sequence: u64,
    },

    /// The BLAKE3 frame digest failed.
    #[error("BLAKE3 digest mismatch at sequence {sequence}")]
    DigestMismatch {
        /// Invalid frame sequence.
        sequence: u64,
    },

    /// A transaction descriptor has the wrong length or contents.
    #[error("malformed transaction descriptor at sequence {sequence}")]
    MalformedTransaction {
        /// Invalid frame sequence.
        sequence: u64,
    },

    /// An operation or commit appeared without its matching begin frame.
    #[error("{kind} frame at sequence {sequence} has no matching transaction begin")]
    TransactionBoundary {
        /// Frame kind being validated.
        kind: &'static str,
        /// Invalid frame sequence.
        sequence: u64,
    },

    /// The committed operation count or digest differs from its descriptor.
    #[error("transaction content mismatch at commit sequence {sequence}")]
    TransactionContentMismatch {
        /// Invalid commit sequence.
        sequence: u64,
    },

    /// A transaction identifier was reused for different contents.
    #[error(
        "transaction identifier {transaction_id} was already committed with different contents"
    )]
    IdempotencyConflict {
        /// Reused identifier.
        transaction_id: Uuid,
    },

    /// A transaction cannot be committed without at least one operation.
    #[error("a transaction must contain at least one operation")]
    EmptyTransaction,

    /// The operation count cannot be represented by the disk format.
    #[error("transaction has too many operations")]
    TooManyOperations,

    /// The sequence space has been exhausted.
    #[error("log sequence space is exhausted")]
    SequenceExhausted,

    /// A segment base sequence and digest do not form a canonical anchor.
    #[error("invalid log segment anchor")]
    InvalidAnchor,

    /// The writer observed an uncertain I/O result and must be reopened.
    #[error("durable log writer is poisoned; reopen it before writing again")]
    Poisoned,
}

/// Durable identity of a committed transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitReceipt {
    /// Caller-supplied idempotency key.
    pub transaction_id: Uuid,
    /// Sequence of the durable commit frame.
    pub commit_sequence: u64,
    /// Digest of the commit frame and its chain prefix.
    pub commit_digest: [u8; 32],
    /// Digest of the canonical operation list.
    pub transaction_digest: [u8; 32],
}

/// Result of an idempotent append request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendOutcome {
    /// New frames were written and synchronized.
    Committed(CommitReceipt),
    /// The exact transaction was already durable; no frames were appended.
    Existing(CommitReceipt),
}

/// A committed transaction reconstructed from the verified log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredTransaction {
    /// Durable commit identity.
    pub receipt: CommitReceipt,
    /// Opaque operation payloads in original order.
    pub operations: Vec<Vec<u8>>,
}

/// Evidence produced while opening and validating a segment.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryReport {
    /// Sequence immediately preceding this segment, or zero for the first segment.
    pub base_sequence: u64,
    /// Digest immediately preceding this segment, or all zero for the first segment.
    pub base_digest: [u8; 32],
    /// Unique committed transactions in commit order.
    pub transactions: Vec<RecoveredTransaction>,
    /// Complete but uncommitted transaction attempts ignored during recovery.
    pub ignored_uncommitted_transactions: u64,
    /// Repeated commits with the same id and content, deduplicated during replay.
    pub duplicate_commits: u64,
    /// Incomplete bytes removed from the physical tail.
    pub truncated_tail_bytes: u64,
    /// Length after any incomplete tail was removed.
    pub valid_bytes: u64,
    /// Last complete frame sequence, including uncommitted attempts.
    pub last_sequence: u64,
    /// Digest of the last complete frame.
    pub last_digest: [u8; 32],
}

/// A newly opened writer together with its recovery evidence.
#[derive(Debug)]
pub struct OpenedLog<'directory> {
    /// Exclusive writer handle.
    pub log: DurableLog,
    /// Verified replay and tail-repair report.
    pub recovery: RecoveryReport,
    directory_lock: PhantomData<&'directory crate::DataDirectory>,
}

impl OpenedLog<'_> {
    pub(crate) fn new(log: DurableLog, recovery: RecoveryReport) -> Self {
        Self {
            log,
            recovery,
            directory_lock: PhantomData,
        }
    }
}

/// Append-only transaction log with synchronous commit durability.
#[derive(Debug)]
pub struct DurableLog {
    file: File,
    next_sequence: u64,
    previous_digest: [u8; 32],
    committed: HashMap<Uuid, CommitReceipt>,
    poisoned: bool,
    #[cfg(test)]
    fail_next_sync: bool,
}

impl DurableLog {
    /// Opens, verifies, and repairs only an incomplete physical tail.
    ///
    /// Full frames with invalid checksums, digests, versions, sequences, or
    /// transaction boundaries are rejected as corruption and never truncated.
    ///
    /// # Errors
    ///
    /// Returns an error for I/O failures or any complete invalid frame.
    #[cfg(test)]
    pub(crate) fn open_file(
        path: impl AsRef<Path>,
    ) -> Result<(DurableLog, RecoveryReport), LogError> {
        Self::open_file_at(path, 0, [0; 32])
    }

    pub(crate) fn open_file_at(
        path: impl AsRef<Path>,
        base_sequence: u64,
        base_digest: [u8; 32],
    ) -> Result<(DurableLog, RecoveryReport), LogError> {
        if (base_sequence == 0) != (base_digest == [0; 32]) {
            return Err(LogError::InvalidAnchor);
        }
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existed = path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        if !existed {
            file.sync_all()?;
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                File::open(parent)?.sync_all()?;
            }
        }
        let recovery = scan(&mut file, base_sequence, base_digest)?;
        let physical_length = file.metadata()?.len();
        if physical_length != recovery.valid_bytes {
            file.set_len(recovery.valid_bytes)?;
            file.sync_data()?;
        }
        file.seek(SeekFrom::End(0))?;

        let committed = recovery
            .transactions
            .iter()
            .map(|transaction| (transaction.receipt.transaction_id, transaction.receipt))
            .collect();
        let next_sequence = recovery
            .last_sequence
            .checked_add(1)
            .ok_or(LogError::SequenceExhausted)?;
        let log = Self {
            file,
            next_sequence,
            previous_digest: recovery.last_digest,
            committed,
            poisoned: false,
            #[cfg(test)]
            fail_next_sync: false,
        };
        Ok((log, recovery))
    }

    /// Appends and synchronizes one atomic transaction.
    ///
    /// Retrying the same identifier and operation bytes returns the original
    /// receipt without appending. Reusing it with different bytes fails.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid bounds, conflicting idempotency keys, or
    /// filesystem failures. Any append/sync failure poisons the writer so the
    /// caller must reopen and recover before another write.
    pub fn append_transaction(
        &mut self,
        transaction_id: Uuid,
        operations: &[Vec<u8>],
    ) -> Result<AppendOutcome, LogError> {
        if self.poisoned {
            return Err(LogError::Poisoned);
        }
        if operations.is_empty() {
            return Err(LogError::EmptyTransaction);
        }
        let operation_count =
            u32::try_from(operations.len()).map_err(|_| LogError::TooManyOperations)?;
        for operation in operations {
            if operation.len() > MAX_PAYLOAD_LENGTH {
                return Err(LogError::PayloadTooLarge {
                    length: operation.len(),
                    maximum: MAX_PAYLOAD_LENGTH,
                });
            }
        }

        let transaction_digest = transaction_digest(operations, operation_count)?;
        if let Some(receipt) = self.committed.get(&transaction_id).copied() {
            return if receipt.transaction_digest == transaction_digest {
                Ok(AppendOutcome::Existing(receipt))
            } else {
                Err(LogError::IdempotencyConflict { transaction_id })
            };
        }

        let descriptor = encode_descriptor(operation_count, transaction_digest);
        let append_result = self.append_new_transaction(
            transaction_id,
            operations,
            &descriptor,
            transaction_digest,
        );
        if append_result.is_err() {
            self.poisoned = true;
        }
        append_result
    }

    pub(crate) fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    #[cfg(test)]
    pub(crate) fn inject_sync_failure(&mut self) {
        self.fail_next_sync = true;
    }

    fn append_new_transaction(
        &mut self,
        transaction_id: Uuid,
        operations: &[Vec<u8>],
        descriptor: &[u8; DESCRIPTOR_LENGTH],
        transaction_digest: [u8; 32],
    ) -> Result<AppendOutcome, LogError> {
        self.append_frame(FrameKind::Begin, transaction_id, descriptor)?;
        for operation in operations {
            self.append_frame(FrameKind::Operation, transaction_id, operation)?;
        }
        let receipt = self.append_frame(FrameKind::Commit, transaction_id, descriptor)?;
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_sync) {
            return Err(io::Error::other("injected log sync failure").into());
        }
        self.file.sync_data()?;

        let receipt = CommitReceipt {
            transaction_id,
            commit_sequence: receipt.sequence,
            commit_digest: receipt.digest,
            transaction_digest,
        };
        self.committed.insert(transaction_id, receipt);
        Ok(AppendOutcome::Committed(receipt))
    }

    fn append_frame(
        &mut self,
        kind: FrameKind,
        transaction_id: Uuid,
        payload: &[u8],
    ) -> Result<WrittenFrame, LogError> {
        let frame = Frame {
            kind,
            sequence: self.next_sequence,
            transaction_id,
            previous_digest: self.previous_digest,
            digest: [0; 32],
            payload: payload.to_vec(),
        };
        let encoded = frame.encode()?;
        let digest = copy_array(&encoded[80..112]);
        self.file.write_all(&encoded)?;
        let written = WrittenFrame {
            sequence: self.next_sequence,
            digest,
        };
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(LogError::SequenceExhausted)?;
        self.previous_digest = digest;
        Ok(written)
    }
}

#[derive(Clone, Copy, Debug)]
struct WrittenFrame {
    sequence: u64,
    digest: [u8; 32],
}

#[derive(Debug)]
struct PendingTransaction {
    transaction_id: Uuid,
    operation_count: u32,
    expected_digest: [u8; 32],
    operations: Vec<Vec<u8>>,
}

fn scan(
    file: &mut File,
    base_sequence: u64,
    base_digest: [u8; 32],
) -> Result<RecoveryReport, LogError> {
    file.seek(SeekFrom::Start(0))?;
    let physical_length = file.metadata()?.len();
    let mut report = RecoveryReport {
        base_sequence,
        base_digest,
        last_sequence: base_sequence,
        last_digest: base_digest,
        ..RecoveryReport::default()
    };
    let mut offset = 0_u64;
    let mut expected_sequence = base_sequence
        .checked_add(1)
        .ok_or(LogError::SequenceExhausted)?;
    let mut expected_previous_digest = base_digest;
    let mut pending: Option<PendingTransaction> = None;
    let mut committed: HashMap<Uuid, CommitReceipt> = HashMap::new();

    loop {
        let mut header = [0_u8; HEADER_LENGTH];
        match read_exact_or_tail(file, &mut header)? {
            ReadStatus::End => break,
            ReadStatus::Partial => {
                report.truncated_tail_bytes = physical_length.saturating_sub(offset);
                break;
            }
            ReadStatus::Complete => {}
        }
        let length = payload_length(&header, offset)?;
        let mut payload = vec![0_u8; length];
        if read_exact_or_tail(file, &mut payload)? != ReadStatus::Complete {
            report.truncated_tail_bytes = physical_length.saturating_sub(offset);
            break;
        }

        let frame = Frame::decode(&header, payload, offset)?;
        if frame.sequence != expected_sequence {
            return Err(LogError::InvalidSequence {
                offset,
                expected: expected_sequence,
                found: frame.sequence,
            });
        }
        if frame.previous_digest != expected_previous_digest {
            return Err(LogError::PreviousDigestMismatch {
                sequence: frame.sequence,
            });
        }

        apply_frame(&frame, &mut pending, &mut committed, &mut report)?;
        let frame_length = u64::try_from(HEADER_LENGTH + frame.payload.len()).map_err(|_| {
            LogError::PayloadTooLarge {
                length: frame.payload.len(),
                maximum: MAX_PAYLOAD_LENGTH,
            }
        })?;
        offset = offset
            .checked_add(frame_length)
            .ok_or(LogError::SequenceExhausted)?;
        report.valid_bytes = offset;
        report.last_sequence = frame.sequence;
        report.last_digest = frame.digest;
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or(LogError::SequenceExhausted)?;
        expected_previous_digest = frame.digest;
    }

    if pending.is_some() {
        report.ignored_uncommitted_transactions =
            report.ignored_uncommitted_transactions.saturating_add(1);
    }
    Ok(report)
}

fn apply_frame(
    frame: &Frame,
    pending: &mut Option<PendingTransaction>,
    committed: &mut HashMap<Uuid, CommitReceipt>,
    report: &mut RecoveryReport,
) -> Result<(), LogError> {
    match frame.kind {
        FrameKind::Begin => {
            if pending.is_some() {
                report.ignored_uncommitted_transactions =
                    report.ignored_uncommitted_transactions.saturating_add(1);
            }
            let (operation_count, expected_digest) =
                decode_descriptor(&frame.payload, frame.sequence)?;
            *pending = Some(PendingTransaction {
                transaction_id: frame.transaction_id,
                operation_count,
                expected_digest,
                operations: Vec::new(),
            });
            Ok(())
        }
        FrameKind::Operation => {
            let Some(current) = pending
                .as_mut()
                .filter(|current| current.transaction_id == frame.transaction_id)
            else {
                return Err(LogError::TransactionBoundary {
                    kind: "operation",
                    sequence: frame.sequence,
                });
            };
            current.operations.push(frame.payload.clone());
            Ok(())
        }
        FrameKind::Commit => {
            let Some(current) = pending
                .take()
                .filter(|current| current.transaction_id == frame.transaction_id)
            else {
                return Err(LogError::TransactionBoundary {
                    kind: "commit",
                    sequence: frame.sequence,
                });
            };
            let (operation_count, commit_digest) =
                decode_descriptor(&frame.payload, frame.sequence)?;
            let actual_count =
                u32::try_from(current.operations.len()).map_err(|_| LogError::TooManyOperations)?;
            let actual_digest = transaction_digest(&current.operations, actual_count)?;
            if operation_count != current.operation_count
                || commit_digest != current.expected_digest
                || actual_count != operation_count
                || actual_digest != commit_digest
            {
                return Err(LogError::TransactionContentMismatch {
                    sequence: frame.sequence,
                });
            }

            let receipt = CommitReceipt {
                transaction_id: frame.transaction_id,
                commit_sequence: frame.sequence,
                commit_digest: frame.digest,
                transaction_digest: actual_digest,
            };
            if let Some(existing) = committed.get(&frame.transaction_id) {
                if existing.transaction_digest != actual_digest {
                    return Err(LogError::IdempotencyConflict {
                        transaction_id: frame.transaction_id,
                    });
                }
                report.duplicate_commits = report.duplicate_commits.saturating_add(1);
            } else {
                committed.insert(frame.transaction_id, receipt);
                report.transactions.push(RecoveredTransaction {
                    receipt,
                    operations: current.operations,
                });
            }
            Ok(())
        }
    }
}

fn encode_descriptor(operation_count: u32, digest: [u8; 32]) -> [u8; DESCRIPTOR_LENGTH] {
    let mut descriptor = [0_u8; DESCRIPTOR_LENGTH];
    descriptor[..4].copy_from_slice(&operation_count.to_le_bytes());
    descriptor[4..].copy_from_slice(&digest);
    descriptor
}

fn decode_descriptor(payload: &[u8], sequence: u64) -> Result<(u32, [u8; 32]), LogError> {
    if payload.len() != DESCRIPTOR_LENGTH {
        return Err(LogError::MalformedTransaction { sequence });
    }
    let operation_count = u32::from_le_bytes(copy_array(&payload[..4]));
    if operation_count == 0 {
        return Err(LogError::MalformedTransaction { sequence });
    }
    let digest = copy_array(&payload[4..]);
    Ok((operation_count, digest))
}

pub(crate) fn transaction_digest(
    operations: &[Vec<u8>],
    operation_count: u32,
) -> Result<[u8; 32], LogError> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(TRANSACTION_DOMAIN);
    hasher.update(&u64::from(operation_count).to_le_bytes());
    for operation in operations {
        let length = u64::try_from(operation.len()).map_err(|_| LogError::PayloadTooLarge {
            length: operation.len(),
            maximum: MAX_PAYLOAD_LENGTH,
        })?;
        hasher.update(&length.to_le_bytes());
        hasher.update(operation);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn copy_array<const N: usize>(source: &[u8]) -> [u8; N] {
    let mut output = [0_u8; N];
    output.copy_from_slice(source);
    output
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs::OpenOptions,
        io::{Read, Seek, SeekFrom, Write},
    };

    use uuid::Uuid;

    use super::{AppendOutcome, DurableLog, LogError, OpenedLog, frame::HEADER_LENGTH};
    use crate::test_support::TestDirectory;

    fn open_for_test(path: &std::path::Path) -> Result<OpenedLog<'static>, LogError> {
        let (log, recovery) = DurableLog::open_file(path)?;
        Ok(OpenedLog::new(log, recovery))
    }

    #[test]
    fn committed_transaction_recovers_in_order() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-recovery")?;
        let path = temporary.path().join("segment.hylog");
        let transaction_id = Uuid::now_v7();
        let mut opened = open_for_test(&path)?;
        let outcome = opened
            .log
            .append_transaction(transaction_id, &[b"put:a=1".to_vec(), b"put:b=2".to_vec()])?;
        assert!(matches!(outcome, AppendOutcome::Committed(_)));
        drop(opened);

        let reopened = open_for_test(&path)?;
        assert_eq!(reopened.recovery.transactions.len(), 1);
        assert_eq!(
            reopened.recovery.transactions[0].operations,
            [b"put:a=1".to_vec(), b"put:b=2".to_vec()]
        );
        Ok(())
    }

    #[test]
    fn idempotency_survives_reopen() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-idempotency")?;
        let path = temporary.path().join("segment.hylog");
        let transaction_id = Uuid::now_v7();
        let operations = [b"same".to_vec()];
        let mut opened = open_for_test(&path)?;
        let first = opened.log.append_transaction(transaction_id, &operations)?;
        drop(opened);

        let mut reopened = open_for_test(&path)?;
        let second = reopened
            .log
            .append_transaction(transaction_id, &operations)?;
        assert!(matches!(first, AppendOutcome::Committed(_)));
        assert!(matches!(second, AppendOutcome::Existing(_)));

        let conflict = reopened
            .log
            .append_transaction(transaction_id, &[b"different".to_vec()]);
        assert!(matches!(
            conflict,
            Err(LogError::IdempotencyConflict { .. })
        ));
        Ok(())
    }

    #[test]
    fn truncates_only_an_incomplete_tail() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-tail")?;
        let path = temporary.path().join("segment.hylog");
        let mut opened = open_for_test(&path)?;
        opened
            .log
            .append_transaction(Uuid::now_v7(), &[b"durable".to_vec()])?;
        drop(opened);
        let valid_length = std::fs::metadata(&path)?.len();

        OpenOptions::new()
            .append(true)
            .open(&path)?
            .write_all(b"partial")?;
        let reopened = open_for_test(&path)?;
        assert_eq!(reopened.recovery.truncated_tail_bytes, 7);
        assert_eq!(std::fs::metadata(&path)?.len(), valid_length);
        assert_eq!(reopened.recovery.transactions.len(), 1);
        Ok(())
    }

    #[test]
    fn rejects_complete_corruption_without_truncating() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-corruption")?;
        let path = temporary.path().join("segment.hylog");
        let mut opened = open_for_test(&path)?;
        opened
            .log
            .append_transaction(Uuid::now_v7(), &[b"durable".to_vec()])?;
        drop(opened);
        let original_length = std::fs::metadata(&path)?.len();

        let payload_offset = u64::try_from(HEADER_LENGTH * 2)? + 36;
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        file.seek(SeekFrom::Start(payload_offset))?;
        let mut byte = [0_u8; 1];
        file.read_exact(&mut byte)?;
        byte[0] ^= 0x01;
        file.seek(SeekFrom::Start(payload_offset))?;
        file.write_all(&byte)?;
        file.sync_all()?;
        drop(file);

        let result = open_for_test(&path);
        assert!(matches!(result, Err(LogError::ChecksumMismatch { .. })));
        assert_eq!(std::fs::metadata(&path)?.len(), original_length);
        Ok(())
    }

    #[test]
    fn retry_supersedes_an_uncommitted_attempt() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-retry")?;
        let path = temporary.path().join("segment.hylog");
        let transaction_id = Uuid::now_v7();
        let operations = [b"complete".to_vec()];

        let mut opened = open_for_test(&path)?;
        let digest = super::transaction_digest(&operations, 1)?;
        let descriptor = super::encode_descriptor(1, digest);
        opened
            .log
            .append_frame(super::FrameKind::Begin, transaction_id, &descriptor)?;
        opened
            .log
            .append_frame(super::FrameKind::Operation, transaction_id, b"incomplete")?;
        opened.log.file.sync_data()?;
        drop(opened);

        let mut recovered = open_for_test(&path)?;
        assert_eq!(recovered.recovery.ignored_uncommitted_transactions, 1);
        recovered
            .log
            .append_transaction(transaction_id, &operations)?;
        drop(recovered);

        let final_open = open_for_test(&path)?;
        assert_eq!(final_open.recovery.transactions.len(), 1);
        assert_eq!(final_open.recovery.transactions[0].operations, operations);
        Ok(())
    }

    #[test]
    fn every_incomplete_transaction_prefix_is_atomic() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-byte-cuts")?;
        let seed_path = temporary.path().join("seed.hylog");
        let target_path = temporary.path().join("cut.hylog");
        let mut seed = open_for_test(&seed_path)?;
        seed.log
            .append_transaction(Uuid::now_v7(), &[b"first".to_vec(), b"second".to_vec()])?;
        drop(seed);
        let complete = std::fs::read(&seed_path)?;

        for cut in 0..complete.len() {
            std::fs::write(&target_path, &complete[..cut])?;
            let recovered = open_for_test(&target_path)?;
            assert!(
                recovered.recovery.transactions.is_empty(),
                "cut at byte {cut} exposed an uncommitted transaction"
            );
            drop(recovered);
        }

        std::fs::write(&target_path, &complete)?;
        let recovered = open_for_test(&target_path)?;
        assert_eq!(recovered.recovery.transactions.len(), 1);
        Ok(())
    }

    #[test]
    fn future_frame_version_fails_before_payload_allocation() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-future-version")?;
        let path = temporary.path().join("segment.hylog");
        let mut opened = open_for_test(&path)?;
        opened
            .log
            .append_transaction(Uuid::now_v7(), &[b"durable".to_vec()])?;
        drop(opened);

        let mut bytes = std::fs::read(&path)?;
        bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
        bytes[36..44].copy_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&path, &bytes)?;

        let result = open_for_test(&path);
        assert!(matches!(
            result,
            Err(LogError::UnsupportedVersion {
                found: 2,
                supported: 1,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn anchored_segment_continues_the_global_digest_chain() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("log-anchored-segment")?;
        let first_path = temporary.path().join("first.hylog");
        let second_path = temporary.path().join("second.hylog");
        let mut first = open_for_test(&first_path)?;
        first
            .log
            .append_transaction(Uuid::now_v7(), &[b"before-compaction".to_vec()])?;
        drop(first);
        let (_, first_recovery) = DurableLog::open_file(&first_path)?;

        let (mut second, empty_recovery) = DurableLog::open_file_at(
            &second_path,
            first_recovery.last_sequence,
            first_recovery.last_digest,
        )?;
        assert_eq!(empty_recovery.base_sequence, first_recovery.last_sequence);
        assert_eq!(empty_recovery.last_digest, first_recovery.last_digest);
        let outcome = second.append_transaction(Uuid::now_v7(), &[b"after-compaction".to_vec()])?;
        let AppendOutcome::Committed(receipt) = outcome else {
            return Err("new anchored transaction was not committed".into());
        };
        assert_eq!(receipt.commit_sequence, first_recovery.last_sequence + 3);
        drop(second);

        let (_, reopened) = DurableLog::open_file_at(
            &second_path,
            first_recovery.last_sequence,
            first_recovery.last_digest,
        )?;
        assert_eq!(reopened.transactions.len(), 1);

        let wrong_anchor =
            DurableLog::open_file_at(&second_path, first_recovery.last_sequence, [9; 32]);
        assert!(matches!(
            wrong_anchor,
            Err(LogError::PreviousDigestMismatch { .. })
        ));
        Ok(())
    }
}
