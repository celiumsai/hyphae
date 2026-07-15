# ADR-0005: Workspace layers have one-way dependencies

- Status: Accepted
- Date: 2026-07-15
- Owners: Celiums Solutions LLC

## Context

Storage internals, delivery surfaces, generated wire models, and optional
providers must evolve independently. Cycles or framework imports would make
internal implementation details part of the product contract.

## Decision

The initial workspace contains core, storage, query, retrieval, contracts,
server, client, and CLI crates. Core owns stable values only. Storage, query,
and retrieval may depend on core. Server and client may depend on contracts
and core, but never on each other. The CLI composes public libraries and is
the only executable artifact.

The future engine coordinator and embeddable facade are added only when the
durability primitives they expose are real. SDKs, MCP, providers, and
framework adapters remain outside the Rust core dependency graph.

## Consequences

- Internal formats can change without leaking into clients.
- Dependency direction is reviewable from Cargo metadata.
- Some early crates intentionally expose no public behavior until their
  invariants have tests.

## Verification

CI builds every workspace target. Architecture tests will reject forbidden
dependency edges when the coordinator is introduced.
