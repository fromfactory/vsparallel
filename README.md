# VSParallel

VSParallel is a local-first desktop companion for developers working across
multiple VS Code windows. It brings every workspace into one clear overview,
showing workspace activity, focus, and coarse Codex and Claude Code lifecycle
status at a glance.

Switch between projects instantly, return to the workspace that needs your
attention, and access active workspaces directly from the native system tray—all
without interrupting your flow.

VSParallel operates entirely on the current device. It has no account system,
telemetry, analytics, advertising, or application-initiated network requests.
It does not extract, log, retain, or transmit prompts, responses, source code,
terminal contents, transcripts, or Git data. Optional lifecycle hooks receive
documented provider event payloads, extract only the event name, session ID, and
working directory, and discard all unselected fields. See
[PRIVACY.md](PRIVACY.md) for the complete local-data and cleanup policy.

## Requirements and platform support

- VS Code 1.85 or newer
- VS Code's `code` command-line interface
- The platform WebView and runtime libraries required by Tauri 2

| Platform | GitHub Release downloads |
| --- | --- |
| Ubuntu Linux | x86-64 Debian package and AppImage |
| macOS 12.3+ | Universal DMG for Apple silicon and Intel |
| Windows | x86-64 NSIS installer |

Build macOS and Windows packages on native runners. Production signing and
notarization are not configured yet.

See the [release and download guide](docs/releases.md) for the tag-based release
process, exact files to download, and current signing warnings.

On Ubuntu and other GNOME-based desktops, the shell must provide an
AppIndicator or StatusNotifier host for the tray icon to appear.

## Install and set up

Install the package for your platform, launch VSParallel, and then:

1. Select the settings gear beside **Refresh**.
2. Install the **VS Code companion**.
3. Optionally install **Codex lifecycle hooks**, **Claude Code lifecycle
   hooks**, or both.
4. Reload VS Code windows that were already open and restart affected provider
   sessions.
5. After installing Codex hooks, run `/hooks` in Codex, review the three
   VSParallel handlers, and trust them.

The final Codex review is an intentional security boundary. VSParallel does not
attempt to approve user hooks on your behalf.

The bundled companion extension reports workspace, focus, heartbeat, and
provider-extension presence through documented VS Code APIs. Optional lifecycle
hooks add coarse **Activity detected**, **Turn finished**,
**Failed/interrupted**, and **Unknown** states. Extension activation alone is
never presented as active work.

Use a workspace card to open its exact target in a new VS Code window, or use
the tray menu to ask VS Code to activate an active target. Closing the main
window hides it when the tray is available; choose **Quit VSParallel** from the
tray menu to stop the application.

### Installed components

The VS Code companion is embedded in the desktop application and installed
through VS Code's supported extension command. Temporary installation files are
removed after the installed version is verified. The extension ID is
`vsparallel.vsparallel-companion`.

The Codex and Claude Code integrations merge VSParallel-owned handlers into the
providers' user configuration. Existing unrelated settings and hooks are
preserved, writes are atomic, and a one-time backup is created before the first
change.

To use VS Code Insiders or another installation, set
`VSPARALLEL_CODE_COMMAND` to its absolute executable path before launching
VSParallel.

## How it works

```text
VS Code companion ─ workspace/focus/extension heartbeat ─┐
Codex hooks ───────────── coarse lifecycle marker ────────┼─ local state ─ Rust core ─ Tauri UI
Claude Code hooks ─────── coarse lifecycle marker ────────┘                  │
                                                                              └─ native tray
```

The desktop application uses Tauri 2, a Rust backend, and static
HTML/CSS/JavaScript. The dependency-free companion extension writes local
workspace heartbeats. Optional provider hooks write only coarse lifecycle
records. The Rust backend validates these records and serves the same snapshot
to the main window and native tray menu.

Lifecycle state is hook-derived rather than an internal provider progress feed.
Records are associated with workspaces by local path, and exact native-window
foregrounding remains subject to VS Code and operating-system focus behavior.
Remote, virtual, and untitled workspaces can be listed but do not expose a
verified local open target.

