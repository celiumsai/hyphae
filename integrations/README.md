# Optional integrations

PliegoRS, Astro, Next, and Vite adapters live here outside the engine. Every
adapter uses only versioned public clients, remains opt-in, and has tests
proving its host software builds without Hyphae installed.

- `pliegors/`: consumer-owned Rust application-state boundary;
- `javascript/`: optional Astro, Next, and Vite package subpaths;
- `host-smoke/`: independent host builds whose lockfile contains no Hyphae.

See [`docs/integrations/optional-adapters.md`](../docs/integrations/optional-adapters.md).
