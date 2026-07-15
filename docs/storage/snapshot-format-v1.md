# Logical snapshot format v1

Status: normative for disk format `1` while Hyphae remains pre-`0.1.0`.

A snapshot is a portable, logical image of the materialized KV state at one
verified log checkpoint. It never copies redb implementation files. Entries
are emitted in strict bytewise key order so identical logical state and
checkpoint produce identical content.

All integers are unsigned little-endian.

## Header

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | Magic ASCII `HYSNAP01` |
| 8 | 2 | Disk format version (`1`) |
| 10 | 2 | Reserved flags; must be zero |
| 12 | 8 | Materialized commit-frame sequence, or zero for an empty log |
| 20 | 32 | Commit-frame BLAKE3 digest, or all zero for an empty log |
| 52 | 8 | Entry count |
| 60 | 8 | Payload length |
| 68 | 4 | CRC32C of header bytes `0..68` followed by the payload |
| 72 | 32 | BLAKE3 of header bytes `0..72` followed by the payload |

## Entry encoding

The payload contains exactly `entry count` records:

| Size | Field |
|---:|---|
| 4 | Key length |
| 8 | Value length |
| N | Non-empty binary key, limited to 1 MiB |
| M | Binary value |

Keys must be strictly increasing and therefore cannot repeat. Values are
streamed during creation and verification; verification does not allocate a
buffer proportional to value size.

## Creation and verification

Hyphae streams the caught-up redb index into a unique temporary file, writes
and synchronizes the completed header and payload, verifies the temporary
file from disk, then atomically renames it into `snapshots/`. On Unix the
snapshot directory is synchronized after the rename. A canonical snapshot
already present for the same checkpoint is verified before reuse.

Verification rejects future versions, flags, length overflows or mismatches,
invalid or unordered keys, trailing or truncated bytes, CRC32C mismatch, and
BLAKE3 mismatch. The checkpoint digest links the logical state to a verified
commit in the authoritative log when the snapshot is restored.

CRC32C and BLAKE3 detect accidental corruption. They are not signatures and
do not authenticate a directory against an attacker able to rewrite the
snapshot and log together.
