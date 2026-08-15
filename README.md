# VSParallel

VSParallel is a local-first desktop dashboard for developers who work across
multiple VS Code, Cursor, Antigravity, and Zed windows. It shows open and recent
workspaces, coarse coding-agent activity, and selected provider usage in one
place. Use the panel or system tray to return to the workspace that needs your
attention.

![VSParallel showing workspace activity and provider usage](assets/demo.gif)

VSParallel has no account system or advertising, and it sends no product
telemetry or analytics to the project. It does not send workspace records,
prompts, responses, source code, terminal contents, transcripts, or Git data to
the project. See [Privacy](PRIVACY.md) for the full local-data and
network-connection summary.

## What it supports

| Editor or surface | What VSParallel can show | Setup |
| --- | --- | --- |
| VS Code | Live windows, focus, recent workspaces, and optional Codex or Claude Code activity | Install the companion; add provider hooks if wanted |
| Cursor IDE | Live windows, focus, recent workspaces, and Cursor agent activity | Install the combined companion and hook integration |
| Cursor Agents Window | Recent hook activity; an experimental bridge can refine the status of matched threads | Optional and limited by Cursor's Desktop Bridge rollout |
| Antigravity IDE | Live windows, focus, recent workspaces, and agent activity | Install the combined companion and hook integration |
| Antigravity 2.0 | Recent activity after an agent turn | Install the Antigravity hook integration |
| Zed | Open and recent local workspaces plus saved Zed Agent activity, model, and token data | Automatic read-only discovery |

Companion-backed VS Code, Cursor IDE, and Antigravity IDE workspaces with a
verified local target can be opened or focused from VSParallel. Zed Stable
workspaces can also be opened through the local `zed` launcher. Hook-only rows
are recent observations, so they are not marked focused or made openable.

Zed Preview, Nightly, and Dev observations are shown as **Recent** because
VSParallel cannot reliably match their processes to a release channel. Remote
Zed workspaces are omitted. Other remote editor windows may be listed, but this
release cannot collect hooks or provider usage across the remote boundary.

### Usage cards

The dashboard keeps different metrics clearly labeled:

| Provider | Metric | Source |
| --- | --- | --- |
| Codex | Account rate limits | Signed-in local Codex process |
| Claude Code | Account rate limits | Signed-in local Claude CLI, with an optional status-line fallback |
| Antigravity | Model quota | Antigravity CLI 1.1.11+ using its read-only `/usage` command |
| Gemini CLI | Tokens in the latest model call | Optional `AfterModel` hook |
| Zed Agent | Cumulative tokens in the newest eligible local thread | Read-only Zed data |
| Cursor | CLI context remaining, or tokens in the latest local agent turn | Optional status line and Cursor hooks |

Gemini, Zed, and Cursor token or context values are local usage signals, not
subscription or billing quota.

## Download and requirements

