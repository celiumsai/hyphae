#!/usr/bin/env python3
"""Extract and exercise one native Hyphae release archive without a network."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import stat
import subprocess
import tarfile
import tempfile
import tomllib
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
            "disk_format_version": 1,
            "engine_version": expected_version,
            "product": "hyphae",
        }
        if version != expected:
            raise RuntimeError(f"installed version mismatch: {version!r}")

        run_json(binary, ["put", "--key", "alpha", "--json", '{"group":"x","score":10}'], environment)
        run_json(binary, ["put", "--key", "beta", "--json", '{"group":"x","score":20}'], environment)
        read = run_json(binary, ["get", "--key", "alpha"], environment)
        if read.get("record", {}).get("value") != {"group": "x", "score": 10}:
            raise RuntimeError("installed binary returned the wrong durable value")
        query = run_json(
            binary,
            ["query", "--field", "group", "--equals", '"x"', "--sort", "score"],
            environment,
        )
        if [row["key_hex"] for row in query.get("rows", [])] != ["616c706861", "62657461"]:
            raise RuntimeError("installed binary returned the wrong global query order")
        run_json(binary, ["snapshot"], environment)
        run_json(binary, ["compact"], environment)

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
        if restored_value.get("record", {}).get("value") != {"group": "x", "score": 10}:
            raise RuntimeError("installed restore did not preserve the durable value")
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
