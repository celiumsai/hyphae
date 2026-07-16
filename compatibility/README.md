# On-disk compatibility fixtures

Each versioned fixture is a byte-for-byte historical Hyphae data directory.
The engine test reconstructs it without generated indexes, opens it, verifies
the expected records, and proves that durable idempotency receipts survive.

Fixtures are immutable once their disk format ships. A new disk format adds a
new directory and test case; it never rewrites an older fixture.

Regenerate the pre-release format-1 fixture only while format 1 is still under
development:

```sh
python3 tools/generate_compatibility_fixture.py \
  --binary target/debug/hyphae \
  --check compatibility/v1/data-directory.json
```

The generator deliberately omits the materialized Redb index so the test also
proves that recovery reconstructs disposable indexes from authoritative data.
