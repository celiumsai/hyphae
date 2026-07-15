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

The detailed proof and server threat models must be accepted before Phases 4
and 5 close.
