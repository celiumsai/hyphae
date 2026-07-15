# Hyphae repository instructions

## Product boundary

- Hyphae is an autonomous Rust data engine: one binary and one data directory.
- The default product must work offline without a database, cache, cloud,
  embedding provider, or LLM.
- PliegoRS, Mycelium, Hyphae Network, Celiums Network, cognitive experiments,
  hosted SaaS concerns, billing, and cloud operations are outside this repo.
- Integrations and semantic providers consume only public versioned contracts.

## Historical source

- Historical repositories are frozen read-only inputs.
- Do not copy or cherry-pick historical code without an accepted entry in
  docs/porting/ledger.md.
- Keep provenance, license, transformation, inherited tests, and human review
  explicit for every accepted port.

## Engineering rules

- Use English for code, contracts, commit messages, and repository docs.
- Keep unsafe Rust forbidden unless an accepted ADR narrows an audited use.
- Change public behavior contract-first.
- Add failure-path tests for durable behavior.
- Do not claim a roadmap phase complete without its exit evidence.
- Never add an automation attribution trailer to a commit.
