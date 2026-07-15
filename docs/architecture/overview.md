# Architecture overview

Hyphae separates durable semantics from delivery surfaces.

```text
hyphae-cli (single binary)
  ├─ embedded facade
  ├─ HTTP server /v1
  └─ MCP adapter
          │
public contracts + client
          │
engine: atomic operations, query, retrieval, proofs
          │
append-only log ── snapshots ── rebuildable redb indexes
```

For the durable KV path, a write is acknowledged in two ordered stages:

1. canonical mutation frames and their commit frame are appended and synced;
2. the mutations and commit checkpoint are applied atomically to redb.

The log is authoritative. If stage 2 fails, the commit receipt remains valid,
the live handle refuses potentially stale reads, and reopen verifies the log
before replaying every missing commit. A redb checkpoint is accepted only when
its sequence and digest identify the same commit in the verified log.

Logical snapshots stream sorted KV state instead of copying redb internals.
Each snapshot records the exact verified log checkpoint and has independent
CRC32C and BLAKE3 validation. The normative layout is documented in
[`snapshot-format-v1.md`](../storage/snapshot-format-v1.md).

## Layer rules

- `hyphae-core` owns stable domain values and invariants, not I/O.
- `hyphae-storage` owns disk format, recovery, snapshots, and indexes.
- `hyphae-query` owns a deterministic typed AST and reference semantics.
- `hyphae-retrieval` owns exact vector scoring and provider-neutral
  abstention; it has no default provider.
- `hyphae-contracts` exposes wire models tied to canonical contracts.
- `hyphae-server` and `hyphae-client` communicate only through `/v1` models.
- `hyphae-cli` is the only executable artifact and composes libraries.

The future `hyphae-engine` and embeddable `hyphae` facade will coordinate
storage, query, retrieval, and proofs after the durability primitives are
proven. They are intentionally not fictional stubs in the initial workspace.

## Data directory

```text
data/
├─ FORMAT
├─ LOCK
├─ manifest/
├─ log/
├─ snapshots/
├─ indexes/
├─ blobs/
└─ tmp/
```

Restore targets a new empty directory. Temporary output is verified and
atomically promoted; an existing live directory is never overwritten in
place.
