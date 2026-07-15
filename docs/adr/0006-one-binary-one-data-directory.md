# ADR-0006: One binary and one data directory define the base experience

- Status: Accepted
- Date: 2026-07-15
- Owners: Celiums Solutions LLC

## Context

A base installation assembled from multiple daemons, databases, or provider
credentials would contradict Hyphae's autonomous product boundary.

## Decision

The repository produces one executable named hyphae. Embedded use remains a
Rust library, while local operations, HTTP serving, MCP, backup, restore,
migration, doctor, and verification are subcommands of the same executable.

One data directory contains the format marker, writer lock, log, manifests,
snapshots, rebuildable indexes, blobs, and temporary files. A new local
instance requires only that directory to be writable. The engine performs no
network request by default.

## Consequences

- Packaging and installation have one primary artifact.
- Operational commands share the same format and verification code.
- Optional adapters and providers cannot become hidden installation
  requirements.

## Verification

Release tests install one archive, create a fresh data directory, exercise
offline operations, and assert that the default process makes no outbound
connection.
