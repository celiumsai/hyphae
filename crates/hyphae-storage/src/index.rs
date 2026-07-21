// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Bound,
    path::Path,
    time::{Duration, Instant},
};

use hyphae_core::{
    Q15Vector, VectorMetric, VectorSpaceDefinition, VectorSpaceName, VectorValueError,
};
use hyphae_query::{DocumentError, FieldPath, Value, decode_document};
use hyphae_retrieval::{
    LexicalError, LexicalField, LexicalIndexDefinition, LexicalMaterializedCorpus,
    LexicalMaterializedDocument, tokenize_v1,
};
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
const VECTOR_SPACES: TableDefinition<&str, &[u8]> = TableDefinition::new("hyphae_vector_spaces_v1");
const VECTORS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("hyphae_vectors_v1");
const LEXICAL_INDEXES: TableDefinition<&str, &[u8]> =
    TableDefinition::new("hyphae_lexical_indexes_v1");
const LEXICAL_DOCUMENTS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("hyphae_lexical_documents_v1");
const LEXICAL_POSTINGS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("hyphae_lexical_postings_v1");
const LEXICAL_STATS: TableDefinition<&str, &[u8]> = TableDefinition::new("hyphae_lexical_stats_v1");
const APPLIED_SEQUENCE: &str = "applied_sequence";
const APPLIED_DIGEST: &str = "applied_digest";
const RECEIPT_LENGTH: usize = 72;
type RawKvEntry = (Vec<u8>, Vec<u8>);

