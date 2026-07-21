# ADR-0013: Durable named vector records use disk format 2

- Status: Accepted
- Date: 2026-07-20
- Owners: Celiums Solutions LLC

## Context

Hyphae `0.1.0` accepts host-supplied vectors only for ephemeral exact
retrieval. Disk format `1`, its mutation tags, and logical snapshot contain KV
documents and idempotency receipts only. Adding a Redb vector table would make
an accelerator authoritative and would lose vectors during rebuild, backup,
or restore. Reusing format `1` for new mutation tags would also let an old
binary begin replay before discovering state it cannot understand.

## Decision

The first durable vector mutation promotes a format-1 directory to disk format
`2` while the exclusive directory lock is held. A format-1 directory remains
readable by a 0.2 binary before promotion. Promotion is fail-closed,
idempotent, and crash recoverable:

1. verify and materialize the complete format-1 state;
2. write and verify a format-2 logical snapshot containing KV state,
   idempotency receipts, vector-space definitions, and vectors;
3. create a new format-2 log segment anchored to the verified checkpoint;
4. commit a format-2 manifest generation;
5. atomically replace the `FORMAT` marker with
   `hyphae-disk-format=2\n`; and
6. retire format-1 log and snapshot material only after the new generation is
   recoverable.

An old binary rejects a promoted directory at `FORMAT` before replay. A new
binary continues to verify immutable format-1 compatibility fixtures and
format-1 result proofs.

Vector identity is `(vector_space, object_key)`. `object_key` is the same
nonempty bounded binary identity used by KV records. `vector_space` is a
canonical ASCII identifier matching `[A-Za-z][A-Za-z0-9._-]{0,127}`. A named
space has one immutable dimension in `1..=4096` and cosine is the only metric
in 0.2. Deleting every vector does not delete the space definition.

Stored elements are signed Q15 integers in `[-32767, 32767]`; `-32768` is
invalid. A vector must be nonzero. The public canonical representation is the
integer sequence itself, not source floating-point bytes. Provider adapters
may offer explicit quantization helpers but cannot change stored values.

Space creation, vector upsert, and vector delete are typed logical mutations
in the same append-only transaction grammar as KV mutations. Mixed
document/vector batches are atomic in the embedded engine. Public HTTP
requests are atomic within one request; separate HTTP requests do not imply a
cross-request transaction.

Vectors remain separate from canonical documents. Redb vector tables and
later search indexes are rebuildable projections. Snapshots, backup, restore,
compaction, doctor, and index rebuild cover vector definitions and records.

## Consequences

- Canonical vectors require two bytes per dimension and portable integer
  decoding.
- Clients converting floating embeddings must make quantization visible.
- Format promotion is an operational boundary and requires a verified backup.
- Snapshot format 2 and migration fixtures are required before vector writes
  ship.
- Collections, tenants, multiple metrics, and ANN remain outside 0.2.

## Alternatives considered

- Persisting `f32` or `f64` was rejected because proof reexecution would bind
  implicit floating-point behavior and use more space.
- Extending mutation format 1 without changing `FORMAT` was rejected because
  an old binary would not reject before replay.
- A sidecar vector database was rejected because it would split durable
  authority and atomicity.
- Encoding vectors inside documents was rejected because documents deliberately
  exclude floating point and vector lifecycle is independent.

## Verification

- Format-1 and format-2 immutable fixtures.
- Crash injection at every promotion boundary.
- Mixed-batch atomicity and exact-idempotency tests.
- Snapshot, compaction, backup, restore, and index-deletion tests with vectors.
- Mutation and snapshot decoder fuzzing.
