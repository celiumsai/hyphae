# Data model

Hyphae stores records. A record is one globally unique, nonempty binary key
and one deterministic structured value. Collections, table schemas, SQL
types, and implicit coercions are not part of `0.1.0`.

## Keys

The Rust APIs accept arbitrary nonempty bytes up to 1 MiB. Public `/v1`
contracts encode keys as even-length hexadecimal in `key_hex` or `keys_hex`.
Hexadecimal is case-insensitive on input and lowercase on output.

The local `put`, `get`, and `delete` commands accept UTF-8 text through
`--key`; the bytes of that UTF-8 string are the durable key. Use the embedded
API or `/v1` when a key is not valid UTF-8.

```text
CLI key "alpha"  -> bytes 61 6c 70 68 61 -> API key_hex "616c706861"
```

## Values

The canonical value variants and total cross-type order are:

```text
null < boolean < signed 64-bit integer < UTF-8 string < bytes < array < object
```

Arrays preserve element order. Objects are ordered by exact UTF-8 field name.
Numbers must fit `i64`; floating point, decimal, NaN, and infinity are
rejected instead of being rounded.

### Bytes on the wire

Natural JSON has no byte type. API v1 reserves an object containing only
`$hyphae_bytes_hex` with a string value:

```json
{"$hyphae_bytes_hex":"0001feff"}
```

That exact one-property shape represents bytes, not an ordinary object.
Other objects are decoded recursively. The local CLI accepts ordinary JSON
but does not interpret the reserved byte envelope on input; use `/v1` or Rust
to store byte values.

## Field paths

The query domain represents a path as an ordered list of exact object keys.
For example, `['profile', 'name']` resolves `profile.name` without parsing a
JSONPath language. An empty path selects the root. Traversal through a missing
field or non-object is “missing”, which is distinct from explicit null.

API requests use arrays of segments:

```json
{"op":"exists","path":["profile","name"]}
```

The simplified local CLI accepts dot-separated nonempty segments, so it
cannot address a literal field name containing a dot. The full Rust and HTTP
contracts can.

## Transactions and batches

Every durable mutation is one atomic batch with a UUID transaction ID. The
local CLI creates a UUIDv7 unless `--transaction-id` is provided; API v1 does
the same when `transaction_id` is null or absent.

- A new canonical batch returns `status: committed`.
- An exact retry with the same UUID returns `status: existing` and the
  original commit identity.
- The same UUID with different operations is an idempotency conflict.
- Duplicate keys within one batch are rejected before append.
- Deleting a missing key is a successful durable operation.

A commit receipt contains the transaction UUID, authoritative commit-frame
sequence, commit digest, and canonical transaction digest.

## Documents and storage encoding

The public value is encoded into Hyphae's bounded canonical document format
before it enters the append-only mutation log. The format limits depth,
decoded nodes, lengths, and trailing bytes. It is independent of JSON object
serialization and is verified again when read.

See [document format v1](../storage/document-format-v1.md),
[mutation format v1](../storage/mutation-format-v1.md), and
[structured query semantics](../query/reference-semantics-v1.md).
