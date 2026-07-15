# ADR-0008: Snapshot-witness result proofs

- Status: Accepted
- Date: 2026-07-15
- Owners: Celiums Solutions LLC

## Context

Hyphae must let an offline verifier establish that a durable KV lookup or
structured query result belongs to one exact committed state. Arbitrary
filters, multi-field sorts, cursors, and aggregates also require proving that
matching records were not silently omitted. A membership path alone cannot
prove completeness for an arbitrary query unless every possible query index
is authenticated or a substantially more complex proof system is introduced.

The append-only log already provides a verified commit sequence and digest.
The canonical logical snapshot already binds the complete sorted KV state to
that checkpoint and survives log-prefix retirement.

## Decision

Result proof format v1 is a small canonical envelope containing the exact
operation and result. It references one canonical logical snapshot by digest
and the snapshot's verified log checkpoint. The snapshot is the complete
offline witness and is transferred or retained separately from the proof.

The offline verifier must:

1. verify the proof framing, CRC32C, BLAKE3 digest, and canonical payload;
2. compare its computed anchor digest with an anchor pinned by the caller;
3. verify the complete snapshot and require its snapshot/checkpoint fields to
   equal the proof anchor;
4. decode every structured document under explicit resource limits;
5. reexecute the embedded lookup or query using the reference semantics; and
6. require byte-for-byte logical equality with the embedded result.

Proof creation runs the operation and creates or reuses a snapshot while the
same process owns the exclusive data-directory writer lock. No concurrent
writer can move the checkpoint between those steps.

The anchor digest is domain-separated over checkpoint sequence, checkpoint
digest, and snapshot digest. It must be stored or communicated through a
channel the verifier trusts. A proof can demonstrate self-consistency without
that external expectation, but it cannot detect rollback and is not accepted
as trusted verification.

Format v1 covers durable KV get results, including absence, and structured
query results. Exact retrieval over vectors supplied directly by a caller is
not a durable record result and therefore has no snapshot provenance claim.
Any future persisted-vector or provider adapter must define its candidate
witness through a public versioned contract.

## Consequences

- Verification is offline and requires no database, network, model, key, or
  provider.
- Query completeness is checked by deterministic reexecution, not asserted by
  the producer.
- One snapshot can verify many small proof files at the same checkpoint.
- Verification cost is linear in the snapshot and bounded explicitly. This is
  the deliberate correctness-first reference implementation.
- Future authenticated indexes, multiproofs, or optional signatures may make
  witnesses smaller without weakening or silently changing v1 semantics.
- An attacker controlling the proof, snapshot, and the verifier's trusted
  anchor can rewrite all three. This is the existing whole-directory trust
  limitation, not an authenticity claim.

## Rejected alternatives

- A record hash alone proves neither membership nor query completeness.
- A Merkle membership path proves one key but not arbitrary filter/sort
  completeness.
- Requiring a cloud anchor, signing service, blockchain, or external database
  violates the autonomous base product.
- Treating redb files as witnesses would expose a replaceable internal format.

## Verification

The gate includes successful offline replay plus edit, delete, insert,
reorder, replay, rollback, truncation, and bit-flip cases against proof and
snapshot material. Each failure must be explicit; partial verification is not
a successful outcome.
