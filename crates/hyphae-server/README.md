<p align="center"><a href="https://hyphae.dev"><img alt="Hyphae" src="https://raw.githubusercontent.com/celiumsai/hyphae/main/.github/assets/hyphae-lockup.svg" width="320"></a></p>

# hyphae-server

[![crates.io](https://img.shields.io/crates/v/hyphae-server?logo=rust)](https://crates.io/crates/hyphae-server)
[![docs.rs](https://img.shields.io/docsrs/hyphae-server)](https://docs.rs/hyphae-server)

Secure, loopback-first HTTP server for the [Hyphae](https://hyphae.dev) data
engine and its proof-bearing `/v1` API.

```toml
[dependencies]
hyphae-server = "0.2.0"
```

Remote bind requires explicit authentication. Request, result, concurrency,
memory, and timeout limits are part of the public behavior.

Apache-2.0. Source, threat model, and security policy:
[`celiumsai/hyphae`](https://github.com/celiumsai/hyphae).
