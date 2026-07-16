# Troubleshooting

Start with the exact `hyphae version --json`, command, exit status, stderr,
and `X-Request-Id` for HTTP failures. Never edit a live data directory or the
only forensic copy to make an error disappear.

## Data directory is already locked

Another process owns the same directory, or the current process already has
an embedded engine open. Stop the intended owner cleanly and retry. Do not
delete `LOCK`; the operating system lock, not the file's existence, protects
ownership.

For a server, confirm that a previous process has exited before invoking
`doctor`, backup, or a local CLI read.

## Unsupported or malformed disk format

An older binary refuses a future `FORMAT` version and any binary refuses a
noncanonical marker. Record the versions and restore a verified backup with a
compatible binary. Do not rewrite `FORMAT` or point an older binary at a
directory already migrated by a newer one.

## Corrupt log, snapshot, manifest, or document

Hyphae fails closed on complete checksum/digest-chain corruption. Preserve a
filesystem copy, run `doctor` against the copy if safe, retain stderr, and
restore the newest independently verified backup to a new path. An incomplete
final log frame after a process kill is different: recovery may truncate only
that tail and reports `truncated_tail_bytes`.

## Idempotency conflict

The supplied transaction UUID was already committed with a different
canonical mutation batch. Do not generate a third interpretation. Retrieve
the original request from application logs or use a new UUID only for a
genuinely new operation. Exact retries return the original receipt.

## Query limit or timeout

The complete query failed; no partial rows or aggregation are valid. Inspect
`/v1/capabilities`, reduce scan scope/result size/filter complexity, paginate
with the returned logical cursor, or deliberately configure a bounded Rust
server policy. The simplified local CLI always uses reference defaults.

## `busy` or `unavailable`

`busy` means the bounded server admission semaphore is saturated. Respect
`Retry-After`, use bounded backoff, and avoid unbounded client fan-out.

`unavailable` means readiness is false, including the case where a log commit
became durable but its rebuildable index update failed. Preserve any returned
commit receipt, stop the process, and reopen; recovery replays the durable log.
Retry the same transaction UUID to resolve the definite receipt.

## `unauthorized`

Data routes require one valid bearer token when server authentication is
configured. Health and capability routes remain public. Confirm that the
client uses the root origin, the selected token file/environment is correct,
and no newline was embedded. Missing and wrong tokens intentionally return the
same response.

## Token file rejected

Tokens must be 32–4,096 visible ASCII bytes. On Unix the file must grant no
group or other permissions; use `chmod 600` or stricter. Hyphae removes one
trailing line ending but rejects embedded CR/LF. On Windows, restrict the ACL
to the service account.

## Invalid JSON or integer

Documents support signed 64-bit integers, not floating point. TypeScript must
use `bigint` outside JavaScript's safe integer range; the SDK rejects unsafe
numbers locally. API byte values use the reserved
`{"$hyphae_bytes_hex":"..."}` envelope. See the [data model](../concepts/data-model.md).

## Proof verification failed

Treat the result as unverified. Check that:

1. the `.hyproof` and snapshot are the exact pair from the response;
2. the expected anchor is 64 hexadecimal characters and came from the trusted
   channel for that checkpoint;
3. neither artifact was truncated, reordered, or modified;
4. verifier limits admit the artifact sizes and query work.

A stale proof against a newer trusted anchor must fail. Do not replace the
expected anchor with the value found inside the untrusted proof.

## Backup or restore rejected

A backup directory must contain exactly `BACKUP.json` and `snapshot.hysnap`
with no symlinks or extra entries. Backup and restore destinations must not
exist. Restore never merges. Verify the stored copy, restore to a new sibling
path, run `doctor`, and switch the application only after validation.

## Remote client contract failure

Clients reject non-root URLs, malformed JSON/media types, missing or
duplicated request IDs, oversized responses, noncanonical witness paths, and
witness digest/length mismatch. Preserve the request ID and server version;
do not silently decode a response that violates `/v1`.

Stable HTTP status/code meanings are listed in
[API error codes](../api/error-codes-v1.md). The safe incident procedure is in
[doctor and recovery diagnostics](doctor.md#safe-incident-sequence).
