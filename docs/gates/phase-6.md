# Phase 6 equivalent-client gate

Status: complete. Rust, TypeScript, Python, CLI, and MCP pass the common live
suite; any later release commit must repeat that evidence.

## Surfaces covered

- bounded async Rust client depending only on public wire contracts;
- dependency-free TypeScript client with lossless signed-64-bit JSON;
- dependency-free synchronous Python 3.11+ client;
- `hyphae remote` typed CLI operations and witness download;
- `hyphae mcp` bounded MCP `2025-11-25` stdio tools;
- generated TypeScript/Python models tied to one aggregate schema digest;
- one isolated black-box fixture shared by all five consumers.

## Common behavior

The live suite requires versioned/sorted capabilities, atomic put, idempotent
retry, conflicting UUID rejection, proven presence and absence, deterministic
two-page sorting, complete grouped aggregation with missing/null distinction,
delete, and post-delete absence. Rust, TypeScript, Python, and CLI also verify
the downloaded witness header/identity and `HYSNAP01` magic. MCP returns the
same JSON objects as both structured content and text fallback.

## Local commands

```bash
python tools/generate_sdk_models.py --check
(cd sdks/typescript && npm ci --ignore-scripts && npm test)
PYTHONPATH=sdks/python/src python -m unittest discover -s sdks/python/tests -v
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo +1.96.0 test --workspace --all-features --locked
cargo +1.89.0 test --workspace --all-features --locked
cargo build -p hyphae-cli -p hyphae-conformance-rust --locked
python tools/run_conformance.py
cargo deny check
cargo audit
```

The local Windows application-control policy can block newly generated Rust
executables, so live Rust/MCP/CLI evidence is collected under Debian/WSL.
GitHub Actions owns the clean native matrix and a dedicated Ubuntu job runs
the full five-surface conformance suite.

## Explicit limits

The MCP adapter is stdio only, exposes no witness binary tool, and does not
support experimental MCP tasks. The canonical `/v1` witness remains available
through Rust, TypeScript, Python, and CLI. Publishing SDK packages belongs to
the phase-8 release gate; this phase proves their source and package builds.
