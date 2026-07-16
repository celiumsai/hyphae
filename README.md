# Hyphae

Hyphae is an autonomous, embeddable, and verifiable data engine written in
Rust. Its base experience is one `hyphae` binary and one data directory.

> **Status:** private `0.1.0` release candidate. Compatibility guarantees are
> locked only after every release gate is green on the exact release commit.

The durable core, deterministic query/retrieval, offline result proofs, secure
optional `/v1` server, five equivalent public clients, optional framework
adapters, portable recovery, and release-candidate automation are implemented
and validated. The final security workflow gate remains open.

## Product contract

- No external database, cache, cloud, or AI provider is required.
- KV and structured queries work without an LLM or embeddings.
- Semantic retrieval is optional and provider-neutral.
- Every returned record carries verifiable provenance.
- The Rust API, HTTP `/v1` API, TypeScript SDK, Python SDK, CLI, and MCP
  server consume public versioned contracts.
- Integrations are optional. PliegoRS, Astro, Next, Vite, and other software
  must remain fully functional without Hyphae.

## Non-goals

Hyphae is not Mycelium, Hyphae Network, Celiums Network, an AI cognition
runtime, a hosted SaaS, or a framework-specific data layer. The commercial
website, console, cloud operations, billing, multitenancy, benchmark corpora,
and experimental cognitive subsystems live outside this repository.

## Planned shape

```text
application ── public SDK or /v1 ── engine ── append-only log
                                      │
                                      ├── embedded materialized indexes
                                      ├── exact structured query
                                      ├── verifiable result proofs
                                      └── optional retrieval providers
```

The append-only log is durable authority. Embedded indexes are replaceable
and rebuildable. The default path remains entirely local and deterministic.

## Repository map

- `crates/`: Rust engine, storage, query, retrieval, server, client, and CLI.
- `contracts/`: canonical OpenAPI 3.1 and JSON Schema 2020-12 contracts.
- `sdks/`: TypeScript and Python SDKs generated from public contracts.
- `mcp/`: MCP adapter documentation; implementation is the single binary.
- `integrations/`: isolated optional PliegoRS, Astro, Next, and Vite adapters.
- `examples/`: executable examples for embedded and HTTP use.
- `docs/`: architecture, ADRs, operations, release gates, and source ledger.
- `packaging/`: multiplatform packaging, SBOM, and signing definitions.
- `compatibility/`: immutable historical on-disk fixtures.

## Development

The pinned toolchain is declared in `rust-toolchain.toml`.

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
```

Start with [`docs/quickstart.md`](docs/quickstart.md). See
[`docs/api/v1.md`](docs/api/v1.md) for the optional server,
[`docs/operations/install-upgrade.md`](docs/operations/install-upgrade.md) for
installation and recovery operations,
[`docs/roadmap.md`](docs/roadmap.md) for execution order and
[`docs/gates/0.1.0.md`](docs/gates/0.1.0.md) for the release definition of
done.

## Historical source

Historical repositories are frozen inputs, not this repository's history.
No historical source may enter this tree without an audited entry in
`docs/porting/ledger.md`. The ledger starts empty by design.

## License

Apache License 2.0. See `LICENSE` and `NOTICE`.
