// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use thiserror::Error;
use uuid::Uuid;

use crate::{
    AppendOutcome, CommitReceipt, DataDirectory, DataDirectoryError, DurableLog, LogError,
    MaterializedIndexError, Mutation, MutationError, RecoveredTransaction, RecoveryReport,
    SnapshotError, SnapshotInfo, index::MaterializedIndex, mutation::validate_key,
    snapshot::create_snapshot,
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
        let (log, log_recovery) = DurableLog::open_file(directory.initial_log_path())?;
        let index_path = directory.path().join("indexes").join("primary.redb");
        let index = MaterializedIndex::open(index_path)?;
        let replayed_transactions = index.replay(&log_recovery)?;
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
        let outcome = self.log.append_transaction(transaction_id, &operations)?;
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

    use super::StorageEngine;
    use crate::{
        AppendOutcome, DataDirectory, Mutation, SnapshotError, test_support::TestDirectory,
        verify_snapshot,
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
        assert_eq!(snapshot.file_bytes, 104);
        assert_eq!(verify_snapshot(&snapshot.path)?, snapshot);
        Ok(())
    }
}
