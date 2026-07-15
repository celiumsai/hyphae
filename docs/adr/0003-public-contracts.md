# ADR-0003: Integrations consume only public versioned contracts

- Status: Accepted
- Date: 2026-07-14
- Owners: Celiums Solutions LLC

## Context

Framework-specific coupling would make Hyphae mandatory for unrelated
software and let internal Rust layouts leak into clients.

## Decision

OpenAPI 3.1 and JSON Schema 2020-12 under `contracts/` are authoritative for
HTTP `/v1`. Rust, TypeScript, Python, CLI, MCP, and optional adapters share
wire fixtures and black-box conformance tests. Non-Rust clients and adapters
may not import internal engine or storage types.

API version, SDK version, and on-disk format version evolve independently.
Breaking wire changes require a new public API version.

## Consequences

- PliegoRS, Astro, Next, Vite, and other software work without Hyphae.
- Generated-code drift and compatibility become CI failures.
- Internal storage can change without forcing client releases.

## Verification

Dependency checks and conformance tests execute every client against the same
fixtures and live server behavior.
