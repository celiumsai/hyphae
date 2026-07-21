# ADR-0017: Lexical retrieval uses pinned deterministic BM25F semantics

- Status: Accepted
- Date: 2026-07-20
- Owners: Celiums Solutions LLC

## Context

Hyphae needs useful offline retrieval when no embedding provider or vectors
exist. A replaceable inverted index must produce the same result as a clear
reference executor and must rebuild from authoritative documents and index
definitions.

## Decision

A lexical index is a named logical definition over explicit canonical
document field paths. Its identifier follows the same ASCII grammar as vector
spaces. Each field has a positive integer weight in millionths. Version 1
uses fixed constants `k1 = 1.2` and `b = 0.75`.

Text normalization is pinned as `hyphae-unicode-tokenizer-v1`:

1. decode UTF-8;
2. apply Unicode NFKC using pinned Unicode tables;
3. apply Unicode default case folding using pinned Unicode tables;
4. split into maximal runs of Unicode alphanumeric code points;
5. discard empty tokens and tokens longer than 256 UTF-8 bytes.

The Rust dependency versions and compatibility corpus pin the tables used by
0.2. Locale, OS collation, and hash-map iteration never affect output.
Phrase, fuzzy, stemming, stop-word, and prefix search are excluded from 0.2.

The reference executor applies the BM25F formula in
[`lexical-reference-semantics-v1.md`](../retrieval/lexical-reference-semantics-v1.md)
using a pinned pure-Rust logarithm and quantizes the final finite score to
integer nanos. Ranking uses score descending, then binary object key
ascending.

Lexical definitions are authoritative logical records. Inverted postings,
document lengths, and corpus statistics are rebuildable projections from the
definition plus canonical documents. Missing/non-string configured fields
contribute no tokens. Document mutation invalidates or updates the projection
in the same materialized transaction.

## Consequences

- Token and score behavior is versioned rather than delegated to the host.
- Index deletion can be repaired without data loss.
- Upgrading Unicode tables or BM25F constants requires a new semantics
  version.
- 0.2 intentionally omits language-specific analysis.

## Alternatives considered

- Host-locale tokenization was rejected as non-deterministic.
- An external search service was rejected as a required dependency.
- Treating postings as authority was rejected because backup/restore and
  proofs must derive from logical records.

## Verification

- Tokenizer compatibility corpus across Linux, macOS, and Windows.
- Reference-versus-index randomized corpus tests.
- BM25F golden scores, ties, field weights, empty corpus, and budget tests.
- Reopen, compaction, backup/restore, and deleted-index rebuild.
- Offline proof reexecution and tokenizer/decoder fuzzing.
