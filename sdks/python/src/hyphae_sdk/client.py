# SPDX-License-Identifier: Apache-2.0
"""Bounded synchronous client for the public Hyphae v1 HTTP API."""

from __future__ import annotations

import json
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from email.message import Message
from typing import Generic, TypeVar, cast

from .generated import (
    CapabilitiesV1,
    CommitReceiptV1,
    DeleteRequestV1,
    ErrorV1,
    GetRequestV1,
    GetResponseV1,
    HealthV1,
    ProofV1,
    PutRequestV1,
    QueryRequestV1,
    QueryResponseV1,
)

DEFAULT_RESPONSE_BYTES = 32 * 1024 * 1024
DEFAULT_WITNESS_BYTES = 512 * 1024 * 1024
DEFAULT_TIMEOUT_SECONDS = 60.0
_CHUNK_BYTES = 64 * 1024
T = TypeVar("T")


@dataclass(frozen=True)
class ApiResponse(Generic[T]):
    """A typed API value and its response correlation identifier."""

    value: T
    request_id: str


class HyphaeClientError(Exception):
    """A local configuration, transport, bound, or contract failure."""


class HyphaeApiError(HyphaeClientError):
    """A stable error declared by the Hyphae v1 API."""

    def __init__(self, status: int, envelope: ErrorV1) -> None:
        self.status = status
        self.code = envelope["code"]
        self.request_id = envelope["request_id"]
        self.server_message = envelope["message"]
        super().__init__(
            f"Hyphae API returned HTTP {status} {self.code} "
            f"(request {self.request_id})"
        )


class HyphaeClient:
    """Dependency-free, bounded client for one root Hyphae HTTP origin."""

    def __init__(
        self,
        base_url: str,
        *,
        bearer_token: str | None = None,
        timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS,
        response_bytes: int = DEFAULT_RESPONSE_BYTES,
        witness_bytes: int = DEFAULT_WITNESS_BYTES,
    ) -> None:
        parsed = urllib.parse.urlsplit(base_url)
        if (
            parsed.scheme not in {"http", "https"}
            or not parsed.netloc
            or parsed.username is not None
            or parsed.password is not None
            or parsed.query
            or parsed.fragment
            or parsed.path not in {"", "/"}
        ):
            raise HyphaeClientError(
                "Hyphae base URL must be a root HTTP(S) origin"
            )
        if any(
            isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0
            for value in (timeout_seconds, response_bytes, witness_bytes)
        ):
            raise HyphaeClientError(
                "client timeout and response limits must be positive numbers"
            )
        if not isinstance(response_bytes, int) or not isinstance(witness_bytes, int):
            raise HyphaeClientError("client byte limits must be integers")
        if bearer_token is not None and (
            not bearer_token or "\r" in bearer_token or "\n" in bearer_token
        ):
            raise HyphaeClientError(
                "invalid bearer token for an HTTP authorization header"
            )
        self._base_url = urllib.parse.urlunsplit(
            (parsed.scheme, parsed.netloc, "/", "", "")
        )
        self._bearer_token = bearer_token
        self._timeout_seconds = float(timeout_seconds)
        self._response_bytes = response_bytes
        self._witness_bytes = witness_bytes

    def capabilities(self) -> ApiResponse[CapabilitiesV1]:
        return cast(ApiResponse[CapabilitiesV1], self._json("v1/capabilities", False))

    def liveness(self) -> ApiResponse[HealthV1]:
        return cast(ApiResponse[HealthV1], self._json("v1/health/live", False))

    def readiness(self) -> ApiResponse[HealthV1]:
        return cast(ApiResponse[HealthV1], self._json("v1/health/ready", False))

    def put(self, request: PutRequestV1) -> ApiResponse[CommitReceiptV1]:
        return cast(ApiResponse[CommitReceiptV1], self._json("v1/kv/put", True, request))

    def delete(self, request: DeleteRequestV1) -> ApiResponse[CommitReceiptV1]:
        return cast(
            ApiResponse[CommitReceiptV1], self._json("v1/kv/delete", True, request)
        )

    def get(self, request: GetRequestV1) -> ApiResponse[GetResponseV1]:
        return cast(ApiResponse[GetResponseV1], self._json("v1/kv/get", True, request))

    def query(self, request: QueryRequestV1) -> ApiResponse[QueryResponseV1]:
        return cast(ApiResponse[QueryResponseV1], self._json("v1/query", True, request))

    def download_witness(self, proof: ProofV1) -> ApiResponse[bytes]:
        expected_path = (
            f"/v1/witnesses/{proof['checkpoint_sequence']}/"
            f"{proof['snapshot_digest']}"
        )
        if proof["witness"]["path"] != expected_path:
            raise HyphaeClientError(
                "proof contains a noncanonical witness reference"
            )
        expected_bytes = proof["witness"]["file_bytes"]
        if (
            isinstance(expected_bytes, bool)
            or not isinstance(expected_bytes, int)
            or expected_bytes < 0
            or expected_bytes > self._witness_bytes
        ):
            raise HyphaeClientError(
                f"Hyphae response exceeded local limit {self._witness_bytes} bytes"
            )
        response = self._open(expected_path[1:], authenticated=True)
        try:
            if response.status < 200 or response.status >= 300:
                raise self._decode_api_error(response)
            if response.status != 200:
                raise HyphaeClientError(
                    f"Hyphae returned unexpected success status {response.status}"
                )
            request_id = _request_id(response.headers)
            if _single_header(response.headers, "digest") != (
                f"blake3={proof['snapshot_digest']}"
            ):
                raise HyphaeClientError(
                    "downloaded witness digest header differs from the proof"
                )
            value = _read_bounded(
                response, self._witness_bytes, self._timeout_seconds
            )
            if len(value) != expected_bytes:
                raise HyphaeClientError(
                    "downloaded witness length differs from the proof"
                )
            return ApiResponse(value, request_id)
        finally:
            response.close()

    def _json(
        self, path: str, authenticated: bool, body: object | None = None
    ) -> ApiResponse[object]:
        response = self._open(path, authenticated=authenticated, body=body)
        try:
            if response.status < 200 or response.status >= 300:
                raise self._decode_api_error(response)
            if response.status != 200:
                raise HyphaeClientError(
                    f"Hyphae returned unexpected success status {response.status}"
                )
            _require_json(response.headers)
            request_id = _request_id(response.headers)
            encoded = _read_bounded(
                response, self._response_bytes, self._timeout_seconds
            )
            try:
                value = _loads_integer_json(encoded)
            except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
                raise HyphaeClientError(
                    "Hyphae response violated the version 1 JSON contract"
                ) from error
            return ApiResponse(value, request_id)
        finally:
            response.close()

    def _open(
        self, path: str, *, authenticated: bool, body: object | None = None
    ):  # type: ignore[no-untyped-def]
        headers: dict[str, str] = {}
        data: bytes | None = None
        method = "GET"
        if authenticated and self._bearer_token is not None:
            headers["Authorization"] = f"Bearer {self._bearer_token}"
        if body is not None:
            method = "POST"
            headers["Content-Type"] = "application/json"
            data = json.dumps(
                body, ensure_ascii=False, separators=(",", ":"), allow_nan=False
            ).encode("utf-8")
        request = urllib.request.Request(
            urllib.parse.urljoin(self._base_url, path),
            data=data,
            headers=headers,
            method=method,
        )
        try:
            return urllib.request.urlopen(request, timeout=self._timeout_seconds)
        except urllib.error.HTTPError as response:
            return response
        except (OSError, urllib.error.URLError, TimeoutError) as error:
            raise HyphaeClientError("Hyphae HTTP transport failed") from error

    def _decode_api_error(self, response) -> HyphaeApiError:  # type: ignore[no-untyped-def]
        _require_json(response.headers)
        request_id = _request_id(response.headers)
        encoded = _read_bounded(
            response, self._response_bytes, self._timeout_seconds
        )
        try:
            value = _loads_integer_json(encoded)
        except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
            raise HyphaeClientError(
                "Hyphae error response violated the version 1 JSON contract"
            ) from error
        if (
            not isinstance(value, dict)
            or not isinstance(value.get("code"), str)
            or not isinstance(value.get("message"), str)
            or not isinstance(value.get("request_id"), str)
        ):
            raise HyphaeClientError(
                "Hyphae error response violated the version 1 JSON contract"
            )
        envelope = cast(ErrorV1, value)
        if envelope["request_id"] != request_id:
            raise HyphaeClientError(
                "Hyphae error envelope request ID differs from its response header"
            )
        return HyphaeApiError(response.status, envelope)


