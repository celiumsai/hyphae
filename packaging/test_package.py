#!/usr/bin/env python3

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from package import build_archive
from finalize_release import (
    create_checksums,
    require_matching_tag,
    validate_release_layout,
    verify_checksums,
)
from provenance import BUILD_TYPE, BUILDER_ID, build_predicate


class PackageTests(unittest.TestCase):
    def test_archives_are_reproducible_and_rooted(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hyphae-package-") as temporary:
            root = Path(temporary)
            binary = root / "binary"
            binary.write_bytes(b"native-binary")
            first_dir = root / "first"
            second_dir = root / "second"
            for target in ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"):
                first = build_archive(binary, target, first_dir, 1_700_000_000)
                second = build_archive(binary, target, second_dir, 1_700_000_000)
                self.assertEqual(first.read_bytes(), second.read_bytes())
                self.assertTrue(first.name.startswith("hyphae-0.1.0-alpha.1-"))

    def test_checksum_manifest_is_complete_and_tamper_evident(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hyphae-checksums-") as temporary:
            root = Path(temporary)
            (root / "hyphae-test.tar.gz").write_bytes(b"archive")
            (root / "hyphae-test.spdx.json").write_text("{}\n", encoding="utf-8")
            create_checksums(root)
            verify_checksums(root)
            (root / "hyphae-test.tar.gz").write_bytes(b"tampered")
            with self.assertRaisesRegex(RuntimeError, "checksum mismatch"):
                verify_checksums(root)

    def test_release_tag_and_slsa_predicate_are_bound_to_source(self) -> None:
        require_matching_tag("v0.1.0-alpha.1")
        with self.assertRaisesRegex(RuntimeError, "does not match"):
            require_matching_tag("v0.1.0")
        predicate = build_predicate(
            target="x86_64-unknown-linux-gnu",
            commit="a" * 40,
            git_ref="refs/tags/v0.1.0-alpha.1",
            invocation_id="https://github.com/celiumsai/hyphae/actions/runs/1/attempts/1",
            runner_os="Linux",
            runner_arch="X64",
        )
        definition = predicate["buildDefinition"]
        details = predicate["runDetails"]
        self.assertEqual(definition["buildType"], BUILD_TYPE)
        self.assertEqual(details["builder"]["id"], BUILDER_ID)
        self.assertEqual(
            definition["resolvedDependencies"][0]["digest"]["gitCommit"],
            "a" * 40,
        )

    def test_release_layout_rejects_unknown_or_missing_supply_chain_files(self) -> None:
        with tempfile.TemporaryDirectory(prefix="hyphae-release-layout-") as temporary:
            root = Path(temporary)
            archive = "hyphae-test.tar.gz"
            for name in (
                archive,
                f"{archive}.provenance.json",
                f"{archive}.intoto.sigstore.json",
                "hyphae-test.spdx.json",
                "hyphae-test.cdx.json",
            ):
                (root / name).write_text("{}\n", encoding="utf-8")
            validate_release_layout(root, final=False)
            (root / "unexpected.txt").write_text("unexpected\n", encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "unexpected files"):
                validate_release_layout(root, final=False)
            (root / "unexpected.txt").unlink()
            with self.assertRaisesRegex(RuntimeError, "SHA256SUMS"):
                validate_release_layout(root, final=True)


if __name__ == "__main__":
    unittest.main()
