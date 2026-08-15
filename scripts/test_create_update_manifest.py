#!/usr/bin/env python3
"""Regression tests for the release updater-manifest generator."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
GENERATOR = REPOSITORY_ROOT / "scripts" / "create-update-manifest.py"


class UpdateManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.assets = Path(self.temporary.name) / "assets"
        self.assets.mkdir()
        self.output = Path(self.temporary.name) / "latest.json"

        for name in (
            "VSParallel_1.2.3_amd64.deb",
            "VSParallel_1.2.3_universal.app.tar.gz",
            "VSParallel_1.2.3_x64-setup.exe",
        ):
            artifact = self.assets / name
            artifact.write_bytes(b"release artifact")
            Path(f"{artifact}.sig").write_text(f"signature-for-{name}\n", encoding="utf-8")

    def run_generator(self, tag: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(GENERATOR),
                "--assets",
                str(self.assets),
                "--repository",
                "fromfactory/vsparallel",
                "--tag",
                tag,
                "--output",
                str(self.output),
            ],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_stable_release_contains_installer_aware_signed_entries(self) -> None:
        result = self.run_generator("v1.2.3")
        self.assertEqual(result.returncode, 0, result.stderr)

        manifest = json.loads(self.output.read_text(encoding="utf-8"))
        self.assertEqual(manifest["version"], "1.2.3")
        self.assertEqual(
            set(manifest["platforms"]),
            {
                "linux-x86_64-deb",
                "windows-x86_64-nsis",
                "darwin-aarch64-app",
                "darwin-x86_64-app",
                "windows-x86_64",
                "darwin-aarch64",
                "darwin-x86_64",
            },
        )
        linux = manifest["platforms"]["linux-x86_64-deb"]
        self.assertEqual(
            linux["url"],
            "https://github.com/fromfactory/vsparallel/releases/download/"
            "v1.2.3/VSParallel_1.2.3_amd64.deb",
        )
        self.assertEqual(
            linux["signature"],
            "signature-for-VSParallel_1.2.3_amd64.deb",
        )

    def test_prerelease_tag_cannot_replace_the_stable_update_channel(self) -> None:
        result = self.run_generator("v1.2.3-beta.1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not a stable semantic version", result.stderr)
        self.assertFalse(self.output.exists())

    def test_stable_version_identifiers_reject_leading_zeroes(self) -> None:
        for tag in ("v01.2.3", "v1.02.3", "v1.2.03"):
            with self.subTest(tag=tag):
                result = self.run_generator(tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("not a stable semantic version", result.stderr)
                self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
