# Roadmap to 0.1.0

The phases are ordered gates. A later phase may be prototyped early, but it
cannot be declared complete while an earlier gate is red.

Current status: Phases 0 and 1 are complete. Phase 2 and phase 3
implementations and local validation are complete; their cross-platform
remote CI gates remain open. See [`gates/phase-2.md`](gates/phase-2.md) and
[`gates/phase-3.md`](gates/phase-3.md).

| Phase | Outcome | Exit evidence |
|---|---|---|
| 0 | Product boundary, license, ADRs, source matrix | Accepted ADRs and an auditable porting ledger |
| 1 | Clean repository, workspace, CI, RustSec, secret scanning, docs | Green baseline on Linux, macOS, and Windows |
| 2 | Durable local core | Crash recovery, atomic/idempotent writes, snapshots, migrations, checksums, compaction |
| 3 | Correct query and retrieval | KV, filters, aggregates, stable global merge, budgets, abstention, quality tests |
| 4 | Verifiable provenance | Offline verification for every returned result and tamper tests |
| 5 | Secure `/v1` API | OpenAPI-first compatibility, authentication, limits, loopback default |
| 6 | Equivalent clients | Rust, TypeScript, Python, CLI, and MCP pass one conformance suite |
| 7 | Optional adapters | PliegoRS, Astro, Next, and Vite adapters use only public contracts |
| 8 | Release candidate | Multiplatform packages, SBOM, signatures, backup/restore, fuzz/load gates |

The first end-to-end durable proof is deliberately narrow: use one binary to
write data, interrupt it during a write, restart, query the committed state,
and verify the result offline without network, external database, embedding,
or LLM.
