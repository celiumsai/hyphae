# ADR-0014: Durable exact retrieval is the normative 0.2 vector path

- Status: Accepted
- Date: 2026-07-20
- Owners: Celiums Solutions LLC

## Context

The 0.1 reference executor already performs exact global cosine retrieval over
host-supplied `f64` vectors. Durable retrieval needs a public request model,
consistent snapshot semantics, deterministic scores, explicit budgets, and
typed abstention without introducing an embedding provider or approximate
index.

## Decision

Exact retrieval scans every logical vector in one named space from a single
materialized checkpoint. The materialized vector table must match the log
checkpoint before it can serve a result. Any durable-but-unmaterialized commit
causes stale-index refusal until replay or rebuild succeeds.

Requests contain:

- a canonical vector-space identifier;
- a nonzero Q15 query vector with the space's exact dimension;
- nonzero `limit`;
- inclusive `minimum_score_nanos` in
  `[-1_000_000_000, 1_000_000_000]`;
- `minimum_margin_nanos` in `[0, 2_000_000_000]`; and
- server-owned resource limits.

The canonical score is defined by ADR-0015. Candidates sort by canonical score
descending and then binary object key ascending. Every candidate is considered
before the final limit. 0.2 exact retrieval has no document filter.

Outcomes are either matches or typed abstention:

- `no_candidates`;
- `below_threshold`; or
- `ambiguous`.

Malformed vectors, unknown spaces, dimension mismatch, duplicate logical
identity, exhausted budgets, timeout, or stale materialization are errors and
never abstentions. Budget or timeout failure returns no partial ranking.

Candidate count, decoded bytes, dimension, returned matches, and one monotonic
deadline are bounded across scan, decode, score, sort, and result creation.

## Consequences

- Complexity is linear in the vectors in one space.
- Exact retrieval is the quality oracle for all future indexes.
- HNSW, disk ANN, per-shard truncation, and provider behavior cannot change
  0.2 results.
- The existing host-supplied f64 reference API remains available for 0.1
  compatibility but is not the durable/public 0.2 contract.

## Alternatives considered

- ANN-first retrieval was rejected because 0.2 requires complete,
  proof-replayable results.
- Returning an empty success for policy rejection was rejected because it
  erases evidence.
- Floating thresholds were rejected because they would bypass the canonical
  score representation.

## Verification

- Brute-force oracle and randomized shard/input-order properties.
- Ties, near-ties, thresholds, margins, empty spaces, wrong dimensions,
  budgets, timeouts, and large dimensions.
- Equal results before and after reopen, compaction, backup/restore, and index
  rebuild.
- Cross-platform golden vectors using canonical score integers.
