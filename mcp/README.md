# MCP adapter

The MCP adapter is the optional `hyphae mcp` stdio mode of the single binary.
It is a bounded client of an already running Hyphae `/v1` server. It never
opens a data directory, starts a listener, or imports engine/storage internals.

## Start and configure

```bash
hyphae serve --data-dir ./hyphae-data
hyphae mcp --base-url http://127.0.0.1:8787
```

An MCP host normally spawns the second command. A complete host-neutral
example is in [`examples/mcp/host-config.json`](../examples/mcp/host-config.json):

```json
{
  "mcpServers": {
    "hyphae": {
      "command": "hyphae",
      "args": ["mcp", "--base-url", "http://127.0.0.1:8787"]
    }
  }
}
```

Set `HYPHAE_BASE_URL` instead of `--base-url` when the host supports process
environment configuration. For an authenticated server, use a restricted
`--bearer-token-file`/`HYPHAE_BEARER_TOKEN_FILE` or
`HYPHAE_BEARER_TOKEN`. Never put the bearer value in the argument list.

## Protocol

- MCP revision: `2025-11-25`.
- Transport: newline-delimited JSON-RPC 2.0 over stdin/stdout.
- Maximum complete input message: 4 MiB.
- Lifecycle: `initialize`, `notifications/initialized`, `ping`, `tools/list`,
  and `tools/call`.
- Tool-list pagination and MCP tasks are not supported.

Malformed requests receive normal JSON-RPC errors. A valid tool call whose
Hyphae request fails returns an MCP tool result with `isError: true`; it does
not terminate the stdio session. Successful calls return both compact text and
`structuredContent` matching the canonical output schema.

## Tools

| Tool | Input/output contract | Behavior |
|---|---|---|
| `hyphae_capabilities` | Empty / `CapabilitiesV1` | Read effective API features and limits |
| `hyphae_put` | `PutRequestV1` / `CommitReceiptV1` | Atomic durable record batch |
| `hyphae_get` | `GetRequestV1` / `GetResponseV1` | Proven key presence or absence |
| `hyphae_delete` | `DeleteRequestV1` / `CommitReceiptV1` | Atomic durable delete batch |
| `hyphae_query` | `QueryRequestV1` / `QueryResponseV1` | Proven deterministic structured query |
| `hyphae_define_vector_space` | `DefineVectorSpaceRequestV1` / `CommitReceiptV1` | Define/reuse an immutable durable Q15 vector space |
| `hyphae_put_vectors` | `PutVectorsRequestV1` / `CommitReceiptV1` | Atomic durable vector batch |
| `hyphae_delete_vectors` | `DeleteVectorsRequestV1` / `CommitReceiptV1` | Atomic durable vector deletion batch |
| `hyphae_retrieve_exact` | `ExactRetrievalRequestV1` / `ExactRetrievalResponseV1` | Proven exact cosine retrieval |
| `hyphae_define_lexical_index` | `DefineLexicalIndexRequestV1` / `CommitReceiptV1` | Define/reuse a provider-free lexical index |
| `hyphae_retrieve_lexical` | `LexicalRetrievalRequestV1` / `LexicalRetrievalResponseV1` | Proven BM25F retrieval |
| `hyphae_retrieve_hybrid` | `HybridRetrievalRequestV1` / `HybridRetrievalResponseV1` | Proven RRF fusion |

Input/output schemas are embedded from `contracts/json-schema`; the adapter
does not maintain a parallel hand-written model. Put/delete are annotated as
destructive and require host/user authorization. Read tools are annotated
read-only. Supplying an explicit transaction UUID makes exact mutation retries
durably idempotent, but the MCP annotation remains conservative.

MCP does not expose backup, restore, compaction, server lifecycle, raw witness
download, offline verification, or filesystem operations. Use the local CLI
or public SDK for those authorized workflows. Proof objects returned by
get/query/retrieval remain normal v1 public models; download the witness with
an SDK or `hyphae remote witness` before offline verification.

## Conformance

The common live suite initializes a fresh MCP session, lists the twelve tools,
executes KV/vector/lexical/hybrid behavior, validates positive and negative
structured outputs, and compares them with Rust, TypeScript, Python, and
remote CLI clients:

```bash
cargo build -p hyphae-cli -p hyphae-conformance-rust --locked
(cd sdks/typescript && npm ci --ignore-scripts && npm run build)
python tools/run_conformance.py
```

See [public clients](../docs/clients/v1.md#mcp), [configuration](../docs/configuration.md),
and [API v1](../docs/api/v1.md).
