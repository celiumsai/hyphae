# Configuration reference

Hyphae has no required configuration file. The single binary receives
explicit command options and four optional environment variables. A fresh
data directory is initialized on first open.

## Environment variables

| Variable | Used by | Meaning |
|---|---|---|
| `HYPHAE_DATA_DIR` | Local data commands, `serve` | Data directory when `--data-dir` is absent |
| `HYPHAE_BASE_URL` | `remote`, `mcp` | Root HTTP(S) origin when `--base-url` is absent |
| `HYPHAE_BEARER_TOKEN_FILE` | `serve`, `remote`, `mcp` | Restricted token file when the option is absent |
| `HYPHAE_BEARER_TOKEN` | `serve`, `remote`, `mcp` | Token value used only when no token file was selected |

An explicit command option wins over its corresponding environment variable.
If a token file is selected, its contents win over `HYPHAE_BEARER_TOKEN`.
There is intentionally no command-line option containing the token value.

## Data directory ownership

Every command that opens a data directory obtains an operating-system
exclusive lock and retains it for the command lifetime. The server retains it
until graceful shutdown. A second owner fails rather than sharing one writer.

Use a local filesystem whose durability and atomic rename behavior match the
operating system. Do not place a live directory inside a backup, allow another
process to edit its files, or synchronize individual files while Hyphae owns
it. Application code should treat every internal path as opaque.

## Bearer tokens

The server requires a bearer token before binding a non-loopback address. A
token must contain 32 through 4,096 visible ASCII bytes without whitespace.
Hyphae retains only a BLAKE3 digest in server state and compares candidates in
constant time.

One optional trailing LF or CRLF is removed from a file. Embedded newlines are
rejected. On Unix, token files with any group or other permission bits are
rejected:

```bash
umask 077
printf '%s\n' 'replace-with-at-least-32-visible-ascii-bytes' > hyphae.token
hyphae serve --data-dir ./data --bind 0.0.0.0:8787 \
  --bearer-token-file ./hyphae.token
```

On Windows, restrict the file with the owning account's ACL. Environment
variables avoid argv exposure but may still be visible to privileged process
inspection. Choose the secret channel appropriate for the host.

Bearer authentication is not transport encryption. Put any remotely exposed
server behind a trusted TLS boundary; Hyphae `0.2.0` does not manage
certificates.

## Server defaults

`hyphae serve` binds `127.0.0.1:8787`. It exposes no option for changing
resource budgets because the packaged binary has one audited default policy.
The effective values are returned by `/v1/capabilities` and summarized in
[product capabilities](product/capabilities.md#default-hard-and-service-limits).

Rust applications embedding `hyphae-server` can construct `ServerConfig` and
replace `ServerLimits` before `HyphaeServer::open`. All budgets must be
positive and canonical depth, node, and proof bounds cannot be exceeded.

## Public client defaults

The Rust, TypeScript, and Python clients accept only a root `http` or `https`
origin without credentials, path prefix, query, or fragment. Their local
defaults are:

| Client bound | Default |
|---|---:|
| Complete request/response deadline | 60 seconds |
| JSON response | 32 MiB |
| Snapshot witness | 512 MiB |

These client-side bounds are independent from, and may be stricter than, the
server policy. A bearer token is attached only to protected data routes; the
public health and capability routes do not need it.

## Version surfaces

`hyphae version --json` reports the product/engine version, API path version,
and disk format independently. A change to one does not imply a change to the
others. See [compatibility and versioning](compatibility/versioning.md).
