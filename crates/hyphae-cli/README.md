<p align="center"><a href="https://hyphae.dev"><img alt="Hyphae" src="https://raw.githubusercontent.com/celiumsai/hyphae/main/.github/assets/hyphae-lockup.svg" width="320"></a></p>

# hyphae-cli

[![crates.io](https://img.shields.io/crates/v/hyphae-cli?logo=rust)](https://crates.io/crates/hyphae-cli)
[![GitHub release](https://img.shields.io/github/v/release/celiumsai/hyphae?logo=github)](https://github.com/celiumsai/hyphae/releases/latest)

The single `hyphae` executable: local data engine, operations CLI, `/v1`
server, remote client, offline proof verifier, and MCP stdio adapter.

```bash
cargo install hyphae-cli --version 0.1.0 --locked
hyphae version --json
```

The base deployment is one binary and one data directory. KV, structured
query, recovery, backup/restore, and verification work without an external
database, cache, cloud, embedding provider, or LLM.

Apache-2.0. Quickstart, release verification, and security policy:
[`celiumsai/hyphae`](https://github.com/celiumsai/hyphae).
