# Phase 2 durable-core gate

Status: complete. Durable-core tests pass in the native Linux, macOS, and
Windows matrix; any later release commit must repeat that evidence.

## Invariants covered

- one operating-system writer lock per data directory;
- canonical versioned `FORMAT`, mutation, log, snapshot, and manifest bytes;
- append-only transaction visibility only after a synchronized commit frame;
- stable UUID idempotency, including after log-prefix retirement;
- redb as a rebuildable index with atomic KV, receipt, and checkpoint updates;
- replay after a durable log commit but failed index update;
- incomplete-tail repair without truncating complete corruption;
- logical snapshots with CRC32C, BLAKE3, sorted KV, and receipt ledger;
- immutable-manifest bootstrap migration for early format-1 directories;
- compaction through snapshot, anchored next segment, manifest commit, then
  best-effort retired-segment cleanup;
- index reconstruction from snapshot plus later active-segment commits.

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
blocks newly generated test executables. Clippy and documentation still build
the Windows targets locally; GitHub Actions must provide the clean native
Windows, macOS, and Linux runtime matrix.

## Failure evidence

The storage suite covers every byte cut before a complete transaction,
partial physical tails, complete corruption, future versions, conflicting
UUID reuse, missing-index replay, divergent checkpoints, corrupted snapshots,
interrupted manifest creation, prepared segments without a manifest,
manifest commit before retired-log cleanup, uncertain log synchronization,
and a materialized-index failure after a durable commit.
