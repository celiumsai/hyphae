<p align="center"><a href="https://hyphae.dev"><img alt="Hyphae" src="https://raw.githubusercontent.com/celiumsai/hyphae/main/.github/assets/hyphae-lockup.svg" width="320"></a></p>

# hyphae-engine

[![crates.io](https://img.shields.io/crates/v/hyphae-engine?logo=rust)](https://crates.io/crates/hyphae-engine)
[![docs.rs](https://img.shields.io/docsrs/hyphae-engine)](https://docs.rs/hyphae-engine)

The recommended embeddable facade for [Hyphae](https://hyphae.dev), an
autonomous, durable, and verifiable Rust data engine.

```toml
[dependencies]
hyphae-engine = "0.1.0"
```

Open one data directory, store structured records, run deterministic queries,
create snapshots and backups, and emit portable result proofs. The base path
works offline without an external database, cache, cloud, embedding provider,
or LLM.

Apache-2.0. Source, examples, and security policy:
[`celiumsai/hyphae`](https://github.com/celiumsai/hyphae).
