// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use thiserror::Error;
use uuid::Uuid;

/// Maximum KV entries returned by one ordered storage scan page.
pub const MAX_SCAN_PAGE_ENTRIES: usize = 4_096;

use crate::log::transaction_digest;
use crate::{
    AppendOutcome, CommitReceipt, DataDirectory, DataDirectoryError, DurableLog, LogError,
    MaterializedIndexError, Mutation, MutationError, RecoveredTransaction, RecoveryReport,
    SnapshotError, SnapshotInfo,
    index::MaterializedIndex,
    manifest::StorageManifest,
    mutation::validate_key,
    snapshot::{create_snapshot, verify_snapshot},
};

/// Failure while opening or operating the durable embedded storage engine.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The data directory could not be initialized or exclusively locked.
    #[error(transparent)]
    DataDirectory(#[from] DataDirectoryError),

    /// The authoritative log rejected or failed an operation.
    #[error(transparent)]
    Log(#[from] LogError),

    /// The rebuildable materialized index failed before a new log commit.
    #[error("materialized index failure: {source}")]
    Index {
        /// Rebuildable-index failure.
        #[source]
        source: Box<MaterializedIndexError>,
    },

    /// A mutation violates the stable binary codec.
    #[error(transparent)]
    Mutation(#[from] MutationError),

    /// The log commit is durable but its index update failed.
    #[error("transaction {receipt:?} is durable but not materialized; reopen to recover")]
    CommittedButNotIndexed {
        /// Receipt proving that the log commit succeeded.
        receipt: CommitReceipt,
        /// Rebuildable-index failure.
        #[source]
        source: Box<MaterializedIndexError>,
    },

    /// Reads and further writes are blocked after an index update failure.
    #[error("materialized index is stale; reopen storage to replay the durable log")]
    StaleIndex,

    /// Snapshot creation or verification failed.
    #[error("snapshot failure: {source}")]
    Snapshot {
        /// Underlying snapshot failure.
        #[source]
        source: Box<SnapshotError>,
    },

    /// The immutable manifest generation space is exhausted.
    #[error("storage manifest generation space is exhausted")]
    ManifestGenerationExhausted,

    /// A prepared compaction segment unexpectedly contains complete frames.
    #[error("prepared compaction segment is not empty: {path}")]
    PreparedSegmentNotEmpty {
        /// Unexpected nonempty segment path.
        path: PathBuf,
    },

    /// A KV scan page size is zero or exceeds the hard storage bound.
    #[error("scan page size {requested} is outside 1..={maximum}")]
    InvalidScanLimit {
        /// Requested page size.
        requested: usize,
        /// Hard maximum page size.
        maximum: usize,
    },
}

/// Recovery evidence returned when the complete embedded storage layer opens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageRecoveryReport {
    /// Authoritative log verification and tail-repair evidence.
    pub log: RecoveryReport,
    /// Durable transactions newly applied to the materialized index.
    pub replayed_transactions: u64,
}

/// A newly opened storage engine and its recovery evidence.
#[derive(Debug)]
pub struct OpenedStorage {
    /// Ready-to-use storage engine.
    pub storage: StorageEngine,
    /// Evidence from log validation and index replay.
    pub recovery: StorageRecoveryReport,
}

/// Evidence for one successfully committed compaction generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactionReport {
    /// Newly active immutable manifest generation.
    pub generation: u64,
    /// Snapshot anchoring the retired log prefix.
    pub snapshot: SnapshotInfo,
    /// Segment that became inactive after the manifest commit.
    pub retired_segment: PathBuf,
    /// Whether best-effort physical cleanup removed the retired segment.
    pub retired_segment_removed: bool,
}

/// One binary KV entry in canonical key order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvEntry {
    /// Binary key.
    pub key: Vec<u8>,
    /// Opaque binary value.
    pub value: Vec<u8>,
}

