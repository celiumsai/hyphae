#!/usr/bin/env python3
"""Extract and exercise one native Hyphae release archive without a network."""

from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import socket
import stat
import subprocess
import tarfile
import tempfile
import time
import tomllib
import urllib.request
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any


ROOT = Path(__file__).resolve().parents[1]


def validate_member(name: str) -> PurePosixPath:
    path = PurePosixPath(name)
    if path.is_absolute() or not path.parts or ".." in path.parts:
        raise RuntimeError(f"unsafe archive member: {name}")
    return path


def extract_archive(archive: Path, destination: Path) -> None:
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                validate_member(member.filename)
                mode = member.external_attr >> 16
                if stat.S_ISLNK(mode):
                    raise RuntimeError(f"archive symlink is forbidden: {member.filename}")
            bundle.extractall(destination)
        return
    if archive.name.endswith(".tar.gz"):
        with tarfile.open(archive, "r:gz") as bundle:
            for member in bundle.getmembers():
                relative = validate_member(member.name)
                target = destination.joinpath(*relative.parts)
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                    continue
                if not member.isfile():
                    raise RuntimeError(f"non-file archive member is forbidden: {member.name}")
                target.parent.mkdir(parents=True, exist_ok=True)
                source = bundle.extractfile(member)
                if source is None:
                    raise RuntimeError(f"archive member cannot be read: {member.name}")
                with source, target.open("wb") as output:
                    shutil.copyfileobj(source, output)
                target.chmod(member.mode & 0o777)
        return
    raise RuntimeError(f"unsupported release archive: {archive}")


def run_json(binary: Path, arguments: list[str], environment: dict[str, str]) -> Any:
    result = subprocess.run(
        (str(binary), *arguments),
        check=True,
        capture_output=True,
        text=True,
        env=environment,
        timeout=60,
    )
    return json.loads(result.stdout)


def workspace_version() -> str:
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    return manifest["workspace"]["package"]["version"]


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, sort_keys=True), encoding="utf-8", newline="\n")


def reserve_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def wait_until_live(base_url: str, process: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"installed server exited early with status {process.returncode}")
        try:
            with urllib.request.urlopen(f"{base_url}/v1/health/live", timeout=1) as response:
                if response.status == 200:
                    return
        except OSError:
            time.sleep(0.1)
    raise RuntimeError("installed server did not become live")


def exercise_retrieval(
    binary: Path,
    live: Path,
    root: Path,
    environment: dict[str, str],
    *,
    phase: str,
    initialize: bool,
    expected_outcomes: dict[str, Any] | None = None,
) -> dict[str, Any]:
    phase_root = root / phase
    phase_root.mkdir()
    port = reserve_loopback_port()
    base_url = f"http://127.0.0.1:{port}"
    process = subprocess.Popen(
        (str(binary), "serve", "--data-dir", str(live), "--bind", f"127.0.0.1:{port}"),
        env=environment,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        wait_until_live(base_url, process)
        requests = {
            "vector-space": {
                "vector_space": {
                    "name": "semantic",
                    "dimension": 2,
                    "metric": "cosine_q15_nanos",
                }
            },
            "vectors": {
                "vector_space": "semantic",
                "vectors": [
                    {"key_hex": "616c706861", "values": [32767, 0]},
                    {"key_hex": "62657461", "values": [0, 32767]},
                ],
            },
            "lexical-index": {
                "lexical_index": {
                    "name": "content",
                    "fields": [
                        {"path": ["title"], "weight_micros": 2_000_000},
                        {"path": ["body"], "weight_micros": 1_000_000},
                    ],
                }
            },
            "exact": {
                "vector_space": "semantic",
                "query": [32767, 0],
                "limit": 2,
                "minimum_score_nanos": -1_000_000_000,
                "minimum_margin_nanos": 0,
                "timeout_ms": 5000,
            },
            "lexical": {
                "lexical_index": "content",
                "query": "durable memory",
                "limit": 2,
                "timeout_ms": 5000,
            },
        }
        requests["hybrid"] = {
            "lexical": requests["lexical"],
            "vector": requests["exact"],
            "lexical_weight": 1,
            "vector_weight": 1,
            "limit": 2,
        }
        request_paths: dict[str, Path] = {}
        for name, value in requests.items():
            path = phase_root / f"{name}.json"
            write_json(path, value)
            request_paths[name] = path

        remote = ["remote", "--base-url", base_url]
        if initialize:
            for command, request_name in (
                ("define-vector-space", "vector-space"),
                ("put-vectors", "vectors"),
                ("define-lexical-index", "lexical-index"),
            ):
                run_json(
                    binary,
                    [*remote, command, "--request", str(request_paths[request_name])],
                    environment,
                )

        outcomes: dict[str, Any] = {}
        verification_inputs: list[tuple[str, Path, Path, str]] = []
        for kind in ("exact", "lexical", "hybrid"):
            response = run_json(
                binary,
                [*remote, f"retrieve-{kind}", "--request", str(request_paths[kind])],
                environment,
            )
            outcome = response.get("outcome")
            if not isinstance(outcome, dict):
                raise RuntimeError(f"installed {kind} retrieval omitted its outcome")
            outcomes[kind] = outcome
            proof = response.get("proof")
            if not isinstance(proof, dict) or "data" not in proof or "anchor_digest" not in proof:
                raise RuntimeError(f"installed {kind} retrieval omitted its proof")
            proof_json = phase_root / f"{kind}-proof.json"
            write_json(proof_json, proof)
            proof_file = phase_root / f"{kind}.hyrproof"
            proof_file.write_bytes(base64.b64decode(str(proof["data"]), validate=True))
            witness = phase_root / f"{kind}.hysnap"
            run_json(
                binary,
                [
                    *remote,
                    "witness",
                    "--proof",
                    str(proof_json),
                    "--out",
                    str(witness),
                ],
                environment,
            )
            verification_inputs.append(
                (kind, proof_file, witness, str(proof["anchor_digest"]))
            )
        if expected_outcomes is not None and outcomes != expected_outcomes:
            raise RuntimeError(
                f"installed retrieval changed during {phase}: "
                f"expected {expected_outcomes!r}, got {outcomes!r}"
            )
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)

    # Verification happens only after the serving process is gone. This proves
    # the installed verifier needs neither the live data directory handle nor
    # a reachable server.
    for kind, proof_file, witness, anchor_digest in verification_inputs:
        run_json(
            binary,
            [
                "verify-retrieval",
                "--kind",
                kind,
                "--proof",
                str(proof_file),
                "--snapshot",
                str(witness),
                "--anchor",
                anchor_digest,
            ],
            environment,
        )
    return outcomes


