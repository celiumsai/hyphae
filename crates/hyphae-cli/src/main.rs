// SPDX-License-Identifier: Apache-2.0

//! Command-line entry point for the single Hyphae executable.

mod json_value;

use std::{
    error::Error,
    io::{BufWriter, Write, stdout},
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};
use hyphae_core::current_version;
use hyphae_engine::HyphaeEngine;
use hyphae_query::{
    Cursor, ExecutionLimits, FieldPath, Filter, MetricValue, NullPlacement, Query, Record,
    SortDirection, SortField,
};
use hyphae_storage::{AppendOutcome, CommitReceipt, CompactionOutcome, SnapshotInfo};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::json_value::{encode_hex, parse_json, to_json};

#[derive(Debug, Error, Eq, PartialEq)]
enum CliError {
    #[error("field path must contain nonempty dot-separated segments")]
    InvalidFieldPath,
}

#[derive(Debug, Parser)]
#[command(
    name = "hyphae",
    version,
    about = "Autonomous, embeddable, and verifiable data engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print independently versioned product surfaces.
    Version {
        /// Emit a machine-readable JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Atomically store one structured JSON document.
    Put {
        /// Owned Hyphae data directory.
        #[arg(long, env = "HYPHAE_DATA_DIR")]
        data_dir: PathBuf,
        /// UTF-8 key; the Rust API also accepts arbitrary binary keys.
        #[arg(long)]
        key: String,
        /// JSON value. Numbers must be signed 64-bit integers.
        #[arg(long = "json")]
        value: String,
        /// Optional idempotency UUID; defaults to a new `UUIDv7`.
        #[arg(long)]
        transaction_id: Option<Uuid>,
    },
    /// Read and verify one structured document.
    Get {
        /// Owned Hyphae data directory.
        #[arg(long, env = "HYPHAE_DATA_DIR")]
        data_dir: PathBuf,
        /// UTF-8 key.
        #[arg(long)]
        key: String,
    },
    /// Atomically delete one structured document.
    Delete {
        /// Owned Hyphae data directory.
        #[arg(long, env = "HYPHAE_DATA_DIR")]
        data_dir: PathBuf,
        /// UTF-8 key.
        #[arg(long)]
        key: String,
        /// Optional idempotency UUID; defaults to a new `UUIDv7`.
        #[arg(long)]
        transaction_id: Option<Uuid>,
    },
    /// Execute deterministic structured query without AI.
    Query {
        /// Owned Hyphae data directory.
        #[arg(long, env = "HYPHAE_DATA_DIR")]
        data_dir: PathBuf,
        /// Dot-separated exact object path for equality filtering.
        #[arg(long, requires = "equals")]
        field: Option<String>,
        /// JSON equality literal; requires `--field`.
        #[arg(long, requires = "field")]
        equals: Option<String>,
        /// Dot-separated exact object path for sorting.
        #[arg(long)]
        sort: Option<String>,
        /// Sort non-null values descending.
        #[arg(long)]
        descending: bool,
        /// Place missing and null before non-null values.
        #[arg(long)]
        nulls_first: bool,
        /// Final page size after global ordering.
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Create or reuse a verified logical snapshot.
    Snapshot {
        /// Owned Hyphae data directory.
        #[arg(long, env = "HYPHAE_DATA_DIR")]
        data_dir: PathBuf,
    },
    /// Commit an anchored compaction generation.
    Compact {
        /// Owned Hyphae data directory.
        #[arg(long, env = "HYPHAE_DATA_DIR")]
        data_dir: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Version { json } => print_version(json),
        Command::Put {
            data_dir,
            key,
            value,
            transaction_id,
        } => put(&data_dir, key, &value, transaction_id),
        Command::Get { data_dir, key } => get(&data_dir, key.as_bytes()),
        Command::Delete {
            data_dir,
            key,
            transaction_id,
        } => delete(&data_dir, key.as_bytes(), transaction_id),
        Command::Query {
            data_dir,
            field,
            equals,
            sort,
            descending,
            nulls_first,
            limit,
        } => query(
            &data_dir,
            QueryArguments {
                field,
                equals,
                sort,
                descending,
                nulls_first,
                limit,
            },
        ),
        Command::Snapshot { data_dir } => snapshot(&data_dir),
        Command::Compact { data_dir } => compact(&data_dir),
    }
}

fn print_version(json_output: bool) -> Result<(), Box<dyn Error>> {
    let version = current_version();
    if json_output {
        print_json(&json!({
            "product": version.product,
            "engine_version": version.engine,
            "api_version": version.api,
            "disk_format_version": version.disk_format,
        }))
    } else {
        let mut output = BufWriter::new(stdout().lock());
        writeln!(
            output,
            "{} {} (api {}, disk format {})",
            version.product, version.engine, version.api, version.disk_format
        )?;
        Ok(())
    }
}

fn put(
    data_dir: &Path,
    key: String,
    encoded_json: &str,
    transaction_id: Option<Uuid>,
) -> Result<(), Box<dyn Error>> {
    let value = parse_json(encoded_json)?;
    let mut opened = HyphaeEngine::open(data_dir)?;
    let transaction_id = transaction_id.unwrap_or_else(Uuid::now_v7);
    let outcome = opened
        .engine
        .put_record(transaction_id, &Record::new(key.into_bytes(), value))?;
    print_json(&receipt_json(outcome))
}

fn get(data_dir: &Path, key: &[u8]) -> Result<(), Box<dyn Error>> {
    let opened = HyphaeEngine::open(data_dir)?;
    let value = opened.engine.get_record(key)?.map(|record| {
        json!({
            "key_hex": encode_hex(&record.key),
            "value": to_json(&record.value),
        })
    });
    print_json(&json!({
        "found": value.is_some(),
        "record": value,
    }))
}

fn delete(data_dir: &Path, key: &[u8], transaction_id: Option<Uuid>) -> Result<(), Box<dyn Error>> {
    let mut opened = HyphaeEngine::open(data_dir)?;
    let outcome = opened
        .engine
        .delete_record(transaction_id.unwrap_or_else(Uuid::now_v7), key)?;
    print_json(&receipt_json(outcome))
}

struct QueryArguments {
    field: Option<String>,
    equals: Option<String>,
    sort: Option<String>,
    descending: bool,
    nulls_first: bool,
    limit: usize,
}

fn query(data_dir: &Path, arguments: QueryArguments) -> Result<(), Box<dyn Error>> {
    let filter = match (arguments.field, arguments.equals) {
        (Some(field), Some(equals)) => Filter::Compare {
            path: parse_field_path(&field)?,
            operator: hyphae_query::CompareOperator::Equal,
            value: parse_json(&equals)?,
        },
        (None, None) => Filter::MatchAll,
        _ => return Err(CliError::InvalidFieldPath.into()),
    };
    let sort = arguments
        .sort
        .map(|field| {
            Ok::<SortField, CliError>(SortField {
                path: parse_field_path(&field)?,
                direction: if arguments.descending {
                    SortDirection::Descending
                } else {
                    SortDirection::Ascending
                },
                nulls: if arguments.nulls_first {
                    NullPlacement::First
                } else {
                    NullPlacement::Last
                },
            })
        })
        .transpose()?
        .into_iter()
        .collect();
    let request = Query {
        filter,
        sort,
        cursor: None,
        limit: arguments.limit,
        aggregation: None,
    };
    let opened = HyphaeEngine::open(data_dir)?;
    let result = opened.engine.query(&request, &ExecutionLimits::default())?;
    let rows = result
        .rows
        .iter()
        .map(|record| {
            json!({
                "key_hex": encode_hex(&record.key),
                "value": to_json(&record.value),
            })
        })
        .collect::<Vec<_>>();
    print_json(&json!({
        "rows": rows,
        "next_cursor": result.next_cursor.as_ref().map(cursor_json),
        "aggregation": result.aggregation.as_ref().map(aggregation_json),
        "scanned_records": result.scanned_records,
        "matched_records": result.matched_records,
    }))
}

fn snapshot(data_dir: &Path) -> Result<(), Box<dyn Error>> {
    let opened = HyphaeEngine::open(data_dir)?;
    print_json(&snapshot_json(&opened.engine.snapshot()?))
}

fn compact(data_dir: &Path) -> Result<(), Box<dyn Error>> {
    let mut opened = HyphaeEngine::open(data_dir)?;
    let value = match opened.engine.compact()? {
        CompactionOutcome::NoChanges { snapshot } => json!({
            "status": "no_changes",
            "snapshot": snapshot_json(&snapshot),
        }),
        CompactionOutcome::Compacted(report) => json!({
            "status": "compacted",
            "generation": report.generation,
            "snapshot": snapshot_json(&report.snapshot),
            "retired_segment": report.retired_segment,
            "retired_segment_removed": report.retired_segment_removed,
        }),
    };
    print_json(&value)
}

fn parse_field_path(path: &str) -> Result<FieldPath, CliError> {
    let segments = path.split('.').collect::<Vec<_>>();
    if segments.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return Err(CliError::InvalidFieldPath);
    }
    Ok(FieldPath::new(segments))
}

