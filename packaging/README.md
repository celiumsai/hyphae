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
before creating a release.

A manual workflow run executes native build, provenance, SBOM, signing, and
verification, then uploads a private candidate artifact without publishing a
release. Publication is reachable only from an explicit `v*` tag, and
`finalize_release.py` rejects a tag that does not equal `v` plus the workspace
version. The repository remains private and untagged until the complete
`0.1.0` gate is green.

Run the deterministic unit checks with:

```bash
python packaging/test_package.py
```

Consumer verification is documented in
[`../docs/release/verification.md`](../docs/release/verification.md).