The record formats and privacy boundary are documented in the
[versioned metadata protocol](docs/protocol.md).

## Privacy and local data

VSParallel stores only the metadata required for the workspace overview:

- local workspace paths, display names, focus state, and heartbeat timestamps;
- whether the configured Codex and Claude Code extensions are installed and
  active in a VS Code window; and
- coarse lifecycle state, a one-way hash of the provider session identifier,
  working directory, and timestamp when optional hooks are enabled.

| Platform | Default state directory |
| --- | --- |
| Linux/Unix | `$XDG_STATE_HOME/vsparallel` or `~/.local/state/vsparallel` |
| macOS | `~/Library/Application Support/VSParallel` |
| Windows | `%LOCALAPPDATA%\VSParallel` |

Set `VSPARALLEL_STATE_DIR` to the same absolute path for VSParallel, VS Code,
Codex, and Claude Code only when overriding these defaults.

Stale heartbeats are hidden after 60 seconds, and lifecycle state older than 24
hours is shown as **Unknown**. VSParallel bounds record parsing and reports
omitted or malformed records in diagnostics, but it does not automatically
delete old records.

Provider configuration backups are complete copies of the original files and
may contain unrelated environment values or secrets that were already present.
They are created with owner-only permissions on Unix and are never uploaded by
VSParallel. See [PRIVACY.md](PRIVACY.md) for exact file locations and removal
instructions.

## Development

### Prerequisites

- Rust stable with Cargo
- VS Code 1.85 or newer with `code` available
- The [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/)

Python 3 is optional and used by the standalone developer VSIX packager and icon
generator. Regenerating icon assets additionally requires Pillow. Node package
managers are not required.

### Run locally

From the repository root:

```bash
./scripts/run-dev.sh
```

The script also removes VS Code Snap-specific GTK/GIO environment settings when
necessary. The equivalent direct command is:

```bash
cargo run --locked --bin vsparallel
```

To use a different VS Code executable:

```bash
VSPARALLEL_CODE_COMMAND=/absolute/path/to/code-insiders ./scripts/run-dev.sh
```

### Test

Run the complete repository check:

```bash
./scripts/check.sh
```

This checks Rust formatting, runs Clippy with warnings denied, executes the Rust
test suite, and runs the JavaScript interface and companion tests when a
Node-compatible runner is available.

### Build release packages

Install the pinned Tauri CLI and use the release wrapper:

```bash
cargo install tauri-cli --version 2.11.4 --locked
./scripts/build-bundles.sh
```

The wrapper uses the locked dependency graph, removes incompatible Snap GUI
environment settings, and remaps local checkout and home paths out of compiled
panic locations. Build macOS and Windows packages on native runners; production
signing and notarization are not configured yet.

Pushing a matching version tag such as `v0.1.0` runs the same wrapper on Ubuntu,
macOS, and Windows and publishes the resulting packages with the companion VSIX.
See [Releases](docs/releases.md) for the complete procedure.

To build only the release executable:

```bash
cargo build --release --locked --bin vsparallel
```

For a local Linux developer installation:

```bash
./scripts/install-desktop.sh
```

### Companion development

Open `companion/` in VS Code and start an Extension Development Host. There is
no dependency installation step. The optional standalone packager is:

```bash
python3 companion/package_vsix.py
```

Production installation uses the deterministic VSIX assembled and tested by
the Rust backend.

## Uninstall and remove local data

1. Open **Setup & diagnostics** in VSParallel.
2. Uninstall each enabled lifecycle integration.
3. Uninstall the VS Code companion.
4. Reload open VS Code windows and provider sessions.
5. Uninstall the desktop package with the operating system's package manager.

For a local Linux developer installation:

```bash
./scripts/uninstall-desktop.sh
```

Integration removal intentionally preserves the state directory and one-time
configuration backups. To erase those files, quit VSParallel and VS Code, then
follow the cleanup instructions in [PRIVACY.md](PRIVACY.md).

## License

VSParallel is available under the [MIT License](LICENSE).
