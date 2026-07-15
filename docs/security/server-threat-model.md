# `/v1` server threat model

## Trust boundaries

The HTTP listener accepts untrusted transport metadata and request bytes. The
embedded engine and its owned data directory are trusted only after normal
format, log, snapshot, and index verification. A bearer token authenticates
access to one process; it does not establish multitenancy or end-to-end TLS.

## Defaults

- No listener starts during ordinary embedded or CLI operations.
- `hyphae serve` defaults to `127.0.0.1:8787`.
- A non-loopback bind is rejected before `bind(2)` unless bearer
  authentication is configured.
- Data operations never contact an upstream service.

## Enforced limits

- complete request body bytes and receive time before durable work starts;
- JSON nesting depth and total nodes;
- key, document, batch, filter, sort, group, metric, page, scan, match, proof,
  snapshot witness, and response sizes;
- admitted concurrent data operations;
- cooperative query and verification timeouts.

Limit failures return a stable error and no partial query or proof result.
Once a durable write begins it is allowed to reach a definite receipt rather
than being cancelled behind the engine's back.

A commit that is durable but not materialized still returns its receipt, then
marks readiness unavailable. Subsequent engine operations fail closed until a
restart replays the authoritative log. Existing verified witnesses remain
downloadable during that state.

## Authentication

Configured bearer tokens are hashed before comparison. Missing and incorrect
credentials return the same `unauthorized` response. Tokens are never logged,
echoed, or included in request IDs. Health endpoints disclose no records or
configuration secrets.

Tokens contain at least 32 visible ASCII bytes. The CLI accepts them from an
environment variable or restricted file, never from an argv value. Unix token
files with group/other permissions are rejected; Windows operators must apply
an account-only ACL.

Remote deployment still requires a trusted TLS boundary. Hyphae 0.1.0 does
not claim built-in certificate management, OAuth, tenant isolation, WAF,
rate-limit coordination across processes, or denial-of-service resistance
beyond its documented local limits.

## Error disclosure

Public errors expose a stable code, bounded human message, and request ID.
Internal filesystem paths, tokens, Rust backtraces, document contents, and
dependency errors are not returned. Detailed diagnostics remain local logs.
