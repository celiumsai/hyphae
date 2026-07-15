# Local quickstart

This quickstart exercises Hyphae as one executable and one owned data
directory. It does not start or contact a database, cache, cloud service,
embedding provider, or LLM.

## Build the binary

The repository pins its Rust toolchain in `rust-toolchain.toml`.

```bash
cargo build --release --locked -p hyphae-cli
export HYPHAE_DATA_DIR="$PWD/hyphae-data"
./target/release/hyphae version --json
```

On PowerShell, use the Windows executable and environment syntax:

```powershell
cargo build --release --locked -p hyphae-cli
$env:HYPHAE_DATA_DIR = Join-Path $PWD "hyphae-data"
.\target\release\hyphae.exe version --json
```

## Store and read structured documents

JSON numbers in the phase-3 structured value contract are signed 64-bit
integers. Every command reopens and verifies the same durable directory.

```bash
./target/release/hyphae put --key alpha --json '{"group":"x","score":10}'
./target/release/hyphae put --key beta --json '{"group":"x","score":20}'
./target/release/hyphae get --key alpha
```

`put` returns a transaction ID, commit sequence, commit digest, and
transaction digest. Supplying the same `--transaction-id` with the same
canonical operation returns `existing`; reusing it for different operations
fails.

## Query without AI

```bash
./target/release/hyphae query --field group --equals '"x"' --sort score
./target/release/hyphae query --sort score --descending --limit 1
```

The engine scans the complete logical dataset, applies the filter, performs
the global deterministic sort, and applies the final limit. Binary key order
is the mandatory final tie-breaker. The output includes scan and match counts
and a logical continuation cursor when more rows exist.

## Snapshot and compact

```bash
./target/release/hyphae snapshot
./target/release/hyphae compact
./target/release/hyphae query --sort score --descending --limit 2
```

The final query must return the same logical rows after compaction. The
black-box test in `crates/hyphae-cli/tests/single_binary.rs` executes this
complete flow, including durable idempotency and a fresh process for every
command.

## Current boundary

This phase exposes durable KV documents, deterministic structured query,
snapshot, and compaction. Offline result proofs arrive in phase 4; the
OpenAPI-first `/v1` server arrives in phase 5. Semantic retrieval already has
provider-neutral exact reference semantics, but no embedding provider is
enabled or required.
