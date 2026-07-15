# Phase 4 verifiable-provenance gate

Status: implementation and local validation complete; cross-platform remote
CI evidence is required before the roadmap phase is declared closed.

## Invariants covered

- one canonical proof envelope binds an exact operation and complete result;
- KV presence and absence are verified against the complete logical state;
- structured query filters, global ordering, cursor, limit, aggregations, and
  counts are reexecuted rather than trusted from the proof producer;
- the canonical logical snapshot is the complete portable offline witness;
- snapshot sequence and digest bind the witness to one verified log commit;
- a domain-separated anchor digest makes replay and rollback detectable when
  pinned by the caller;
- CRC32C and BLAKE3 cover proof and snapshot artifacts independently;
- proof, snapshot, entry, decoded-byte, query-shape, query-work, and timeout
  limits fail without a partially verified result;
- verification performs no network request and needs no external database,
  cache, model, embedding provider, signing service, or cloud anchor.

## Adversarial evidence

- canonical codec tests cover complete get/query round trips and bit flips;
- black-box tests prove present get, absent get, filtered/sorted query,
  snapshot witness loading, and deterministic offline replay;
- proof edits, result insertion, deletion, reordering, and value replacement
  are re-encoded with valid checksums/digests and still fail reexecution;
- truncation, raw bit flips, wrong snapshots, stale-proof replay, and rollback
  against a newer trusted anchor fail explicitly;
- the existing snapshot and log suites continue to reject insertion,
  deletion, reordering, corruption, chain breaks, and incomplete tails in the
  underlying durable witness.

## Public reference

- [`ADR-0008`](../adr/0008-snapshot-witness-result-proofs.md) records why v1
  uses a complete snapshot witness instead of claiming query completeness from
  a membership path.
- [`result-proof-v1.md`](../provenance/result-proof-v1.md) defines the
  canonical bytes and trust model.
- `HyphaeEngine::get_record_with_proof` and
  `HyphaeEngine::query_with_proof` create proofs at the locked checkpoint.
- `verify_result_proof` accepts only a caller-pinned anchor and reexecutes
  completely offline.
- `hyphae get/query --proof-out` and `hyphae verify` expose the same path
  through the single binary.

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

Runtime tests execute under Debian/WSL when local Windows application control
blocks newly generated test executables. GitHub Actions must still provide
clean native Windows, macOS, and Linux runtime evidence.

## Explicit limit

CRC32C and BLAKE3 are integrity mechanisms, not signatures. If an attacker
controls the proof, snapshot, and the verifier's trusted anchor expectation,
the attacker can replace the entire local history. Optional signatures or
external anchors may strengthen this later but are never required by the base
engine.
