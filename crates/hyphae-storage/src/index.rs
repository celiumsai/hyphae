// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use thiserror::Error;

use crate::{Mutation, MutationError, RecoveredTransaction, RecoveryReport};

const KV: TableDefinition<&[u8], &[u8]> = TableDefinition::new("hyphae_kv_v1");
const METADATA: TableDefinition<&str, &[u8]> = TableDefinition::new("hyphae_metadata_v1");
const APPLIED_SEQUENCE: &str = "applied_sequence";
const APPLIED_DIGEST: &str = "applied_digest";

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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IndexCheckpoint {
    pub(crate) sequence: u64,
    pub(crate) digest: Option<[u8; 32]>,
}

#[derive(Debug)]
pub(crate) struct MaterializedIndex {
    database: Database,
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
        transaction.commit()?;
        Ok(Self { database })
    }

    pub(crate) fn replay(&self, recovery: &RecoveryReport) -> Result<u64, MaterializedIndexError> {
        let checkpoint = self.checkpoint()?;
        if checkpoint.sequence == 0 {
            if checkpoint.digest.is_some() {
                return Err(MaterializedIndexError::MalformedCheckpoint);
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
        let mutations = transaction
            .operations
            .iter()
            .map(|operation| Mutation::decode(operation))
            .collect::<Result<Vec<_>, _>>()?;

        let mut write = self.database.begin_write()?;
        write.set_durability(Durability::Immediate)?;
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
        write.commit()?;
        Ok(())
    }

    pub(crate) fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(KV)?;
        let value = table.get(key)?.map(|value| value.value().to_vec());
        Ok(value)
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
}
