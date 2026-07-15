# SPDX-License-Identifier: Apache-2.0
"""Live Python-client runner for the shared version 1 fixture."""

from __future__ import annotations

import json
import os
from pathlib import Path

from hyphae_sdk import HyphaeApiError, HyphaeClient


ROOT = Path(__file__).resolve().parents[3]
fixture = json.loads((ROOT / "conformance" / "v1" / "cases.json").read_text("utf-8"))
assert fixture["version"] == 1

client = HyphaeClient(
    os.environ.get("HYPHAE_BASE_URL", "http://127.0.0.1:8787"),
    bearer_token=os.environ.get("HYPHAE_BEARER_TOKEN"),
)
assert client.liveness().value["status"] == "live"
capabilities = client.capabilities().value
assert capabilities["api_version"] == "v1"
assert capabilities["features"] == sorted(set(capabilities["features"]))

assert client.put(fixture["put_request"]).value["status"] == "committed"
assert client.put(fixture["put_request"]).value["status"] == "existing"
try:
    client.put(fixture["conflict_put_request"])
except HyphaeApiError as error:
    assert error.code == "idempotency_conflict"
else:
    raise AssertionError("idempotency conflict was accepted")

present = client.get(fixture["present_get_request"]).value
assert present["found"] is True
assert present["record"]["key_hex"] == "61"
assert client.download_witness(present["proof"]).value.startswith(b"HYSNAP01")

absent = client.get(fixture["absent_get_request"]).value
assert absent["found"] is False and absent.get("record") is None

first_page = client.query(fixture["query_request"]).value
assert [record["key_hex"] for record in first_page["rows"]] == fixture["expected"]["first_page_keys"]
assert first_page["matched_records"] == fixture["expected"]["matched_records"]
assert first_page.get("aggregation") == fixture["expected"]["aggregation"]
second_request = {**fixture["query_request"], "cursor": first_page.get("next_cursor")}
second_page = client.query(second_request).value
assert [record["key_hex"] for record in second_page["rows"]] == fixture["expected"]["second_page_keys"]
assert second_page.get("next_cursor") is None

client.delete(fixture["delete_request"])
assert client.get({"key_hex": "62"}).value["found"] is False

print(json.dumps({"client": "python", "status": "ok"}, separators=(",", ":")))