def _single_header(headers: Message, name: str) -> str | None:
    values = headers.get_all(name, failobj=[])
    if len(values) != 1 or not values[0] or "," in values[0]:
        return None
    return values[0]


def _loads_integer_json(encoded: bytes) -> object:
    def reject_non_integer(token: str) -> object:
        raise ValueError(f"Hyphae JSON number is not an integer: {token}")

    return json.loads(
        encoded.decode("utf-8", errors="strict"),
        parse_float=reject_non_integer,
        parse_constant=reject_non_integer,
    )


def _request_id(headers: Message) -> str:
    value = _single_header(headers, "x-request-id")
    if value is None:
        raise HyphaeClientError(
            "Hyphae response has no single valid X-Request-Id header"
        )
    return value


def _require_json(headers: Message) -> None:
    content_type = _single_header(headers, "content-type")
    media_type = content_type.split(";", 1)[0].strip().lower() if content_type else ""
    if media_type != "application/json" and not (
        media_type.startswith("application/") and media_type.endswith("+json")
    ):
        raise HyphaeClientError(
            "Hyphae response did not use a JSON content type"
        )


def _read_bounded(response, maximum: int, timeout_seconds: float) -> bytes:  # type: ignore[no-untyped-def]
    declared = _single_header(response.headers, "content-length")
    if declared is not None:
        if not declared.isascii() or not declared.isdigit() or int(declared) > maximum:
            raise HyphaeClientError(
                f"Hyphae response exceeded local limit {maximum} bytes"
            )
    deadline = time.monotonic() + timeout_seconds
    chunks: list[bytes] = []
    length = 0
    while True:
        if time.monotonic() > deadline:
            raise HyphaeClientError("Hyphae HTTP response deadline elapsed")
        chunk = response.read(_CHUNK_BYTES)
        if not chunk:
            break
        length += len(chunk)
        if length > maximum:
            raise HyphaeClientError(
                f"Hyphae response exceeded local limit {maximum} bytes"
            )
        chunks.append(chunk)
    return b"".join(chunks)


__all__ = [
    "ApiResponse",
    "HyphaeApiError",
    "HyphaeClient",
    "HyphaeClientError",
]
