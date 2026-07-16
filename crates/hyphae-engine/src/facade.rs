// SPDX-License-Identifier: Apache-2.0

use std::{collections::BTreeSet, path::Path, time::Instant};

use hyphae_query::{
    ExecutionLimits, Query, QueryError, QueryResult, Record, execute, validate_query,
};
use hyphae_retrieval::{
    RetrievalError, RetrievalLimits, RetrievalOutcome, RetrievalRequest, VectorRecord, retrieve,
};
use hyphae_storage::{
    AppendOutcome, BackupError, BackupInfo, CompactionOutcome, MAX_SCAN_PAGE_ENTRIES, Mutation,
    RestoreInfo, SnapshotInfo, StorageEngine, StorageError, StorageRecoveryReport, restore_backup,
    verify_backup,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    DocumentError, ProofError, ResultProof, ResultProofArtifact, decode_document, encode_document,
};

/// Failure while operating the embeddable Hyphae facade.
#[derive(Debug, Error)]
pub enum EngineError {
    /// Durable embedded storage failed.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// Portable backup creation, verification, or restore failed.
    #[error(transparent)]
    Backup(#[from] BackupError),

    /// Canonical document encoding or verification failed.
    #[error(transparent)]
    Document(#[from] DocumentError),

    /// Structured query validation or execution failed.
    #[error(transparent)]
    Query(#[from] QueryError),

    /// Exact semantic retrieval failed.
    #[error(transparent)]
    Retrieval(#[from] RetrievalError),

    /// Canonical result-proof creation failed.
    #[error(transparent)]
    Proof(#[from] ProofError),

    /// One atomic document batch repeats a key.
    #[error("atomic document batch contains a duplicate key")]
    DuplicateDocumentKey,
}

/// Newly opened embeddable engine and durable recovery evidence.
#[derive(Debug)]
pub struct OpenedEngine {
    /// Ready engine facade.
    pub engine: HyphaeEngine,
    /// Log verification and index replay evidence.
    pub recovery: StorageRecoveryReport,
}

/// Embeddable autonomous Hyphae engine.
#[derive(Debug)]
pub struct HyphaeEngine {
    storage: StorageEngine,
}

impl HyphaeEngine {
    /// Opens one exclusively owned data directory and completes recovery.
    ///
    /// # Errors
    ///
    /// Returns an error for directory contention, corruption, unsupported
    /// formats, snapshot mismatch, or failed index replay.
    pub fn open(path: impl AsRef<Path>) -> Result<OpenedEngine, EngineError> {
        let opened = StorageEngine::open(path)?;
        Ok(OpenedEngine {
            engine: Self {
                storage: opened.storage,
            },
            recovery: opened.recovery,
        })
    }

    /// Returns the owned data-directory path.
    pub fn data_path(&self) -> &Path {
        self.storage.data_path()
    }

    /// Atomically stores one canonical structured record.
    ///
    /// # Errors
    ///
    /// Returns a document codec or durable storage error.
    pub fn put_record(
        &mut self,
        transaction_id: Uuid,
        record: &Record,
    ) -> Result<AppendOutcome, EngineError> {
        self.put_records(transaction_id, std::slice::from_ref(record))
    }

    /// Atomically stores a batch of canonical structured records.
    ///
    /// Encoding every document and checking duplicate keys happens before the
    /// log append, so a codec failure cannot partially commit the batch.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate batch keys, document bounds, key bounds,
    /// idempotency conflicts, or durable storage failures.
    pub fn put_records(
        &mut self,
        transaction_id: Uuid,
        records: &[Record],
    ) -> Result<AppendOutcome, EngineError> {
        let mut keys = BTreeSet::new();
        let mut mutations = Vec::with_capacity(records.len());
        for record in records {
            if !keys.insert(record.key.as_slice()) {
                return Err(EngineError::DuplicateDocumentKey);
            }
            mutations.push(Mutation::put(
                record.key.clone(),
                encode_document(&record.value)?,
            ));
        }
        Ok(self.storage.write(transaction_id, &mutations)?)
    }

    /// Atomically deletes one structured record.
    ///
    /// # Errors
    ///
    /// Returns a key-validation, idempotency, or durable storage error.
    pub fn delete_record(
        &mut self,
        transaction_id: Uuid,
        key: &[u8],
    ) -> Result<AppendOutcome, EngineError> {
        self.delete_records(transaction_id, &[key])
    }

    /// Atomically deletes a batch of structured records.
    ///
    /// Duplicate keys are rejected before the log append. Deleting a missing
    /// key remains a successful durable operation.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate keys, invalid key bounds, idempotency
    /// conflicts, or durable storage failures.
    pub fn delete_records(
        &mut self,
        transaction_id: Uuid,
        keys: &[&[u8]],
    ) -> Result<AppendOutcome, EngineError> {
        let mut unique = BTreeSet::new();
        let mut mutations = Vec::with_capacity(keys.len());
        for key in keys {
            if !unique.insert(*key) {
                return Err(EngineError::DuplicateDocumentKey);
            }
            mutations.push(Mutation::delete(*key));
        }
        Ok(self.storage.write(transaction_id, &mutations)?)
    }

    /// Gets and verifies one structured record by binary key.
    ///
    /// # Errors
    ///
    /// Returns a key, storage, or canonical document verification error.
    pub fn get_record(&self, key: &[u8]) -> Result<Option<Record>, EngineError> {
        self.storage
            .get(key)?
            .map(|encoded| {
                Ok(Record {
                    key: key.to_vec(),
                    value: decode_document(&encoded)?,
                })
            })
            .transpose()
    }

    /// Gets one structured record and binds the complete result, including
    /// absence, to a canonical snapshot witness.
    ///
    /// # Errors
    ///
    /// Returns a key, storage, document, snapshot, or result-proof error.
    pub fn get_record_with_proof(&self, key: &[u8]) -> Result<ResultProofArtifact, EngineError> {
        let result = self.get_record(key)?;
        let snapshot = self.snapshot()?;
        let proof = ResultProof::for_get(&snapshot, key.to_vec(), result)?;
        Ok(ResultProofArtifact { proof, snapshot })
    }

    /// Executes deterministic structured query over all durable documents.
    ///
    /// Storage scan and document decoding consume the same wall-clock timeout;
    /// the reference executor receives only the remaining duration.
    ///
    /// # Errors
    ///
    /// Returns a storage, document, query validation, global budget, aggregate,
    /// or timeout error. No partial page is returned.
    pub fn query(
        &self,
        query: &Query,
        limits: &ExecutionLimits,
    ) -> Result<QueryResult, EngineError> {
        validate_query(query, limits)?;
        let started = Instant::now();
        let mut records = Vec::new();
        let mut after = None;
        loop {
            if started.elapsed() >= limits.timeout {
                return Err(QueryError::TimedOut.into());
            }
            let loaded = u64::try_from(records.len()).unwrap_or(u64::MAX);
            let remaining = limits.max_scanned_records.saturating_sub(loaded);
            let remaining_entries = match usize::try_from(remaining) {
                Ok(value) => value,
                Err(_) => usize::MAX,
            };
            let page_limit = remaining_entries
                .saturating_add(1)
                .min(MAX_SCAN_PAGE_ENTRIES);
            let page = self.storage.scan_page(after.as_deref(), page_limit)?;
            for entry in page.entries {
                if u64::try_from(records.len()).unwrap_or(u64::MAX) >= limits.max_scanned_records {
                    return Err(QueryError::ScannedBudgetExceeded {
                        maximum: limits.max_scanned_records,
                    }
                    .into());
                }
                records.push(Record {
                    key: entry.key,
                    value: decode_document(&entry.value)?,
                });
            }
            let Some(next_after) = page.next_after else {
                break;
            };
            after = Some(next_after);
        }

        let elapsed = started.elapsed();
        let Some(timeout) = limits.timeout.checked_sub(elapsed) else {
            return Err(QueryError::TimedOut.into());
        };
        if timeout.is_zero() {
            return Err(QueryError::TimedOut.into());
        }
        let execution_limits = ExecutionLimits {
            timeout,
            ..limits.clone()
        };
        Ok(execute(&[records.as_slice()], query, &execution_limits)?)
    }

    /// Executes one structured query and binds its complete logical result to
    /// a canonical snapshot witness at the same locked checkpoint.
    ///
    /// # Errors
    ///
    /// Returns any ordinary query error plus snapshot or proof creation
    /// failures. No proof is returned for a partial or failed query.
    pub fn query_with_proof(
        &self,
        query: &Query,
        limits: &ExecutionLimits,
    ) -> Result<ResultProofArtifact, EngineError> {
        let result = self.query(query, limits)?;
        let snapshot = self.snapshot()?;
        let proof = ResultProof::for_query(&snapshot, query.clone(), result)?;
        Ok(ResultProofArtifact { proof, snapshot })
    }

    /// Executes exact provider-neutral vector retrieval without persisting or
    /// producing embeddings.
    ///
    /// # Errors
    ///
    /// Returns vector, shape, duplicate-key, budget, or timeout errors.
    pub fn retrieve_vectors(
        shards: &[&[VectorRecord]],
        request: &RetrievalRequest,
        limits: &RetrievalLimits,
    ) -> Result<RetrievalOutcome, EngineError> {
        Ok(retrieve(shards, request, limits)?)
    }

    /// Creates or reuses a verified logical snapshot.
    ///
    /// # Errors
    ///
    /// Returns a stale-handle, index, or snapshot error.
    pub fn snapshot(&self) -> Result<SnapshotInfo, EngineError> {
        Ok(self.storage.snapshot()?)
    }

    /// Commits an anchored compaction generation.
    ///
    /// # Errors
    ///
    /// Returns a stale-handle, snapshot, segment, or manifest error.
    pub fn compact(&mut self) -> Result<CompactionOutcome, EngineError> {
        Ok(self.storage.compact()?)
    }

    /// Creates an atomic portable backup at the locked logical checkpoint.
    ///
    /// # Errors
    ///
    /// Returns a snapshot, destination, synchronization, or promotion error.
    pub fn backup(&self, destination: impl AsRef<Path>) -> Result<BackupInfo, EngineError> {
        Ok(self.storage.backup(destination)?)
    }

    /// Verifies a portable backup without opening a live data directory.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed layout, metadata mismatch, or corrupt
    /// snapshot.
    pub fn verify_backup(path: impl AsRef<Path>) -> Result<BackupInfo, EngineError> {
        Ok(verify_backup(path)?)
    }

    /// Restores a backup to a new atomically activated data directory.
    ///
    /// # Errors
    ///
    /// Returns an error before destination activation if verification, index
    /// reconstruction, reopen, or filesystem synchronization fails.
    pub fn restore_backup(
        backup: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<RestoreInfo, EngineError> {
        Ok(restore_backup(backup, destination)?)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::PathBuf};

    use hyphae_query::{
        AggregationPlan, CompareOperator, FieldPath, Filter, Metric, MetricValue, NamedMetric,
        NullPlacement, SortDirection, SortField, Value,
    };
    use uuid::Uuid;

    use super::{EngineError, ExecutionLimits, HyphaeEngine, Query, Record};

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> std::io::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "hyphae-engine-{name}-{}-{}",
                std::process::id(),
                Uuid::now_v7()
            ));
            fs::create_dir_all(&path)?;
            Ok(Self { path })
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }

    fn value(score: i64, group: &str) -> Value {
        Value::Object(BTreeMap::from([
            ("group".to_owned(), Value::String(group.to_owned())),
            ("score".to_owned(), Value::Integer(score)),
        ]))
    }

    #[test]
    fn durable_documents_query_identically_after_compaction_and_reopen()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = TestDirectory::new("engine-query-reopen")?;
        let root = temporary.path().join("data");
        let mut opened = HyphaeEngine::open(&root)?;
        opened.engine.put_records(
            Uuid::now_v7(),
            &[
                Record::new(b"a", value(10, "x")),
                Record::new(b"b", value(8, "x")),
                Record::new(b"c", value(7, "y")),
                Record::new(b"d", value(2, "y")),
            ],
        )?;
        let request = Query {
            filter: Filter::Compare {
                path: FieldPath::field("score"),
                operator: CompareOperator::GreaterOrEqual,
                value: Value::Integer(7),
            },
            sort: vec![SortField {
                path: FieldPath::field("score"),
                direction: SortDirection::Descending,
                nulls: NullPlacement::Last,
            }],
            cursor: None,
            limit: 2,
            aggregation: Some(AggregationPlan {
                group_by: Vec::new(),
                metrics: vec![NamedMetric {
                    name: "count".to_owned(),
                    metric: Metric::Count,
                }],
            }),
        };
        let before = opened.engine.query(&request, &ExecutionLimits::default())?;
        assert_eq!(before.rows.len(), 2);
        assert_eq!(
            before
                .aggregation
                .as_ref()
                .map(|aggregation| { aggregation.groups[0].metrics[0].value.clone() }),
            Some(MetricValue::Count(3))
        );
        opened.engine.compact()?;
        drop(opened);

        let reopened = HyphaeEngine::open(&root)?;
        let after = reopened
            .engine
            .query(&request, &ExecutionLimits::default())?;
        assert_eq!(before, after);
        assert_eq!(
            reopened.engine.get_record(b"a")?.map(|record| record.value),
            Some(value(10, "x"))
        );
        Ok(())
    }

    #[test]
    fn facade_enforces_scan_budget_before_building_a_partial_page()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = TestDirectory::new("engine-query-budget")?;
        let mut opened = HyphaeEngine::open(temporary.path().join("data"))?;
        opened.engine.put_records(
            Uuid::now_v7(),
            &[
                Record::new(b"a", Value::Null),
                Record::new(b"b", Value::Null),
            ],
        )?;
        let limits = ExecutionLimits {
            max_scanned_records: 1,
            ..ExecutionLimits::default()
        };
        let result = opened.engine.query(
            &Query {
                filter: Filter::MatchAll,
                sort: Vec::new(),
                cursor: None,
                limit: 1,
                aggregation: None,
            },
            &limits,
        );
        assert!(matches!(
            result,
            Err(EngineError::Query(
                hyphae_query::QueryError::ScannedBudgetExceeded { maximum: 1 }
            ))
        ));
        Ok(())
    }
}
