#!/usr/bin/env python3
"""Review dependency changes without requiring a hosted dependency-graph API."""

from __future__ import annotations

import argparse
import json
import subprocess
import tomllib
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
CARGO_LOCKS = ("Cargo.lock", "fuzz/Cargo.lock")
NPM_LOCKS = (
    "sdks/typescript/package-lock.json",
    "integrations/javascript/package-lock.json",
    "integrations/host-smoke/package-lock.json",
)
PYTHON_MANIFESTS = ("sdks/python/pyproject.toml",)


def git(*arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ("git", *arguments),
        cwd=ROOT,
        check=check,
        text=True,
        capture_output=True,
    )


def read_revision(revision: str, path: str) -> str | None:
    result = git("show", f"{revision}:{path}", check=False)
    return result.stdout if result.returncode == 0 else None


def cargo_dependencies(text: str | None) -> dict[str, dict[str, Any]]:
    if text is None:
        return {}
    parsed = tomllib.loads(text)
    dependencies: dict[str, dict[str, Any]] = {}
    for package in parsed.get("package", []):
        source = package.get("source")
        if source is None:
            continue
        name = package["name"]
        version = package["version"]
        key = f"{name}@{version}|{source}"
        checksum = package.get("checksum")
        if source.startswith("registry+") and not checksum:
            raise ValueError(f"registry dependency lacks checksum: {name}@{version}")
        dependencies[key] = {"checksum": checksum, "source": source}
    return dependencies


def npm_dependencies(text: str | None) -> dict[str, dict[str, Any]]:
    if text is None:
        return {}
    parsed = json.loads(text)
    dependencies: dict[str, dict[str, Any]] = {}
    for location, package in parsed.get("packages", {}).items():
        marker = "node_modules/"
        if marker not in location or package.get("link") is True:
            continue
        name = package.get("name") or location.rsplit(marker, maxsplit=1)[1]
        version = package.get("version")
        if not version:
            raise ValueError(f"npm dependency lacks version: {location}")
        resolved = package.get("resolved")
        integrity = package.get("integrity")
        if resolved and resolved.startswith("http") and not integrity:
            raise ValueError(f"npm dependency lacks integrity: {name}@{version}")
        key = f"{name}@{version}|{location}"
        dependencies[key] = {
            "dev": bool(package.get("dev", False)),
            "integrity": integrity,
            "resolved": resolved,
        }
    return dependencies


def python_dependencies(text: str | None) -> dict[str, dict[str, Any]]:
    if text is None:
        return {}
    parsed = tomllib.loads(text)
    project = parsed.get("project", {})
    groups: dict[str, list[str]] = {"runtime": project.get("dependencies", [])}
    groups.update(project.get("optional-dependencies", {}))
    build = parsed.get("build-system", {}).get("requires", [])
    groups["build"] = build
    dependencies: dict[str, dict[str, Any]] = {}
    for group, requirements in groups.items():
        for requirement in requirements:
            dependencies[f"{group}|{requirement}"] = {"group": group}
    return dependencies


def changed_dependency_files(base: str) -> set[str]:
    result = git("diff", "--name-only", f"{base}...HEAD", "--")
    return {line.strip().replace("\\", "/") for line in result.stdout.splitlines() if line.strip()}


def validate_manifest_lock_pairs(changed: set[str]) -> None:
    rust_manifests = {path for path in changed if path.endswith("Cargo.toml")}
    if any(not path.startswith("fuzz/") for path in rust_manifests) and "Cargo.lock" not in changed:
        raise ValueError("workspace Cargo.toml changed without Cargo.lock")
    if "fuzz/Cargo.toml" in changed and "fuzz/Cargo.lock" not in changed:
        raise ValueError("fuzz/Cargo.toml changed without fuzz/Cargo.lock")
    for manifest in (path for path in changed if path.endswith("package.json")):
        lock = str(Path(manifest).with_name("package-lock.json")).replace("\\", "/")
        if lock not in changed:
            raise ValueError(f"{manifest} changed without {lock}")


def dependency_diff(
    base: dict[str, dict[str, Any]], current: dict[str, dict[str, Any]]
) -> dict[str, Any]:
    shared = base.keys() & current.keys()
    return {
        "added": sorted(current.keys() - base.keys()),
        "removed": sorted(base.keys() - current.keys()),
        "metadata_changed": sorted(key for key in shared if base[key] != current[key]),
    }


def review(base: str) -> dict[str, Any]:
    git("cat-file", "-e", f"{base}^{{commit}}")
    changed = changed_dependency_files(base)
    validate_manifest_lock_pairs(changed)
    ecosystems: dict[str, Any] = {}
    for path in CARGO_LOCKS:
        ecosystems[path] = dependency_diff(
            cargo_dependencies(read_revision(base, path)),
            cargo_dependencies((ROOT / path).read_text(encoding="utf-8")),
        )
    for path in NPM_LOCKS:
        ecosystems[path] = dependency_diff(
            npm_dependencies(read_revision(base, path)),
            npm_dependencies((ROOT / path).read_text(encoding="utf-8")),
        )
    for path in PYTHON_MANIFESTS:
        ecosystems[path] = dependency_diff(
            python_dependencies(read_revision(base, path)),
            python_dependencies((ROOT / path).read_text(encoding="utf-8")),
        )
    return {
        "version": 1,
        "base": base,
        "head": git("rev-parse", "HEAD").stdout.strip(),
        "changed_dependency_files": sorted(
            path
            for path in changed
            if path.endswith(("Cargo.toml", "Cargo.lock", "package.json", "package-lock.json", "pyproject.toml"))
        ),
        "ecosystems": ecosystems,
        "status": "ok",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", required=True)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    report = review(arguments.base)
    arguments.output.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    counts = {
        path: {key: len(value) for key, value in result.items()}
        for path, result in report["ecosystems"].items()
    }
    print(json.dumps({"status": "ok", "changes": counts}, sort_keys=True))


if __name__ == "__main__":
    main()
