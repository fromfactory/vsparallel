# Releases

Pushing a version tag builds the Tauri application on native GitHub-hosted
runners. After all checks and platform builds pass, the workflow publishes the
packages, updater-signed artifacts, update manifest, and companion VSIX in one
GitHub Release.

## Updater signing

Released builds already contain the updater public key from
`src-tauri/tauri.conf.json`. The matching private key and optional password must
stay in GitHub Actions secrets. Do not generate or commit a replacement key for
an existing distribution: installed copies trust the public key shipped with
them and would reject updates signed with a different key.

For a new distribution that has never shipped, configure signing before its
first release:

1. Generate the updater keypair outside the repository and protect it with a
   strong password:

   ```bash
   cargo tauri signer generate -w /secure/path/vsparallel.key
   ```

   This creates the private key at the selected path and the public key at the
   same path with `.pub` appended. Back up both the private key and its password
   securely. Losing them prevents installed copies from accepting future
   updates.
2. Copy the complete contents of the `.pub` file into
   `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`. The public key is
   safe and required to be committed; never commit the private key.
3. Add the private key content as the GitHub Actions repository secret
   `TAURI_SIGNING_PRIVATE_KEY`. If the key has a password, add it as
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; otherwise that secret may be omitted.

The release workflow passes `src-tauri/tauri.release.conf.json` only to release
builds. That overlay enables Tauri v2 updater artifacts without requiring every
ordinary local bundle build to have the private signing key. The workflow signs
the `.deb`, `.AppImage`, universal `.app.tar.gz`, and NSIS updater artifacts,
rejects a private/public key mismatch, collects their `.sig` sidecars, and
creates `latest.json` with `scripts/create-update-manifest.py`.

Updater signatures verify update provenance and are separate from macOS
Developer ID signing/notarization and Windows Authenticode signing.

## Create a release

1. Confirm that the committed updater public key is unchanged and the matching
   GitHub Actions secrets are configured.
2. Set the same application version in `src-tauri/tauri.conf.json` and the
   `[package]` section of `src-tauri/Cargo.toml`. Run `cargo check` to update
   `Cargo.lock`.
3. If the companion changed, keep its independent version synchronized between
   `companion/package.json` and `companion/extension.vsixmanifest`.
4. Run `npm ci` and `./scripts/check.sh`, commit the release changes, and push
   the commit.
5. Tag that commit with the application version and push the tag:

   ```bash
   git tag v1.2.3
   git push origin v1.2.3
   ```

The tag must be `v` followed by the exact configured application version. The
tag push starts `.github/workflows/release.yml`; no separate release command is
needed.

## Platforms and downloads

| Platform | Build | Download |
| --- | --- | --- |
| Linux x86-64 | Built on Ubuntu 22.04 | Use `VSParallel_<version>_amd64.deb` on Debian or Ubuntu. Use `VSParallel_<version>_amd64.AppImage` on other compatible distributions. |
| macOS 12.3+ | Universal (Apple silicon and Intel) | Use `VSParallel_<version>_universal.dmg`. |
| Windows | x86-64 | Use the `VSParallel_<version>_x64-setup.exe` NSIS installer. |
| VS Code-compatible editors 1.85+ | Platform independent | `vsparallel-companion-<companion-version>.vsix` is the optional standalone companion for VS Code, Cursor, and Antigravity IDE. Most users should install it from VSParallel instead. |

An AppImage downloaded through a browser may need to be made executable with
`chmod +x VSParallel_*.AppImage` before it can be launched.

## In-app updater assets

The release also contains updater-compatible `.deb`, `.AppImage`, universal
macOS `.app.tar.gz`, and NSIS assets, each with a Tauri `.sig` sidecar, plus
`latest.json`. The manifest uses installer-aware Tauri 2.10+ platform keys so
Debian installations keep using Debian packages, AppImage installations keep
using AppImages, and the universal macOS archive serves both Apple silicon and
Intel clients. It embeds each `.sig` file's content and points downloads at the
tagged GitHub Release.

The configured endpoint is:

```text
https://github.com/fromfactory/vsparallel/releases/latest/download/latest.json
```

It returns 404 until a non-draft GitHub Release containing the manifest exists.
VSParallel treats that, missing development signing configuration, and other
background check failures as non-fatal. A complete end-to-end update test needs
an installed signed release and a second, strictly newer SemVer release.

## Current limitations

- The macOS application is ad-hoc signed, not Developer ID signed or notarized.
  Gatekeeper may require the user to confirm the app with **Open** or in
  **System Settings > Privacy & Security**.
- The Windows installer is not code signed, so Microsoft Defender SmartScreen
  may show an unknown-publisher warning.
- The Debian package, AppImage, and VSIX are not signed in their native package
  formats, and separate release checksums are not currently generated. Tauri
  `.sig` sidecars authenticate updater downloads only; they do not replace
  notarization, platform code signing, or package-manager signing.
- Linux and Windows packages are x86-64 only. macOS uses one universal package.

Native jobs and standard Tauri configuration are kept separate, so production
macOS notarization and Windows code signing can be added later without changing
the package formats or release trigger.
