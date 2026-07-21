#!/usr/bin/env python3
"""Repeated kill/restart and backup/restore correctness gate."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
import urllib.request
from pathlib import Path

from run_conformance import free_loopback_port, wait_until_live


ROOT = Path(__file__).resolve().parents[1]


def post_json(base_url: str, path: str, body: object) -> dict[str, object]:
    request = urllib.request.Request(
        f"{base_url}{path}",
        data=json.dumps(body, separators=(",", ":")).encode("utf-8"),
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.load(response)


def start(binary: Path, data: Path) -> tuple[subprocess.Popen[bytes], str]:
    port = free_loopback_port()
    base_url = f"http://127.0.0.1:{port}"
    process = subprocess.Popen(
        (str(binary), "serve", "--data-dir", str(data), "--bind", f"127.0.0.1:{port}"),
        cwd=ROOT,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    wait_until_live(base_url, process)
    return process, base_url


def assert_count(base_url: str, count: int) -> None:
    result = post_json(
        base_url,
        "/v1/query",
        {"filter": {"op": "match_all"}, "sort": [], "limit": count + 1, "timeout_ms": 30_000},
    )
    if result.get("matched_records") != count or len(result.get("rows", [])) != count:
        raise RuntimeError(f"expected {count} durable records, received {result!r}")


def assert_retrieval(base_url: str, expected: int) -> None:
    if expected == 0:
        return
    exact = post_json(
        base_url,
        "/v1/retrieve/exact",
        {
            "vector_space": "soak",
            "query": [32_767, 0, 0, 0, 0, 0, 0, 0],
            "limit": min(8, expected),
            "minimum_score_nanos": -1_000_000_000,
            "minimum_margin_nanos": 0,
            "timeout_ms": 30_000,
        },
    )
    lexical = post_json(
        base_url,
        "/v1/retrieve/lexical",
        {
            "lexical_index": "soak-text",
            "query": "durable memory",
            "limit": min(8, expected),
            "timeout_ms": 30_000,
        },
    )
    hybrid = post_json(
        base_url,
        "/v1/retrieve/hybrid",
        {
            "lexical": {
                "lexical_index": "soak-text",
                "query": "durable memory",
                "limit": min(8, expected),
                "timeout_ms": 30_000,
            },
            "vector": {
                "vector_space": "soak",
                "query": [32_767, 0, 0, 0, 0, 0, 0, 0],
                "limit": min(8, expected),
                "minimum_score_nanos": -1_000_000_000,
                "minimum_margin_nanos": 0,
                "timeout_ms": 30_000,
            },
            "lexical_weight": 1,
            "vector_weight": 1,
            "limit": min(8, expected),
        },
    )
    for name, result in (("exact", exact), ("lexical", lexical), ("hybrid", hybrid)):
        outcome = result.get("outcome")
        proof = result.get("proof")
        if not isinstance(outcome, dict) or not outcome.get("matches"):
            raise RuntimeError(f"{name} retrieval lost durable results: {result!r}")
        if not isinstance(proof, dict) or proof.get("encoding") != "base64":
            raise RuntimeError(f"{name} retrieval lost its proof: {result!r}")


def run_cli(binary: Path, *arguments: str) -> dict[str, object]:
    completed = subprocess.run(
        (str(binary), *arguments),
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )
    return json.loads(completed.stdout)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cycles", type=int, default=4)
    parser.add_argument("--writes-per-cycle", type=int, default=32)
    arguments = parser.parse_args()
    if arguments.cycles <= 0 or arguments.writes_per_cycle <= 0:
        raise ValueError("cycles and writes-per-cycle must be positive")

    target = Path(os.environ.get("HYPHAE_TARGET_DIR", ROOT / "target"))
    suffix = ".exe" if os.name == "nt" else ""
    binary = Path(os.environ.get("HYPHAE_BIN", target / "debug" / f"hyphae{suffix}"))
    if not binary.is_file():
        raise RuntimeError(f"Hyphae executable not found: {binary}")

    total = 0
    with tempfile.TemporaryDirectory(prefix="hyphae-soak-") as temporary:
        root = Path(temporary)
        data = root / "data"
        for cycle in range(arguments.cycles):
            process, base_url = start(binary, data)
            try:
                assert_count(base_url, total)
                if cycle == 0:
                    vector_space = post_json(
                        base_url,
                        "/v1/vector-spaces/define",
                        {
                            "vector_space": {
                                "name": "soak",
                                "dimension": 8,
                                "metric": "cosine_q15_nanos",
                            }
                        },
                    )
                    lexical_index = post_json(
                        base_url,
                        "/v1/lexical-indexes/define",
                        {
                            "lexical_index": {
                                "name": "soak-text",
                                "fields": [
                                    {"path": ["body"], "weight_micros": 1_000_000},
                                    {"path": ["title"], "weight_micros": 2_000_000},
                                ],
                            }
                        },
                    )
                    if vector_space.get("status") != "committed":
                        raise RuntimeError("soak vector space did not commit")
                    if lexical_index.get("status") != "committed":
                        raise RuntimeError("soak lexical index did not commit")
                else:
                    assert_retrieval(base_url, total)
                for offset in range(arguments.writes_per_cycle):
                    index = cycle * arguments.writes_per_cycle + offset
                    response = post_json(
                        base_url,
                        "/v1/kv/put",
                        {
                            "records": [
                                {
                                    "key_hex": index.to_bytes(8, "big").hex(),
                                    "value": {
                                        "body": f"durable memory cycle {cycle}",
                                        "cycle": cycle,
                                        "sequence": index,
                                        "title": f"Hyphae soak item {index}",
                                    },
                                }
                            ]
                        },
                    )
                    if response.get("status") != "committed":
                        raise RuntimeError(f"write did not commit: {response!r}")
                    values = [0] * 8
                    values[index % len(values)] = 32_767
                    vector_response = post_json(
                        base_url,
                        "/v1/vectors/put",
                        {
                            "vector_space": "soak",
                            "vectors": [
                                {
                                    "key_hex": index.to_bytes(8, "big").hex(),
                                    "values": values,
                                }
                            ],
                        },
                    )
                    if vector_response.get("status") != "committed":
                        raise RuntimeError(f"vector did not commit: {vector_response!r}")
                    total += 1
            finally:
                process.kill()
                process.wait(timeout=5)

        index_path = data / "indexes" / "primary.redb"
        index_path.unlink()
        process, base_url = start(binary, data)
        try:
            assert_count(base_url, total)
            assert_retrieval(base_url, total)
        finally:
            process.terminate()
            process.wait(timeout=5)

        backup = root / "backup"
        restored = root / "restored"
        created = run_cli(binary, "backup", "--data-dir", str(data), "--out", str(backup))
        if created.get("status") != "created":
            raise RuntimeError("soak backup was not created")
        verified = run_cli(binary, "backup-verify", "--backup", str(backup))
        if verified.get("status") != "verified":
            raise RuntimeError("soak backup was not verified")
        activated = run_cli(
            binary,
            "restore",
            "--backup",
            str(backup),
            "--data-dir",
            str(restored),
        )
        if activated.get("status") != "restored":
            raise RuntimeError("soak restore was not activated")
        if run_cli(binary, "doctor", "--data-dir", str(restored)).get("status") != "healthy":
            raise RuntimeError("restored soak directory is not healthy")
        process, base_url = start(binary, restored)
        try:
            assert_count(base_url, total)
            assert_retrieval(base_url, total)
        finally:
            process.terminate()
            process.wait(timeout=5)

    print(
        json.dumps(
            {"version": 1, "status": "ok", "cycles": arguments.cycles, "records": total},
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
