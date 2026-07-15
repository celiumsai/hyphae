// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
use std::cell::Cell;
use std::{ops::Bound, path::Path};

use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use thiserror::Error;

use uuid::Uuid;

use crate::{
    CommitReceipt, Mutation, MutationError, RecoveredTransaction, RecoveryReport,
    snapshot::{SnapshotError, SnapshotInfo, SnapshotRecordVisitor, read_snapshot_records},
};

const KV: TableDefinition<&[u8], &[u8]> = TableDefinition::new("hyphae_kv_v1");
const METADATA: TableDefinition<&str, &[u8]> = TableDefinition::new("hyphae_metadata_v1");
const IDEMPOTENCY: TableDefinition<&[u8], &[u8]> = TableDefinition::new("hyphae_idempotency_v1");
const APPLIED_SEQUENCE: &str = "applied_sequence";
const APPLIED_DIGEST: &str = "applied_digest";
const RECEIPT_LENGTH: usize = 72;
type RawKvEntry = (Vec<u8>, Vec<u8>);

/// Failure while opening, verifying, or updating the rebuildable redb index.
#[derive(Debug, Error)]
pub enum MaterializedIndexError {
    /// redb could not open or create its database file.
    #[error("failed to open materialized index: {0}")]
    Database(#[from] redb::DatabaseError),

    /// redb could not begin a transaction.
    #[error("failed to begin materialized-index transaction: {0}")]
    Transaction(#[from] redb::TransactionError),

    /// A redb table could not be opened.
    #[error("failed to open materialized-index table: {0}")]
    Table(#[from] redb::TableError),

    /// A redb table read or write failed.
    #[error("materialized-index storage failure: {0}")]
    Storage(#[from] redb::StorageError),

    /// A redb transaction could not be committed.
    #[error("failed to commit materialized-index transaction: {0}")]
    Commit(#[from] redb::CommitError),

    /// A redb durability mode could not be selected.
    #[error("failed to select materialized-index durability: {0}")]
    Durability(#[from] redb::SetDurabilityError),

    /// A committed operation is not a valid canonical mutation.
    #[error("invalid committed mutation: {0}")]
    Mutation(#[from] MutationError),

    /// Stored index metadata has an invalid length or combination.
    #[error("malformed materialized-index checkpoint")]
    MalformedCheckpoint,

    /// The index checkpoint does not identify a commit in the verified log.
    #[error("materialized index checkpoint at sequence {sequence} diverges from the log")]
    Diverged {
        /// Checkpoint sequence that could not be verified.
        sequence: u64,
    },

    /// A persisted idempotency receipt is malformed or conflicts with the log.
    #[error("materialized idempotency receipt for {transaction_id} diverges from the log")]
    IdempotencyDiverged {
        /// Transaction identifier with conflicting durable identity.
        transaction_id: Uuid,
    },

    /// A persisted idempotency key is not a UUID.
    #[error("materialized idempotency key is malformed")]
    MalformedIdempotencyKey,

    /// A test-only injected index failure occurred.
    #[cfg(test)]
    #[error("injected materialized-index failure")]
    InjectedFailure,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IndexCheckpoint {
    pub(crate) sequence: u64,
    pub(crate) digest: Option<[u8; 32]>,
}

#[derive(Debug)]
pub(crate) struct MaterializedIndex {
    database: Database,
    #[cfg(test)]
    fail_next_apply: Cell<bool>,
}

impl MaterializedIndex {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self, MaterializedIndexError> {
        let database = Database::create(path)?;
        let mut transaction = database.begin_write()?;
        transaction.set_durability(Durability::Immediate)?;
        {
            let _table = transaction.open_table(KV)?;
        }
        {
            let _table = transaction.open_table(METADATA)?;
        }
        {
            let _table = transaction.open_table(IDEMPOTENCY)?;
        }
        transaction.commit()?;
        Ok(Self {
            database,
            #[cfg(test)]
            fail_next_apply: Cell::new(false),
        })
    }

    pub(crate) fn restore_from_snapshot(
        index_path: &Path,
        snapshot_path: &Path,
    ) -> Result<SnapshotInfo, SnapshotError> {
        let index = Self::open(index_path)?;
        let mut write = index
            .database
            .begin_write()
            .map_err(MaterializedIndexError::from)?;
        write
            .set_durability(Durability::Immediate)
            .map_err(MaterializedIndexError::from)?;
        let snapshot = {
            let mut visitor = IndexRestoreVisitor { write: &mut write };
            read_snapshot_records(snapshot_path, &mut visitor)?
        };
        {
            let mut metadata = write
                .open_table(METADATA)
                .map_err(MaterializedIndexError::from)?;
            if snapshot.checkpoint_sequence > 0 {
                let Some(digest) = snapshot.checkpoint_digest else {
                    return Err(SnapshotError::Invalid {
                        reason: "nonempty snapshot lacks a checkpoint digest",
                    });
                };
                metadata
                    .insert(
                        APPLIED_SEQUENCE,
                        snapshot.checkpoint_sequence.to_le_bytes().as_slice(),
                    )
                    .map_err(MaterializedIndexError::from)?;
                metadata
                    .insert(APPLIED_DIGEST, digest.as_slice())
                    .map_err(MaterializedIndexError::from)?;
            }
        }
        write.commit().map_err(MaterializedIndexError::from)?;
        Ok(snapshot)
    }

    pub(crate) fn replay(&self, recovery: &RecoveryReport) -> Result<u64, MaterializedIndexError> {
        let checkpoint = self.checkpoint()?;
        if checkpoint.sequence == 0 {
            if checkpoint.digest.is_some() || recovery.base_sequence != 0 {
                return Err(MaterializedIndexError::MalformedCheckpoint);
            }
        } else if checkpoint.sequence == recovery.base_sequence {
            if checkpoint.digest != Some(recovery.base_digest) {
                return Err(MaterializedIndexError::Diverged {
                    sequence: checkpoint.sequence,
                });
            }
        } else {
            let Some(transaction) = recovery
                .transactions
                .iter()
                .find(|transaction| transaction.receipt.commit_sequence == checkpoint.sequence)
            else {
                return Err(MaterializedIndexError::Diverged {
                    sequence: checkpoint.sequence,
                });
            };
            if checkpoint.digest != Some(transaction.receipt.commit_digest) {
                return Err(MaterializedIndexError::Diverged {
                    sequence: checkpoint.sequence,
                });
            }
        }

        self.reconcile_idempotency(recovery)?;

        let mut replayed = 0_u64;
        for transaction in recovery
            .transactions
            .iter()
            .filter(|transaction| transaction.receipt.commit_sequence > checkpoint.sequence)
        {
            self.apply(transaction)?;
            replayed = replayed.saturating_add(1);
        }
        Ok(replayed)
    }

    pub(crate) fn apply(
        &self,
        transaction: &RecoveredTransaction,
    ) -> Result<(), MaterializedIndexError> {
        #[cfg(test)]
        if self.fail_next_apply.replace(false) {
            return Err(MaterializedIndexError::InjectedFailure);
        }
        let mutations = transaction
            .operations
            .iter()
            .map(|operation| Mutation::decode(operation))
            .collect::<Result<Vec<_>, _>>()?;

        let mut write = self.database.begin_write()?;
        write.set_durability(redb::Durability::Immediate)?;
        {
            let mut table = write.open_table(KV)?;
            for mutation in mutations {
                match mutation {
                    Mutation::Put { key, value } => {
                        table.insert(key.as_slice(), value.as_slice())?;
                    }
                    Mutation::Delete { key } => {
                        table.remove(key.as_slice())?;
                    }
                }
            }
        }
        {
            let mut metadata = write.open_table(METADATA)?;
            metadata.insert(
                APPLIED_SEQUENCE,
                transaction.receipt.commit_sequence.to_le_bytes().as_slice(),
            )?;
            metadata.insert(APPLIED_DIGEST, transaction.receipt.commit_digest.as_slice())?;
        }
        {
            let mut idempotency = write.open_table(IDEMPOTENCY)?;
            idempotency.insert(
                transaction.receipt.transaction_id.as_bytes().as_slice(),
                encode_receipt(&transaction.receipt).as_slice(),
            )?;
        }
        write.commit()?;
        Ok(())
    }

    pub(crate) fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(KV)?;
        let value = table.get(key)?.map(|value| value.value().to_vec());
        Ok(value)
    }

    pub(crate) fn scan_after(
        &self,
        after: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<RawKvEntry>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(KV)?;
        let bounds = (
            after.map_or(Bound::Unbounded, Bound::Excluded),
            Bound::Unbounded,
        );
        let mut entries = Vec::with_capacity(limit);
        for entry in table.range::<&[u8]>(bounds)?.take(limit) {
            let (key, value) = entry?;
            entries.push((key.value().to_vec(), value.value().to_vec()));
        }
        Ok(entries)
    }

    #[cfg(test)]
    pub(crate) fn inject_apply_failure(&self) {
        self.fail_next_apply.set(true);
    }

    pub(crate) fn for_each_entry(
        &self,
        mut visitor: impl FnMut(&[u8], &[u8]),
    ) -> Result<(), MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(KV)?;
        for entry in table.iter()? {
            let (key, value) = entry?;
            visitor(key.value(), value.value());
        }
        Ok(())
    }

    pub(crate) fn receipt(
        &self,
        transaction_id: Uuid,
    ) -> Result<Option<CommitReceipt>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(IDEMPOTENCY)?;
        table
            .get(transaction_id.as_bytes().as_slice())?
            .map(|encoded| decode_receipt(transaction_id, encoded.value()))
            .transpose()
    }

    pub(crate) fn for_each_receipt(
        &self,
        mut visitor: impl FnMut(&CommitReceipt),
    ) -> Result<(), MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(IDEMPOTENCY)?;
        for entry in table.iter()? {
            let (key, value) = entry?;
            let transaction_id = Uuid::from_slice(key.value())
                .map_err(|_| MaterializedIndexError::MalformedIdempotencyKey)?;
            let receipt = decode_receipt(transaction_id, value.value())?;
            visitor(&receipt);
        }
        Ok(())
    }

    pub(crate) fn checkpoint(&self) -> Result<IndexCheckpoint, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let metadata = read.open_table(METADATA)?;
        let sequence = metadata
            .get(APPLIED_SEQUENCE)?
            .map(|value| decode_sequence(value.value()))
            .transpose()?
            .unwrap_or(0);
        let digest = metadata
            .get(APPLIED_DIGEST)?
            .map(|value| decode_digest(value.value()))
            .transpose()?;
        Ok(IndexCheckpoint { sequence, digest })
    }

    fn reconcile_idempotency(
        &self,
        recovery: &RecoveryReport,
    ) -> Result<(), MaterializedIndexError> {
        let mut write = self.database.begin_write()?;
        write.set_durability(Durability::Immediate)?;
        {
            let mut table = write.open_table(IDEMPOTENCY)?;
            for transaction in &recovery.transactions {
                let receipt = transaction.receipt;
                if let Some(encoded) = table.get(receipt.transaction_id.as_bytes().as_slice())? {
                    let existing = decode_receipt(receipt.transaction_id, encoded.value())?;
                    if existing != receipt {
                        return Err(MaterializedIndexError::IdempotencyDiverged {
                            transaction_id: receipt.transaction_id,
                        });
                    }
                } else {
                    table.insert(
                        receipt.transaction_id.as_bytes().as_slice(),
                        encode_receipt(&receipt).as_slice(),
                    )?;
                }
            }
        }
        write.commit()?;
        Ok(())
    }
}

struct IndexRestoreVisitor<'transaction> {
    write: &'transaction mut redb::WriteTransaction,
}

impl SnapshotRecordVisitor for IndexRestoreVisitor<'_> {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), SnapshotError> {
        let mut table = self
            .write
            .open_table(KV)
            .map_err(MaterializedIndexError::from)?;
        table
            .insert(key, value)
            .map_err(MaterializedIndexError::from)?;
        Ok(())
    }

