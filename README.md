# VSParallel

VSParallel is a local-first desktop companion for developers working across
multiple VS Code and Antigravity IDE windows. It brings every workspace into
one clear overview, showing workspace activity, focus, and coarse Antigravity,
Codex, and Claude Code lifecycle status at a glance. Antigravity 2.0 activity
can also appear as a recent workspace, with the limitations described below.

Switch between projects instantly, return to the workspace that needs your
attention, and access active workspaces directly from the native system tray—all
without interrupting your flow.

![VSParallel showing workspace activity and Codex and Claude Code usage at a glance](assets/demo.gif)

VSParallel has no account system, telemetry, analytics, or advertising. Its
workspace and provider monitoring remain local. VSParallel asks the user's
installed Codex `app-server` and signed-in Claude CLI for live rate-limit
percentages and reset times. Each provider subprocess owns its authentication
and any network connection. VSParallel does not read or store either
credential, and it does not persist the live usage responses.

VSParallel does not extract, log, retain, or transmit prompts, responses,
source code, terminal contents, transcripts, or Git data. Optional lifecycle
hooks receive documented provider event payloads and construct new,
privacy-minimal records containing only a hashed session/conversation key,
local workspace path, coarse state, and timestamp. Live provider usage handling
and Claude Code's local status-line fallback similarly retain only rate-limit
percentages and reset times. Antigravity also writes a path-free hook-health
receipt containing fixed event/surface/outcome values, a timestamp, and record
count so Setup can distinguish configured from observed execution. See
[PRIVACY.md](PRIVACY.md) for the complete local-data and cleanup policy.

## Requirements and platform support

- VS Code 1.85 or newer, Antigravity IDE, or both
- The command-line launcher for each editor you want VSParallel to open
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

On macOS, copy `VSParallel.app` to `/Applications` before installing hooks. If
hooks were installed while the app ran from a mounted `/Volumes` DMG or an App
Translocation path, relaunch the installed app and choose **Repair** for each
affected integration.

1. Select the settings gear beside **Refresh**.
2. Install the companion for **VS Code**, **Antigravity IDE**, or both.
3. Optionally install **Antigravity activity hooks**, **Codex lifecycle
   hooks**, **Claude Code lifecycle hooks**, or any combination of them.
4. Reload editor windows that were already open and restart affected provider
   sessions.
5. For Antigravity 2.0, open a saved **Project** and start a new agent turn.
   Merely opening or selecting the Project does not fire a lifecycle hook. If
   that Project has `.agents/hooks.json`, its workspace hooks take precedence
   over the global hook, so add the VSParallel handlers there as well or remove
   the override.
6. After installing Codex hooks, run `/hooks` in Codex, review the three
   VSParallel handlers, and trust them.

The final Codex review is an intentional security boundary. VSParallel reads
Codex's resulting user-level trust status but never attempts to approve user
hooks on your behalf. Workspace settings can still disable hooks. Opening Codex
or `/hooks` does not create an activity marker; after trusting the handlers,
submit a prompt from the monitored workspace.

The bundled companion extension reports workspace, focus, heartbeat,
provider-extension presence, and a closed editor identifier through documented
VS Code-compatible APIs. Installed in either VS Code or Antigravity IDE, it
provides the same exact-window tracking and trusted open target. Optional
lifecycle hooks add coarse **Activity detected**, **Turn finished**, and
**Failed/interrupted** states. Before the first matching hook event, the UI says
**No activity yet**; lifecycle information older than 24 hours becomes
**Unknown**. Extension activation alone is never presented as active work.

Antigravity activity monitoring installs a named `vsparallel` entry in
Antigravity's documented global `~/.gemini/config/hooks.json`. Its
`PreInvocation`, `PostToolUse`, and `Stop` handlers derive recent local
workspace activity from `workspacePaths`. Antigravity 2.0, Antigravity IDE, and
the Antigravity CLI share this hook file. VSParallel reduces the documented
`transcriptPath` or fallback `artifactDirectoryPath` root to a bounded product
label and immediately discards the path; CLI and unrecognized events are
ignored. These lifecycle hooks begin with an agent/model invocation—they do not
run when the standalone app merely opens or selects a Project. Setup therefore
distinguishes a configured hook that is **awaiting agent turn** from one whose
execution has been observed. A Project-level `.agents/hooks.json` can override
the global configuration. A hook-only **Antigravity 2.0** or **Antigravity IDE**
row is evidence of recent activity only: it is never marked live or focused
and cannot be opened by VSParallel. If the same IDE path has a companion
heartbeat, the activity is associated with that exact window.