def verify_install(directory: Path) -> dict[str, Any]:
    archives = sorted(
        path
        for path in directory.iterdir()
        if path.is_file() and (path.name.endswith(".tar.gz") or path.name.endswith(".zip"))
    )
    if len(archives) != 1:
        raise RuntimeError(f"expected exactly one native archive, found {len(archives)}")
    archive = archives[0]
    with tempfile.TemporaryDirectory(prefix="hyphae-installed-") as temporary:
        root = Path(temporary)
        installed = root / "installed"
        installed.mkdir()
        extract_archive(archive, installed)
        binaries = [
            path
            for path in installed.rglob("*")
            if path.is_file() and path.name in {"hyphae", "hyphae.exe"}
        ]
        if len(binaries) != 1:
            raise RuntimeError(f"expected exactly one installed binary, found {len(binaries)}")
        binary = binaries[0]
        environment = os.environ.copy()
        live = root / "hyphae-data"
        environment["HYPHAE_DATA_DIR"] = str(live)

        version = run_json(binary, ["version", "--json"], environment)
        expected_version = workspace_version()
        expected = {
            "api_version": "v1",
            "disk_format_version": 2,
            "engine_version": expected_version,
            "product": "hyphae",
        }
        if version != expected:
            raise RuntimeError(f"installed version mismatch: {version!r}")

        alpha = {
            "body": "offline agent memory",
            "group": "x",
            "score": 10,
            "title": "Durable memory",
        }
        beta = {
            "body": "exact vector retrieval",
            "group": "x",
            "score": 20,
            "title": "Fast search",
        }
        run_json(binary, ["put", "--key", "alpha", "--json", json.dumps(alpha)], environment)
        run_json(binary, ["put", "--key", "beta", "--json", json.dumps(beta)], environment)
        read = run_json(binary, ["get", "--key", "alpha"], environment)
        if read.get("record", {}).get("value") != alpha:
            raise RuntimeError("installed binary returned the wrong durable value")
        query = run_json(
            binary,
            ["query", "--field", "group", "--equals", '"x"', "--sort", "score"],
            environment,
        )
        if [row["key_hex"] for row in query.get("rows", [])] != ["616c706861", "62657461"]:
            raise RuntimeError("installed binary returned the wrong global query order")

        baseline_retrieval = exercise_retrieval(
            binary,
            live,
            root,
            environment,
            phase="baseline-retrieval",
            initialize=True,
        )
        run_json(binary, ["snapshot"], environment)
        run_json(binary, ["compact"], environment)
        exercise_retrieval(
            binary,
            live,
            root,
            environment,
            phase="compacted-retrieval",
            initialize=False,
            expected_outcomes=baseline_retrieval,
        )

        proof = root / "result.hyproof"
        proven = run_json(
            binary,
            ["query", "--sort", "score", "--descending", "--limit", "2", "--proof-out", str(proof)],
            environment,
        )
        proof_metadata = proven["proof"]
        run_json(
            binary,
            [
                "verify",
                "--proof",
                str(proof),
                "--snapshot",
                proof_metadata["snapshot_path"],
                "--anchor",
                proof_metadata["anchor_digest"],
            ],
            environment,
        )

        backup = root / "hyphae-backup"
        restored = root / "hyphae-restored"
        run_json(binary, ["backup", "--data-dir", str(live), "--out", str(backup)], environment)
        run_json(binary, ["backup-verify", "--backup", str(backup)], environment)
        run_json(
            binary,
            ["restore", "--backup", str(backup), "--data-dir", str(restored)],
            environment,
        )
        run_json(binary, ["doctor", "--data-dir", str(restored)], environment)
        restored_value = run_json(
            binary, ["get", "--data-dir", str(restored), "--key", "alpha"], environment
        )
        if restored_value.get("record", {}).get("value") != alpha:
            raise RuntimeError("installed restore did not preserve the durable value")
        exercise_retrieval(
            binary,
            restored,
            root,
            environment,
            phase="restored-retrieval",
            initialize=False,
            expected_outcomes=baseline_retrieval,
        )
        return {
            "archive": archive.name,
            "engine_version": expected_version,
            "status": "ok",
        }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    arguments = parser.parse_args()
    print(json.dumps(verify_install(arguments.directory), sort_keys=True))


if __name__ == "__main__":
    main()
