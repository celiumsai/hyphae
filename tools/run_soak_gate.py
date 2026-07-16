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
                for offset in range(arguments.writes_per_cycle):
                    index = cycle * arguments.writes_per_cycle + offset
                    response = post_json(
                        base_url,
                        "/v1/kv/put",
                        {
                            "records": [
                                {
                                    "key_hex": index.to_bytes(8, "big").hex(),
                                    "value": {"cycle": cycle, "sequence": index},
                                }
                            ]
                        },
                    )
                    if response.get("status") != "committed":
                        raise RuntimeError(f"write did not commit: {response!r}")
                    total += 1
            finally:
                process.kill()
                process.wait(timeout=5)

        process, base_url = start(binary, data)
        try:
            assert_count(base_url, total)
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