    fn receipt(&mut self, receipt: &CommitReceipt) -> Result<(), SnapshotError> {
        let mut table = self
            .write
            .open_table(IDEMPOTENCY)
            .map_err(MaterializedIndexError::from)?;
        table
            .insert(
                receipt.transaction_id.as_bytes().as_slice(),
                encode_receipt(receipt).as_slice(),
            )
            .map_err(MaterializedIndexError::from)?;
        Ok(())
    }
}

fn encode_receipt(receipt: &CommitReceipt) -> [u8; RECEIPT_LENGTH] {
    let mut encoded = [0_u8; RECEIPT_LENGTH];
    encoded[..8].copy_from_slice(&receipt.commit_sequence.to_le_bytes());
    encoded[8..40].copy_from_slice(&receipt.commit_digest);
    encoded[40..72].copy_from_slice(&receipt.transaction_digest);
    encoded
}

fn decode_receipt(
    transaction_id: Uuid,
    encoded: &[u8],
) -> Result<CommitReceipt, MaterializedIndexError> {
    if encoded.len() != RECEIPT_LENGTH {
        return Err(MaterializedIndexError::IdempotencyDiverged { transaction_id });
    }
    Ok(CommitReceipt {
        transaction_id,
        commit_sequence: u64::from_le_bytes(copy_array(&encoded[..8])),
        commit_digest: copy_array(&encoded[8..40]),
        transaction_digest: copy_array(&encoded[40..72]),
    })
}

