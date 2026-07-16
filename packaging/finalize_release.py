#!/usr/bin/env python3
"""Create or verify the canonical checksum manifest for release assets."""

from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path

from package import product_version


ASSET_SUFFIXES = (
    ".tar.gz",
    ".zip",
    ".spdx.json",
    ".cdx.json",
    ".provenance.json",
)
ARCHIVE_SUFFIXES = (".tar.gz", ".zip")


def require_matching_tag(tag: str) -> None:
    expected = f"v{product_version()}"
    if tag != expected:
        raise RuntimeError(f"release tag {tag!r} does not match workspace version {expected!r}")


def release_assets(directory: Path) -> list[Path]:
    assets = sorted(
        path
        for path in directory.iterdir()
        if path.is_file() and path.name.endswith(ASSET_SUFFIXES)
    )
    if not assets:
        raise RuntimeError("no release archives or SBOMs found")
    return assets


def archive_assets(directory: Path) -> list[Path]:
    archives = [
        path for path in release_assets(directory) if path.name.endswith(ARCHIVE_SUFFIXES)
    ]
    if not archives:
        raise RuntimeError("release directory contains no native archives")
    return archives


def validate_release_layout(directory: Path, *, final: bool) -> None:
    entries = list(directory.iterdir())
    if any(not entry.is_file() for entry in entries):
        raise RuntimeError("release directory must contain files only")
    assets = release_assets(directory)
    asset_names = {path.name for path in assets}
    archives = archive_assets(directory)
    spdx = [name for name in asset_names if name.endswith(".spdx.json")]
    cyclonedx = [name for name in asset_names if name.endswith(".cdx.json")]
    if len(spdx) != 1 or len(cyclonedx) != 1:
        raise RuntimeError("release requires exactly one SPDX and one CycloneDX SBOM")
    for archive in archives:
        predicate = f"{archive.name}.provenance.json"
        if predicate not in asset_names:
            raise RuntimeError(f"release archive lacks provenance predicate: {archive.name}")

    required_bundles = {
        f"{archive.name}.intoto.sigstore.json" for archive in archives
    }
    checksum = directory / "SHA256SUMS"
    ordinary_names = set(asset_names)
    if checksum.exists():
        ordinary_names.add(checksum.name)
    if final:
        if not checksum.is_file():
            raise RuntimeError("final release lacks SHA256SUMS")
        required_bundles.update(
            f"{name}.sigstore.json" for name in ordinary_names
        )
        for archive in archives:
            required_bundles.add(f"{archive.name}.spdx.attestation.sigstore.json")
            required_bundles.add(f"{archive.name}.cyclonedx.attestation.sigstore.json")

    actual_names = {entry.name for entry in entries}
    expected_names = ordinary_names | required_bundles
    missing = sorted(expected_names - actual_names)
    unexpected = sorted(actual_names - expected_names)
    if missing:
        raise RuntimeError(f"release directory lacks required files: {missing!r}")
    if unexpected:
        raise RuntimeError(f"release directory contains unexpected files: {unexpected!r}")


def create_checksums(directory: Path) -> Path:
    destination = directory / "SHA256SUMS"
    if destination.exists():
        raise FileExistsError(destination)
    lines = [
        f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n"
        for path in release_assets(directory)
    ]
    destination.write_text("".join(lines), encoding="ascii", newline="\n")
    return destination


def verify_checksums(directory: Path) -> None:
    checksum_path = directory / "SHA256SUMS"
    entries: dict[str, str] = {}
    for line in checksum_path.read_text("ascii").splitlines():
        digest, separator, name = line.partition("  ")
        if separator != "  " or len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
            raise RuntimeError("malformed SHA256SUMS line")
        if not name or Path(name).name != name or name in entries:
            raise RuntimeError("unsafe or duplicate SHA256SUMS filename")
        entries[name] = digest
    expected = {path.name for path in release_assets(directory)}
    if set(entries) != expected:
        raise RuntimeError("SHA256SUMS asset set differs from release directory")
    for name, expected_digest in entries.items():
        actual = hashlib.sha256((directory / name).read_bytes()).hexdigest()
        if actual != expected_digest:
            raise RuntimeError(f"checksum mismatch for {name}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--tag", default=os.environ.get("GITHUB_REF_NAME"))
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--print-expected-tag", action="store_true")
    arguments = parser.parse_args()
    if arguments.print_expected_tag:
        if arguments.directory is not None or arguments.verify:
            raise RuntimeError("--print-expected-tag cannot be combined with release operations")
        print(f"v{product_version()}")
        return 0
    if arguments.directory is None:
        raise RuntimeError("release directory is required")
    if arguments.tag is None:
        raise RuntimeError("release tag is required")
    require_matching_tag(arguments.tag)
    if arguments.verify:
        verify_checksums(arguments.directory)
        validate_release_layout(arguments.directory, final=True)
    else:
        validate_release_layout(arguments.directory, final=False)
        create_checksums(arguments.directory)
        verify_checksums(arguments.directory)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
