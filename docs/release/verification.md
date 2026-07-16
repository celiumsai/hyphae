# Verify a Hyphae release

Do not install an archive until its digest, identity, and provenance all
verify. Replace `VERSION` and `TARGET` with the downloaded release values.

## 1. Verify checksums

Download the archive, both SBOMs, `SHA256SUMS`, and their corresponding
`.sigstore.json` bundles into one directory:

```bash
sha256sum --check SHA256SUMS
```

Every listed archive and SBOM must report `OK`. On PowerShell, compare a file
with its `SHA256SUMS` entry:

```powershell
(Get-FileHash .\hyphae-VERSION-TARGET.zip -Algorithm SHA256).Hash.ToLowerInvariant()
```

## 2. Verify the keyless signature

Use Cosign 3.1.1 or a later compatible verifier. The certificate identity is
bound to the tagged Hyphae release workflow:

```bash
cosign verify-blob \
  --bundle hyphae-VERSION-TARGET.tar.gz.sigstore.json \
  --certificate-identity \
    'https://github.com/celiumsai/hyphae/.github/workflows/release.yml@refs/tags/vVERSION' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  hyphae-VERSION-TARGET.tar.gz
```

Run the same verification for `SHA256SUMS` and both SBOM files. A bundle from
another repository, workflow, branch, or tag must fail the identity check.

## 3. Verify build provenance and SBOM attestations

The native package job emits a SLSA provenance v1 attestation whose subject is
the exact archive digest:

```bash
cosign verify-blob-attestation \
  --bundle hyphae-VERSION-TARGET.tar.gz.intoto.sigstore.json \
  --type slsaprovenance1 \
  --certificate-identity \
    'https://github.com/celiumsai/hyphae/.github/workflows/release.yml@refs/tags/vVERSION' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  hyphae-VERSION-TARGET.tar.gz
```

`verify-blob-attestation` checks that the archive digest appears in the signed
in-toto subject. Inspect the bundle's predicate and require the expected target,
Git commit, full tag ref, release workflow digest, Cargo lockfile digest,
GitHub-hosted builder, and invocation URI.

The archive also has `.spdx.attestation.sigstore.json` and
`.cyclonedx.attestation.sigstore.json` bundles. Verify them with the same
identity and `--type spdxjson` or `--type cyclonedx`, respectively.

## 4. Inspect and smoke-test

The SPDX and CycloneDX JSON files are dependency inventories. Retain them with
the installed binary. Extract the archive into an empty directory and confirm
that it contains one executable plus `LICENSE`, `README.md`, and
`THIRD_PARTY_NOTICES.md`:

```bash
tar -xzf hyphae-VERSION-TARGET.tar.gz
./hyphae-VERSION-TARGET/hyphae version --json
```

The reported product must be `hyphae` and `engine_version` must equal the tag
without the leading `v`. A release tag that differs from the workspace version
is rejected by the publication workflow.
