# Doctor and recovery diagnostics

`doctor` exclusively opens the data directory, verifies its format marker,
latest manifest, snapshot, append-only log chain, checksums, recovery state,
and materialized index, then creates or reuses a canonical snapshot:

```bash
hyphae doctor --data-dir ./hyphae-data
```

Success exits zero with `"status":"healthy"`. The report includes the log
anchor, last verified sequence and digest, valid byte count, replayed
transactions, ignored uncommitted attempts, duplicate commits, truncated tail
bytes, and snapshot identity.

An incomplete final frame caused by process interruption may be truncated
during ordinary recovery and is reported in `truncated_tail_bytes`. Complete
checksum, digest-chain, manifest, snapshot, or format corruption fails loudly;
Hyphae does not silently fall back to an older generation.

## Safe incident sequence

1. Stop every process using the data directory.
2. Preserve a filesystem-level copy for forensic analysis; do not run repair
   tools against the only copy.
3. Run `doctor` and retain stdout, stderr, binary version, and the expected
   trusted result-proof anchors.
4. If `doctor` succeeds, create and verify a logical Hyphae backup.
5. If it fails, restore the newest independently verified Hyphae backup to a
   new directory. Never edit `FORMAT`, manifests, logs, snapshots, or Redb
   files by hand.

`doctor` is a verifier and recovery opener, not an in-place corruption repair
command. Disposable indexes are rebuilt automatically when authoritative
state is valid; authoritative bytes are never guessed.