Download the latest package from [GitHub Releases](https://github.com/fromfactory/vsparallel/releases/latest).

| Platform | Package |
| --- | --- |
| Linux x86-64 | Debian package for Debian/Ubuntu, or AppImage |
| macOS 12.3+ | Universal DMG for Apple silicon and Intel |
| Windows x86-64 | NSIS installer |

VSParallel can show provider usage without an editor. To open workspaces, you
need a supported editor and its command-line launcher. The companion targets VS
Code 1.85 or newer and compatible Cursor and Antigravity IDE versions. On GNOME,
the desktop shell must provide an AppIndicator or StatusNotifier host for the
tray icon.

Current macOS builds are not notarized, and Windows builds are not code signed.
Your operating system may show an unknown-developer or unknown-publisher
warning. See the [release guide](docs/releases.md) for package names, update
signing, and current distribution limitations.

## Set up

On macOS, first copy `VSParallel.app` from the DMG to `/Applications`. Installing
hooks while running from the mounted DMG or an App Translocation path leaves
temporary executable paths in provider settings; relaunch the installed app and
use **Repair** if that happened.

1. Launch VSParallel and select the settings gear beside **Refresh**.
2. Set up the editors you use. Cursor and Antigravity each install their
   companion and activity hooks together. Zed needs no installation.
3. Optionally install Codex lifecycle hooks, Claude Code lifecycle hooks, and
   the Gemini usage hook.
4. Reload editor windows, restart affected provider sessions, and start a new
   agent turn.
5. For Codex, run `/hooks`, review the three VSParallel handlers, and trust them.
   VSParallel never approves hooks for you.

Setup preserves unrelated hooks and settings. If a supported configuration is
changed, VSParallel writes it atomically and keeps a private one-time backup.

Additional notes:

- Cursor IDE monitoring is fully supported. Cursor's separate Agents Window is
  hook-only unless Cursor exposes **Settings > Beta > Desktop Bridge > Allow CLI
  to access desktop agents**. If available, enable it, restart Cursor, and then
  enable the separate experimental option in VSParallel.
- A workspace-level `.agents/hooks.json` can override Antigravity's global hook.
  Add the VSParallel handler there or remove the override if activity is missing.
- Extension activation is not treated as agent activity. A new prompt must
  trigger a lifecycle hook before VSParallel shows an activity state.
- Use **Setup & diagnostics** to repair an integration or inspect why a source is
  unavailable.

Each editor and usage card can be hidden under **Visibility**. These controls
change presentation; they do not uninstall integrations. While any usage card
is visible, the shared snapshot reads all six usage sources, including sources
whose cards are hidden. Hiding every card stops that periodic usage snapshot,
but automatic Zed workspace discovery continues while the app runs and
installed hooks can still update their small local records.

## Privacy in brief

- Workspace and activity records stay on the current device.
- Optional hooks keep only a local workspace path, a one-way session key, a
  coarse state, a timestamp, and a few bounded labels where supported.
- VSParallel ignores prompt, response, source, terminal, transcript, and Git
  content included in provider payloads.
- Zed data is opened read-only. Selected workspace and latest-thread metadata is
  used for the current snapshot and is not copied into VSParallel's state
  directory.
- Codex, Claude, and Antigravity quota checks run through installed provider
  processes. Those processes normally handle authentication and any provider
  connection; the quota collectors do not extract account credentials or
  persist live responses.
- The optional Cursor Desktop Bridge temporarily uses Cursor's local bridge
  token only for each read-only local `listThreads` poll. Integration setup can
  also copy unrelated secrets already present in provider settings into private
  local backup files. Neither is sent to the VSParallel project.
- Installed releases check GitHub Releases for updates after startup. Updates
  are downloaded only when you choose to install one.

Read [PRIVACY.md](PRIVACY.md) for exact storage locations, retention behavior,
configuration backups, website behavior, and deletion steps. Record formats and
validation rules are documented in the [metadata protocol](docs/protocol.md).

## How it works

```text
Editor companions ───────────── workspace, focus, and extension status ─┐
Optional lifecycle hooks ───── coarse agent activity ───────────────────┤
Zed read-only adapter ───────── workspace and saved agent metadata ─────┤
Local usage hooks ───────────── token or context snapshots ─────────────┤
Provider-owned CLI processes ─ live quota held in memory ───────────────┤
                                                                       └─ Tauri desktop app and tray
```

The desktop app uses Tauri 2, a Rust backend, and a framework-free
TypeScript interface. The bundled dependency-free companion writes local
heartbeats for VS Code-compatible editors. Optional integrations write bounded
JSON records under the shared state directory.

## Configuration

State and data-directory overrides must be absolute paths. Absolute executable
paths are recommended for command overrides so every local process resolves the
same program.

| Purpose | Environment variables |
| --- | --- |
| Shared local state | `VSPARALLEL_STATE_DIR` |
| Editor launchers | `VSPARALLEL_CODE_COMMAND`, `VSPARALLEL_CURSOR_COMMAND`, `VSPARALLEL_ANTIGRAVITY_IDE_COMMAND`, `VSPARALLEL_ZED_COMMAND` |
| Provider executables | `VSPARALLEL_CODEX_COMMAND`, `VSPARALLEL_CLAUDE_COMMAND`, `VSPARALLEL_ANTIGRAVITY_COMMAND` |
| Editor profiles | `VSPARALLEL_VSCODE_PROFILE`, `VSPARALLEL_CURSOR_PROFILE`, `VSPARALLEL_ANTIGRAVITY_IDE_PROFILE` |
| Alternate Zed data | `VSPARALLEL_ZED_DATA_DIR` |

When overriding `VSPARALLEL_STATE_DIR`, use the same value for VSParallel and
every companion or hook. Zed has its own data root and uses
`VSPARALLEL_ZED_DATA_DIR` instead.

## Development

Prerequisites:

- Rust stable with Cargo
- Node.js 24 with npm
- [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/)
- VS Code 1.85 or newer for companion development

Run the application:

```bash
npm ci
./scripts/run-dev.sh
```

Run all repository checks:

```bash
./scripts/check.sh
```

Run the website locally:

```bash
npm --prefix website ci
npm --prefix website run dev
```

The UI source is under `ui/`; generated JavaScript is ignored and should not be
edited directly. See the [companion guide](companion/README.md) for extension
development and the [release guide](docs/releases.md) for packaging and
publishing.

## Uninstall and delete local data

1. Open **Setup & diagnostics**.
2. Remove individual integrations, or choose **Uninstall all**.
3. Reload open editors and provider sessions.
4. Uninstall the desktop package with your operating system.

**Uninstall all** disables managed integrations, clears their records, and
turns off the experimental Cursor bridge. Automatic Zed discovery and provider
quota checks remain available. The action does not delete Zed's data, display
preferences, or one-time configuration backups. To remove remaining
VSParallel files, follow the [complete deletion steps](PRIVACY.md#delete-local-data).

For a local Linux developer installation, run:

```bash
./scripts/uninstall-desktop.sh
```

## Documentation

- [Privacy and local data](PRIVACY.md)
- [Metadata protocol](docs/protocol.md)
- [Release and download guide](docs/releases.md)
- [Companion extension](companion/README.md)
- [Codex integration](integrations/codex/README.md)
- [Claude Code integration](integrations/claude/README.md)

## License

VSParallel is available under the [MIT License](LICENSE).