/// One bounded ordered KV scan page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KvPage {
    /// Entries strictly after the requested cursor.
    pub entries: Vec<KvEntry>,
    /// Last emitted key when more entries remain.
    pub next_after: Option<Vec<u8>>,
}

/// Result of an online compaction request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompactionOutcome {
    /// No committed frames exist beyond the already active snapshot anchor.
    NoChanges {
        /// Current verified snapshot.
        snapshot: SnapshotInfo,
    },
    /// A new manifest and anchored segment were committed.
    Compacted(CompactionReport),
}

/// Single-writer durable KV storage composed from the log and rebuildable redb index.
#[derive(Debug)]
pub struct StorageEngine {
    log: DurableLog,
    index: MaterializedIndex,
    index_stale: bool,
    directory: DataDirectory,
}

impl StorageEngine {
    /// Opens a data directory, verifies its log, and catches the index up before use.
    ///
    /// # Errors
    ///
    /// Returns an error for directory contention, log corruption, invalid committed
    /// mutations, a divergent index checkpoint, or filesystem failures.
    pub fn open(path: impl AsRef<Path>) -> Result<OpenedStorage, StorageError> {
        let directory = DataDirectory::open(path)?;
        let index_path = directory.path().join("indexes").join("primary.redb");
        ensure_snapshot_base(&directory, &index_path)?;
        let (base_sequence, base_digest) = directory.log_anchor();
        let (log, log_recovery) =
            DurableLog::open_file_at(directory.active_log_path(), base_sequence, base_digest)?;
        let index = MaterializedIndex::open(index_path)?;
        let replayed_transactions = index.replay(&log_recovery)?;
        let _cleanup_complete = directory.cleanup_retired_logs();
        let storage = Self {
            log,
            index,
            index_stale: false,
            directory,
        };
        Ok(OpenedStorage {
            storage,
            recovery: StorageRecoveryReport {
                log: log_recovery,
                replayed_transactions,
            },
        })
    }

    /// Returns the owned data-directory path.
    pub fn data_path(&self) -> &Path {
        self.directory.path()
    }

    /// Durably commits an atomic batch and then materializes it.
    ///
    /// The log is synchronized before redb is updated. If redb fails, the error
    /// includes the durable commit receipt and this handle blocks reads and writes
    /// until reopen replays the log.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid mutations, idempotency conflicts, log I/O,
    /// or materialized-index failures.
    pub fn write(
        &mut self,
        transaction_id: Uuid,
        mutations: &[Mutation],
    ) -> Result<AppendOutcome, StorageError> {
        if self.index_stale {
            return Err(StorageError::StaleIndex);
        }
        let operations = mutations
            .iter()
            .map(Mutation::encode)
            .collect::<Result<Vec<_>, _>>()?;
        let operation_count =
            u32::try_from(operations.len()).map_err(|_| LogError::TooManyOperations)?;
        let requested_digest = transaction_digest(&operations, operation_count)?;
        if let Some(receipt) = self.index.receipt(transaction_id)? {
            return if receipt.transaction_digest == requested_digest {
                Ok(AppendOutcome::Existing(receipt))
            } else {
                Err(LogError::IdempotencyConflict { transaction_id }.into())
            };
        }
        let outcome = match self.log.append_transaction(transaction_id, &operations) {
            Ok(outcome) => outcome,
            Err(source) => {
                if self.log.is_poisoned() {
                    self.index_stale = true;
                }
                return Err(source.into());
            }
        };
        let AppendOutcome::Committed(receipt) = outcome else {
            return Ok(outcome);
        };

        let transaction = RecoveredTransaction {
            receipt,
            operations,
        };
        if let Err(source) = self.index.apply(&transaction) {
            self.index_stale = true;
            return Err(StorageError::CommittedButNotIndexed {
                receipt,
                source: Box::new(source),
            });
        }
        Ok(outcome)
    }

