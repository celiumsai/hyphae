# Contributing

Hyphae is private until the `0.1.0` gates are green. Access does not imply
permission to publish source, artifacts, benchmarks, or design documents.

## Development rules

1. Keep the base path local: one binary, one data directory, no required
   network or external service.
2. Change public behavior contract-first under `contracts/`.
3. Add or update an ADR for durable format, compatibility, security boundary,
   dependency direction, or provider changes.
4. Add a source-ledger entry before porting any historical code or test.
5. Keep framework adapters and providers outside core crates.
6. Add tests that prove the invariant, including failure behavior.

## Required checks

```console
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
cargo audit
```

Commits must be focused and must not include generated secrets, data
directories, benchmark corpora, or attribution trailers added by automation.