The global usage cards refresh every 60 seconds and when **Refresh** is selected.
Codex usage is available when a signed-in local `codex` executable—on `PATH`
or bundled with a locally installed Codex editor extension—supports
`app-server`. It does not depend on the Codex lifecycle hooks.
Claude Code usage is available when a signed-in `claude` executable—on `PATH`
or bundled with the installed Claude editor extension—supports the CLI/SDK
control-channel usage getter. This is an evolving Claude CLI compatibility
interface, not a documented stable standalone command. VSParallel actively asks
it for the five-hour and seven-day windows, so usage works for graphical Claude
sessions in VS Code-compatible editors that do not run `statusLine`. Lifecycle
hooks and status-line installation are not required for the active query. The
managed `statusLine` cache remains a local fallback for terminal Claude and older versions. If the
user already has a custom status line, VSParallel preserves it; only that
cache's managed refresh is unavailable, and any existing record may remain
visible as stale.

Claude's current full-usage getter also calculates local attribution summaries.
VSParallel prevents it from seeing real session history by running the query
with a new empty, private configuration directory and no session persistence,
while Claude keeps control of its existing secure authentication. The parser
keeps only rate-limit windows and discards account, session, attribution, and
other response fields.

Use a companion-backed workspace card to ask its reporting editor—VS Code or
Antigravity IDE—to open or activate the exact target.
VSParallel then stays available as a compact always-on-top panel: choose another
workspace, restore the full window, or temporarily hide the panel. The panel
uses native vibrancy on macOS and acrylic blur on supported Windows versions;
Linux uses a compositor-friendly translucent surface when native background
blur is unavailable. A short non-focusing visibility check follows delayed VS
Code-compatible editor activation so the panel remains available when an
existing editor window is on another virtual desktop. On macOS it requests the native auxiliary
full-screen Space behavior; on Windows it follows the verified foreground
editor with the public virtual-desktop API.
The tray menu can also activate an active target. Closing the full window hides
it when the tray is available; choose **Quit VSParallel** from the tray menu to
stop the application.

Packaged releases check GitHub Releases for updates once in the background after
startup. When a newer signed version is available, an in-app banner can download
and install it, show progress, and restart VSParallel. Choose **Later** to defer
it for the current session, or use **Check for updates** in **Setup &
diagnostics**. Development builds, an unpublished release endpoint, and
temporary updater failures do not interrupt the local monitor.

### Installed components

The companion is embedded in the desktop application and can be installed
independently through VS Code's or Antigravity IDE's supported extension
command. Temporary installation files are removed after the installed version
is verified. Both installations use the extension ID
`vsparallel.vsparallel-companion`.

The Antigravity, Codex, and Claude Code integrations merge VSParallel-owned
handlers into their user configuration. Existing unrelated settings and hooks
are preserved, writes are atomic, and a one-time backup is created before the
first change. Removing the Antigravity integration removes only the owned
`vsparallel` entry and preserves
`~/.gemini/config/hooks.json.vsparallel.bak` for manual cleanup.

When no `statusLine` is configured, the Claude Code integration also installs a
privacy-minimal fallback usage capture command with a 60-second refresh
interval. An existing custom `statusLine` is unrelated user configuration and
is left unchanged. Lifecycle monitoring and the active Claude usage query can
still work; only VSParallel's managed refresh of the status-line fallback is
disabled.

To use VS Code Insiders or another installation, set
`VSPARALLEL_CODE_COMMAND` to its absolute executable path before launching
VSParallel.

To select a different Antigravity IDE installation, set
`VSPARALLEL_ANTIGRAVITY_IDE_COMMAND` to its absolute executable path before
launching VSParallel. Companion heartbeats contain only the trusted
`vscode`/`antigravity_ide` identifier, never this executable path.

For bundled Codex and Claude executable discovery, VSParallel tries both
configured editor launchers, then reads only the provider entries in the local
registries at `~/.vscode/extensions`, `~/.vscode-insiders/extensions`,
`~/.vscode-oss/extensions`, and `~/.antigravity-ide/extensions`.

Codex usage tries `codex` on `PATH` and the executable bundled with the locally
installed Codex extension in VS Code or Antigravity IDE. To select another
signed-in executable, set
`VSPARALLEL_CODEX_COMMAND` to its absolute path before launching VSParallel.

