# Packaging

`package.py` produces a deterministic archive containing one native `hyphae`
binary plus the license, readme, and third-party notices. It never bundles a
database, cache, model, provider credential, or runtime installer.

```bash
SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
python packaging/package.py \
  --binary target/dist/hyphae \
  --target x86_64-unknown-linux-gnu \
  --output-dir artifacts
```

The release workflow builds native archives for Linux x64, macOS x64/arm64,
and Windows x64. It emits a SHA-256 checksum file, SPDX JSON and
CycloneDX JSON SBOMs, Sigstore bundles for every release asset, and GitHub
Actions SLSA v1 provenance plus SBOM attestations for every native archive
before creating a release. Every package job also extracts its own archive and
executes the documented offline version, KV, query, compaction, result proof,
durable vector/lexical/hybrid retrieval, retrieval-proof verification,
backup/restore, and doctor flow from the installed binary.

A manual workflow run executes native build, provenance, SBOM, signing, and
verification, then uploads a candidate artifact without publishing a release.
Publication is reachable only from an explicit `v*` tag, and
`finalize_release.py` rejects a tag that does not equal `v` plus the workspace
version. A tag may be pushed only after the complete gate is green and
publication is explicitly authorized.

Run the deterministic unit checks with:

```bash
python packaging/test_package.py
```

Consumer verification is documented in
[`../docs/release/verification.md`](../docs/release/verification.md).
