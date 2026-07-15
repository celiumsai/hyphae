# ADR-0004: Versioned append-only log is durable authority

- Status: Accepted
- Date: 2026-07-14
- Owners: Celiums Solutions LLC

## Context

An autonomous engine needs explicit recovery semantics, stable persisted
bytes, verifiable history, and replaceable indexes. Treating an embedded
database's private format as the product contract would couple migrations and
proofs to one implementation.

## Decision

Hyphae persists mutations to a versioned, segmented append-only log. Each
frame has magic, format version, type, sequence, transaction identifier,
payload length, CRC32C, BLAKE3 digest, and previous-frame digest. Transaction
operations become visible only after a durable commit frame.

The default durability mode fsyncs the committed log before applying an
embedded materialized index. The index is reconstructible from a verified
snapshot plus later log segments. The first index implementation is `redb`;
its format is internal and not a public compatibility promise.

The normative byte layout and transaction grammar for format `1` are defined
in [`docs/storage/log-format-v1.md`](../storage/log-format-v1.md). A begin
frame establishes the complete transaction descriptor, and a matching commit
frame makes its operation frames visible. A later begin supersedes an
uncommitted attempt so retry after a crash remains deterministic.

Active segments and retired-prefix snapshot anchors are selected by immutable
generation records defined in
[`docs/storage/manifest-format-v1.md`](../storage/manifest-format-v1.md).

One writer owns a data directory. Recovery rejects future versions, truncates
only incomplete tail bytes, rejects checksum or chain corruption, replays
committed unapplied transactions idempotently, and ignores uncommitted work.

## Consequences

- Crash behavior and provenance share one sequence and digest chain.
- Writes pay an explicit durability cost in the default mode.
- Snapshot/compaction must preserve a verifiable checkpoint for retired
  segments.
- Redb can be replaced without changing public data semantics.

## Verification

Fault-injection tests cover every append, sync, commit, index, manifest, and
rename boundary. Reference-model tests rebuild the index and compare results.
