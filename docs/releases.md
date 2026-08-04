# Releases

Pushing a version tag builds the existing Tauri application on native GitHub
hosted runners and publishes the packages, plus the VS Code companion VSIX, in
one GitHub Release. The Release is created only after the repository checks and
all platform builds succeed.

## Create a release

1. Set the same application version in `src-tauri/tauri.conf.json` and the
   `[package]` section of `src-tauri/Cargo.toml`. Run `cargo check` to update
   `Cargo.lock`.
2. If the companion changed, keep its independent version synchronized between
   `companion/package.json` and `companion/extension.vsixmanifest`.
3. Run `./scripts/check.sh`, commit the release changes, and push the commit.
4. Tag that commit with the application version and push the tag:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

The tag must be `v` followed by the exact configured application version. The
tag push starts `.github/workflows/release.yml`; no separate release command is
needed.

## Platforms and downloads

| Platform | Build | Download |
| --- | --- | --- |
| Ubuntu Linux | Ubuntu 22.04, x86-64 | Use `VSParallel_<version>_amd64.deb` on Debian or Ubuntu. Use `VSParallel_<version>_amd64.AppImage` on other compatible x86-64 distributions. |
| macOS 12.3+ | Universal (Apple silicon and Intel) | Use `VSParallel_<version>_universal.dmg`. |
| Windows | x86-64 | Use the `VSParallel_<version>_x64-setup.exe` NSIS installer. |
| VS Code 1.85+ | Platform independent | `vsparallel-companion-<companion-version>.vsix` is the optional standalone companion package. Most users should install the embedded companion from VSParallel instead. |

An AppImage downloaded through a browser may need to be made executable with
`chmod +x VSParallel_*.AppImage` before it can be launched.

## Current limitations

- The macOS application is ad-hoc signed, not Developer ID signed or notarized.
  Gatekeeper may require the user to confirm the app with **Open** or in
  **System Settings > Privacy & Security**.
- The Windows installer is not code signed, so Microsoft Defender SmartScreen
  may show an unknown-publisher warning.
- The Debian package, AppImage, and VSIX are not separately signed. Release
  checksums are not currently generated.
- Linux and Windows packages are x86-64 only. macOS uses one universal package.
- GitHub Releases provide downloads only; the application does not currently
  implement automatic updates.

Native jobs and standard Tauri configuration are kept separate, so production
macOS notarization and Windows code signing can be added later without changing
the package formats or release trigger.
