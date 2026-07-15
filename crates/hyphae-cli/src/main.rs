// SPDX-License-Identifier: Apache-2.0

//! Command-line entry point for the single Hyphae executable.

mod json_value;
mod mcp;

use std::{
    env,
    error::Error,
    fs,
    io::{BufWriter, Read, Write, stdin, stdout},
    net::SocketAddr,
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};
use hyphae_client::HyphaeClient;
use hyphae_contracts::v1::{DeleteRequestV1, GetRequestV1, ProofV1, PutRequestV1, QueryRequestV1};
use hyphae_core::current_version;
use hyphae_engine::{
    HyphaeEngine, ProvenResult, ResultProofArtifact, VerificationLimits, verify_result_proof,
    write_result_proof,
};
use hyphae_query::{
    Cursor, ExecutionLimits, FieldPath, Filter, MetricValue, NullPlacement, Query, Record,
    SortDirection, SortField,
};
use hyphae_server::{BearerToken, HyphaeServer, ServerConfig};
use hyphae_storage::{AppendOutcome, CommitReceipt, CompactionOutcome, SnapshotInfo};
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::json_value::{decode_hex, encode_hex, parse_json, to_json};

#[derive(Debug, Error, Eq, PartialEq)]
enum CliError {
    #[error("field path must contain nonempty dot-separated segments")]
    InvalidFieldPath,

    #[error("result proof contains an unexpected operation/result variant")]
    UnexpectedProofResult,

    #[error("bearer token environment value is not valid Unicode")]
    InvalidBearerTokenEncoding,

    #[error("bearer token contains an embedded newline")]
    BearerTokenContainsNewline,

    #[cfg(unix)]
    #[error("bearer token file must not grant permissions to group or other users")]
    InsecureBearerTokenFile,
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
        /// Write a portable result proof to a new file.
        #[arg(long)]
        proof_out: Option<PathBuf>,
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
        /// Write a portable result proof to a new file.
        #[arg(long)]
        proof_out: Option<PathBuf>,
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
    /// Verify a result proof completely offline.
    Verify {
        /// Canonical `.hyproof` file.
        #[arg(long)]
        proof: PathBuf,
        /// Canonical logical snapshot witness referenced by the proof.
        #[arg(long)]
        snapshot: PathBuf,
        /// Trusted 32-byte anchor digest as hexadecimal.
        #[arg(long)]
        anchor: String,
    },
    /// Start the optional secure version 1 HTTP server.
    Serve {
        /// Exclusively owned Hyphae data directory.
        #[arg(long, env = "HYPHAE_DATA_DIR")]
        data_dir: PathBuf,
        /// Listener address; non-loopback requires bearer authentication.
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: SocketAddr,
        /// Restricted file containing the bearer token. Alternatively set
        /// `HYPHAE_BEARER_TOKEN` without placing the secret in argv.
        #[arg(long, env = "HYPHAE_BEARER_TOKEN_FILE")]
        bearer_token_file: Option<PathBuf>,
    },
    /// Call a running Hyphae instance through only the public version 1 API.
    Remote {
        /// Root HTTP(S) origin. The token is never accepted on argv.
        #[arg(long, env = "HYPHAE_BASE_URL")]
        base_url: String,
        /// Restricted bearer-token file. Alternatively set
        /// `HYPHAE_BEARER_TOKEN`.
        #[arg(long, env = "HYPHAE_BEARER_TOKEN_FILE")]
        bearer_token_file: Option<PathBuf>,
        #[command(subcommand)]
        operation: RemoteCommand,
    },
    /// Run the optional MCP 2025-11-25 stdio adapter over the public API.
    Mcp {
        /// Root HTTP(S) Hyphae origin. MCP never opens a data directory.
        #[arg(long, env = "HYPHAE_BASE_URL")]
        base_url: String,
        /// Restricted bearer-token file. Alternatively set
        /// `HYPHAE_BEARER_TOKEN`.
        #[arg(long, env = "HYPHAE_BEARER_TOKEN_FILE")]
        bearer_token_file: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum RemoteCommand {
    /// Print public API capabilities and effective limits.
    Capabilities,
    /// Print process liveness.
    Liveness,
    /// Print engine readiness.
    Readiness,
    /// Submit a typed JSON `PutRequestV1` from a file or standard input (`-`).
    Put {
        #[arg(long)]
        request: PathBuf,
    },
    /// Submit a typed JSON `GetRequestV1` from a file or standard input (`-`).
    Get {
        #[arg(long)]
        request: PathBuf,
    },
    /// Submit a typed JSON `DeleteRequestV1` from a file or standard input (`-`).
    Delete {
        #[arg(long)]
        request: PathBuf,
    },
    /// Submit a typed JSON `QueryRequestV1` from a file or standard input (`-`).
    Query {
        #[arg(long)]
        request: PathBuf,
    },
    /// Download the canonical witness referenced by a `ProofV1` JSON file.
    Witness {
        #[arg(long)]
        proof: PathBuf,
        /// New destination file; existing files are never replaced.
        #[arg(long)]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Version { json } => print_version(json),
        Command::Put {
            data_dir,
            key,
            value,
            transaction_id,
        } => put(&data_dir, key, &value, transaction_id),
        Command::Get {
            data_dir,
            key,
            proof_out,
        } => get(&data_dir, key.as_bytes(), proof_out.as_deref()),
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
            proof_out,
        } => query(
            &data_dir,
            QueryArguments {
                field,
                equals,
                sort,
                descending,
                nulls_first,
                limit,
                proof_out,
            },
        ),
        Command::Snapshot { data_dir } => snapshot(&data_dir),
        Command::Compact { data_dir } => compact(&data_dir),
        Command::Verify {
            proof,
            snapshot,
            anchor,
        } => verify(&proof, &snapshot, &anchor),
        Command::Serve {
            data_dir,
            bind,
            bearer_token_file,
        } => serve(data_dir, bind, bearer_token_file.as_deref()).await,
        Command::Remote {
            base_url,
            bearer_token_file,
            operation,
        } => remote(&base_url, bearer_token_file.as_deref(), operation).await,
        Command::Mcp {
            base_url,
            bearer_token_file,
        } => {
            let token = load_remote_bearer_token(bearer_token_file.as_deref())?;
            mcp::run(&base_url, token.as_deref()).await
        }
    }
}

async fn remote(
    base_url: &str,
    bearer_token_file: Option<&Path>,
    operation: RemoteCommand,
) -> Result<(), Box<dyn Error>> {
    let mut builder = HyphaeClient::builder(base_url)?;
    if let Some(token) = load_remote_bearer_token(bearer_token_file)? {
        builder = builder.bearer_token(&token)?;
    }
    let client = builder.build()?;
    match operation {
        RemoteCommand::Capabilities => print_serializable(&client.capabilities().await?.value),
        RemoteCommand::Liveness => print_serializable(&client.liveness().await?.value),
        RemoteCommand::Readiness => print_serializable(&client.readiness().await?.value),
        RemoteCommand::Put { request } => {
            let request: PutRequestV1 = read_json_request(&request)?;
            print_serializable(&client.put(&request).await?.value)
        }
        RemoteCommand::Get { request } => {
            let request: GetRequestV1 = read_json_request(&request)?;
            print_serializable(&client.get(&request).await?.value)
        }
        RemoteCommand::Delete { request } => {
            let request: DeleteRequestV1 = read_json_request(&request)?;
            print_serializable(&client.delete(&request).await?.value)
        }
        RemoteCommand::Query { request } => {
            let request: QueryRequestV1 = read_json_request(&request)?;
            print_serializable(&client.query(&request).await?.value)
        }
        RemoteCommand::Witness { proof, out } => {
            let proof: ProofV1 = read_json_request(&proof)?;
            let witness = client.download_witness(&proof).await?.value;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)?;
            output.write_all(&witness)?;
            output.sync_all()?;
            print_json(&json!({ "path": out, "file_bytes": witness.len() }))
        }
    }
}

