# Versioning policy

Hyphae versions three surfaces independently:

- Product and SDK releases use Semantic Versioning starting with 0.1.0.
- HTTP contracts use an explicit path version such as /v1.
- The data directory carries a numeric on-disk format version.

An SDK release may add helpers without changing /v1. An engine release may
migrate its disk format without changing the HTTP contract. A breaking wire
change requires a new API path version. A future disk format is rejected by
an older binary rather than guessed or downgraded.

No untagged private candidate is a public compatibility release. The `0.1.0`
policy becomes active only for an explicitly authorized tag whose exact
commit passes every release gate.
