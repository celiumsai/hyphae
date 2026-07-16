# SDKs

Hyphae ships three bounded clients for the public `/v1` API:

| Client | Location | Runtime floor | Runtime dependencies |
|---|---|---:|---|
| Rust | `crates/hyphae-client` | Rust 1.89 | Reqwest/Rustls through Cargo |
| TypeScript | [`typescript`](typescript/README.md) | Node.js 20 | None |
| Python | [`python`](python/README.md) | Python 3.11 | None |

Every client accepts one root HTTP(S) origin, optional bearer authentication,
a complete deadline, a JSON response bound, and a snapshot witness bound.
They expose capabilities, liveness, readiness, put, delete, get, query, and
witness download. They reject malformed success/error envelopes and require
one matching `X-Request-Id`.

TypeScript/Python models are generated from canonical JSON Schema and checked
in. Regenerate after contract changes and verify no drift:

```bash
python tools/generate_sdk_models.py
python tools/generate_sdk_models.py --check
```

All clients pass the same live black-box fixture as remote CLI and MCP. No SDK
opens storage, imports engine internals, or requires an optional provider. See
[public clients](../docs/clients/v1.md) and [API v1](../docs/api/v1.md).
