from __future__ import annotations

import json
import unittest

from tools.review_dependencies import (
    cargo_dependencies,
    dependency_diff,
    npm_dependencies,
    python_dependencies,
)


class DependencyReviewTests(unittest.TestCase):
    def test_cargo_registry_dependency_requires_and_keeps_checksum(self) -> None:
        parsed = cargo_dependencies(
            'version = 4\n[[package]]\nname = "demo"\nversion = "1.2.3"\n'
            'source = "registry+https://example.invalid/index"\nchecksum = "abc"\n'
        )
        self.assertEqual(next(iter(parsed.values()))["checksum"], "abc")

    def test_npm_dependency_keeps_integrity_and_scope(self) -> None:
        lock = {
            "packages": {
                "": {"name": "root"},
                "node_modules/@scope/demo": {
                    "version": "2.0.0",
                    "resolved": "https://example.invalid/demo.tgz",
                    "integrity": "sha512-example",
                },
            }
        }
        parsed = npm_dependencies(json.dumps(lock))
        self.assertIn("@scope/demo@2.0.0|node_modules/@scope/demo", parsed)

    def test_python_dependencies_include_runtime_optional_and_build(self) -> None:
        parsed = python_dependencies(
            '[project]\ndependencies = ["one>=1"]\n'
            '[project.optional-dependencies]\ntest = ["two==2"]\n'
            '[build-system]\nrequires = ["three"]\n'
        )
        self.assertEqual(len(parsed), 3)

    def test_diff_reports_added_removed_and_metadata_changes(self) -> None:
        result = dependency_diff(
            {"same": {"checksum": "old"}, "removed": {}},
            {"same": {"checksum": "new"}, "added": {}},
        )
        self.assertEqual(result["added"], ["added"])
        self.assertEqual(result["removed"], ["removed"])
        self.assertEqual(result["metadata_changed"], ["same"])


if __name__ == "__main__":
    unittest.main()