#[derive(Clone, Debug, Eq, PartialEq)]
struct LexicalDocumentProjection {
    field_lengths: Vec<u64>,
    terms: BTreeMap<String, Vec<u64>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LexicalCorpusProjection {
    document_count: u64,
    token_count: u64,
    total_field_lengths: Vec<u64>,
}

/// One durable vector entry materialized from authoritative logical state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorEntry {
    /// Binary object key within the selected vector space.
    pub key: Vec<u8>,
    /// Canonical signed-Q15 vector.
    pub vector: Q15Vector,
}

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

    /// A canonical shared vector value is invalid.
    #[error(transparent)]
    Vector(#[from] VectorValueError),

    /// A vector mutation refers to a space that has not been defined.
    #[error("vector space `{name}` is not defined")]
    UnknownVectorSpace {
        /// Canonical vector-space name.
        name: String,
    },

    /// An existing immutable vector-space definition differs.
    #[error("vector space `{name}` already exists with a different definition")]
    VectorSpaceConflict {
        /// Canonical vector-space name.
        name: String,
    },

    /// Persisted vector-index bytes are not canonical.
    #[error("malformed materialized vector index")]
    MalformedVectorIndex,

    /// A canonical lexical definition is invalid.
    #[error(transparent)]
    Lexical(#[from] LexicalError),

    /// An existing immutable lexical-index definition differs.
    #[error("lexical index `{name}` already exists with a different definition")]
    LexicalIndexConflict {
        /// Canonical lexical-index name.
        name: String,
    },

    /// A lexical retrieval refers to an index that has not been defined.
    #[error("lexical index `{name}` is not defined")]
    UnknownLexicalIndex {
        /// Canonical lexical-index name.
        name: String,
    },

    /// Persisted lexical-index bytes are not canonical.
    #[error("malformed materialized lexical index")]
    MalformedLexicalIndex,

    /// Persisted lexical postings/statistics are not canonical.
    #[error("malformed materialized lexical projection")]
    MalformedLexicalProjection,

    /// A structured document cannot be decoded while maintaining a lexical projection.
    #[error(transparent)]
    Document(#[from] DocumentError),

    /// Reading candidates exceeded the caller's count budget.
    #[error("vector candidate budget exceeded: {maximum}")]
    VectorCandidateBudgetExceeded {
        /// Maximum candidates permitted.
        maximum: u64,
    },

    /// Reading candidates exceeded the caller's byte budget.
    #[error("vector candidate byte budget exceeded: {maximum}")]
    VectorByteBudgetExceeded {
        /// Maximum encoded key and vector bytes permitted.
        maximum: u64,
    },

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
        {
            let _table = transaction.open_table(VECTOR_SPACES)?;
        }
        {
            let _table = transaction.open_table(VECTORS)?;
        }
        {
            let _table = transaction.open_table(LEXICAL_INDEXES)?;
        }
        {
            let _table = transaction.open_table(LEXICAL_DOCUMENTS)?;
        }
        {
            let _table = transaction.open_table(LEXICAL_POSTINGS)?;
        }
        {
            let _table = transaction.open_table(LEXICAL_STATS)?;
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
        self.rebuild_missing_lexical_projections()?;
        Ok(replayed)
    }

    fn rebuild_missing_lexical_projections(&self) -> Result<(), MaterializedIndexError> {
        let definitions = {
            let read = self.database.begin_read()?;
            let indexes = read.open_table(LEXICAL_INDEXES)?;
            let stats = read.open_table(LEXICAL_STATS)?;
            indexes
                .iter()?
                .filter_map(|entry| {
                    let (name, value) = match entry {
                        Ok(entry) => entry,
                        Err(source) => return Some(Err(MaterializedIndexError::from(source))),
                    };
                    match stats.get(name.value()) {
                        Ok(Some(_)) => None,
                        Ok(None) => {
                            let name = match VectorSpaceName::new(name.value().to_owned()) {
                                Ok(name) => name,
                                Err(source) => {
                                    return Some(Err(MaterializedIndexError::from(source)));
                                }
                            };
                            Some(decode_lexical_index_value(&name, value.value()))
                        }
                        Err(source) => Some(Err(MaterializedIndexError::from(source))),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        for definition in definitions {
            let mut write = self.database.begin_write()?;
            write.set_durability(Durability::Immediate)?;
            build_lexical_projection(&write, &definition)?;
            write.commit()?;
        }
        Ok(())
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
        for mutation in mutations {
            match mutation {
                Mutation::Put { key, value } => {
                    update_lexical_projections(&write, &key, Some(&value))?;
                    let mut table = write.open_table(KV)?;
                    table.insert(key.as_slice(), value.as_slice())?;
                }
                Mutation::Delete { key } => {
                    update_lexical_projections(&write, &key, None)?;
                    let mut table = write.open_table(KV)?;
                    table.remove(key.as_slice())?;
                }
                Mutation::DefineVectorSpace { definition } => {
                    apply_vector_space_definition(&write, &definition)?;
                }
                Mutation::UpsertVector { space, key, vector } => {
                    let definition = require_vector_space(&write, &space)?;
                    definition.validate_vector(&vector)?;
                    let composite_key = encode_vector_key(&space, &key);
                    let encoded_vector = encode_vector_value(&vector);
                    let mut table = write.open_table(VECTORS)?;
                    table.insert(composite_key.as_slice(), encoded_vector.as_slice())?;
                }
                Mutation::DeleteVector { space, key } => {
                    let _definition = require_vector_space(&write, &space)?;
                    let composite_key = encode_vector_key(&space, &key);
                    let mut table = write.open_table(VECTORS)?;
                    table.remove(composite_key.as_slice())?;
                }
                Mutation::DefineLexicalIndex { definition } => {
                    apply_lexical_index_definition(&write, &definition)?;
                    build_lexical_projection(&write, &definition)?;
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

    pub(crate) fn validate_mutations(
        &self,
        mutations: &[Mutation],
    ) -> Result<(), MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(VECTOR_SPACES)?;
        let lexical_table = read.open_table(LEXICAL_INDEXES)?;
        let mut pending: std::collections::BTreeMap<VectorSpaceName, VectorSpaceDefinition> =
            std::collections::BTreeMap::new();
        let mut pending_lexical: std::collections::BTreeMap<
            VectorSpaceName,
            LexicalIndexDefinition,
        > = std::collections::BTreeMap::new();
        for mutation in mutations {
            match mutation {
                Mutation::Put { .. } | Mutation::Delete { .. } => {}
                Mutation::DefineVectorSpace { definition } => {
                    let existing = if let Some(existing) = pending.get(&definition.name) {
                        Some(existing.clone())
                    } else {
                        table
                            .get(definition.name.as_str())?
                            .map(|encoded| {
                                decode_vector_space_value(&definition.name, encoded.value())
                            })
                            .transpose()?
                    };
                    if existing
                        .as_ref()
                        .is_some_and(|existing| existing != definition)
                    {
                        return Err(MaterializedIndexError::VectorSpaceConflict {
                            name: definition.name.as_str().to_owned(),
                        });
                    }
                    pending.insert(definition.name.clone(), definition.clone());
                }
                Mutation::UpsertVector { space, vector, .. } => {
                    let definition = if let Some(definition) = pending.get(space) {
                        definition.clone()
                    } else {
                        table
                            .get(space.as_str())?
                            .map(|encoded| decode_vector_space_value(space, encoded.value()))
                            .transpose()?
                            .ok_or_else(|| MaterializedIndexError::UnknownVectorSpace {
                                name: space.as_str().to_owned(),
                            })?
                    };
                    definition.validate_vector(vector)?;
                }
                Mutation::DeleteVector { space, .. } => {
                    let exists =
                        pending.contains_key(space) || table.get(space.as_str())?.is_some();
                    if !exists {
                        return Err(MaterializedIndexError::UnknownVectorSpace {
                            name: space.as_str().to_owned(),
                        });
                    }
                }
                Mutation::DefineLexicalIndex { definition } => {
                    let existing = if let Some(existing) = pending_lexical.get(&definition.name) {
                        Some(existing.clone())
                    } else {
                        lexical_table
                            .get(definition.name.as_str())?
                            .map(|encoded| {
                                decode_lexical_index_value(&definition.name, encoded.value())
                            })
                            .transpose()?
                    };
                    if existing
                        .as_ref()
                        .is_some_and(|existing| existing != definition)
                    {
                        return Err(MaterializedIndexError::LexicalIndexConflict {
                            name: definition.name.as_str().to_owned(),
                        });
                    }
                    pending_lexical.insert(definition.name.clone(), definition.clone());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(KV)?;
        let value = table.get(key)?.map(|value| value.value().to_vec());
        Ok(value)
    }

    pub(crate) fn vector_space(
        &self,
        name: &VectorSpaceName,
    ) -> Result<Option<VectorSpaceDefinition>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(VECTOR_SPACES)?;
        table
            .get(name.as_str())?
            .map(|value| decode_vector_space_value(name, value.value()))
            .transpose()
    }

    pub(crate) fn lexical_index(
        &self,
        name: &VectorSpaceName,
    ) -> Result<Option<LexicalIndexDefinition>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(LEXICAL_INDEXES)?;
        table
            .get(name.as_str())?
            .map(|value| decode_lexical_index_value(name, value.value()))
            .transpose()
    }

    pub(crate) fn lexical_corpus(
        &self,
        definition: &LexicalIndexDefinition,
        query_tokens: &[String],
        max_candidates: u64,
        timeout: Duration,
    ) -> Result<LexicalMaterializedCorpus, MaterializedIndexError> {
        let started = Instant::now();
        let read = self.database.begin_read()?;
        let stats = read.open_table(LEXICAL_STATS)?;
        let encoded_stats = stats
            .get(definition.name.as_str())?
            .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
        let corpus = decode_lexical_corpus(encoded_stats.value(), definition.fields.len())?;
        let postings = read.open_table(LEXICAL_POSTINGS)?;
        let documents = read.open_table(LEXICAL_DOCUMENTS)?;
        let mut candidate_keys = BTreeSet::<Vec<u8>>::new();
        for token in query_tokens {
            check_lexical_timeout(started, timeout)?;
            let prefix = encode_lexical_posting_prefix(&definition.name, token)?;
            let upper = prefix_upper_bound(&prefix);
            let bounds = (
                Bound::Included(prefix.as_slice()),
                upper.as_deref().map_or(Bound::Unbounded, Bound::Excluded),
            );
            for entry in postings.range::<&[u8]>(bounds)? {
                check_lexical_timeout(started, timeout)?;
                let (key, _value) = entry?;
                let candidate = decode_lexical_posting_key(key.value(), &prefix)?;
                candidate_keys.insert(candidate);
                if u64::try_from(candidate_keys.len()).unwrap_or(u64::MAX) > max_candidates {
                    return Err(MaterializedIndexError::Lexical(
                        LexicalError::CandidateBudgetExceeded {
                            maximum: max_candidates,
                        },
                    ));
                }
            }
        }
        let mut materialized = Vec::with_capacity(candidate_keys.len());
        for key in candidate_keys {
            check_lexical_timeout(started, timeout)?;
            let encoded_key = encode_lexical_document_key(&definition.name, &key)?;
            let encoded = documents
                .get(encoded_key.as_slice())?
                .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
            let projection = decode_lexical_document(encoded.value(), definition.fields.len())?;
            materialized.push(LexicalMaterializedDocument {
                key,
                field_lengths: projection.field_lengths,
                term_frequencies: query_tokens
                    .iter()
                    .map(|token| {
                        (
                            token.clone(),
                            projection
                                .terms
                                .get(token)
                                .cloned()
                                .unwrap_or_else(|| vec![0; definition.fields.len()]),
                        )
                    })
                    .collect(),
            });
        }
        Ok(LexicalMaterializedCorpus {
            document_count: corpus.document_count,
            token_count: corpus.token_count,
            total_field_lengths: corpus.total_field_lengths,
            documents: materialized,
        })
    }

    pub(crate) fn scan_vectors(
        &self,
        space: &VectorSpaceName,
        max_candidates: u64,
        max_bytes: u64,
    ) -> Result<Vec<VectorEntry>, MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(VECTORS)?;
        let mut entries = Vec::new();
        let mut consumed_bytes = 0_u64;
        for entry in table.iter()? {
            let (raw_key, raw_vector) = entry?;
            let Some(key) = decode_vector_key_for_space(raw_key.value(), space)? else {
                continue;
            };
            if u64::try_from(entries.len()).unwrap_or(u64::MAX) >= max_candidates {
                return Err(MaterializedIndexError::VectorCandidateBudgetExceeded {
                    maximum: max_candidates,
                });
            }
            let record_bytes = u64::try_from(key.len())
                .ok()
                .and_then(|key_bytes| {
                    u64::try_from(raw_vector.value().len())
                        .ok()
                        .and_then(|vector_bytes| key_bytes.checked_add(vector_bytes))
                })
                .ok_or(MaterializedIndexError::VectorByteBudgetExceeded { maximum: max_bytes })?;
            consumed_bytes = consumed_bytes
                .checked_add(record_bytes)
                .ok_or(MaterializedIndexError::VectorByteBudgetExceeded { maximum: max_bytes })?;
            if consumed_bytes > max_bytes {
                return Err(MaterializedIndexError::VectorByteBudgetExceeded {
                    maximum: max_bytes,
                });
            }
            entries.push(VectorEntry {
                key,
                vector: decode_vector_value(raw_vector.value())?,
            });
        }
        Ok(entries)
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

    pub(crate) fn for_each_vector_space(
        &self,
        mut visitor: impl FnMut(&VectorSpaceDefinition),
    ) -> Result<(), MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(VECTOR_SPACES)?;
        for entry in table.iter()? {
            let (name, value) = entry?;
            let name = VectorSpaceName::new(name.value().to_owned())?;
            let definition = decode_vector_space_value(&name, value.value())?;
            visitor(&definition);
        }
        Ok(())
    }

    pub(crate) fn for_each_vector(
        &self,
        mut visitor: impl FnMut(&VectorSpaceName, &[u8], &Q15Vector),
    ) -> Result<(), MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(VECTORS)?;
        for entry in table.iter()? {
            let (raw_key, raw_vector) = entry?;
            let (space, key) = decode_vector_key(raw_key.value())?;
            let vector = decode_vector_value(raw_vector.value())?;
            visitor(&space, &key, &vector);
        }
        Ok(())
    }

    pub(crate) fn for_each_lexical_index(
        &self,
        mut visitor: impl FnMut(&LexicalIndexDefinition),
    ) -> Result<(), MaterializedIndexError> {
        let read = self.database.begin_read()?;
        let table = read.open_table(LEXICAL_INDEXES)?;
        for entry in table.iter()? {
            let (name, value) = entry?;
            let name = VectorSpaceName::new(name.value().to_owned())?;
            let definition = decode_lexical_index_value(&name, value.value())?;
            visitor(&definition);
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

fn check_lexical_timeout(
    started: Instant,
    timeout: Duration,
) -> Result<(), MaterializedIndexError> {
    if started.elapsed() >= timeout {
        Err(LexicalError::TimedOut.into())
    } else {
        Ok(())
    }
}

fn apply_vector_space_definition(
    write: &redb::WriteTransaction,
    definition: &VectorSpaceDefinition,
) -> Result<(), MaterializedIndexError> {
    let encoded = encode_vector_space_value(definition);
    let existing = {
        let table = write.open_table(VECTOR_SPACES)?;
        table
            .get(definition.name.as_str())?
            .map(|value| value.value().to_vec())
    };
    if let Some(existing) = existing {
        if existing == encoded {
            return Ok(());
        }
        return Err(MaterializedIndexError::VectorSpaceConflict {
            name: definition.name.as_str().to_owned(),
        });
    }
    let mut table = write.open_table(VECTOR_SPACES)?;
    table.insert(definition.name.as_str(), encoded.as_slice())?;
    Ok(())
}

fn apply_lexical_index_definition(
    write: &redb::WriteTransaction,
    definition: &LexicalIndexDefinition,
) -> Result<(), MaterializedIndexError> {
    let encoded = encode_lexical_index_value(definition)?;
    let existing = {
        let table = write.open_table(LEXICAL_INDEXES)?;
        table
            .get(definition.name.as_str())?
            .map(|value| value.value().to_vec())
    };
    if let Some(existing) = existing {
        if existing == encoded {
            return Ok(());
        }
        return Err(MaterializedIndexError::LexicalIndexConflict {
            name: definition.name.as_str().to_owned(),
        });
    }
    let mut table = write.open_table(LEXICAL_INDEXES)?;
    table.insert(definition.name.as_str(), encoded.as_slice())?;
    Ok(())
}

fn build_lexical_projection(
    write: &redb::WriteTransaction,
    definition: &LexicalIndexDefinition,
) -> Result<(), MaterializedIndexError> {
    if write
        .open_table(LEXICAL_STATS)?
        .get(definition.name.as_str())?
        .is_some()
    {
        return Ok(());
    }
    let entries = {
        let table = write.open_table(KV)?;
        table
            .iter()?
            .map(|entry| {
                let (key, value) = entry?;
                Ok((key.value().to_vec(), value.value().to_vec()))
            })
            .collect::<Result<Vec<_>, redb::StorageError>>()?
    };
    let mut corpus = empty_lexical_corpus(definition.fields.len());
    for (key, encoded) in entries {
        let value = decode_document(&encoded)?;
        let projection = project_lexical_document(&value, definition);
        add_lexical_document(write, definition, &key, &projection, &mut corpus)?;
    }
    let encoded = encode_lexical_corpus(&corpus)?;
    write
        .open_table(LEXICAL_STATS)?
        .insert(definition.name.as_str(), encoded.as_slice())?;
    Ok(())
}

fn update_lexical_projections(
    write: &redb::WriteTransaction,
    key: &[u8],
    encoded_document: Option<&[u8]>,
) -> Result<(), MaterializedIndexError> {
    let definitions = {
        let table = write.open_table(LEXICAL_INDEXES)?;
        table
            .iter()?
            .map(|entry| {
                let (name, value) = entry?;
                let name = VectorSpaceName::new(name.value().to_owned())?;
                decode_lexical_index_value(&name, value.value())
            })
            .collect::<Result<Vec<_>, MaterializedIndexError>>()?
    };
    if definitions.is_empty() {
        return Ok(());
    }
    let decoded = encoded_document.map(decode_document).transpose()?;
    for definition in definitions {
        let encoded_key = encode_lexical_document_key(&definition.name, key)?;
        let existing = {
            let table = write.open_table(LEXICAL_DOCUMENTS)?;
            table
                .get(encoded_key.as_slice())?
                .map(|value| decode_lexical_document(value.value(), definition.fields.len()))
                .transpose()?
        };
        let mut corpus = {
            let table = write.open_table(LEXICAL_STATS)?;
            let encoded = table
                .get(definition.name.as_str())?
                .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
            decode_lexical_corpus(encoded.value(), definition.fields.len())?
        };
        if let Some(existing) = existing {
            remove_lexical_document(write, &definition, key, &existing, &mut corpus)?;
        }
        if let Some(value) = &decoded {
            let projection = project_lexical_document(value, &definition);
            add_lexical_document(write, &definition, key, &projection, &mut corpus)?;
        }
        let encoded = encode_lexical_corpus(&corpus)?;
        write
            .open_table(LEXICAL_STATS)?
            .insert(definition.name.as_str(), encoded.as_slice())?;
    }
    Ok(())
}

fn project_lexical_document(
    value: &Value,
    definition: &LexicalIndexDefinition,
) -> LexicalDocumentProjection {
    let fields = definition
        .fields
        .iter()
        .map(|field| match field.path.resolve(value) {
            Some(Value::String(value)) => tokenize_v1(value),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();
    let field_lengths = fields
        .iter()
        .map(|tokens| u64::try_from(tokens.len()).unwrap_or(u64::MAX))
        .collect();
    let mut terms = BTreeMap::<String, Vec<u64>>::new();
    for (field_index, tokens) in fields.iter().enumerate() {
        for token in tokens {
            let frequencies = terms
                .entry(token.clone())
                .or_insert_with(|| vec![0; definition.fields.len()]);
            frequencies[field_index] = frequencies[field_index].saturating_add(1);
        }
    }
    LexicalDocumentProjection {
        field_lengths,
        terms,
    }
}

fn add_lexical_document(
    write: &redb::WriteTransaction,
    definition: &LexicalIndexDefinition,
    key: &[u8],
    projection: &LexicalDocumentProjection,
    corpus: &mut LexicalCorpusProjection,
) -> Result<(), MaterializedIndexError> {
    corpus.document_count = corpus
        .document_count
        .checked_add(1)
        .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
    for (total, length) in corpus
        .total_field_lengths
        .iter_mut()
        .zip(&projection.field_lengths)
    {
        *total = total
            .checked_add(*length)
            .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
        corpus.token_count = corpus
            .token_count
            .checked_add(*length)
            .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
    }
    {
        let mut postings = write.open_table(LEXICAL_POSTINGS)?;
        for term in projection.terms.keys() {
            let posting_key = encode_lexical_posting_key(&definition.name, term, key)?;
            postings.insert(posting_key.as_slice(), [1_u8].as_slice())?;
        }
    }
    let encoded_key = encode_lexical_document_key(&definition.name, key)?;
    let encoded_projection = encode_lexical_document(projection)?;
    write
        .open_table(LEXICAL_DOCUMENTS)?
        .insert(encoded_key.as_slice(), encoded_projection.as_slice())?;
    Ok(())
}

fn remove_lexical_document(
    write: &redb::WriteTransaction,
    definition: &LexicalIndexDefinition,
    key: &[u8],
    projection: &LexicalDocumentProjection,
    corpus: &mut LexicalCorpusProjection,
) -> Result<(), MaterializedIndexError> {
    corpus.document_count = corpus
        .document_count
        .checked_sub(1)
        .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
    for (total, length) in corpus
        .total_field_lengths
        .iter_mut()
        .zip(&projection.field_lengths)
    {
        *total = total
            .checked_sub(*length)
            .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
        corpus.token_count = corpus
            .token_count
            .checked_sub(*length)
            .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
    }
    {
        let mut postings = write.open_table(LEXICAL_POSTINGS)?;
        for term in projection.terms.keys() {
            let posting_key = encode_lexical_posting_key(&definition.name, term, key)?;
            if postings.remove(posting_key.as_slice())?.is_none() {
                return Err(MaterializedIndexError::MalformedLexicalProjection);
            }
        }
    }
    let encoded_key = encode_lexical_document_key(&definition.name, key)?;
    if write
        .open_table(LEXICAL_DOCUMENTS)?
        .remove(encoded_key.as_slice())?
        .is_none()
    {
        return Err(MaterializedIndexError::MalformedLexicalProjection);
    }
    Ok(())
}

fn empty_lexical_corpus(field_count: usize) -> LexicalCorpusProjection {
    LexicalCorpusProjection {
        document_count: 0,
        token_count: 0,
        total_field_lengths: vec![0; field_count],
    }
}

fn encode_lexical_document(
    projection: &LexicalDocumentProjection,
) -> Result<Vec<u8>, MaterializedIndexError> {
    let field_count = u8::try_from(projection.field_lengths.len())
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
    let term_count = u32::try_from(projection.terms.len())
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
    let mut encoded = vec![1, field_count];
    for length in &projection.field_lengths {
        encoded.extend_from_slice(&length.to_le_bytes());
    }
    encoded.extend_from_slice(&term_count.to_le_bytes());
    for (term, frequencies) in &projection.terms {
        if frequencies.len() != projection.field_lengths.len() {
            return Err(MaterializedIndexError::MalformedLexicalProjection);
        }
        let length = u16::try_from(term.len())
            .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
        encoded.extend_from_slice(&length.to_le_bytes());
        encoded.extend_from_slice(term.as_bytes());
        for frequency in frequencies {
            encoded.extend_from_slice(&frequency.to_le_bytes());
        }
    }
    Ok(encoded)
}

fn decode_lexical_document(
    encoded: &[u8],
    expected_fields: usize,
) -> Result<LexicalDocumentProjection, MaterializedIndexError> {
    if encoded.first() != Some(&1)
        || encoded.get(1).map(|value| usize::from(*value)) != Some(expected_fields)
    {
        return Err(MaterializedIndexError::MalformedLexicalProjection);
    }
    let mut cursor = 2_usize;
    let mut field_lengths = Vec::with_capacity(expected_fields);
    for _ in 0..expected_fields {
        field_lengths.push(read_u64(encoded, &mut cursor)?);
    }
    let term_count = usize::try_from(read_u32(encoded, &mut cursor)?)
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
    let mut terms = BTreeMap::new();
    let mut previous: Option<String> = None;
    for _ in 0..term_count {
        let term_length = usize::from(read_u16(encoded, &mut cursor)?);
        let end = cursor
            .checked_add(term_length)
            .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
        let term = std::str::from_utf8(
            encoded
                .get(cursor..end)
                .ok_or(MaterializedIndexError::MalformedLexicalProjection)?,
        )
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?
        .to_owned();
        cursor = end;
        if term.is_empty() || previous.as_ref().is_some_and(|previous| previous >= &term) {
            return Err(MaterializedIndexError::MalformedLexicalProjection);
        }
        let frequencies = (0..expected_fields)
            .map(|_| read_u64(encoded, &mut cursor))
            .collect::<Result<Vec<_>, _>>()?;
        if frequencies.iter().all(|frequency| *frequency == 0) {
            return Err(MaterializedIndexError::MalformedLexicalProjection);
        }
        previous = Some(term.clone());
        terms.insert(term, frequencies);
    }
    if cursor != encoded.len() {
        return Err(MaterializedIndexError::MalformedLexicalProjection);
    }
    Ok(LexicalDocumentProjection {
        field_lengths,
        terms,
    })
}

fn encode_lexical_corpus(
    corpus: &LexicalCorpusProjection,
) -> Result<Vec<u8>, MaterializedIndexError> {
    let field_count = u8::try_from(corpus.total_field_lengths.len())
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
    let mut encoded = vec![1, field_count];
    encoded.extend_from_slice(&corpus.document_count.to_le_bytes());
    encoded.extend_from_slice(&corpus.token_count.to_le_bytes());
    for length in &corpus.total_field_lengths {
        encoded.extend_from_slice(&length.to_le_bytes());
    }
    Ok(encoded)
}

fn decode_lexical_corpus(
    encoded: &[u8],
    expected_fields: usize,
) -> Result<LexicalCorpusProjection, MaterializedIndexError> {
    if encoded.first() != Some(&1)
        || encoded.get(1).map(|value| usize::from(*value)) != Some(expected_fields)
    {
        return Err(MaterializedIndexError::MalformedLexicalProjection);
    }
    let mut cursor = 2_usize;
    let document_count = read_u64(encoded, &mut cursor)?;
    let token_count = read_u64(encoded, &mut cursor)?;
    let total_field_lengths = (0..expected_fields)
        .map(|_| read_u64(encoded, &mut cursor))
        .collect::<Result<Vec<_>, _>>()?;
    if cursor != encoded.len()
        || total_field_lengths
            .iter()
            .try_fold(0_u64, |sum, value| sum.checked_add(*value))
            != Some(token_count)
    {
        return Err(MaterializedIndexError::MalformedLexicalProjection);
    }
    Ok(LexicalCorpusProjection {
        document_count,
        token_count,
        total_field_lengths,
    })
}

fn encode_lexical_document_key(
    name: &VectorSpaceName,
    key: &[u8],
) -> Result<Vec<u8>, MaterializedIndexError> {
    if key.is_empty() {
        return Err(MaterializedIndexError::MalformedLexicalProjection);
    }
    let name_length = u8::try_from(name.as_str().len())
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
    let mut encoded = Vec::with_capacity(1 + name.as_str().len() + key.len());
    encoded.push(name_length);
    encoded.extend_from_slice(name.as_str().as_bytes());
    encoded.extend_from_slice(key);
    Ok(encoded)
}

fn encode_lexical_posting_prefix(
    name: &VectorSpaceName,
    term: &str,
) -> Result<Vec<u8>, MaterializedIndexError> {
    let name_length = u8::try_from(name.as_str().len())
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
    let term_length = u16::try_from(term.len())
        .map_err(|_| MaterializedIndexError::MalformedLexicalProjection)?;
    let mut encoded = Vec::with_capacity(3 + name.as_str().len() + term.len());
    encoded.push(name_length);
    encoded.extend_from_slice(name.as_str().as_bytes());
    encoded.extend_from_slice(&term_length.to_be_bytes());
    encoded.extend_from_slice(term.as_bytes());
    Ok(encoded)
}

fn encode_lexical_posting_key(
    name: &VectorSpaceName,
    term: &str,
    key: &[u8],
) -> Result<Vec<u8>, MaterializedIndexError> {
    if key.is_empty() {
        return Err(MaterializedIndexError::MalformedLexicalProjection);
    }
    let mut encoded = encode_lexical_posting_prefix(name, term)?;
    encoded.extend_from_slice(key);
    Ok(encoded)
}

fn decode_lexical_posting_key(
    encoded: &[u8],
    prefix: &[u8],
) -> Result<Vec<u8>, MaterializedIndexError> {
    let key = encoded
        .strip_prefix(prefix)
        .filter(|key| !key.is_empty())
        .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
    Ok(key.to_vec())
}

fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut upper = prefix.to_vec();
    for index in (0..upper.len()).rev() {
        if upper[index] != u8::MAX {
            upper[index] = upper[index].saturating_add(1);
            upper.truncate(index + 1);
            return Some(upper);
        }
    }
    None
}

fn read_u16(encoded: &[u8], cursor: &mut usize) -> Result<u16, MaterializedIndexError> {
    read_array(encoded, cursor).map(u16::from_le_bytes)
}

fn read_u32(encoded: &[u8], cursor: &mut usize) -> Result<u32, MaterializedIndexError> {
    read_array(encoded, cursor).map(u32::from_le_bytes)
}

fn read_u64(encoded: &[u8], cursor: &mut usize) -> Result<u64, MaterializedIndexError> {
    read_array(encoded, cursor).map(u64::from_le_bytes)
}

fn read_array<const N: usize>(
    encoded: &[u8],
    cursor: &mut usize,
) -> Result<[u8; N], MaterializedIndexError> {
    let end = cursor
        .checked_add(N)
        .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
    let bytes = encoded
        .get(*cursor..end)
        .ok_or(MaterializedIndexError::MalformedLexicalProjection)?;
    *cursor = end;
    Ok(copy_array(bytes))
}

fn require_vector_space(
    write: &redb::WriteTransaction,
    name: &VectorSpaceName,
) -> Result<VectorSpaceDefinition, MaterializedIndexError> {
    let encoded = {
        let table = write.open_table(VECTOR_SPACES)?;
        table
            .get(name.as_str())?
            .map(|value| value.value().to_vec())
    };
    let Some(encoded) = encoded else {
        return Err(MaterializedIndexError::UnknownVectorSpace {
            name: name.as_str().to_owned(),
        });
    };
    decode_vector_space_value(name, &encoded)
}

fn encode_vector_space_value(definition: &VectorSpaceDefinition) -> [u8; 4] {
    let dimension = definition.dimension.to_le_bytes();
    [dimension[0], dimension[1], definition.metric as u8, 1]
}

fn decode_vector_space_value(
    name: &VectorSpaceName,
    encoded: &[u8],
) -> Result<VectorSpaceDefinition, MaterializedIndexError> {
    if encoded.len() != 4 || encoded[2] != VectorMetric::Cosine as u8 || encoded[3] != 1 {
        return Err(MaterializedIndexError::MalformedVectorIndex);
    }
    let dimension = u16::from_le_bytes(copy_array(&encoded[..2]));
    Ok(VectorSpaceDefinition::cosine(name.clone(), dimension)?)
}

fn encode_lexical_index_value(
    definition: &LexicalIndexDefinition,
) -> Result<Vec<u8>, MaterializedIndexError> {
    let field_count = u8::try_from(definition.fields.len())
        .map_err(|_| MaterializedIndexError::MalformedLexicalIndex)?;
    let mut encoded = vec![1, field_count];
    for field in &definition.fields {
        let segment_count = u8::try_from(field.path.segments().len())
            .map_err(|_| MaterializedIndexError::MalformedLexicalIndex)?;
        encoded.push(segment_count);
        for segment in field.path.segments() {
            let length = u16::try_from(segment.len())
                .map_err(|_| MaterializedIndexError::MalformedLexicalIndex)?;
            encoded.extend_from_slice(&length.to_le_bytes());
            encoded.extend_from_slice(segment.as_bytes());
        }
        encoded.extend_from_slice(&field.weight_micros.to_le_bytes());
    }
    Ok(encoded)
}

fn decode_lexical_index_value(
    name: &VectorSpaceName,
    encoded: &[u8],
) -> Result<LexicalIndexDefinition, MaterializedIndexError> {
    if encoded.first() != Some(&1) {
        return Err(MaterializedIndexError::MalformedLexicalIndex);
    }
    let field_count = usize::from(
        *encoded
            .get(1)
            .ok_or(MaterializedIndexError::MalformedLexicalIndex)?,
    );
    let mut cursor = 2_usize;
    let mut fields = Vec::with_capacity(field_count);
    for _ in 0..field_count {
        let segment_count = usize::from(
            *encoded
                .get(cursor)
                .ok_or(MaterializedIndexError::MalformedLexicalIndex)?,
        );
        cursor = cursor
            .checked_add(1)
            .ok_or(MaterializedIndexError::MalformedLexicalIndex)?;
        let mut segments = Vec::with_capacity(segment_count);
        for _ in 0..segment_count {
            let length_end = cursor
                .checked_add(2)
                .ok_or(MaterializedIndexError::MalformedLexicalIndex)?;
            let length = usize::from(u16::from_le_bytes(copy_array(
                encoded
                    .get(cursor..length_end)
                    .ok_or(MaterializedIndexError::MalformedLexicalIndex)?,
            )));
            cursor = length_end;
            let segment_end = cursor
                .checked_add(length)
                .ok_or(MaterializedIndexError::MalformedLexicalIndex)?;
            let segment = std::str::from_utf8(
                encoded
                    .get(cursor..segment_end)
                    .ok_or(MaterializedIndexError::MalformedLexicalIndex)?,
            )
            .map_err(|_| MaterializedIndexError::MalformedLexicalIndex)?
            .to_owned();
            cursor = segment_end;
            segments.push(segment);
        }
        let weight_end = cursor
            .checked_add(4)
            .ok_or(MaterializedIndexError::MalformedLexicalIndex)?;
        let weight_micros = u32::from_le_bytes(copy_array(
            encoded
                .get(cursor..weight_end)
                .ok_or(MaterializedIndexError::MalformedLexicalIndex)?,
        ));
        cursor = weight_end;
        fields.push(LexicalField {
            path: FieldPath::new(segments),
            weight_micros,
        });
    }
    if cursor != encoded.len() {
        return Err(MaterializedIndexError::MalformedLexicalIndex);
    }
    LexicalIndexDefinition::new(name.clone(), fields).map_err(MaterializedIndexError::from)
}

fn encode_vector_key(space: &VectorSpaceName, key: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(space.as_str().len() + 1 + key.len());
    encoded.extend_from_slice(space.as_str().as_bytes());
    encoded.push(0);
    encoded.extend_from_slice(key);
    encoded
}

fn decode_vector_key(encoded: &[u8]) -> Result<(VectorSpaceName, Vec<u8>), MaterializedIndexError> {
    let space_end = encoded
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(MaterializedIndexError::MalformedVectorIndex)?;
    let space = encoded
        .get(..space_end)
        .filter(|space| !space.is_empty())
        .ok_or(MaterializedIndexError::MalformedVectorIndex)?;
    let key = encoded
        .get(space_end + 1..)
        .filter(|key| !key.is_empty())
        .ok_or(MaterializedIndexError::MalformedVectorIndex)?;
    let space =
        std::str::from_utf8(space).map_err(|_| MaterializedIndexError::MalformedVectorIndex)?;
    Ok((VectorSpaceName::new(space.to_owned())?, key.to_vec()))
}

fn decode_vector_key_for_space(
    encoded: &[u8],
    expected: &VectorSpaceName,
) -> Result<Option<Vec<u8>>, MaterializedIndexError> {
    let (space, key) = decode_vector_key(encoded)?;
    Ok((space == *expected).then_some(key))
}

fn encode_vector_value(vector: &Q15Vector) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(2 + 2 * vector.as_slice().len());
    encoded.extend_from_slice(&vector.dimension().to_le_bytes());
    for value in vector.as_slice() {
        encoded.extend_from_slice(&value.to_le_bytes());
    }
    encoded
}

fn decode_vector_value(encoded: &[u8]) -> Result<Q15Vector, MaterializedIndexError> {
    let dimension_bytes = encoded
        .get(..2)
        .ok_or(MaterializedIndexError::MalformedVectorIndex)?;
    let dimension = usize::from(u16::from_le_bytes(copy_array(dimension_bytes)));
    let expected_length = dimension
        .checked_mul(2)
        .and_then(|length| length.checked_add(2))
        .ok_or(MaterializedIndexError::MalformedVectorIndex)?;
    if encoded.len() != expected_length {
        return Err(MaterializedIndexError::MalformedVectorIndex);
    }
    let mut values = Vec::with_capacity(dimension);
    for chunk in encoded[2..].chunks_exact(2) {
        values.push(i16::from_le_bytes(copy_array(chunk)));
    }
    Ok(Q15Vector::new(values)?)
}

struct IndexRestoreVisitor<'transaction> {
    write: &'transaction mut redb::WriteTransaction,
}

impl SnapshotRecordVisitor for IndexRestoreVisitor<'_> {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), SnapshotError> {
        update_lexical_projections(self.write, key, Some(value)).map_err(SnapshotError::from)?;
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

    fn vector_space(&mut self, definition: &VectorSpaceDefinition) -> Result<(), SnapshotError> {
        let mut table = self
            .write
            .open_table(VECTOR_SPACES)
            .map_err(MaterializedIndexError::from)?;
        let encoded = encode_vector_space_value(definition);
        table
            .insert(definition.name.as_str(), encoded.as_slice())
            .map_err(MaterializedIndexError::from)?;
        Ok(())
    }

    fn lexical_index(&mut self, definition: &LexicalIndexDefinition) -> Result<(), SnapshotError> {
        apply_lexical_index_definition(self.write, definition).map_err(SnapshotError::from)?;
        build_lexical_projection(self.write, definition).map_err(SnapshotError::from)?;
        Ok(())
    }

    fn vector(
        &mut self,
        space: &VectorSpaceName,
        key: &[u8],
        vector: &Q15Vector,
    ) -> Result<(), SnapshotError> {
        let mut table = self
            .write
            .open_table(VECTORS)
            .map_err(MaterializedIndexError::from)?;
        let encoded_key = encode_vector_key(space, key);
        let encoded_vector = encode_vector_value(vector);
        table
            .insert(encoded_key.as_slice(), encoded_vector.as_slice())
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