    /// Reads a binary value from the caught-up materialized index.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or oversized key, an index read failure,
    /// or a handle made stale by a prior post-commit index failure.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        if self.index_stale {
            return Err(StorageError::StaleIndex);
        }
        validate_key(key)?;
        Ok(self.index.get(key)?)
    }

    /// Scans one bounded page in strict binary-key order.
    ///
    /// `after` is exclusive. A returned `next_after` is present only when at
    /// least one additional entry exists.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale handle, invalid cursor key, invalid page
    /// size, or materialized-index failure.
    pub fn scan_page(&self, after: Option<&[u8]>, limit: usize) -> Result<KvPage, StorageError> {
        if self.index_stale {
            return Err(StorageError::StaleIndex);
        }
        if let Some(key) = after {
            validate_key(key)?;
        }
        if limit == 0 || limit > MAX_SCAN_PAGE_ENTRIES {
            return Err(StorageError::InvalidScanLimit {
                requested: limit,
                maximum: MAX_SCAN_PAGE_ENTRIES,
            });
        }
        let mut raw = self.index.scan_after(after, limit.saturating_add(1))?;
        let has_more = raw.len() > limit;
        raw.truncate(limit);
        let next_after = has_more
            .then(|| raw.last().map(|(key, _)| key.clone()))
            .flatten();
        Ok(KvPage {
            entries: raw
                .into_iter()
                .map(|(key, value)| KvEntry { key, value })
                .collect(),
            next_after,
        })
    }

    /// Returns the internal materialized-index path for diagnostics.
    pub fn index_path(&self) -> PathBuf {
        self.directory.path().join("indexes").join("primary.redb")
    }

    /// Creates or reuses a verified logical snapshot at the current index checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the live index is stale, cannot be streamed, or the
    /// snapshot cannot be synchronized, verified, and atomically promoted.
    pub fn snapshot(&self) -> Result<SnapshotInfo, StorageError> {
        if self.index_stale {
            return Err(StorageError::StaleIndex);
        }
        let snapshots = self.directory.path().join("snapshots");
        let temporary = self.directory.path().join("tmp");
        Ok(create_snapshot(&self.index, &snapshots, &temporary)?)
    }

    /// Retires the active log prefix behind a verified logical snapshot.
    ///
    /// The new empty segment is synchronized before an immutable manifest
    /// generation selects it. Physical deletion of the retired segment happens
    /// only after that commit and is reported independently.
    ///
    /// # Errors
    ///
    /// Returns an error while preparing the snapshot, segment, or manifest. A
    /// poisoned or stale handle must be reopened before compaction.
    pub fn compact(&mut self) -> Result<CompactionOutcome, StorageError> {
        if self.index_stale {
            return Err(StorageError::StaleIndex);
        }
        let snapshot = self.snapshot()?;
        let current = self.directory.manifest();
        if snapshot.checkpoint_sequence == 0
            || snapshot.checkpoint_sequence == current.base_sequence
        {
            return Ok(CompactionOutcome::NoChanges { snapshot });
        }
        let generation = current
            .generation
            .checked_add(1)
            .ok_or(StorageError::ManifestGenerationExhausted)?;
        let Some(base_digest) = snapshot.checkpoint_digest else {
            return Err(SnapshotError::Invalid {
                reason: "compaction snapshot lacks a checkpoint digest",
            }
            .into());
        };
        let next = StorageManifest {
            generation,
            active_segment: generation,
            base_sequence: snapshot.checkpoint_sequence,
            base_digest,
            snapshot_digest: snapshot.snapshot_digest,
        };
        let next_segment = self.directory.log_path(generation);
        let (next_log, prepared) =
            DurableLog::open_file_at(&next_segment, next.base_sequence, next.base_digest)?;
        if prepared.valid_bytes != 0 {
            return Err(StorageError::PreparedSegmentNotEmpty { path: next_segment });
        }

        let retired_segment = self.directory.active_log_path();
        self.directory.commit_manifest(next)?;
        let retired_log = std::mem::replace(&mut self.log, next_log);
        drop(retired_log);
        let retired_segment_removed = remove_retired_segment(&retired_segment);
        Ok(CompactionOutcome::Compacted(CompactionReport {
            generation,
            snapshot,
            retired_segment,
            retired_segment_removed,
        }))
    }
}