async fn serve(
    data_dir: PathBuf,
    bind: SocketAddr,
    bearer_token_file: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let mut config = ServerConfig::new(&data_dir);
    config.bind = bind;
    config.bearer_token = load_bearer_token(bearer_token_file)?;
    let bound = HyphaeServer::open(config)?.bind().await?;
    eprintln!(
        "hyphae serving {} with data directory {}",
        bound.local_addr(),
        data_dir.display()
    );
    bound
        .run_with_shutdown(async {
            let _signal_result = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn load_bearer_token(path: Option<&Path>) -> Result<Option<BearerToken>, Box<dyn Error>> {
    let Some(mut encoded) = load_bearer_token_bytes(path)? else {
        return Ok(None);
    };
    if encoded.last() == Some(&b'\n') {
        encoded.pop();
        if encoded.last() == Some(&b'\r') {
            encoded.pop();
        }
    }
    if encoded.contains(&b'\n') || encoded.contains(&b'\r') {
        return Err(CliError::BearerTokenContainsNewline.into());
    }
    Ok(Some(BearerToken::new(encoded)?))
}

fn load_remote_bearer_token(path: Option<&Path>) -> Result<Option<String>, Box<dyn Error>> {
    let Some(mut encoded) = load_bearer_token_bytes(path)? else {
        return Ok(None);
    };
    if encoded.last() == Some(&b'\n') {
        encoded.pop();
        if encoded.last() == Some(&b'\r') {
            encoded.pop();
        }
    }
    if encoded.contains(&b'\n') || encoded.contains(&b'\r') {
        return Err(CliError::BearerTokenContainsNewline.into());
    }
    String::from_utf8(encoded)
        .map(Some)
        .map_err(|_| CliError::InvalidBearerTokenEncoding.into())
}

fn load_bearer_token_bytes(path: Option<&Path>) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    if let Some(path) = path {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let metadata = fs::metadata(path)?;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(CliError::InsecureBearerTokenFile.into());
            }
        }
        return Ok(Some(fs::read(path)?));
    }
    let Some(value) = env::var_os("HYPHAE_BEARER_TOKEN") else {
        return Ok(None);
    };
    value
        .into_string()
        .map(|value| Some(value.into_bytes()))
        .map_err(|_| CliError::InvalidBearerTokenEncoding.into())
}

