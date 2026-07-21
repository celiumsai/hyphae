# ADR-0018: Hybrid retrieval uses explainable reciprocal-rank fusion

- Status: Accepted
- Date: 2026-07-20
- Owners: Celiums Solutions LLC

## Context

Vector and lexical retrieval recover different relevant records. Hyphae needs
a provider-free fusion rule that does not compare incompatible raw scores and
that is deterministic, explainable, bounded, and replayable in a proof.

## Decision

Hybrid semantics version `rrf-v1` executes one lexical branch and one exact
vector branch at the same consistent snapshot. Each branch has its own
candidate limit. Duplicate object keys are combined by identity.

Ranks are one-based. With fixed `k = 60` and positive integer branch weights
in `1..=1_000_000`, each present contribution is:

```text
floor(weight * 1_000_000_000 / (60 + rank))
```

The canonical fusion score is the checked sum of lexical and vector
contributions. Results sort by fusion score descending and then binary object
key ascending. Raw branch scores never enter the fusion formula.

The explanation for each result contains lexical rank/score when present,
vector rank/score when present, both integer contributions, fusion score, and
final rank.

If one branch has no candidates or policy-abstains, the other branch may
produce a typed single-modality outcome and the absent reason is preserved.
If both branches abstain, hybrid returns typed abstention with both reasons.
Malformed input, unknown definitions, budgets, timeout, or stale indexes are
errors and produce no partial result.

Proof generation binds both canonical branch requests, branch outcomes,
fusion parameters, explanations, and the final outcome. Offline verification
reexecutes both branches and fusion.

## Consequences

- Hybrid ranking is independent of incompatible vector/BM25F score scales.
- Candidate limits are part of observable semantics.
- Single-modality fallback is explicit rather than silent.
- Reranking and learned fusion remain outside 0.2.

## Alternatives considered

- Weighted addition of raw vector and lexical scores was rejected because the
  scales are not comparable.
- Unexplained fallback was rejected because clients and proofs could not
  distinguish degraded execution.
- Configurable `k` was deferred to keep 0.2 conformance small.

## Verification

- Golden RRF vectors for duplicates, ties, missing branches, branch
  abstention, weights, and candidate limits.
- Input-order and platform invariance.
- Offline proof reexecution and tamper tests.
- No-partial-result tests for every branch and fusion failure.
