<p align="center"><a href="https://hyphae.dev"><img alt="Hyphae" src="https://raw.githubusercontent.com/celiumsai/hyphae/main/.github/assets/hyphae-lockup.svg" width="320"></a></p>

# hyphae-query

[![crates.io](https://img.shields.io/crates/v/hyphae-query?logo=rust)](https://crates.io/crates/hyphae-query)
[![docs.rs](https://img.shields.io/docsrs/hyphae-query)](https://docs.rs/hyphae-query)

Pure deterministic query types and reference execution for
[Hyphae](https://hyphae.dev). It provides structured values, filters, global
sorting, logical cursors, aggregations, and explicit execution budgets.

```toml
[dependencies]
hyphae-query = "0.1.0"
```

The executor has no database, network, embedding, or LLM dependency. Budget
or timeout exhaustion returns an error rather than partial success.

Apache-2.0. Source and security policy:
[`celiumsai/hyphae`](https://github.com/celiumsai/hyphae).
