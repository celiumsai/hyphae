# HTTP request examples

These files are complete API v1 request bodies. Start a loopback server, then
submit them through the remote mode of the same binary:

```bash
export HYPHAE_DATA_DIR="$PWD/example-server-data"
hyphae serve
```

In another shell:

```bash
hyphae remote --base-url http://127.0.0.1:8787 put --request examples/http/put.json
hyphae remote --base-url http://127.0.0.1:8787 get --request examples/http/get.json
hyphae remote --base-url http://127.0.0.1:8787 query --request examples/http/query.json
hyphae remote --base-url http://127.0.0.1:8787 delete --request examples/http/delete.json
```

`query.json` demonstrates a recursive comparison, deterministic sort, grouped
`count` and `sum`, and an explicit query timeout. Successful get/query
responses contain a `proof`; save that object and use `remote witness` to
download its canonical snapshot before offline verification.

The canonical payload definitions are in
[`contracts/json-schema`](../../contracts/README.md).
