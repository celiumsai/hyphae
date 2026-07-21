# Backup and restore

Hyphae backups are local, portable, verified logical snapshots. They require
no server, database, cloud account, AI provider, or network connection.

## Create and verify

Stop any separate Hyphae process that owns the directory. The backup command
itself opens and exclusively locks the data directory:

```bash
hyphae backup --data-dir ./hyphae-data --out ./backups/hyphae-2026-07-15
hyphae backup-verify --backup ./backups/hyphae-2026-07-15
```

The output statuses must be `created` and `verified`. A backup contains exactly
`BACKUP.json` and `snapshot.hysnap`. Store the whole directory. Do not modify
either file, add files, or place the backup inside the live data directory.
Hyphae refuses to overwrite an existing destination.

The operator must exclusively control the destination parent while creation
or restore is running. Do not let another process create, rename, or replace
entries there concurrently.

## Restore

Restore always targets a path that does not yet exist:

```bash
hyphae restore \
  --backup ./backups/hyphae-2026-07-15 \
  --data-dir ./hyphae-restored
hyphae doctor --data-dir ./hyphae-restored
hyphae get --data-dir ./hyphae-restored --key alpha
```

The command verifies the source, reconstructs storage in a sibling staging
directory, rebuilds its embedded index, reopens it, and compares the checkpoint
before the final destination becomes visible. A corrupt source fails without
activating `./hyphae-restored`.

For disk format 2, the backup identity covers KV entries, vector-space
definitions, vectors, lexical-index definitions, and durable receipts. Restore
rebuilds Redb only from those logical sections. Validate at least one exact
and one lexical retrieval after restore when the application uses them.

Restore does not merge data and never modifies the source backup. To replace
an existing installation, restore to a new path, run `doctor`, stop the old
process, and switch the application to the verified new directory.

## Retention test

A backup is not proven merely because it was created. For every retention
cycle:

1. run `backup-verify` on the stored copy;
2. restore it to a disposable new directory;
3. run `doctor` and an application-specific read/query check;
   include exact/lexical/hybrid checks for retrieval-enabled data;
4. remove only the disposable restored directory after validation.

Keep at least one independently stored generation according to the operator's
recovery-point requirements. Encryption, media replication, and retention are
operator policies outside the Hyphae data format.
