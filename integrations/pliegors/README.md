# Optional PliegoRS adapter boundary

`hyphae-pliegors` is an opt-in application-state wrapper around the public
`hyphae-client` crate. It contains no PliegoRS code, imports no private
PliegoRS API, and never opens a Hyphae data directory.

When neither `HYPHAE_BASE_URL` nor `HYPHAE_BEARER_TOKEN` exists,
`PliegoHyphaeConfig::from_env()` returns `Ok(None)`. A PliegoRS application can
therefore keep Hyphae completely absent. When enabled, the application owns
the decision to place the cloneable `PliegoHyphae` value into its public state
mechanism.

This repository intentionally does not prescribe or copy a PliegoRS internal
state API. A separate PliegoRS-side change may consume this public crate later;
Hyphae never requires it.