fn receipt_json(outcome: AppendOutcome) -> serde_json::Value {
    let (status, receipt) = match outcome {
        AppendOutcome::Committed(receipt) => ("committed", receipt),
        AppendOutcome::Existing(receipt) => ("existing", receipt),
    };
    commit_receipt_json(status, receipt)
}

fn commit_receipt_json(status: &str, receipt: CommitReceipt) -> serde_json::Value {
    json!({
        "status": status,
        "transaction_id": receipt.transaction_id,
        "commit_sequence": receipt.commit_sequence,
        "commit_digest": encode_hex(&receipt.commit_digest),
        "transaction_digest": encode_hex(&receipt.transaction_digest),
    })
}

fn cursor_json(cursor: &Cursor) -> serde_json::Value {
    json!({
        "sort_values": cursor.sort_values.iter().map(|value| {
            value.as_ref().map_or(serde_json::Value::Null, to_json)
        }).collect::<Vec<_>>(),
        "key_hex": encode_hex(&cursor.key),
    })
}

fn aggregation_json(aggregation: &hyphae_query::AggregationResult) -> serde_json::Value {
    json!({
        "grouped": aggregation.grouped,
        "groups": aggregation.groups.iter().map(|group| json!({
            "key": group.key.iter().map(|value| {
                value.as_ref().map_or(serde_json::Value::Null, to_json)
            }).collect::<Vec<_>>(),
            "metrics": group.metrics.iter().map(|metric| json!({
                "name": metric.name,
                "value": metric_json(&metric.value),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn metric_json(metric: &MetricValue) -> serde_json::Value {
    match metric {
        MetricValue::Count(value) => json!(value),
        MetricValue::Integer(None) | MetricValue::Value(None) => serde_json::Value::Null,
        MetricValue::Integer(Some(value)) => i64::try_from(*value).map_or_else(
            |_| serde_json::Value::String(value.to_string()),
            |value| json!(value),
        ),
        MetricValue::Value(Some(value)) => to_json(value),
    }
}

fn snapshot_json(snapshot: &SnapshotInfo) -> serde_json::Value {
    json!({
        "path": snapshot.path,
        "checkpoint_sequence": snapshot.checkpoint_sequence,
        "checkpoint_digest": snapshot.checkpoint_digest.map(|digest| encode_hex(&digest)),
        "entry_count": snapshot.entry_count,
        "receipt_count": snapshot.receipt_count,
        "snapshot_digest": encode_hex(&snapshot.snapshot_digest),
        "file_bytes": snapshot.file_bytes,
    })
}

fn print_json(value: &serde_json::Value) -> Result<(), Box<dyn Error>> {
    let mut output = BufWriter::new(stdout().lock());
    serde_json::to_writer_pretty(&mut output, value)?;
    writeln!(output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CliError, parse_field_path};

    #[test]
    fn cli_field_paths_reject_empty_segments() {
        assert_eq!(
            parse_field_path("nested.value").map(|path| path.segments().len()),
            Ok(2)
        );
        assert!(matches!(
            parse_field_path("nested..value"),
            Err(CliError::InvalidFieldPath)
        ));
    }
}
