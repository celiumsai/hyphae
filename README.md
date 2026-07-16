# Hyphae

Hyphae is an autonomous, embeddable, and verifiable data engine written in
Rust. Its base deployment is one native `hyphae` executable and one data
directory. KV, structured query, recovery, and verification work offline
without a database, cache, cloud service, embedding provider, or LLM.

> **Status:** private, untagged `0.1.0` release candidate. Publication remains
> disabled until every release gate is green on the exact selected commit.

## What Hyphae does

- Atomically stores and deletes structured records under binary keys.
- Recovers an append-only checksummed/digest-chained log and rebuilds embedded
  Redb indexes.
- Makes mutation retries durable and idempotent through caller-visible UUIDs.
- Executes deterministic filters, global sorting, logical cursors, and
  grouped/global aggregations with hard budgets and no partial results.
- Performs exact provider-neutral cosine retrieval with explicit abstention
  when a Rust host supplies vectors.
- Creates canonical snapshots, commits anchored compaction generations, and
  rejects unsupported or corrupt formats.
- Produces portable result proofs and reexecutes them offline against a
  caller-pinned anchor and complete snapshot witness.
- Creates, verifies, and atomically restores portable logical backups; `doctor`
  reports complete local recovery evidence.
- Optionally exposes a secure, loopback-first OpenAPI `/v1` server.
- Supplies equivalent Rust, TypeScript, Python, remote CLI, and MCP clients.
- Keeps PliegoRS, Astro, Next, and Vite adapters optional and outside the core.

The API server always returns proofs for successful get/query operations. The
embedded facade exposes ordinary and proof-bearing methods; the local CLI
creates proof files explicitly with `--proof-out`.

See the complete [capability matrix](docs/product/capabilities.md), including
surface differences, default limits, and deliberate non-capabilities.

## Five-minute local flow

```bash
cargo build --release --locked -p hyphae-cli
export HYPHAE_DATA_DIR="$PWD/hyphae-data"

./target/release/hyphae version --json
./target/release/hyphae put \
  --key alpha --json '{"group":"x","score":10}'
./target/release/hyphae query \
  --field group --equals '"x"' --sort score \
  --proof-out result.hyproof
./target/release/hyphae backup --out ./hyphae-backup
./target/release/hyphae backup-verify --backup ./hyphae-backup
./target/release/hyphae doctor
```

The query response names the snapshot and anchor needed by `hyphae verify`.
The [quickstart](docs/quickstart.md) covers Windows syntax, compaction,
restore, offline proof verification, the optional server, and clients.

## Architecture

```text
application
  ├─ embedded Rust facade ───────────────────────────────┐
  ├─ local CLI                                           │
  └─ /v1 clients (Rust / TypeScript / Python / CLI / MCP)│
                             │                           │
                       secure HTTP server                │
                             └──────────────┬────────────┘
                                            ▼
                         engine: documents / query / proof
                              │                  │
                   optional exact retrieval     │
                              │                  ▼
                              └──── append-only durable log
                                      │        │
                                  snapshots  rebuildable index
```

The append-only log is durable authority. Embedded indexes are replaceable.
One operating-system lock gives one engine/server exclusive ownership of a
data directory. See the [architecture overview](docs/architecture/overview.md)
and versioned [storage specifications](docs/README.md#durable-formats).

## Public surfaces

| Surface | Purpose |
|---|---|
| `hyphae` binary | Local engine, operations, server, remote client, verifier, MCP |
| `hyphae-engine` | Recommended embeddable Rust facade |
| `/v1` | Stable proof-bearing HTTP contract |
| `hyphae-client` | Bounded async Rust HTTP client |
| `@celiums/hyphae` | Dependency-free TypeScript client |
| `hyphae-sdk` | Dependency-free Python client |
| MCP stdio | Five schema-bound tools over `/v1` |
| Optional adapters | PliegoRS, Astro, Next, and Vite consumers |

OpenAPI 3.1 and JSON Schema 2020-12 under `contracts/` are the canonical wire
contracts. Integrations consume public clients only; hosts continue to build
and run with Hyphae absent.

## Documentation

Start at the [documentation index](docs/README.md). Key guides:

- [Capabilities and limits](docs/product/capabilities.md)
- [Quickstart](docs/quickstart.md)
- [CLI reference](docs/cli/reference.md)
- [Configuration](docs/configuration.md)
- [Data model](docs/concepts/data-model.md)
- [Embed in Rust](docs/embedding/rust.md)
- [HTTP API v1](docs/api/v1.md)
- [Public clients](docs/clients/v1.md)
- [Operations and troubleshooting](docs/operations/troubleshooting.md)
- [Security model](docs/security/threat-model.md)
- [Release verification](docs/release/verification.md)

## Product boundary

Hyphae is not Mycelium, Hyphae Network, Celiums Network, an AI cognition
runtime, a hosted SaaS, or a framework-specific data layer. It does not ship
SQL, replication, clustering, built-in TLS, at-rest encryption, multitenancy,
billing, a control plane, an embedding model, or an LLM.

Applications own process supervision, remote TLS termination, filesystem
permissions, backup media policy, and optional embedding providers. Semantic
providers can supply vectors to the Rust retrieval API but never become a core
dependency or authority.

## Repository map

- `crates/`: Rust storage, engine, query, retrieval, contracts, server, client,
  and single CLI.
- `contracts/`: canonical OpenAPI and JSON Schemas.
- `sdks/`: TypeScript and Python clients/models.
- `mcp/`: MCP adapter guide; implementation is in the single binary.
- `integrations/`: optional PliegoRS, Astro, Next, and Vite adapters.
- `examples/`: maintained embedded, HTTP, and MCP examples.
- `docs/`: product, architecture, operations, security, normative formats,
  ADRs, and release gates.
- `packaging/`: deterministic multiplatform archives and release verification.
- `compatibility/`: immutable historical on-disk fixtures.

## Development

The repository pins its toolchain and enforces format, Clippy, tests,
rustdoc, contracts, documentation, dependency policy, secret scanning,
cross-platform packages, fuzzing, and recovery stress.

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
python tools/check_documentation.py --binary target/debug/hyphae
```

See [CONTRIBUTING.md](CONTRIBUTING.md) and the
[development guide](docs/development.md).

## Historical source

Historical repositories are frozen inputs, not this repository's history. No
historical source may enter this tree without an audited entry in the
[porting ledger](docs/porting/ledger.md). Hyphae Network is not modified by
this project.

## License

Apache License 2.0. See [LICENSE](LICENSE), [NOTICE](NOTICE), and
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
