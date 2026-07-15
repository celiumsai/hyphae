# ADR-0002: Apache-2.0 and audited historical porting

- Status: Accepted
- Date: 2026-07-14
- Owners: Celiums Solutions LLC

## Context

Hyphae must be embeddable, needs an explicit patent grant, and may reuse
audited concepts or code from Apache-2.0 historical work. Historical trees
also contain inconsistent or incomplete license declarations.

## Decision

All original source and documentation in this repository use Apache-2.0.
Every source file carries an SPDX identifier once it contains executable code.

Historical material may enter only after the ledger records the repository,
commit, path, license, copyright, decision, transformation, inherited tests,
and reviewer. Unknown or incompatible provenance blocks the port. Concepts
may be reimplemented from the public contract without copying source.

## Consequences

- One license covers code, contracts, and documentation.
- Patent terms are explicit.
- A source audit is part of code review, not a later cleanup.
- Ported notices remain in `NOTICE` or `THIRD_PARTY_NOTICES.md` as required.

## Verification

CI runs license/source policy checks; reviewers compare every ported file to
its ledger entry.
