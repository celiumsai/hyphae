# Changelog

All notable changes are documented here. Hyphae follows Semantic Versioning
for public APIs after `0.1.0`; on-disk format versions are tracked separately.

## [Unreleased]

No changes yet.

## [0.2.0] - 2026-07-21

### Added

- Added durable named vector spaces and atomic vector mutations to disk format
  `2`, including snapshot, compaction, backup, restore, migration, and
  derived-index rebuild coverage.
- Added deterministic exact vector retrieval with canonical signed Q15 cosine
  scores, bounded execution, explicit abstention, and stable binary-key ties.
- Added provider-free lexical retrieval with pinned Unicode normalization and
  BM25F-compatible integer scoring.
- Added deterministic hybrid retrieval using reciprocal-rank fusion and
  per-modality explanations.
- Added `retrieval-proof-v1`, including canonical encoding, offline
  verification, and request/result/witness/semantics tamper detection.
- Added additive `/v1` schemas, OpenAPI paths, server routes, generated models,
  Rust/TypeScript/Python clients, remote CLI commands, MCP tools, and shared
  conformance cases for vector, lexical, and hybrid retrieval.
- Added immutable disk-format-2 and retrieval golden fixtures, generators, and
  synchronization checks.
- Added retrieval benchmarks, load and restart/restore soak gates, retrieval
  proof fuzzing, in-flight write interruption recovery, and local release
  evidence.

### Fixed

- Made the `hyphae-contracts` tarball self-contained by shipping byte-identical
  OpenAPI and JSON Schema assets inside the crate.
- Reused the packaged contract constants from the CLI MCP adapter and included
  the engine compatibility fixture in its crate tarball.
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
