# Phase 5 secure `/v1` API gate

Status: implementation and local validation complete; native Linux, macOS,
and Windows CI evidence is required before the roadmap phase is declared
closed.

## Invariants covered

- no listener starts for embedded or ordinary CLI operations;
- `hyphae serve` defaults to `127.0.0.1:8787`;
- non-loopback bind without a strong bearer token fails before `bind(2)`;
- configured tokens are retained only as BLAKE3 digests and compared in
  constant time;
- health/capability routes are data-free and every data/witness route shares
  the conditional authentication policy;
- all data paths call `hyphae-engine` and retain its single-writer lock;
- body bytes/time, JSON depth/nodes, batch, query shape/work/result/time,
  proof, witness, response, and concurrency are bounded;
- KV get and structured query always return a canonical proof and exact
  snapshot-witness reference;
- request IDs agree between headers and JSON errors;
- unknown routes, wrong methods, malformed requests, authentication failures,
  timeouts, saturation, and limits never expose framework text;
- graceful shutdown is exercised against a real ephemeral TCP listener.
- a durable receipt remains definite if index materialization fails, after
  which readiness fails closed until log replay on restart;

## Contract evidence

- OpenAPI 3.1 defines the complete eight-route surface and all declared
  statuses.
- Eleven JSON Schema 2020-12 documents are generated from typed Rust wire
  models and checked for byte-model drift.
- External OpenAPI schema references resolve during tests.
- Natural JSON, reserved bytes, fixed-width integers, aggregation missing/null
  identity, and compatibility rules are documented under `contracts/`.

## Security evidence

- missing and incorrect bearer credentials produce the same public error;
- duplicate authorization headers are rejected;
- remote exposure is configuration-invalid without authentication;
- token files never enter argv and Unix group/other-readable files fail;
- slow request bodies time out before durable work begins;
- semaphore exhaustion returns `busy` without beginning an operation;
- witness paths are derived from parsed sequence/digest values and snapshots
  are reverified before streaming;
- TLS, OAuth, multitenancy, distributed rate limiting, and hosted ingress
  remain explicit non-goals.

## Local commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo +1.96.0 test --workspace --all-features --locked
cargo +1.89.0 test --workspace --all-features --locked
cargo deny check
cargo audit
```

The local Windows application-control policy may block newly generated Rust
executables. Local runtime evidence is therefore collected under Debian/WSL;
the release gate still requires native GitHub Actions evidence on every
supported operating system.
