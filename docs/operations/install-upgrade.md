# Install, upgrade, and migration

## Install a release archive

First verify the release as described in
[`../release/verification.md`](../release/verification.md). Extract exactly
one archive for the host platform and place `hyphae` or `hyphae.exe` on the
operator-controlled executable path:

```bash
tar -xzf hyphae-VERSION-x86_64-unknown-linux-gnu.tar.gz
install -m 0755 hyphae-VERSION-x86_64-unknown-linux-gnu/hyphae "$HOME/.local/bin/hyphae"
hyphae version --json
```

On Windows, expand the `.zip`, move `hyphae.exe` to a directory on `PATH`, and
run:

```powershell
hyphae.exe version --json
```

No service, database, cache, model, account, or installer is required. A first
write initializes the directory named by `--data-dir` or `HYPHAE_DATA_DIR`.

## Upgrade safely

1. Record `hyphae version --json` and retain the old executable.
2. Stop the server or any process owning the data directory.
3. Create and independently verify a Hyphae backup.
4. Verify and install the new native archive without deleting the old binary.
5. Run the new binary's `doctor` against the data directory.
6. Start the application and execute a known read and query.

```bash
old-hyphae backup --data-dir ./hyphae-data --out ./pre-upgrade-backup
old-hyphae backup-verify --backup ./pre-upgrade-backup
new-hyphae doctor --data-dir ./hyphae-data
new-hyphae get --data-dir ./hyphae-data --key alpha
```

Hyphae detects the on-disk format before opening data. A binary refuses a
future unsupported format. Required format migrations are versioned,
idempotent, and executed while the directory lock is held; release notes must
name any migration before an upgrade is authorized.

Rollback means stopping the new process and restoring the pre-upgrade backup
to a new directory with a binary that supports that backup's disk format. Do
not point an older binary at a directory already migrated to a newer format.

## Source build

For development only, the repository pins the toolchain:

```bash
cargo build --release --locked -p hyphae-cli
./target/release/hyphae version --json
```

Release archives, not local source builds, are the supported installation
artifact for `0.1.0` and later.
