# Embed Hyphae in Rust

Hyphae is a library workspace as well as one executable. Embedding gives an
application direct local access without HTTP while preserving the same
durable formats and reference semantics.

The public packages are versioned together on crates.io. Use exact versions
when reproducibility matters; use workspace paths only while developing the
engine and an embedding application together.

## Choose a crate

| Crate | Use it for |
|---|---|
| `hyphae-engine` | Recommended durable facade: documents, query, retrieval, proofs, snapshot, compaction, backup/restore |
| `hyphae-storage` | Lower-level append log, KV bytes, snapshots, backup, and recovery primitives |
| `hyphae-query` | Pure deterministic query AST and reference executor |
| `hyphae-retrieval` | Pure exact cosine ranking and abstention over caller-owned vectors |
| `hyphae-contracts` | Versioned `/v1` models plus embedded OpenAPI and JSON Schemas |
| `hyphae-server` | Loopback-first HTTP server around one owned engine |
| `hyphae-client` | Bounded async HTTP client; never opens local storage |
| `hyphae-core` | Product/API/disk version constants |

Applications normally start with `hyphae-engine`:

```toml
[dependencies]
hyphae-engine = "=0.2.0"
hyphae-query = "=0.2.0"
uuid = { version = "1", features = ["v7"] }
```

## Open and own a directory

```rust,no_run
use hyphae_engine::HyphaeEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opened = HyphaeEngine::open("./hyphae-data")?;
    println!("replayed={}", opened.recovery.replayed_transactions);
    let _engine = opened.engine;
    Ok(())
}
```

The returned engine owns the operating-system lock. Opening the same path a
second time fails. `recovery` records verified replay, ignored uncommitted
attempts, duplicate commits, and truncated incomplete tail bytes.

## Write, read, and query

Mutations require a caller-supplied UUID so the application controls durable
idempotency. Encode the same logical operation with the same UUID when
retrying an uncertain request.

```rust,no_run
use std::collections::BTreeMap;
use hyphae_engine::HyphaeEngine;
use hyphae_query::{
    ExecutionLimits, Filter, Query, Record, Value,
};
use uuid::Uuid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut opened = HyphaeEngine::open("./hyphae-data")?;
    let document = Value::Object(BTreeMap::from([
        ("group".into(), Value::String("x".into())),
        ("score".into(), Value::Integer(10)),
    ]));
    opened.engine.put_record(
        Uuid::now_v7(),
        &Record::new(b"alpha", document),
    )?;

    let result = opened.engine.query(
        &Query {
            filter: Filter::MatchAll,
            sort: Vec::new(),
            cursor: None,
            limit: 100,
            aggregation: None,
        },
        &ExecutionLimits::default(),
    )?;
    assert_eq!(result.rows.len(), 1);
    Ok(())
}
```

`put_records` and `delete_records` are atomic batches and reject duplicate
keys before append. `get_record` returns an ordinary verified document read.
Query errors, budgets, and timeouts return no partial result.

Run the complete maintained example:

```bash
cargo run -p hyphae-engine --example embedded -- ./example-data
```

## Create proof-bearing results

Use `get_record_with_proof` or `query_with_proof` when the caller needs a
portable result. Each returns the canonical proof and its complete logical
snapshot witness from the same locked checkpoint.

```rust,no_run
use hyphae_engine::{
    HyphaeEngine, VerificationLimits, verify_result_proof, write_result_proof,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opened = HyphaeEngine::open("./hyphae-data")?;
    let artifact = opened.engine.get_record_with_proof(b"alpha")?;
    let expected_anchor = artifact.proof.anchor_digest();
    write_result_proof("result.hyproof", &artifact.proof)?;
    let report = verify_result_proof(
        "result.hyproof",
        &artifact.snapshot.path,
        expected_anchor,
        &VerificationLimits::default(),
    )?;
    assert_eq!(report.anchor_digest, expected_anchor);
    Ok(())
}
```

In a real trust boundary, obtain `expected_anchor` independently from the
proof/snapshot pair. The verifier does not make a producer-controlled anchor
trusted merely because the artifacts are internally consistent.

## Exact vector retrieval

`HyphaeEngine::retrieve_vectors` is a pure associated operation over vectors
the caller supplies. It does not persist vectors or produce embeddings.
Dimensions, finite/nonzero values, global duplicate keys, work budget, and
timeout are validated before a partial ranking can escape.

Provider adapters belong in the host application and must convert their
output into `VectorRecord`. Keep provider credentials and model-specific
behavior outside the engine.

## Snapshot, compaction, and recovery

- `snapshot()` creates or reuses the canonical logical checkpoint witness.
- `compact()` atomically selects a snapshot-anchored generation.
- `backup(destination)` creates a portable backup at the locked checkpoint.
- `verify_backup(path)` verifies a backup without a live engine.
- `restore_backup(source, destination)` restores only to a new path.

Do not manipulate internal files or share one data directory among engine
instances. Use application-level coordination around the one owned handle.

## Embed the HTTP server

`ServerConfig::new(path)` creates the loopback default. An application may
set `bind`, `bearer_token`, and a reduced/validated `ServerLimits`, then call
`HyphaeServer::open`, `bind`, and `run_with_shutdown`. Non-loopback bind
without authentication fails before a socket is opened.

See [configuration](../configuration.md), [architecture](../architecture/overview.md),
and the generated Rust API documentation (`cargo doc --workspace --no-deps`).
