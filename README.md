# Hyphae

Hyphae is an autonomous, embeddable, and verifiable data engine written in
Rust. Its base experience is one `hyphae` binary and one data directory.

> **Status:** private pre-release. The current version is
> `0.1.0-alpha.1`; none of the `0.1.0` compatibility guarantees apply until
> every release gate is green.

The durable core and deterministic query/retrieval implementation are locally
validated. Their cross-platform CI evidence remains open.

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
- `mcp/`: MCP package consuming the public client contract.
- `examples/`: executable examples for embedded and HTTP use.
- `docs/`: architecture, ADRs, operations, release gates, and source ledger.
- `packaging/`: multiplatform packaging, SBOM, and signing definitions.

## Development

The pinned toolchain is declared in `rust-toolchain.toml`.

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --all-features --no-deps --locked
```

Start with [`docs/quickstart.md`](docs/quickstart.md). See
[`docs/roadmap.md`](docs/roadmap.md) for execution order and
[`docs/gates/0.1.0.md`](docs/gates/0.1.0.md) for the release definition of
done.

## Historical source

Historical repositories are frozen inputs, not this repository's history.
No historical source may enter this tree without an audited entry in
`docs/porting/ledger.md`. The ledger starts empty by design.

## License

Apache License 2.0. See `LICENSE` and `NOTICE`.
