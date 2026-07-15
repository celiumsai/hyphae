# Baseline threat model

Hyphae 0.1.0 protects local data integrity, authenticated remote access, and
bounded resource consumption. It assumes the operating system correctly
enforces file ownership and process isolation.

## In scope

- Accidental truncation, partial writes, bit flips, and interrupted recovery.
- Replay, reorder, insertion, deletion, and rollback attempts against log
  history and result proofs.
- Malformed or excessive API input.
- Unauthorized access when the server is explicitly bound beyond loopback.
- Dependency, license, and secret exposure in the source and build pipeline.

## Explicit limitation

A local checkpoint detects corruption and partial manipulation. An attacker
who controls the entire data directory and every trusted checkpoint can
rewrite both history and its local roots. External signatures or anchors may
strengthen that model later, but they are optional and never required for the
base engine.

The result-proof model below is the accepted Phase 4 trust decision. The
detailed server threat model must be accepted before Phase 5 closes.

## Result-proof trust model

Result proof v1 uses a canonical logical snapshot as the complete offline
witness. The verifier checks the proof, checks the snapshot, reexecutes the
embedded operation, and compares the complete result. This detects edits,
insertions, deletions, reordering, truncation, and bit flips in either
artifact.

Rollback and replay detection require the caller to supply an expected anchor
digest that was pinned outside the proof/snapshot pair. Self-consistency alone
is useful for diagnostics but is not trusted verification. The normative
contract is [`result-proof-v1.md`](../provenance/result-proof-v1.md).
