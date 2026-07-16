#!/usr/bin/env python3
"""Bounded local HTTP load gate with correctness and resource assertions."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import math
import os
import subprocess
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

from run_conformance import free_loopback_port, wait_until_live


ROOT = Path(__file__).resolve().parents[1]


def post_json(base_url: str, path: str, body: object) -> dict[str, object]:
    encoded = json.dumps(body, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        f"{base_url}{path}",
        data=encoded,
        headers={"content-type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=15) as response:
        if response.status != 200:
            raise RuntimeError(f"unexpected HTTP status {response.status}")
        return json.load(response)


def read_rss_bytes(process_id: int) -> int | None:
    status = Path(f"/proc/{process_id}/status")
    if not status.is_file():
        return None
    for line in status.read_text("ascii").splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1]) * 1024
    return None


def monitor_rss(process_id: int, stop: threading.Event, maximum: list[int]) -> None:
    while not stop.wait(0.02):
        current = read_rss_bytes(process_id)
        if current is not None:
            maximum[0] = max(maximum[0], current)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--operations", type=int, default=256)
    parser.add_argument("--concurrency", type=int, default=8)
    parser.add_argument("--max-seconds", type=float, default=90.0)
    parser.add_argument("--max-p95-ms", type=float, default=2_000.0)
    parser.add_argument("--max-rss-mib", type=int, default=512)
    arguments = parser.parse_args()
    if arguments.operations <= 0 or arguments.concurrency <= 0:
        raise ValueError("operations and concurrency must be positive")

    target = Path(os.environ.get("HYPHAE_TARGET_DIR", ROOT / "target"))
    suffix = ".exe" if os.name == "nt" else ""
    binary = Path(os.environ.get("HYPHAE_BIN", target / "debug" / f"hyphae{suffix}"))
    if not binary.is_file():
        raise RuntimeError(f"Hyphae executable not found: {binary}")

    port = free_loopback_port()
    base_url = f"http://127.0.0.1:{port}"
    with tempfile.TemporaryDirectory(prefix="hyphae-load-") as data:
        with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
            process = subprocess.Popen(
                (str(binary), "serve", "--data-dir", data, "--bind", f"127.0.0.1:{port}"),
                cwd=ROOT,
                stdout=stdout,
                stderr=stderr,
            )
            stop_monitor = threading.Event()
            peak_rss = [0]
            monitor = threading.Thread(
                target=monitor_rss,
                args=(process.pid, stop_monitor, peak_rss),
                daemon=True,
            )
            monitor.start()
            try:
                wait_until_live(base_url, process)
                started = time.monotonic()

                def put(index: int) -> float:
                    request_started = time.monotonic()
                    result = post_json(
                        base_url,
                        "/v1/kv/put",
                        {
                            "records": [
                                {
                                    "key_hex": index.to_bytes(8, "big").hex(),
                                    "value": {"group": index % 16, "sequence": index},
                                }
                            ]
                        },
                    )
                    if result.get("status") != "committed":
                        raise RuntimeError(f"put {index} was not committed: {result!r}")
                    return (time.monotonic() - request_started) * 1000

                with concurrent.futures.ThreadPoolExecutor(
                    max_workers=arguments.concurrency
                ) as executor:
                    latencies = list(executor.map(put, range(arguments.operations)))
                elapsed = time.monotonic() - started
                query = post_json(
                    base_url,
                    "/v1/query",
                    {
                        "filter": {"op": "match_all"},
                        "sort": [
                            {
                                "path": ["sequence"],
                                "direction": "ascending",
                                "nulls": "last",
                            }
                        ],
                        "limit": arguments.operations,
                        "timeout_ms": 30_000,
                    },
                )
                rows = query.get("rows")
                if query.get("matched_records") != arguments.operations or not isinstance(rows, list):
                    raise RuntimeError("load gate final query lost committed records")
                expected_keys = [index.to_bytes(8, "big").hex() for index in range(arguments.operations)]
                if [row.get("key_hex") for row in rows] != expected_keys:
                    raise RuntimeError("load gate final global ordering differs from committed keys")

                ordered = sorted(latencies)
                p95 = ordered[math.ceil(len(ordered) * 0.95) - 1]
                if elapsed > arguments.max_seconds:
                    raise RuntimeError(f"load gate exceeded {arguments.max_seconds} seconds")
                if p95 > arguments.max_p95_ms:
                    raise RuntimeError(f"load gate p95 {p95:.3f} ms exceeded limit")
                maximum_rss = arguments.max_rss_mib * 1024 * 1024
                if peak_rss[0] > maximum_rss:
                    raise RuntimeError(f"load gate RSS {peak_rss[0]} exceeded {maximum_rss}")
                print(
                    json.dumps(
                        {
                            "version": 1,
                            "status": "ok",
                            "operations": arguments.operations,
                            "concurrency": arguments.concurrency,
                            "elapsed_seconds": round(elapsed, 6),
                            "p95_ms": round(p95, 3),
                            "peak_rss_bytes": peak_rss[0] or None,
                        },
                        separators=(",", ":"),
                    )
                )
            except Exception:
                stderr.seek(0)
                diagnostic = stderr.read().decode("utf-8", errors="replace")
                if diagnostic:
                    print(diagnostic)
                raise
            finally:
                stop_monitor.set()
                monitor.join(timeout=1)
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
