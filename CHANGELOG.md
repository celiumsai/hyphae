# Changelog

All notable changes are documented here. Hyphae follows Semantic Versioning
for public APIs after `0.1.0`; on-disk format versions are tracked separately.

## [Unreleased]

### Fixed

- Made the `hyphae-contracts` tarball self-contained by shipping byte-identical
  OpenAPI and JSON Schema assets inside the crate.
- Reused the packaged contract constants from the CLI MCP adapter and included
  the engine compatibility fixture in its crate tarball.

### Added

- Added a release-readiness audit that rejects compile-time assets outside a
  crate or missing from its generated `cargo package` file list.

## [0.1.0] - 2026-07-16

### Added

- Autonomous product boundary and release gates.
- Clean Rust workspace and public contract layout.
- Audited source-porting policy.
- Append-only durable storage with recovery, idempotency, snapshots,
  migrations, and anchored compaction.
- Deterministic structured query, exact provider-neutral retrieval,
  abstention, budgets, and generative correctness tests.
- Embeddable engine facade and autonomous KV/query/snapshot/compaction CLI.
- Canonical snapshot-witness result proofs with caller-pinned anchors and
  complete offline reexecution.
- Secure OpenAPI-first `/v1` server with bounded requests and loopback default.
- Equivalent Rust, TypeScript, Python, CLI, and MCP public clients with one
  black-box conformance suite.
- Optional PliegoRS, Astro, Next, and Vite adapters with public-only dependency
  enforcement and host-without-Hyphae production-build tests.
- Portable logical backups, atomic verified restores, and complete local
  `doctor` diagnostics in the single binary.
- Immutable on-disk compatibility fixtures and deterministic multiplatform
  release archives.
- SPDX/CycloneDX SBOMs, SHA-256 manifests, SLSA v1 build provenance, keyless
  Sigstore signature/attestation bundles, bounded fuzzing, and
  load/kill-restart soak gates.
- Canonical documentation hub, complete capability/CLI/configuration/data
  model/embedding/operations references, package-specific SDK and MCP guides,
  maintained embedded/HTTP/MCP examples, and automated documentation drift
  validation.
- Public Rust crates for every supported library, the `hyphae` binary, and the
  optional PliegoRS adapter, with package-specific README and docs.rs metadata.
- Official project identity, release badges, website links, and public
  crates.io installation guidance.
