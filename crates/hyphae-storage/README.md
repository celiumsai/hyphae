<p align="center"><a href="https://hyphae.dev"><img alt="Hyphae" src="https://raw.githubusercontent.com/celiumsai/hyphae/main/.github/assets/hyphae-lockup.svg" width="320"></a></p>

# hyphae-storage

[![crates.io](https://img.shields.io/crates/v/hyphae-storage?logo=rust)](https://crates.io/crates/hyphae-storage)
[![docs.rs](https://img.shields.io/docsrs/hyphae-storage)](https://docs.rs/hyphae-storage)

Durable local storage primitives for [Hyphae](https://hyphae.dev): an
append-only checksummed and digest-chained log, atomic/idempotent mutation,
recovery, snapshots, compaction, backups, and verified restore.

```toml
[dependencies]
hyphae-storage = "0.1.0"
```

Most applications should use `hyphae-engine`. Use this lower-level crate only
when directly implementing the documented durable formats and ownership model.

Apache-2.0. Source and security policy:
[`celiumsai/hyphae`](https://github.com/celiumsai/hyphae).
