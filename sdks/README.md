# SDKs

Rust embedding is delivered by the workspace facade added with the durable
engine. TypeScript and Python SDKs are introduced in Phase 6 and must pass the
same black-box conformance suite as the Rust client.

No SDK may import storage internals or require an optional provider.
