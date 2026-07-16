#!/usr/bin/env python3
"""Generate a SLSA provenance v1 predicate for one native archive build."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = "https://github.com/celiumsai/hyphae"
WORKFLOW_PATH = ".github/workflows/release.yml"
BUILD_TYPE = "https://slsa-framework.github.io/github-actions-buildtypes/workflow/v1"
BUILDER_ID = "https://github.com/actions/runner/github-hosted"


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def build_predicate(
    *,
    target: str,
    commit: str,
    git_ref: str,
    invocation_id: str,
    runner_os: str,
    runner_arch: str,
) -> dict[str, object]:
    if re.fullmatch(r"[0-9a-f]{40}", commit) is None:
        raise ValueError("commit must be a lowercase 40-character Git object ID")
    if not git_ref.startswith("refs/"):
        raise ValueError("git-ref must be a full refs/ path")
    if not invocation_id.startswith("https://github.com/"):
        raise ValueError("invocation-id must be an HTTPS GitHub run URI")
    if not all((target, runner_os, runner_arch)):
        raise ValueError("target and runner identity must be nonempty")

    workflow = ROOT / WORKFLOW_PATH
    lockfile = ROOT / "Cargo.lock"
    return {
        "buildDefinition": {
            "buildType": BUILD_TYPE,
            "externalParameters": {
                "profile": "dist",
                "target": target,
                "workflow": {
                    "path": f"/{WORKFLOW_PATH}",
                    "ref": git_ref,
                    "repository": REPOSITORY,
                },
            },
            "internalParameters": {
                "runner_arch": runner_arch,
                "runner_os": runner_os,
                "rust_toolchain": "1.96.0",
            },
            "resolvedDependencies": [
                {
                    "digest": {"gitCommit": commit},
                    "uri": f"git+{REPOSITORY}@{git_ref}",
                },
                {
                    "digest": {"sha256": digest(lockfile)},
                    "uri": f"{REPOSITORY}/blob/{commit}/Cargo.lock",
                },
                {
                    "digest": {"sha256": digest(workflow)},
                    "uri": f"{REPOSITORY}/blob/{commit}/{WORKFLOW_PATH}",
                },
            ],
        },
        "runDetails": {
            "builder": {"id": BUILDER_ID},
            "metadata": {"invocationId": invocation_id},
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--git-ref", required=True)
    parser.add_argument("--invocation-id", required=True)
    parser.add_argument("--runner-os", required=True)
    parser.add_argument("--runner-arch", required=True)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    predicate = build_predicate(
        target=arguments.target,
        commit=arguments.commit,
        git_ref=arguments.git_ref,
        invocation_id=arguments.invocation_id,
        runner_os=arguments.runner_os,
        runner_arch=arguments.runner_arch,
    )
    arguments.output.write_text(
        json.dumps(predicate, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
