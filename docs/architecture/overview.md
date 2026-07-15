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
