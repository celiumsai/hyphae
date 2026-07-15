# SDKs

The bounded Rust HTTP client lives in `crates/hyphae-client`. TypeScript and
Python SDKs are generated from canonical JSON Schema models and implemented in
this directory. TypeScript and Python have no runtime package dependencies.

All three pass the same live black-box fixture as CLI and MCP. No SDK imports
storage internals or requires an optional provider. See
[`docs/clients/v1.md`](../docs/clients/v1.md).
