# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import io
import json
import unittest
from email.message import Message
from unittest.mock import patch

from hyphae_sdk import HyphaeApiError, HyphaeClient, HyphaeClientError


class FakeResponse:
    def __init__(self, status: int, body: bytes, headers: dict[str, str]) -> None:
        self.status = status
        self._body = io.BytesIO(body)
        self.headers = Message()
        for name, value in headers.items():
            self.headers[name] = value

    def read(self, size: int = -1) -> bytes:
        return self._body.read(size)

    def close(self) -> None:
        pass


class ClientTests(unittest.TestCase):
    def test_rejects_non_origins_and_unsafe_secrets(self) -> None:
        with self.assertRaises(HyphaeClientError):
            HyphaeClient("file:///tmp/hyphae")
        with self.assertRaises(HyphaeClientError):
            HyphaeClient("https://example.test/prefix")
        with self.assertRaises(HyphaeClientError):
            HyphaeClient("https://example.test", bearer_token="bad\nsecret")

    @patch("urllib.request.urlopen")
    def test_decodes_correlated_json(self, urlopen) -> None:  # type: ignore[no-untyped-def]
        urlopen.return_value = FakeResponse(
            200,
            json.dumps({"status": "live"}).encode(),
            {"Content-Type": "application/json", "X-Request-Id": "request-1"},
        )
        response = HyphaeClient("https://example.test").liveness()
        self.assertEqual(response.value, {"status": "live"})
        self.assertEqual(response.request_id, "request-1")

    @patch("urllib.request.urlopen")
    def test_exposes_stable_api_errors(self, urlopen) -> None:  # type: ignore[no-untyped-def]
        urlopen.return_value = FakeResponse(
            409,
            json.dumps(
                {
                    "code": "idempotency_conflict",
                    "message": "conflict",
                    "request_id": "request-2",
                }
            ).encode(),
            {"Content-Type": "application/json", "X-Request-Id": "request-2"},
        )
        with self.assertRaises(HyphaeApiError) as caught:
            HyphaeClient("https://example.test").put({"records": []})
        self.assertEqual(caught.exception.code, "idempotency_conflict")

    @patch("urllib.request.urlopen")
    def test_enforces_streaming_byte_bound(self, urlopen) -> None:  # type: ignore[no-untyped-def]
        urlopen.return_value = FakeResponse(
            200,
            b'{"status":"live"}',
            {"Content-Type": "application/json", "X-Request-Id": "request-3"},
        )
        with self.assertRaises(HyphaeClientError):
            HyphaeClient("https://example.test", response_bytes=4).liveness()

    @patch("urllib.request.urlopen")
    def test_preserves_large_integers_and_rejects_floats(self, urlopen) -> None:  # type: ignore[no-untyped-def]
        urlopen.return_value = FakeResponse(
            200,
            b'{"status":"live","sequence":9223372036854775807}',
            {"Content-Type": "application/json", "X-Request-Id": "request-4"},
        )
        response = HyphaeClient("https://example.test").liveness()
        self.assertEqual(response.value["sequence"], 9223372036854775807)  # type: ignore[typeddict-item]

        urlopen.return_value = FakeResponse(
            200,
            b'{"status":"live","invalid":1.5}',
            {"Content-Type": "application/json", "X-Request-Id": "request-5"},
        )
        with self.assertRaises(HyphaeClientError):
            HyphaeClient("https://example.test").liveness()


if __name__ == "__main__":
    unittest.main()
