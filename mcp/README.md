# MCP adapter

The MCP adapter is the optional `hyphae mcp` stdio mode of the single binary.
It consumes the public Rust HTTP client and implements stable MCP revision
`2025-11-25` with bounded newline-delimited JSON-RPC messages.

It exposes five versioned structured tools with canonical JSON Schema input
and output contracts. It does not own storage semantics, open a data
directory, or call internal engine types. See
[`docs/clients/v1.md`](../docs/clients/v1.md#mcp).
