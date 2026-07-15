# ADR-0007: Reference semantics precede optimized query and retrieval

- Status: Accepted
- Date: 2026-07-15
- Owners: Celiums Solutions LLC

## Context

Filters, sorting, pagination, aggregation, shard merge, and semantic ranking
can each look correct locally while producing a wrong global result. Provider
behavior and floating-point corner cases add more ambiguity. Hyphae needs one
small executable definition against which future indexes and distributed
adapters can be checked.

## Decision

`hyphae-query` owns a deterministic reference executor over typed logical
records. Structured values exclude floating point: integers, strings, bytes,
booleans, null, arrays, and ordered objects have a documented total order.
Missing paths remain distinct from explicit null for filters and grouping.

All shards are scanned into one logical match set before sorting, cursor
application, final limit, or aggregation output. Sort ties end with the binary
record key ascending; duplicate global keys are errors. Aggregations cover the
complete filtered set before cursor pagination. Runtime work, result, group,
shape, and monotonic timeout budgets fail explicitly instead of returning a
partial result.

`hyphae-retrieval` owns exact linear vector scoring and stable global merge.
It accepts vectors directly and has no default embedding or model provider.
Non-finite values and dimension mismatches are errors. Threshold and ambiguity
policies return an explicit abstention outcome rather than an empty success or
fabricated match.

Optimized indexes and provider adapters must pass the same reference vectors
and conformance corpus before replacing these semantics.

## Consequences

- The reference path favors clarity and bounded correctness over throughput.
- Structured numeric query is integer-only until a canonical decimal format is
  accepted; semantic scoring remains finite `f64` with total tie-breaking.
- Cursor bytes become a later public-contract encoding of an already fixed
  logical position.
- Per-shard limiting before global merge is nonconforming.
- Semantic retrieval remains optional and cannot affect KV or structured query.

## Verification

Reference tests cover heterogeneous values, missing/null behavior, sort ties,
cursor continuation, cross-shard top-k, grouped/global aggregates, overflow,
duplicate keys, every budget class, timeout injection, vector dimensions,
score ties, thresholds, ambiguity margins, and abstention reasons.
