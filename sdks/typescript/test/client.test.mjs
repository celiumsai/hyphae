// SPDX-License-Identifier: Apache-2.0

import assert from "node:assert/strict";
import test from "node:test";

import {
  HyphaeApiError,
  HyphaeClient,
  HyphaeClientError,
  parseHyphaeJson,
  stringifyHyphaeJson,
} from "../dist/index.js";

test("client rejects non-origin URLs and unsafe secrets", () => {
  assert.throws(() => new HyphaeClient("file:///tmp/hyphae"), HyphaeClientError);
  assert.throws(() => new HyphaeClient("https://example.test/prefix"), HyphaeClientError);
  assert.throws(
    () => new HyphaeClient("https://example.test", { bearerToken: "bad\nsecret" }),
    HyphaeClientError,
  );
});

test("client decodes a correlated bounded JSON response", async () => {
  const client = new HyphaeClient("https://example.test", {
    fetch: async () => new Response('{"status":"live"}', {
      status: 200,
      headers: { "content-type": "application/json", "x-request-id": "request-1" },
    }),
  });
  assert.deepEqual(await client.liveness(), { value: { status: "live" }, requestId: "request-1" });
});

test("client exposes stable API errors", async () => {
  const client = new HyphaeClient("https://example.test", {
    fetch: async () => new Response(
      '{"code":"idempotency_conflict","message":"conflict","request_id":"request-2"}',
      {
        status: 409,
        headers: { "content-type": "application/json", "x-request-id": "request-2" },
      },
    ),
  });
  await assert.rejects(
    client.put({ records: [] }),
    (error) => error instanceof HyphaeApiError && error.status === 409 &&
      error.code === "idempotency_conflict" && error.requestId === "request-2",
  );
});

test("client enforces the streaming byte bound", async () => {
  const client = new HyphaeClient("https://example.test", {
    responseBytes: 4,
    fetch: async () => new Response('{"status":"live"}', {
      status: 200,
      headers: { "content-type": "application/json", "x-request-id": "request-3" },
    }),
  });
  await assert.rejects(client.liveness(), HyphaeClientError);
});

test("Hyphae JSON preserves every signed 64-bit integer", () => {
  const decoded = parseHyphaeJson('{"minimum":-9223372036854775808,"maximum":9223372036854775807}');
  assert.deepEqual(decoded, {
    minimum: -9223372036854775808n,
    maximum: 9223372036854775807n,
  });
  assert.equal(
    stringifyHyphaeJson(decoded),
    '{"minimum":-9223372036854775808,"maximum":9223372036854775807}',
  );
  assert.throws(() => stringifyHyphaeJson({ rounded: 9007199254740992 }), TypeError);
});

test("client emits bigint request values as exact integer tokens", async () => {
  let body;
  const client = new HyphaeClient("https://example.test", {
    fetch: async (_url, options) => {
      body = options?.body;
      return new Response(
        '{"status":"committed","transaction_id":"t","commit_sequence":1,"commit_digest":"d","transaction_digest":"x"}',
        {
          status: 200,
          headers: { "content-type": "application/json", "x-request-id": "request-4" },
        },
      );
    },
  });
  await client.put({ records: [{ key_hex: "61", value: 9223372036854775807n }] });
  assert.equal(body, '{"records":[{"key_hex":"61","value":9223372036854775807}]}');
});
