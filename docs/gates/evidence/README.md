# Hyphae 0.2 local evidence

These machine-readable files are observations from local release-candidate
gates. They are not hosted Linux/macOS/Windows release evidence and do not
authorize publication.

- `0.2-retrieval-benchmark-*.json`: deterministic mixed retrieval benchmark
  under the [0.2 methodology](../../performance/retrieval-benchmark-0.2.md).
- `0.2-score-model-benchmark-*.json`: canonical score-model comparison used by
  ADR-0015.
- `0.2-load-gate-*.json`: concurrent public HTTP write and proof-bearing
  retrieval gate.
- `0.2-soak-gate-*.json`: kill/restart, index rebuild, backup, and restore
  gate.
- `0.2-fuzz-*.json`: bounded fuzz execution counts and crash status.
- `0.2-cargo-audit-*.json`: dependency vulnerability audit result.
- `0.2-dependency-review-local.json`: reviewed dependency and lockfile delta
  from the `0.1.0` baseline.

Environment details and command parameters live inside each report when the
producer supports them. A final hosted run must be tied to the exact release
commit before `v0.2.0` can be published.