Claude Code usage can use either `claude` on `PATH` or the executable bundled
with the installed Claude extension in VS Code or Antigravity IDE, trying the
other source if the first query fails. To select another signed-in executable,
set
`VSPARALLEL_CLAUDE_COMMAND` to its absolute path before launching VSParallel.

## How it works

```text
VS Code / Antigravity IDE companion ─ exact workspace/editor heartbeat ─┐
Antigravity agent hooks ── recent product/path lifecycle marker ────────┤
Codex hooks ──────────────── coarse lifecycle marker ───────────────────┤─ local state ─┐
Claude Code hooks ────────── coarse lifecycle marker ───────────────────┤               │
Claude Code statusLine ───── fallback usage cache ──────────────────────┘               │
Claude CLI control ───────── live usage percentages/reset times ────────────────────────┤─ Rust core ─ Tauri UI
Codex app-server ─────────── hook trust + live usage percentages/reset times ───────────┘             └─ native tray
```

The desktop application uses Tauri 2, a Rust backend, and a framework-free
HTML/CSS/TypeScript UI compiled to static JavaScript. The dependency-free
companion extension writes local workspace heartbeats. Optional provider hooks
write only coarse lifecycle records. Claude and Codex limits are fetched live
through their provider-owned processes and are not written to the state
directory. The Claude Code status-line fallback writes a separate global
`usage/claude.json` record containing only captured percentages, reset times,
and a capture timestamp. The Rust backend validates these records and serves
UI-safe snapshots to the main window; the workspace snapshot also feeds the
native tray menu.

Lifecycle state is hook-derived rather than an internal provider progress feed.
Records are associated with workspaces by local path, and exact native-window
foregrounding remains subject to the companion-backed editor and
operating-system focus behavior. Antigravity hooks identify their product
surface only after an agent turn and do not expose exact live-window state, so
only a VS Code or Antigravity IDE heartbeat can establish liveness, focus, and
an open target. Antigravity 2.0's saved-project registry does not identify the
currently open Project and is not used as a presence signal.
Remote, virtual, and untitled workspaces can be listed but do not expose a
verified local open target. VSParallel shows whether a companion-backed editor
window and its provider extensions are local or remote, but the desktop app
cannot query
usage or receive lifecycle hooks across the remote-host boundary in this
release.

The record formats and privacy boundary are documented in the
[versioned metadata protocol](docs/protocol.md).

## Privacy and local data

VSParallel stores only the metadata required for the workspace overview:

- local workspace paths, display names, editor identity, focus state, and
  heartbeat timestamps;
- whether the configured Codex and Claude Code extensions are installed and
  active in a VS Code or Antigravity IDE window/profile and, when known,
  whether they run in the local or remote extension host; and
- coarse lifecycle state, a one-way hash of the provider session or
  conversation identifier, working directory, and timestamp when optional
  hooks are enabled;
- Antigravity hook execution health containing fixed event, surface, and
  outcome values plus timestamp and workspace-record count; and
- Claude Code five-hour and weekly usage percentages, their optional reset
  times, and the local capture timestamp only when the managed status-line
  fallback cache is available.

Live Codex and Claude usage is held only in memory while VSParallel is running.
VSParallel does not write live usage or provider account details to the state
directory. The provider cards show the lowest remaining percentage among the
available windows so the compact summary does not overstate capacity. A recent,
unexpired value may remain visible with a **Stale** badge for up to 15 minutes
if a refresh temporarily fails.

| Platform | Default state directory |
| --- | --- |
| Linux/Unix | `$XDG_STATE_HOME/vsparallel` or `~/.local/state/vsparallel` |
| macOS | `~/Library/Application Support/VSParallel` |
| Windows | `%LOCALAPPDATA%\VSParallel` |

Set `VSPARALLEL_STATE_DIR` to the same absolute path for VSParallel, the
supported editors, Codex, Claude Code, and Antigravity only when overriding
these defaults.

Stale heartbeats are hidden after 60 seconds, and lifecycle state older than 24
hours is shown as **Unknown**. VSParallel bounds record parsing and reports
omitted or malformed records in diagnostics, but it does not automatically
delete old records.

