# Phase 3 query and retrieval gate

Status: complete. Query/retrieval tests pass in the native Linux, macOS, and
Windows matrix; any later release commit must repeat that evidence.

## Invariants covered

- canonical, bounded, checksummed structured document bytes over durable KV;
- ordered exclusive KV scans from the rebuildable materialized index;
- exact typed filters with documented missing and null behavior;
- deterministic multi-field sorting with binary-key tie-breaking;
- logical cursor pagination independent of shard partition and input order;
- aggregation over the complete filtered set before pagination;
- checked integer sums and bounded groups, metrics, filters, sorts, scans,
  matches, results, dimensions, candidates, and time;
- no silent partial response after a budget or timeout failure;
- exact global cosine ranking before final limit, with stable key ties;
- explicit abstention for no candidates, weak matches, or ambiguity;
- no default model, embedding provider, network request, or external service;
- one-binary black-box persistence, idempotency, query, snapshot, compaction,
  reopen, and result-equivalence coverage.

## Reference and executable evidence

- `docs/query/reference-semantics-v1.md` is the structured query reference.
- `docs/retrieval/reference-semantics-v1.md` is the retrieval reference.
- `crates/hyphae-query/tests/reference_properties.rs` varies record values,
  shard partition, shard order, page size, and cursor traversal.
- `crates/hyphae-retrieval/tests/reference_properties.rs` varies vector
  values, shard partition, shard order, and result limits.
- `crates/hyphae-retrieval/tests/reference_quality.rs` fixes labeled positive
  and deliberately ambiguous retrieval cases.
- `crates/hyphae-cli/tests/single_binary.rs` runs the autonomous black-box
  product flow with a fresh process per operation.

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

## Deferred by design

Phase 3 does not claim result proofs, an HTTP API, persisted embeddings,
provider adapters, or public SDK equivalence. Those belong to phases 4–7 and
must consume the public semantics established here.
