# Publish the Rust crates

Hyphae publishes ten independently consumable Rust packages at the same
product version. crates.io publication is permanent: an uploaded version
cannot be overwritten or deleted. Run this procedure only from the exact
release commit after its complete hosted gate is green.

## Preconditions

1. Confirm `git status --short` is empty and `git describe --exact-match`
   reports the intended `vVERSION` tag.
2. Confirm CI, Security, Dependency Review, Fuzz, Stress, and the native Release
   matrix succeeded on that exact commit.
3. Run the workspace tests and inspect every package file list:

   ```bash
   cargo test --workspace --all-features --locked
   for crate in hyphae-core hyphae-query hyphae-retrieval hyphae-storage \
     hyphae-engine hyphae-contracts hyphae-client hyphae-server \
     hyphae-pliegors hyphae-cli; do
     cargo package --locked --list -p "$crate"
   done
   ```

4. Authenticate with a least-privilege crates.io token using `cargo login`.
   Never place the token in a command line, repository file, workflow log, or
   shell history.

## Dependency order

Publish one package at a time in this order:

```bash
cargo publish --locked -p hyphae-core
cargo publish --locked -p hyphae-query
cargo publish --locked -p hyphae-retrieval
cargo publish --locked -p hyphae-storage
cargo publish --locked -p hyphae-engine
cargo publish --locked -p hyphae-contracts
cargo publish --locked -p hyphae-client
cargo publish --locked -p hyphae-server
cargo publish --locked -p hyphae-pliegors
cargo publish --locked -p hyphae-cli
```

After each upload, wait until crates.io and the registry index expose that
exact version before publishing a dependent package. Do not bypass package
verification. If a publish returns an ambiguous network result, query
crates.io for the version before retrying; never assume the upload failed.

## Verify consumers

Use clean temporary projects, not workspace paths:

```bash
cargo install hyphae-cli --version VERSION --locked
hyphae version --json
```

Also create a minimal Rust application with exact `=VERSION` dependencies on
`hyphae-engine` and `hyphae-query`, build it with `--locked`, and verify that
docs.rs has accepted every library package. Record the crates.io URLs and the
Git tag in the GitHub release notes.

Once the initial packages exist, configure crates.io trusted publishing for
the release workflow so future releases use short-lived OIDC credentials
instead of a stored API token.