fn decode_sequence(encoded: &[u8]) -> Result<u64, MaterializedIndexError> {
    if encoded.len() != 8 {
        return Err(MaterializedIndexError::MalformedCheckpoint);
    }
    Ok(u64::from_le_bytes(copy_array(encoded)))
}

fn decode_digest(encoded: &[u8]) -> Result<[u8; 32], MaterializedIndexError> {
    if encoded.len() != 32 {
        return Err(MaterializedIndexError::MalformedCheckpoint);
    }
    Ok(copy_array(encoded))
}

fn copy_array<const N: usize>(source: &[u8]) -> [u8; N] {
    let mut output = [0_u8; N];
    output.copy_from_slice(source);
    output
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use uuid::Uuid;

    use super::{MaterializedIndex, MaterializedIndexError};
    use crate::{DurableLog, Mutation, test_support::TestDirectory};

    fn recovery_with_operation(
        path: &std::path::Path,
        operation: Vec<u8>,
    ) -> Result<crate::RecoveryReport, Box<dyn Error>> {
        let (mut log, _) = DurableLog::open_file(path)?;
        log.append_transaction(Uuid::now_v7(), &[operation])?;
        drop(log);
        let (_, recovery) = DurableLog::open_file(path)?;
        Ok(recovery)
    }

    #[test]
    fn checkpoint_rejects_a_different_log_history() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("index-divergence")?;
        let first = recovery_with_operation(
            &temporary.path().join("first.hylog"),
            Mutation::put(b"key", b"first").encode()?,
        )?;
        let second = recovery_with_operation(
            &temporary.path().join("second.hylog"),
            Mutation::put(b"key", b"second").encode()?,
        )?;
        let index = MaterializedIndex::open(temporary.path().join("index.redb"))?;
        assert_eq!(index.replay(&first)?, 1);

        let result = index.replay(&second);
        assert!(matches!(
            result,
            Err(MaterializedIndexError::Diverged { sequence: 3 })
        ));
        assert_eq!(index.get(b"key")?, Some(b"first".to_vec()));
        Ok(())
    }

    #[test]
    fn invalid_committed_operation_never_advances_checkpoint() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("index-invalid-operation")?;
        let recovery = recovery_with_operation(
            &temporary.path().join("segment.hylog"),
            b"not-a-mutation".to_vec(),
        )?;
        let index = MaterializedIndex::open(temporary.path().join("index.redb"))?;

        assert!(matches!(
            index.replay(&recovery),
            Err(MaterializedIndexError::Mutation(_))
        ));
        assert!(matches!(
            index.replay(&recovery),
            Err(MaterializedIndexError::Mutation(_))
        ));
        Ok(())
    }

    #[test]
    fn replay_backfills_a_missing_idempotency_receipt() -> Result<(), Box<dyn Error>> {
        let temporary = TestDirectory::new("index-idempotency-backfill")?;
        let recovery = recovery_with_operation(
            &temporary.path().join("segment.hylog"),
            Mutation::put(b"key", b"value").encode()?,
        )?;
        let receipt = recovery.transactions[0].receipt;
        let index = MaterializedIndex::open(temporary.path().join("index.redb"))?;
        assert_eq!(index.replay(&recovery)?, 1);
        assert_eq!(index.receipt(receipt.transaction_id)?, Some(receipt));

        let mut write = index.database.begin_write()?;
        write.set_durability(redb::Durability::Immediate)?;
        {
            let mut table = write.open_table(super::IDEMPOTENCY)?;
            table.remove(receipt.transaction_id.as_bytes().as_slice())?;
        }
        write.commit()?;
        assert_eq!(index.receipt(receipt.transaction_id)?, None);

        assert_eq!(index.replay(&recovery)?, 0);
        assert_eq!(index.receipt(receipt.transaction_id)?, Some(receipt));
        Ok(())
    }
}