Integration configuration backups are complete copies of the original files and
may contain unrelated environment values or secrets that were already present.
They are created with owner-only permissions on Unix and are never uploaded by
VSParallel. See [PRIVACY.md](PRIVACY.md) for exact file locations and removal
instructions.

## Development

### Prerequisites

- Rust stable with Cargo
- Node.js 24 with npm
- VS Code 1.85 or newer with `code` available
- The [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/)

Python 3 is optional and used by the standalone developer VSIX packager and icon
generator. Regenerating icon assets additionally requires Pillow.

The UI source lives in `ui/*.ts`. `npm run build:ui` emits ignored runtime files
under `ui/generated/`; edit the TypeScript sources rather than those artifacts.

### Run locally

From the repository root:

```bash
npm ci
./scripts/run-dev.sh
```

The script compiles the TypeScript UI and removes VS Code Snap-specific GTK/GIO
environment settings before Cargo starts. VSParallel repeats that environment
cleanup at application startup. The equivalent direct commands are also safe
from a VS Code Snap integrated terminal:

```bash
npm run build:ui
cargo run --locked --bin vsparallel
```

To use a different VS Code executable:

```bash
VSPARALLEL_CODE_COMMAND=/absolute/path/to/code-insiders ./scripts/run-dev.sh
```

To use a different Antigravity IDE executable:

```bash
VSPARALLEL_ANTIGRAVITY_IDE_COMMAND=/absolute/path/to/antigravity-ide ./scripts/run-dev.sh
```

If you use a named editor profile, select the same profile for companion setup:

```bash
VSPARALLEL_VSCODE_PROFILE=Work ./scripts/run-dev.sh
VSPARALLEL_ANTIGRAVITY_IDE_PROFILE=Agents ./scripts/run-dev.sh
```

To use a Codex executable that is not the `codex` found on `PATH`:

```bash
VSPARALLEL_CODEX_COMMAND=/absolute/path/to/codex ./scripts/run-dev.sh
```

To force a Claude executable instead of the `claude` found on `PATH` or the
binary bundled with the Claude extension in VS Code or Antigravity IDE:

```bash
VSPARALLEL_CLAUDE_COMMAND=/absolute/path/to/claude ./scripts/run-dev.sh
```

### Test

Run the complete repository check:

```bash
./scripts/check.sh
```

This compiles and strictly type-checks the TypeScript UI, runs its interface
tests, checks Rust formatting, runs Clippy with warnings denied, executes the
Rust test suite, and runs the JavaScript companion tests.

### Website

The GitHub Pages website is a lightweight Vite and Vanilla TypeScript project
in `website/`. Its dependencies and lockfile are separate from the desktop
application tooling. To start the development server from the repository root:

```bash
npm --prefix website ci
npm --prefix website run dev
```

Open `http://localhost:5173/vsparallel/` and stop the server with `Ctrl+C`. To
build and preview the production output under the same repository base path:

```bash
npm --prefix website run build
npm --prefix website run preview
```

Open `http://localhost:4173/vsparallel/`. Pushes to `main` that change the
website run the separate Pages workflow in `.github/workflows/pages.yml`; the
tag-triggered desktop release workflow is unchanged. Before the first
deployment, select **GitHub Actions** as the publishing source under the
repository's **Settings > Pages**.

### Build release packages

Install the pinned Tauri CLI and use the release wrapper:

```bash
npm ci
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
npm run build:ui
cargo build --release --locked --bin vsparallel
```

For a local Linux developer installation:

```bash
./scripts/install-desktop.sh
```

### Companion development

Open `companion/` in VS Code or Antigravity IDE and start an Extension
Development Host. There is no dependency installation step. The optional
standalone packager is:

```bash
python3 companion/package_vsix.py
```

Production installation uses the deterministic VSIX assembled and tested by
the Rust backend.

## Uninstall and remove local data

1. Open **Setup & diagnostics** in VSParallel.
2. Uninstall each enabled lifecycle or Antigravity activity integration.
3. Uninstall each installed VS Code or Antigravity IDE companion.
4. Reload open editor windows and provider sessions.
5. Uninstall the desktop package with the operating system's package manager.

For a local Linux developer installation:

```bash
./scripts/uninstall-desktop.sh
```

Integration removal intentionally preserves the state directory and one-time
configuration backups. To erase those files, quit VSParallel and the supported
editors, then follow the cleanup instructions in [PRIVACY.md](PRIVACY.md).

## License

VSParallel is available under the [MIT License](LICENSE).
