# Python SDK

`hyphae-sdk` is the synchronous, bounded Python client for API v1. It requires
Python 3.11 or newer, uses only the standard library at runtime, and includes
typed generated models plus a `py.typed` marker. The `0.2.0` candidate
is not published until release authorization.

## Test from this repository

```bash
PYTHONPATH=sdks/python/src \
  python -m unittest discover -s sdks/python/tests -v
```

## Use

```python
import os
from hyphae_sdk import HyphaeClient

client = HyphaeClient(
    "http://127.0.0.1:8787",
    bearer_token=os.getenv("HYPHAE_BEARER_TOKEN"),
    timeout_seconds=60.0,
    response_bytes=32 * 1024 * 1024,
    witness_bytes=512 * 1024 * 1024,
)

receipt = client.put({
    "records": [{"key_hex": "616c706861", "value": {"score": 10}}]
})
response = client.get({"key_hex": "616c706861"})
witness = client.download_witness(response.value["proof"])

print(receipt.value["status"], response.request_id, len(witness.value))
```

Methods are `capabilities`, `liveness`, `readiness`, `put`, `delete`, `get`,
`query`, and `download_witness`. Every result is an immutable `ApiResponse`
containing `value` and `request_id`. Python integers preserve Hyphae's full
signed 64-bit document domain; floating-point JSON is rejected.

## Errors and bounds

- `HyphaeApiError` is a valid server-declared v1 error and exposes `status`,
  stable `code`, `request_id`, and `server_message`.
- `HyphaeClientError` covers local configuration, transport, deadline, size,
  media-type, request-ID, JSON contract, or witness verification failure.

The client accepts only a root HTTP(S) origin. Response reading is chunked and
bounded by both size and a monotonic deadline. Witness download validates the
canonical path, BLAKE3 digest header, and exact length from the proof.

See [public client semantics](../../docs/clients/v1.md),
[data model](../../docs/concepts/data-model.md), and
[error codes](../../docs/api/error-codes-v1.md).