fn remove_retired_segment(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => {
            #[cfg(unix)]
            if let Some(parent) = path.parent()
                && sync_directory(parent).is_err()
            {
                return false;
            }
            true
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

fn ensure_snapshot_base(directory: &DataDirectory, index_path: &Path) -> Result<(), StorageError> {
    let manifest = directory.manifest();
    if manifest.base_sequence == 0 {
        return Ok(());
    }
    let snapshot_path = directory.snapshot_path(manifest.base_sequence);
    let verified = verify_snapshot(&snapshot_path)?;
    if verified.checkpoint_sequence != manifest.base_sequence
        || verified.checkpoint_digest != Some(manifest.base_digest)
        || verified.snapshot_digest != manifest.snapshot_digest
    {
        return Err(SnapshotError::Invalid {
            reason: "snapshot does not match active storage manifest",
        }
        .into());
    }
    if index_path.exists() {
        return Ok(());
    }

    let temporary_path = directory
        .path()
        .join("tmp")
        .join(format!("index-restore-{}.redb.tmp", Uuid::now_v7()));
    let restored = MaterializedIndex::restore_from_snapshot(&temporary_path, &snapshot_path)?;
    if restored != verified {
        return Err(SnapshotError::Invalid {
            reason: "snapshot changed while rebuilding the materialized index",
        }
        .into());
    }
    std::fs::rename(&temporary_path, index_path)
        .map_err(SnapshotError::from)
        .map_err(StorageError::from)?;
    #[cfg(unix)]
    sync_directory(index_path.parent().ok_or(SnapshotError::Invalid {
        reason: "materialized index path has no parent",
    })?)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StorageError> {
    std::fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(SnapshotError::from)
        .map_err(StorageError::from)
}

impl From<MaterializedIndexError> for StorageError {
    fn from(source: MaterializedIndexError) -> Self {
        Self::Index {
            source: Box::new(source),
        }
    }
}

impl From<SnapshotError> for StorageError {
    fn from(source: SnapshotError) -> Self {
        Self::Snapshot {
            source: Box::new(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        fs::{self, OpenOptions},
        io::{Seek, SeekFrom, Write},
    };

    use uuid::Uuid;

    use super::{CompactionOutcome, DurableLog, StorageEngine, StorageError, StorageManifest};
    use crate::{
        AppendOutcome, DataDirectory, Mutation, SnapshotError, index::MaterializedIndex,
        test_support::TestDirectory, verify_snapshot,
    };

    #[test]
    fn atomic_batches_persist_and_delete() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-kv")?;
        let root = temporary.path().join("data");
        let mut opened = StorageEngine::open(&root)?;
        opened.storage.write(
            Uuid::now_v7(),
            &[Mutation::put(b"a", b"one"), Mutation::put(b"b", b"two")],
        )?;
        assert_eq!(opened.storage.get(b"a")?, Some(b"one".to_vec()));
        opened
            .storage
            .write(Uuid::now_v7(), &[Mutation::delete(b"a")])?;
        drop(opened);

        let reopened = StorageEngine::open(&root)?;
        assert_eq!(reopened.storage.get(b"a")?, None);
        assert_eq!(reopened.storage.get(b"b")?, Some(b"two".to_vec()));
        assert_eq!(reopened.recovery.replayed_transactions, 0);
        Ok(())
    }

    #[test]
    fn exact_retry_does_not_reapply_a_batch() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-idempotency")?;
        let transaction_id = Uuid::now_v7();
        let mutations = [Mutation::put(b"key", b"value")];
        let mut opened = StorageEngine::open(temporary.path())?;

        let first = opened.storage.write(transaction_id, &mutations)?;
        let second = opened.storage.write(transaction_id, &mutations)?;
        assert!(matches!(first, AppendOutcome::Committed(_)));
        assert!(matches!(second, AppendOutcome::Existing(_)));
        assert_eq!(opened.storage.get(b"key")?, Some(b"value".to_vec()));

        let conflict = opened
            .storage
            .write(transaction_id, &[Mutation::put(b"key", b"different")]);
        assert!(matches!(
            conflict,
            Err(super::StorageError::Log(
                crate::LogError::IdempotencyConflict { .. }
            ))
        ));
        Ok(())
    }

    #[test]
    fn reopen_replays_a_commit_missing_from_the_index() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-index-replay")?;
        let root = temporary.path().join("data");
        let directory = DataDirectory::open(&root)?;
        let mutation = Mutation::put(b"recovered", b"yes");
        let mut log = directory.open_log()?;
        log.log
            .append_transaction(Uuid::now_v7(), &[mutation.encode()?])?;
        drop(log);
        drop(directory);

        let reopened = StorageEngine::open(&root)?;
        assert_eq!(reopened.recovery.replayed_transactions, 1);
        assert_eq!(reopened.storage.get(b"recovered")?, Some(b"yes".to_vec()));
        Ok(())
    }

    #[test]
    fn logical_snapshot_is_stable_and_detects_payload_corruption() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-snapshot")?;
        let mut opened = StorageEngine::open(temporary.path().join("data"))?;
        opened.storage.write(
            Uuid::now_v7(),
            &[
                Mutation::put(b"beta", b"second"),
                Mutation::put(b"alpha", b"first"),
            ],
        )?;

        let created = opened.storage.snapshot()?;
        assert_eq!(created.checkpoint_sequence, 4);
        assert!(created.checkpoint_digest.is_some());
        assert_eq!(created.entry_count, 2);
        assert_eq!(created.receipt_count, 1);
        assert_eq!(verify_snapshot(&created.path)?, created);
        assert_eq!(opened.storage.snapshot()?, created);

        let corrupted_path = temporary.path().join("corrupted.hysnap");
        fs::copy(&created.path, &corrupted_path)?;
        let mut corrupted = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&corrupted_path)?;
        corrupted.seek(SeekFrom::End(-1))?;
        corrupted.write_all(&[0xff])?;
        corrupted.sync_all()?;
        drop(corrupted);

        assert!(matches!(
            verify_snapshot(&corrupted_path),
            Err(SnapshotError::Invalid {
                reason: "CRC32C mismatch"
            })
        ));
        Ok(())
    }

    #[test]
    fn empty_storage_has_a_canonical_empty_snapshot() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-empty-snapshot")?;
        let opened = StorageEngine::open(temporary.path().join("data"))?;

        let snapshot = opened.storage.snapshot()?;
        assert_eq!(snapshot.checkpoint_sequence, 0);
        assert_eq!(snapshot.checkpoint_digest, None);
        assert_eq!(snapshot.entry_count, 0);
        assert_eq!(snapshot.receipt_count, 0);
        assert_eq!(snapshot.file_bytes, 112);
        assert_eq!(verify_snapshot(&snapshot.path)?, snapshot);
        Ok(())
    }

    #[test]
    fn snapshot_rebuilds_kv_and_idempotency_state() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-snapshot-restore")?;
        let root = temporary.path().join("data");
        let transaction_id = Uuid::now_v7();
        let mut opened = StorageEngine::open(&root)?;
        let outcome = opened.storage.write(
            transaction_id,
            &[
                Mutation::put(b"alpha", b"one"),
                Mutation::put(b"beta", b"two"),
            ],
        )?;
        let AppendOutcome::Committed(receipt) = outcome else {
            return Err("new transaction was not committed".into());
        };
        let snapshot = opened.storage.snapshot()?;
        drop(opened);

        let restored_path = root.join("tmp/restored.redb");
        assert_eq!(
            MaterializedIndex::restore_from_snapshot(&restored_path, &snapshot.path)?,
            snapshot
        );
        let restored = MaterializedIndex::open(&restored_path)?;
        assert_eq!(restored.get(b"alpha")?, Some(b"one".to_vec()));
        assert_eq!(restored.get(b"beta")?, Some(b"two".to_vec()));
        assert_eq!(restored.receipt(transaction_id)?, Some(receipt));
        Ok(())
    }

    #[test]
    fn compaction_retires_history_and_snapshot_rebuilds_the_index() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-compaction")?;
        let root = temporary.path().join("data");
        let first_id = Uuid::now_v7();
        let mut opened = StorageEngine::open(&root)?;
        let first = opened
            .storage
            .write(first_id, &[Mutation::put(b"before", b"one")])?;
        let AppendOutcome::Committed(first_receipt) = first else {
            return Err("first transaction was not committed".into());
        };

        let compacted = opened.storage.compact()?;
        let CompactionOutcome::Compacted(report) = compacted else {
            return Err("committed history was not compacted".into());
        };
        assert_eq!(report.generation, 2);
        assert!(report.retired_segment_removed);
        assert!(!report.retired_segment.exists());
        assert!(root.join("log/00000000000000000002.hylog").is_file());
        assert!(matches!(
            opened.storage.compact()?,
            CompactionOutcome::NoChanges { .. }
        ));

        assert_eq!(
            opened
                .storage
                .write(first_id, &[Mutation::put(b"before", b"one")])?,
            AppendOutcome::Existing(first_receipt)
        );
        let second_id = Uuid::now_v7();
        let second = opened
            .storage
            .write(second_id, &[Mutation::put(b"after", b"two")])?;
        let AppendOutcome::Committed(second_receipt) = second else {
            return Err("second transaction was not committed".into());
        };
        assert_eq!(
            second_receipt.commit_sequence,
            first_receipt.commit_sequence + 3
        );
        drop(opened);

        fs::remove_file(root.join("indexes/primary.redb"))?;
        let mut rebuilt = StorageEngine::open(&root)?;
        assert_eq!(rebuilt.recovery.replayed_transactions, 1);
        assert_eq!(rebuilt.storage.get(b"before")?, Some(b"one".to_vec()));
        assert_eq!(rebuilt.storage.get(b"after")?, Some(b"two".to_vec()));
        assert_eq!(
            rebuilt
                .storage
                .write(first_id, &[Mutation::put(b"before", b"one")])?,
            AppendOutcome::Existing(first_receipt)
        );
        Ok(())
    }

    #[test]
    fn orphan_prepared_segment_is_ignored_until_manifest_commit() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-compaction-orphan")?;
        let root = temporary.path().join("data");
        let mut opened = StorageEngine::open(&root)?;
        opened
            .storage
            .write(Uuid::now_v7(), &[Mutation::put(b"key", b"value")])?;
        let snapshot = opened.storage.snapshot()?;
        let base_digest = snapshot
            .checkpoint_digest
            .ok_or("snapshot checkpoint digest is absent")?;
        let orphan_path = opened.storage.directory.log_path(2);
        let (orphan, recovery) =
            DurableLog::open_file_at(&orphan_path, snapshot.checkpoint_sequence, base_digest)?;
        assert_eq!(recovery.valid_bytes, 0);
        drop(orphan);
        drop(opened);

        let mut reopened = StorageEngine::open(&root)?;
        assert_eq!(reopened.storage.directory.manifest().generation, 1);
        assert_eq!(reopened.storage.get(b"key")?, Some(b"value".to_vec()));
        assert!(matches!(
            reopened.storage.compact()?,
            CompactionOutcome::Compacted(_)
        ));
        assert_eq!(reopened.storage.directory.manifest().generation, 2);
        Ok(())
    }

    #[test]
    fn committed_manifest_wins_before_retired_log_cleanup() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-compaction-committed")?;
        let root = temporary.path().join("data");
        let mut opened = StorageEngine::open(&root)?;
        opened
            .storage
            .write(Uuid::now_v7(), &[Mutation::put(b"key", b"value")])?;
        let snapshot = opened.storage.snapshot()?;
        let base_digest = snapshot
            .checkpoint_digest
            .ok_or("snapshot checkpoint digest is absent")?;
        let next = StorageManifest {
            generation: 2,
            active_segment: 2,
            base_sequence: snapshot.checkpoint_sequence,
            base_digest,
            snapshot_digest: snapshot.snapshot_digest,
        };
        let (prepared, recovery) = DurableLog::open_file_at(
            opened.storage.directory.log_path(2),
            next.base_sequence,
            next.base_digest,
        )?;
        assert_eq!(recovery.valid_bytes, 0);
        drop(prepared);
        opened.storage.directory.commit_manifest(next)?;
        let retired_path = opened.storage.directory.log_path(1);
        assert!(retired_path.is_file());
        drop(opened);

        let reopened = StorageEngine::open(&root)?;
        assert_eq!(reopened.storage.directory.manifest().generation, 2);
        assert_eq!(reopened.storage.get(b"key")?, Some(b"value".to_vec()));
        assert!(!retired_path.exists());
        Ok(())
    }

    #[test]
    fn uncertain_log_sync_blocks_the_handle_until_recovery() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-injected-log-sync")?;
        let root = temporary.path().join("data");
        let mut opened = StorageEngine::open(&root)?;
        opened.storage.log.inject_sync_failure();

        let result = opened
            .storage
            .write(Uuid::now_v7(), &[Mutation::put(b"recovered", b"yes")]);
        assert!(matches!(
            result,
            Err(StorageError::Log(crate::LogError::Io(_)))
        ));
        assert!(matches!(
            opened.storage.get(b"recovered"),
            Err(StorageError::StaleIndex)
        ));
        assert!(matches!(
            opened.storage.snapshot(),
            Err(StorageError::StaleIndex)
        ));
        assert!(matches!(
            opened.storage.compact(),
            Err(StorageError::StaleIndex)
        ));
        drop(opened);

        let reopened = StorageEngine::open(&root)?;
        assert_eq!(reopened.recovery.replayed_transactions, 1);
        assert_eq!(reopened.storage.get(b"recovered")?, Some(b"yes".to_vec()));
        Ok(())
    }

    #[test]
    fn post_commit_index_failure_recovers_from_the_log() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-injected-index")?;
        let root = temporary.path().join("data");
        let mut opened = StorageEngine::open(&root)?;
        opened.storage.index.inject_apply_failure();

        let result = opened
            .storage
            .write(Uuid::now_v7(), &[Mutation::put(b"durable", b"yes")]);
        assert!(matches!(
            result,
            Err(StorageError::CommittedButNotIndexed { .. })
        ));
        assert!(matches!(
            opened.storage.get(b"durable"),
            Err(StorageError::StaleIndex)
        ));
        drop(opened);

        let reopened = StorageEngine::open(&root)?;
        assert_eq!(reopened.recovery.replayed_transactions, 1);
        assert_eq!(reopened.storage.get(b"durable")?, Some(b"yes".to_vec()));
        Ok(())
    }

    #[test]
    fn kv_scan_pages_are_strictly_ordered_and_exclusive() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("storage-scan-page")?;
        let mut opened = StorageEngine::open(temporary.path().join("data"))?;
        opened.storage.write(
            Uuid::now_v7(),
            &[
                Mutation::put(b"c", b"three"),
                Mutation::put(b"a", b"one"),
                Mutation::put(b"b", b"two"),
            ],
        )?;

        let first = opened.storage.scan_page(None, 2)?;
        assert_eq!(
            first
                .entries
                .iter()
                .map(|entry| entry.key.as_slice())
                .collect::<Vec<_>>(),
            [b"a".as_slice(), b"b".as_slice()]
        );
        assert_eq!(first.next_after, Some(b"b".to_vec()));

        let second = opened.storage.scan_page(first.next_after.as_deref(), 2)?;
        assert_eq!(second.entries[0].key, b"c");
        assert_eq!(second.next_after, None);
        Ok(())
    }
}
