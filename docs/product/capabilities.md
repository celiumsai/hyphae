# Product capabilities and limits

Hyphae `0.1.0` is a local-first Rust data engine. The base deployment is one
native `hyphae` executable and one exclusively owned data directory. It starts
no listener unless `serve` is selected and requires no database, cache,
cloud account, model, embedding provider, or LLM.

## Capability matrix

| Capability | Embedded Rust | Local CLI | HTTP `/v1` | Remote CLI / SDKs | MCP |
|---|---:|---:|---:|---:|---:|
| Atomic KV put/delete | Batch | Single record | Batch | Batch | Batch |
| Exact KV get | Yes | Yes | Yes | Yes | Yes |
| Deterministic structured query | Full AST | Simplified | Full AST | Full AST | Full AST |
| Grouped/global aggregation | Yes | No | Yes | Yes | Yes |
| Logical cursor pagination | Yes | Output only | Yes | Yes | Yes |
| Result proof creation | Explicit method | `--proof-out` | Always | Always | Always |
| Offline proof verification | Yes | Yes | Download artifacts | Download artifacts | Via returned proof |
| Exact vector retrieval | Yes | No | No | No | No |
| Snapshot and compaction | Yes | Yes | No | No | No |
| Backup, verify, restore | Yes | Yes | No | No | No |
| Complete doctor report | Recovery primitives | Yes | No | No | No |
| Start the HTTP server | Library | `serve` | N/A | N/A | N/A |

“Full AST” includes recursive filters, deterministic multi-field sorting,
cursors, grouping, and `count`/`sum`/`min`/`max`. The local `query` command is
intentionally a convenient subset: match-all or one equality filter, one sort
field, and a final limit. Use the embedded API or `/v1` for the complete query
surface.

## Durable storage

- The append-only framed log is authoritative; the embedded Redb index is
  rebuildable.
- Mutation batches are atomic and identified by UUID transaction IDs.
- An exact transaction retry returns the original receipt. Reusing the UUID
  with different canonical operations is rejected.
- Recovery truncates only an incomplete final frame. Complete checksum,
  digest-chain, manifest, snapshot, and format corruption fails closed.
- One operating-system lock permits one writer/engine owner per data
  directory.
- Logical snapshots are canonical and portable. Compaction anchors a new log
  generation to a verified snapshot before retiring the previous segment.
- Backups contain a verified snapshot plus manifest and restore only to a new
  atomically activated directory.

The exact layouts are documented under [durable formats](../README.md#durable-formats).

## Structured data and query

Records have nonempty binary keys and deterministic values: null, boolean,
signed 64-bit integer, UTF-8 string, bytes, array, or ordered object. Floating
point is deliberately absent. Query evaluates the complete logical dataset,
merges globally, then applies sort, cursor, and final limit. Binary key
ascending is always the final tie-breaker.

Filters support existence, same-type ordered comparison, prefix, contains,
recursive all/any, and negation. Aggregation runs over the complete filtered
set before pagination. Every budget or timeout failure returns no partial
logical result. See [data model](../concepts/data-model.md) and
[query semantics](../query/reference-semantics-v1.md).

## Retrieval without a provider dependency

The Rust engine exposes exact global cosine retrieval over vectors supplied by
the caller. It validates every candidate, applies a deterministic key
tie-breaker, and can abstain for no candidates, insufficient score, or an
ambiguous top margin. Hyphae does not create or persist embeddings and does
not ship a provider. Retrieval is not exposed by API v1. See
[retrieval semantics](../retrieval/reference-semantics-v1.md).

## Verifiable results

The embedded facade has ordinary and proof-bearing get/query methods. The
local CLI creates a proof when `--proof-out` is provided. Every successful
`/v1` get or query, and therefore every get/query call through remote CLI,
SDK, or MCP, includes canonical proof bytes and a reference to the exact
snapshot witness.

Offline verification checks the proof, caller-pinned anchor, snapshot, and
complete reexecution of the operation. A self-consistent proof is not an
external trust anchor; the expected anchor digest must come from a channel the
verifier trusts. See [result proof v1](../provenance/result-proof-v1.md).

## Delivery surfaces

- `hyphae`: local engine operations, recovery tools, secure server, remote
  client, offline verifier, and MCP adapter in one executable.
- `hyphae-engine`: embeddable durable facade.
- `hyphae-storage`, `hyphae-query`, and `hyphae-retrieval`: lower-level public
  libraries for callers that need the individual reference components.
- `hyphae-contracts`: typed v1 wire models and embedded OpenAPI/JSON Schemas.
- `hyphae-server` and `hyphae-client`: optional HTTP server and bounded Rust
  client.
- TypeScript and Python SDKs: bounded clients generated from public contracts.
- Astro, Next, Vite, and PliegoRS adapters: opt-in consumers outside the core.

## Default hard and service limits

The server publishes its effective values at `GET /v1/capabilities`.

| Limit | Default |
|---|---:|
| Binary key | 1 MiB |
| Canonical document | 16 MiB |
| JSON request body | 4 MiB |
| JSON depth | 64 |
| JSON nodes per request | 100,000 |
| Atomic batch items | 1,000 |
| Query scanned records | 1,000,000 |
| Query matched records | 100,000 |
| Query returned rows | 1,000 |
| Aggregation groups | 10,000 |
| Filter nodes / depth | 256 / 64 |
| Sort / group / metric fields | 16 / 8 / 32 |
| Concurrent data operations | 16 |
| Query timeout | 30 seconds |
| JSON response | 32 MiB |
| Encoded proof before base64 | 16 MiB |
| Downloadable witness | 512 MiB |

The proof codec itself accepts at most 64 MiB; the default server policy is
stricter. Programmatic server users may reduce positive limits but cannot
raise canonical hard bounds. Local query and retrieval use their reference
defaults unless the embedding application supplies different validated
limits.

## Deliberate non-capabilities in 0.1.0

Hyphae does not provide SQL, joins, floating-point documents, user-defined
indexes, a distributed protocol, replication, clustering, built-in TLS,
encryption at rest, access-control roles, multitenancy, billing, a hosted
control plane, background daemon installation, an embedding model, or an LLM.
It is not Mycelium, Hyphae Network, Celiums Network, or a cognitive runtime.

Application hosts own process supervision, TLS termination for remote access,
filesystem permissions, backup media encryption/retention, and any optional
embedding provider. Post-0.1 ideas are recorded separately in the
[roadmap](../roadmap.md#post-01-candidates); they are not current features.