fn read_json_request<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    let mut encoded = Vec::new();
    if path == Path::new("-") {
        stdin().lock().read_to_end(&mut encoded)?;
    } else {
        encoded = fs::read(path)?;
    }
    Ok(serde_json::from_slice(&encoded)?)
}

fn print_serializable(value: &impl serde::Serialize) -> Result<(), Box<dyn Error>> {
    print_json(&serde_json::to_value(value)?)
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

fn get(data_dir: &Path, key: &[u8], proof_out: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let opened = HyphaeEngine::open(data_dir)?;
    let (record, proof) = if let Some(proof_path) = proof_out {
        let artifact = opened.engine.get_record_with_proof(key)?;
        write_result_proof(proof_path, &artifact.proof)?;
        let ProvenResult::Get(record) = artifact.proof.result() else {
            return Err(CliError::UnexpectedProofResult.into());
        };
        (record.clone(), Some(proof_json(proof_path, &artifact)))
    } else {
        (opened.engine.get_record(key)?, None)
    };
    let value = record.as_ref().map(record_json);
    print_json(&json!({
        "found": value.is_some(),
        "record": value,
        "proof": proof,
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
    proof_out: Option<PathBuf>,
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
    let (result, proof) = if let Some(proof_path) = arguments.proof_out.as_deref() {
        let artifact = opened
            .engine
            .query_with_proof(&request, &ExecutionLimits::default())?;
        write_result_proof(proof_path, &artifact.proof)?;
        let ProvenResult::Query(result) = artifact.proof.result() else {
            return Err(CliError::UnexpectedProofResult.into());
        };
        (result.clone(), Some(proof_json(proof_path, &artifact)))
    } else {
        (
            opened.engine.query(&request, &ExecutionLimits::default())?,
            None,
        )
    };
    print_json(&query_result_json(&result, proof.as_ref()))
}

fn verify(
    proof_path: &Path,
    snapshot_path: &Path,
    encoded_anchor: &str,
) -> Result<(), Box<dyn Error>> {
    let expected_anchor = decode_hex::<32>(encoded_anchor)?;
    let report = verify_result_proof(
        proof_path,
        snapshot_path,
        expected_anchor,
        &VerificationLimits::default(),
    )?;
    print_json(&json!({
        "status": "verified",
        "anchor_digest": encode_hex(&report.anchor_digest),
        "proof_digest": encode_hex(&report.proof_digest),
        "checkpoint_sequence": report.anchor.checkpoint_sequence,
        "checkpoint_digest": report.anchor.checkpoint_digest.map(|digest| encode_hex(&digest)),
        "snapshot_digest": encode_hex(&report.anchor.snapshot_digest),
        "result": proven_result_json(&report.result),
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

fn proof_json(path: &Path, artifact: &ResultProofArtifact) -> serde_json::Value {
    json!({
        "path": path,
        "snapshot_path": artifact.snapshot.path,
        "checkpoint_sequence": artifact.proof.anchor().checkpoint_sequence,
        "checkpoint_digest": artifact.proof.anchor().checkpoint_digest.map(|digest| encode_hex(&digest)),
        "snapshot_digest": encode_hex(&artifact.proof.anchor().snapshot_digest),
        "anchor_digest": encode_hex(&artifact.proof.anchor_digest()),
        "proof_digest": encode_hex(&artifact.proof.proof_digest()),
    })
}

fn record_json(record: &Record) -> serde_json::Value {
    json!({
        "key_hex": encode_hex(&record.key),
        "value": to_json(&record.value),
    })
}

fn query_result_json(
    result: &hyphae_query::QueryResult,
    proof: Option<&serde_json::Value>,
) -> serde_json::Value {
    json!({
        "rows": result.rows.iter().map(record_json).collect::<Vec<_>>(),
        "next_cursor": result.next_cursor.as_ref().map(cursor_json),
        "aggregation": result.aggregation.as_ref().map(aggregation_json),
        "scanned_records": result.scanned_records,
        "matched_records": result.matched_records,
        "proof": proof,
    })
}

fn proven_result_json(result: &ProvenResult) -> serde_json::Value {
    match result {
        ProvenResult::Get(record) => json!({
            "type": "get",
            "found": record.is_some(),
            "record": record.as_ref().map(record_json),
        }),
        ProvenResult::Query(result) => json!({
            "type": "query",
            "result": query_result_json(result, None),
        }),
    }
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
    use std::{error::Error, fs};

    use uuid::Uuid;

    use super::{CliError, load_bearer_token, parse_field_path};

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

    #[test]
    fn bearer_token_file_accepts_one_terminal_newline() -> Result<(), Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!("hyphae-token-{}", Uuid::now_v7()));
        fs::write(&path, b"0123456789abcdef0123456789abcdef\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        let token = load_bearer_token(Some(&path));
        let _ignored = fs::remove_file(path);
        assert!(token?.is_some());
        Ok(())
    }
}
