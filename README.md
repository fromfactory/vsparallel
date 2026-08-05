# VSParallel

VSParallel is a local-first desktop companion for developers working across
multiple VS Code windows. It brings every workspace into one clear overview,
showing workspace activity, focus, and coarse Codex and Claude Code lifecycle
status at a glance.

Switch between projects instantly, return to the workspace that needs your
attention, and access active workspaces directly from the native system tray—all
without interrupting your flow.

VSParallel has no account system, telemetry, analytics, or advertising. Its
workspace and Claude Code monitoring remain local. To populate the Codex usage
card, VSParallel asks the user's installed, signed-in Codex `app-server` for
live rate-limit percentages and reset times; that Codex subprocess may contact
the Codex service using its own existing sign-in. VSParallel does not read or
store the credential.

VSParallel does not extract, log, retain, or transmit prompts, responses,
source code, terminal contents, transcripts, or Git data. Optional lifecycle
hooks receive documented provider event payloads, extract only the event name,
session ID, and working directory, and discard all unselected fields. Claude
Code usage capture similarly extracts only rate-limit percentages and reset
times from local status-line input. See [PRIVACY.md](PRIVACY.md) for the complete
local-data and cleanup policy.

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

The macOS glass panel uses Tauri's transparent-window private API. This is
compatible with the project's direct DMG distribution, but not with Mac App
Store submission.

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

The global usage cards refresh every 60 seconds and when **Refresh** is selected.
Codex usage is available when the `codex` executable is installed, supports
`app-server`, and is signed in. It does not depend on the Codex lifecycle hooks.
Claude Code usage is captured locally through a VSParallel-managed `statusLine`
command installed with the Claude integration. If the user already has a custom
Claude Code status line, VSParallel preserves it and reports Claude usage as
unavailable instead of replacing it. Claude Code must supply rate-limit data at
least once before a percentage can appear.

Use a workspace card to ask VS Code to open or activate its exact target.
VSParallel then stays available as a compact always-on-top panel: choose another
workspace, restore the full window, or temporarily hide the panel. The panel
uses native vibrancy on macOS and acrylic blur on supported Windows versions;
Linux uses a compositor-friendly translucent surface when native background
blur is unavailable. A short non-focusing visibility check follows delayed VS
Code activation so the panel remains available when an existing editor window
is on another virtual desktop. On macOS it requests the native auxiliary
full-screen Space behavior; on Windows it follows the verified foreground
editor with the public virtual-desktop API.
The tray menu can also activate an active target. Closing the full window hides
it when the tray is available; choose **Quit VSParallel** from the tray menu to
stop the application.

### Installed components

The VS Code companion is embedded in the desktop application and installed
through VS Code's supported extension command. Temporary installation files are
removed after the installed version is verified. The extension ID is
`vsparallel.vsparallel-companion`.

The Codex and Claude Code integrations merge VSParallel-owned handlers into the
providers' user configuration. Existing unrelated settings and hooks are
preserved, writes are atomic, and a one-time backup is created before the first
change.

When no `statusLine` is configured, the Claude Code integration also installs a
privacy-minimal usage capture command with a 60-second refresh interval. An
existing custom `statusLine` is unrelated user configuration and is left
unchanged; lifecycle monitoring can still work, but Claude usage capture remains
unavailable.

To use VS Code Insiders or another installation, set
`VSPARALLEL_CODE_COMMAND` to its absolute executable path before launching
VSParallel.

Codex usage looks for `codex` on `PATH` by default. If the signed-in executable
is elsewhere, set `VSPARALLEL_CODEX_COMMAND` to its absolute path before
launching VSParallel.

## How it works

```text
VS Code companion ─ workspace/focus/extension heartbeat ─┐
Codex hooks ───────────── coarse lifecycle marker ────────┼─ local state ─┐
Claude Code hooks ─────── coarse lifecycle marker ────────┤               │
Claude Code statusLine ── usage percentages/reset times ──┘               ├─ Rust core ─ Tauri UI
Codex app-server ───────── live usage percentages/reset times ─────────────┘      │
                                                                                  └─ native tray
```

The desktop application uses Tauri 2, a Rust backend, and static
HTML/CSS/JavaScript. The dependency-free companion extension writes local
workspace heartbeats. Optional provider hooks write only coarse lifecycle
records. The Claude Code status-line command writes a separate global
`usage/claude.json` record containing only captured percentages, reset times,
and a capture timestamp. Codex limits are fetched live through Codex
`app-server` and are not written to the state directory. The Rust backend
validates these records and serves UI-safe snapshots to the main window; the
workspace snapshot also feeds the native tray menu.

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
  working directory, and timestamp when optional hooks are enabled; and
- Claude Code five-hour and weekly usage percentages, their optional reset
  times, and the local capture timestamp when managed status-line capture is
  available.

Codex usage is held only in memory while VSParallel is running. VSParallel does
not write Codex usage or Codex account details to the state directory. The
provider cards show the lowest remaining percentage among the available windows
so the compact summary does not overstate capacity. A recent, unexpired value
may remain visible with a **Stale** badge for up to 15 minutes if a refresh
temporarily fails.

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

The script removes VS Code Snap-specific GTK/GIO environment settings before
Cargo starts. VSParallel repeats that cleanup at application startup, so the
equivalent direct command is also safe from a VS Code Snap integrated terminal:

```bash
cargo run --locked --bin vsparallel
```

To use a different VS Code executable:

```bash
VSPARALLEL_CODE_COMMAND=/absolute/path/to/code-insiders ./scripts/run-dev.sh
```

To use a Codex executable that is not the `codex` found on `PATH`:

```bash
VSPARALLEL_CODEX_COMMAND=/absolute/path/to/codex ./scripts/run-dev.sh
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
