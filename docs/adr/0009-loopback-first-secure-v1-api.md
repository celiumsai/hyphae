# ADR-0009: Loopback-first secure `/v1` API

- Status: Accepted
- Date: 2026-07-15
- Owners: Celiums Solutions LLC

## Context

Hyphae needs one optional HTTP delivery surface without turning the embedded
engine into a network dependency. A listener expands the threat model:
untrusted JSON, oversized bodies, slow or concurrent work, bearer secrets,
remote exposure, unstable framework errors, and proof/witness transfer all
become public behavior.

## Decision

The only HTTP contract is OpenAPI 3.1 under `/v1`. The single `hyphae` binary
starts no listener by default; `hyphae serve` binds to `127.0.0.1:8787` unless
configured otherwise.

Any non-loopback IP address requires an explicit bearer token before the
socket is opened. The token is supplied through an environment variable or a
restricted token file, never required by the autonomous embedded mode. Data,
query, proof, and witness endpoints require authentication whenever a token
is configured. Liveness remains side-effect free; readiness reports whether
the engine was opened and is serving.

Durable data operations are:

- `POST /v1/kv/put`;
- `POST /v1/kv/get`;
- `POST /v1/kv/delete`;
- `POST /v1/query`;
- `GET /v1/witnesses/{checkpoint_sequence}/{snapshot_digest}`.

KV get and query responses always include result-proof bytes plus the trusted
anchor material and a witness reference. Write and delete responses include
the durable commit receipt. No public endpoint bypasses `hyphae-engine`.

The server owns one data-directory writer lock and serializes durable engine
access behind bounded admission. JSON is read under a byte limit, parsed to a
generic tree, checked for depth and node limits, then converted to typed wire
models. Batch size, query shape, scan/match/result/group work, response bytes,
proof bytes, verification witness bytes, concurrency, and query timeout are
bounded by server policy. A limit failure returns no partial logical result.

Every error uses the versioned JSON envelope and a stable machine code. Every
response carries a generated request ID. Framework default text errors are
not part of the public contract.

## Consequences

- Embedded callers and CLI operations remain fully functional without HTTP.
- Accidental LAN/Internet exposure fails before bind rather than warning.
- Large proof-bearing responses can fail with `result_too_large`; clients may
  narrow a query or use pagination.
- Durable writes are not abandoned after commit begins. Query deadlines are
  enforced cooperatively inside the engine.
- Bearer authentication protects transport access but does not replace TLS.
  Operators exposing Hyphae beyond a trusted host must terminate TLS in an
  explicitly configured local deployment boundary.
- Multitenancy, billing, cloud ingress, OAuth, and hosted control planes remain
  outside this repository.

## Verification

Contract fixtures and black-box tests cover loopback defaults, remote-bind
rejection, missing/wrong/correct bearer tokens, body and JSON limits, batch
and query limits, concurrency admission, stable error envelopes, request IDs,
proof-bearing reads, witness download, and graceful shutdown.
