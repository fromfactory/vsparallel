#!/usr/bin/env python3
"""Create Tauri's static latest.json from signed release assets."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from urllib.parse import quote


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--repository", required=True, help="GitHub owner/repository")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def one_asset(directory: Path, pattern: str) -> Path:
    matches = sorted(directory.glob(pattern))
    if len(matches) != 1:
        names = ", ".join(path.name for path in matches) or "none"
        raise SystemExit(f"expected one asset matching {pattern}, found: {names}")
    return matches[0]


def platform_entry(repository: str, tag: str, artifact: Path) -> dict[str, str]:
    signature_path = Path(f"{artifact}.sig")
    if not signature_path.is_file():
        raise SystemExit(f"missing updater signature: {signature_path.name}")
    signature = signature_path.read_text(encoding="utf-8").strip()
    if not signature:
        raise SystemExit(f"empty updater signature: {signature_path.name}")
    url = (
        f"https://github.com/{quote(repository, safe='/')}/releases/download/"
        f"{quote(tag, safe='')}/{quote(artifact.name, safe='')}"
    )
    return {"signature": signature, "url": url}


def main() -> None:
    args = parse_args()
    version = args.tag.removeprefix("v")
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        raise SystemExit(f"release tag is not a supported semantic version: {args.tag}")

    appimage = platform_entry(
        args.repository,
        args.tag,
        one_asset(args.assets, "*.AppImage"),
    )
    deb = platform_entry(args.repository, args.tag, one_asset(args.assets, "*.deb"))
    macos = platform_entry(
        args.repository,
        args.tag,
        one_asset(args.assets, "*.app.tar.gz"),
    )
    nsis = platform_entry(
        args.repository,
        args.tag,
        one_asset(args.assets, "*-setup.exe"),
    )

    manifest = {
        "version": version,
        "notes": "See the GitHub Release for release notes.",
        "platforms": {
            "linux-x86_64-appimage": appimage,
            "linux-x86_64-deb": deb,
            "windows-x86_64-nsis": nsis,
            "darwin-aarch64-app": macos,
            "darwin-x86_64-app": macos,
            # Generic keys retain compatibility with updater clients that do not report
            # their installer type. Installer-aware clients prefer the entries above.
            "linux-x86_64": appimage,
            "windows-x86_64": nsis,
            "darwin-aarch64": macos,
            "darwin-x86_64": macos,
        },
    }
    args.output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
