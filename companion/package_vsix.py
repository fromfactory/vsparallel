#!/usr/bin/env python3
"""Build the dependency-free VSParallel companion VSIX with Python stdlib."""

from __future__ import annotations

import argparse
import json
import zipfile
import xml.etree.ElementTree as ElementTree
from pathlib import Path


ROOT = Path(__file__).resolve().parent
PACKAGE = ROOT / "package.json"
VSIX_MANIFEST = ROOT / "extension.vsixmanifest"
EXTENSION_FILES = ("package.json", "extension.js", "icon.png", "README.md")
FIXED_TIME = (2026, 1, 1, 0, 0, 0)


def add_file(archive: zipfile.ZipFile, source: Path, destination: str) -> None:
    info = zipfile.ZipInfo(destination, FIXED_TIME)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = 0o100644 << 16
    archive.writestr(info, source.read_bytes())


def load_metadata() -> dict[str, object]:
    manifest = json.loads(PACKAGE.read_text(encoding="utf-8"))
    identity = ElementTree.parse(VSIX_MANIFEST).find(".//{*}Identity")
    if identity is None:
        raise ValueError("companion/extension.vsixmanifest has no Identity")

    fields = {
        "name": "Id",
        "version": "Version",
        "publisher": "Publisher",
    }
    for package_field, identity_attribute in fields.items():
        value = manifest.get(package_field)
        if not isinstance(value, str) or not value:
            raise ValueError(f"companion/package.json has no {package_field}")
        if identity.get(identity_attribute) != value:
            raise ValueError(
                f"companion {package_field} disagrees between package.json "
                "and extension.vsixmanifest"
            )
    return manifest


def build(output: Path) -> Path:
    load_metadata()

    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w") as archive:
        add_file(archive, ROOT / "[Content_Types].xml", "[Content_Types].xml")
        add_file(archive, VSIX_MANIFEST, "extension.vsixmanifest")
        for name in EXTENSION_FILES:
            add_file(archive, ROOT / name, f"extension/{name}")
        add_file(archive, ROOT / "LICENSE", "extension/LICENSE.txt")
    return output


def main() -> int:
    version = load_metadata()["version"]
    assert isinstance(version, str)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "output",
        nargs="?",
        type=Path,
        default=ROOT.parent / "dist" / f"vsparallel-companion-{version}.vsix",
    )
    arguments = parser.parse_args()
    output = build(arguments.output.resolve())
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
