#!/usr/bin/env python3
"""Generate the canonical disk-format-1 compatibility fixture."""

from __future__ import annotations

import argparse
import json
import subprocess
import tempfile
from pathlib import Path


TRANSACTION_ID = "018f0000-0000-7000-8000-00000000f001"
VALUE = '{"group":"fixture","score":42}'


def run(binary: Path, *arguments: str) -> dict[str, object]:
    completed = subprocess.run(
        [str(binary), *arguments],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return json.loads(completed.stdout)


def generate(binary: Path) -> dict[str, object]:
    with tempfile.TemporaryDirectory(prefix="hyphae-compat-v1-") as temporary:
        data = Path(temporary) / "data"
        committed = run(
            binary,
            "put",
            "--data-dir",
            str(data),
            "--key",
            "alpha",
            "--json",
            VALUE,
            "--transaction-id",
            TRANSACTION_ID,
        )
        compacted = run(binary, "compact", "--data-dir", str(data))
        if committed.get("status") != "committed":
            raise RuntimeError("fixture transaction was not newly committed")
        if compacted.get("status") != "compacted":
            raise RuntimeError("fixture data directory was not compacted")

        manifests = sorted((data / "manifest").glob("*.hymanifest"))
        if not manifests:
            raise RuntimeError("fixture has no storage manifest")
        manifest = manifests[-1]
        encoded_manifest = manifest.read_bytes()
        if len(encoded_manifest) != 140:
            raise RuntimeError("fixture manifest has an unexpected length")
        active_segment = int.from_bytes(encoded_manifest[24:32], "little")
        base_sequence = int.from_bytes(encoded_manifest[32:40], "little")
        selected = [
            data / "FORMAT",
            manifest,
            data / "log" / f"{active_segment:020}.hylog",
            data / "snapshots" / f"snapshot-{base_sequence:020}.hysnap",
        ]
        files = {
            path.relative_to(data).as_posix(): path.read_bytes().hex()
            for path in selected
        }
        return {
            "disk_format_version": 1,
            "expected": {
                "commit_digest": committed["commit_digest"],
                "commit_sequence": committed["commit_sequence"],
                "key_hex": "616c706861",
                "transaction_digest": committed["transaction_digest"],
                "transaction_id": TRANSACTION_ID,
                "value": {"group": "fixture", "score": 42},
            },
            "files_hex": files,
            "fixture_version": 1,
            "purpose": "Open and recover a compacted disk-format-1 directory without its materialized index.",
        }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--check", type=Path)
    arguments = parser.parse_args()
    generated = json.dumps(generate(arguments.binary), indent=2, sort_keys=True) + "\n"
    if arguments.check is None:
        print(generated, end="")
        return
    checked_in = arguments.check.read_text(encoding="utf-8")
    if checked_in != generated:
        raise SystemExit(f"compatibility fixture is stale: {arguments.check}")


if __name__ == "__main__":
    main()
