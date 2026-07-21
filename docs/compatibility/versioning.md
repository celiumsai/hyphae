# Versioning policy

Hyphae versions three surfaces independently:

- Product and SDK releases use Semantic Versioning starting with `0.1.0`.
- HTTP contracts use an explicit path version such as `/v1`.
- The data directory carries a numeric on-disk format version.

Hyphae `0.2.0` keeps API `/v1` and adds optional request/response shapes and
routes. It advances the current data directory to format `2`. Format 2 adds
authoritative vector-space definitions, signed-Q15 vectors, lexical-index
definitions, and their snapshot/receipt identity.

A 0.2 binary opens format 1 by acquiring the exclusive directory lock,
validating all existing state, and atomically committing the format-2
migration before it writes new logical mutations. The immutable format-1
fixture remains an open/migration compatibility gate. The immutable format-2
fixture proves recovery and retrieval without a materialized Redb index.

An older 0.1 binary rejects format 2 before log replay; downgrade in place is
unsupported. Rollback uses a verified pre-upgrade backup restored to a new
directory with a compatible binary. Future formats are likewise rejected
rather than guessed or partially replayed.

An SDK release may add helpers without changing `/v1`. A breaking wire change
requires a new API path version. Proof formats and retrieval semantics carry
their own explicit versions and are rejected when unsupported.

No untagged candidate is a public compatibility release. A tag is valid only
when its exact commit passes every release gate.
