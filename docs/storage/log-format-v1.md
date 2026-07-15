# Append-only log format v1

Status: normative for disk format `1` while Hyphae remains pre-`0.1.0`.

All integers are unsigned little-endian. A segment is a sequence of frames;
there is no mutable segment header. The first frame has sequence `1` and an
all-zero previous digest. Every later sequence increases by exactly one and
links to the prior frame digest.

## Frame layout

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | Magic ASCII `HYPHAE01` |
| 8 | 2 | Disk format version (`1`) |
| 10 | 1 | Kind: begin `1`, operation `2`, commit `3` |
| 11 | 1 | Reserved flags; must be zero |
| 12 | 8 | Sequence |
| 20 | 16 | UUID transaction identifier |
| 36 | 8 | Payload length |
| 44 | 32 | Previous-frame BLAKE3 digest |
| 76 | 4 | CRC32C of bytes `0..76` followed by payload |
| 80 | 32 | BLAKE3 of bytes `0..80` followed by payload |
| 112 | N | Payload, limited to 16 MiB per frame |

The CRC32C detects accidental corruption efficiently. The BLAKE3 digest is
the frame identity used by the chain and later provenance proofs. Neither is
an authenticity signature; the threat model does not claim resistance to an
attacker who can rewrite the entire directory and recompute every digest.

## Transaction grammar

```text
begin(tx, descriptor)
operation(tx, opaque bytes)+
commit(tx, descriptor)
```

The 36-byte descriptor is an operation count (`u32`) followed by a BLAKE3
transaction digest. The digest domain is `hyphae-transaction-v1` and covers
the operation count plus each operation length and bytes in order. Begin and
commit descriptors must match the observed operations exactly.

A transaction is visible only after its commit frame and after the segment is
synchronized. A later begin supersedes an earlier uncommitted attempt. This
makes retry after a crash deterministic even when the previous attempt left
complete begin or operation frames.

## Recovery

Opening streams and validates every complete frame before exposing replay:

- future versions, unknown kinds or flags, sequence gaps, broken chains,
  checksum failures, digest failures, and invalid transaction grammar fail;
- only an incomplete physical tail is truncated;
- complete uncommitted attempts remain in the history but are ignored;
- a repeated transaction UUID with identical content is replayed once;
- reuse of a committed UUID with different content fails as an idempotency
  conflict.

The materialized index is not part of this format and must be reconstructible
from verified committed transactions.
