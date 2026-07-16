# Changelog

All notable changes are documented here. Hyphae follows Semantic Versioning
for public APIs after `0.1.0`; on-disk format versions are tracked separately.

## [Unreleased]

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
