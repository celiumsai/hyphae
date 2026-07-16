# Security policy

Do not disclose suspected vulnerabilities in public issues, discussions, pull
requests, or chat logs.

Report a vulnerability through GitHub private vulnerability reporting for
`celiumsai/hyphae`, or contact `security@celiums.ai` if that channel is not
available. Include the affected revision, platform, reproduction steps,
impact, and any proposed mitigation.

## Supported versions

| Version | Supported |
|---|---|
| `0.1.x` | Yes |
| `< 0.1.0` | No |

## Baseline security guarantees

- The server binds to loopback by default.
- Remote binding requires explicit configuration and authentication.
- Inputs have body, depth, batch, result, timeout, and concurrency limits.
- Corrupt or future on-disk formats fail closed.
- Result proofs are verifiable offline.
- External providers are optional and cannot enter the core dependency path.

These guarantees are validated release gates for `0.1.0`. Any source change
requires the complete gate matrix to pass again on the new exact commit.
