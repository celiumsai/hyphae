# Historical source ledger

This ledger is an allowlist. If a historical file is not listed as an
accepted `Conserve` port, it must not be copied into this repository.

## Required fields

| Field | Meaning |
|---|---|
| Source repository | Canonical owner/name or immutable local identity |
| Source commit | Full commit SHA |
| Source path | Exact path at that commit |
| License and owner | Verified license and copyright holder |
| Decision | Conserve, Rewrite, or Exclude |
| Destination | New path, or `n/a` |
| Transformation | What changed and why |
| Inherited tests | Tests translated with the source |
| Reviewer | Human approval of provenance and fit |

## Accepted ports

None. The repository begins with clean-room product contracts and original
bootstrap code.

## Audited historical inputs

| Source | Immutable revision | Role |
|---|---|---|
| terrizoaguimor/hyphae-v2 | 3d06318fffb15a151520a35bd8b4f5b49954d6c5 | Cognitive Rust antecedent; read-only audit |
| local historical hyphae tree | 268290e561c309ea24ac12392a6984670c8abccc | Earlier Apache-2.0 Rust antecedent; no configured remote |
| local celiums-hyphae tree | 174ebea2aa0b9df4a4bb4ee59d30c74bf76cb8e7 | Conceptual private antecedent; no code port |
| celiumsai/hyphae-network | b6b630ca44dc549c42a7f921249b1cb210e13337 | Historical distributed product; frozen source only |

These revisions identify what was audited. They do not authorize copying any
file. An accepted port still requires the full per-file record above.

## Audited historical matrix

| Historical area | Decision | Reason |
|---|---|---|
| Cognitive fragments, cascades, conversation, surface realization | Exclude | Outside the autonomous data-engine boundary |
| Ethics and learning subsystems | Exclude | Experimental cognition, not storage/query |
| Eval corpora, benchmarks, papers, vendor scripts | Exclude | Explicitly outside product scope and may require cloud/AI |
| KV state-store implementation | Rewrite | Internal subsystem coupling and no public transactional contract |
| Journal/hash-chain concepts | Rewrite | Useful invariants, but old persistence lacks the new format, atomicity, migrations, and recovery contract |
| Exact vector scoring concepts and tests | Rewrite | Useful correctness baseline; old ordering and provider contract are unsuitable |
| Ingestion/provenance concepts and adversarial cases | Rewrite | Preserve threat cases while defining a public versioned proof contract |
| Anchor, ledger, witness, and keyring concepts | Defer | Optional hardening after local offline proofs; no external anchor is required |

Historical repositories remain untouched. A later accepted code port must add
a new row with immutable source evidence before its implementation commit.
