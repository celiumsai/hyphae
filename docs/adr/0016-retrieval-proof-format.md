# ADR-0016: Retrieval proofs use a separate version-1 format

- Status: Accepted
- Date: 2026-07-20
- Owners: Celiums Solutions LLC

## Context

`result-proof-v1` proves KV get and structured query over a format-1 logical
snapshot. Extending its closed operation tags would change the expectations of
existing verifiers. Durable vector, lexical, and hybrid retrieval need space
definitions, canonical score semantics, modality evidence, and complete
candidate reexecution.

## Decision

Hyphae 0.2 introduces `retrieval-proof-v1` with magic `HYRPF001`, proof version
`1`, and domain `hyphae-retrieval-proof-v1`. It is independent of
`result-proof-v1`, which remains byte-for-byte and semantically unchanged.

The proof binds:

- the snapshot checkpoint sequence and commit digest;
- the complete format-2 logical snapshot digest;
- retrieval operation and semantics version;
- canonical request;
- canonical outcome, including abstention evidence;
- vector-space or lexical-index definition;
- canonical score representation; and
- scanned-candidate/document counts.

The complete logical snapshot is the separate witness. Offline verification
checks framing and digests, compares a caller-pinned anchor, verifies the
snapshot, rebuilds the reference candidate state, reexecutes the request, and
requires exact canonical outcome equality. Resource exhaustion is a
verification error, never partial verification.

Version 1 supports exact vector, lexical, and hybrid reference operations.
It does not claim ANN recall.

## Consequences

- Existing 0.1 proof artifacts and verifiers remain simple and compatible.
- Witnesses are intentionally linear and may be large.
- One format can prove the 0.2 retrieval modes because each operation carries
  an explicit semantics tag.
- Smaller authenticated indexes or signatures can be added later without
  weakening v1.

## Alternatives considered

- Adding retrieval operation tags to `result-proof-v1` was rejected because it
  silently changes an existing format.
- A result hash without reexecution was rejected because it cannot prove
  candidate completeness or ranking.
- Requiring a cloud signer or anchor was rejected because offline autonomy is
  mandatory.

## Verification

- Offline verification after removing access to the source data directory.
- Tamper tests for request, vector/index identity, score, rank, key, omitted or
  inserted result, snapshot, anchor, semantics version, trailing bytes, and
  truncation.
- Decoder/verifier fuzzing and bounded-allocation tests.
- Existing `result-proof-v1` fixtures continue to verify.
