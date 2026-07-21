# ADR-0015: Canonical retrieval scores use integer cosine nanos

- Status: Accepted
- Date: 2026-07-20
- Owners: Celiums Solutions LLC

## Context

Persisted scores, thresholds, ordering, hybrid fusion, and proof bytes cannot
depend on an unspecified floating-point implementation. Hyphae needs one
portable score representation that is bounded, sortable, and exactly
recomputable offline.

## Decision

Durable 0.2 vectors and queries use Q15 elements from ADR-0013. For vectors
`a` and `b`, compute with checked signed/unsigned 128-bit integers:

```text
dot = sum(a[i] * b[i])
norm_product = sum(a[i]^2) * sum(b[i]^2)
denominator = floor_sqrt(norm_product)
absolute_nanos =
  min(1_000_000_000,
      floor((abs(dot) * 1_000_000_000 + denominator / 2)
            / denominator))
score_nanos = sign(dot) * absolute_nanos
```

`floor_sqrt` is the specified integer square root. Zero magnitude, `-32768`,
overflow, or dimension mismatch is invalid. Integer division truncates toward
zero after the explicit positive rounding step. Zero has one representation.

Ranking, threshold, margin, result encoding, and proof verification use
`score_nanos`, not an intermediate float. JSON exposes the signed integer.
Clients may display `score_nanos / 1_000_000_000` as a convenience but that
decimal is not normative.

A reproducible benchmark compared f32 storage, Q15/i16, and Q6/i32 against an
f64 brute-force oracle. On the fixed 2,048-candidate, 32-query,
128-dimensional corpus, Q15 matched top-1 and mean top-10 at `1.0`, used two
bytes per element, and avoided floating-point proof semantics. The checked-in
report records the environment and seed.

## Consequences

- Scoring is deterministic across supported targets given the same integer
  vectors.
- Quantization quality is explicit and occurs before durable storage.
- The score approximates real cosine by a documented integer procedure; it is
  not an IEEE-754 cosine byte-for-byte encoding.
- Future score algorithms require a new retrieval semantics version.

## Alternatives considered

- `f32` storage plus `f64` accumulation matched the benchmark oracle but was
  rejected as the canonical proof path because platform/compiler behavior was
  not part of the contract.
- Q6/i32 also matched the corpus but doubled vector storage with no measured
  ranking benefit.
- Fixed decimal strings were rejected because parsing and square root still
  required a canonical numeric algorithm.

## Verification

- [`0.2-score-model-benchmark-windows-x86_64.json`](../gates/evidence/0.2-score-model-benchmark-windows-x86_64.json)
- Reproducible `tools/benchmark_score_models.py`.
- Golden exact-score vectors, integer-square-root boundary tests, overflow
  tests, and cross-platform conformance.
