# VSParallel

VSParallel is a local-first desktop companion for developers working across
multiple VS Code, Cursor, Antigravity IDE, and Zed windows. It brings every
workspace into one clear overview, showing open and recent workspaces,
companion-reported focus where available, and coarse Cursor Agent,
Antigravity, Zed Agent, Codex, and Claude Code activity at a glance. Zed
workspaces are discovered automatically through a read-only local adapter;
they do not install or use the VSParallel companion. Cursor IDE is fully
supported. Cursor's separate Agents Window has an
experimental, explicitly enabled local bridge that can correlate a running
thread with Cursor hook metadata; without that bridge it falls back to recent
hook-only activity. Antigravity 2.0 can add recent hook-only activity with the
limitations described below.

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
source code, terminal contents, transcripts, or Git data. Optional hook
integrations receive documented provider event payloads and construct new,
privacy-minimal records containing only a one-way key, local workspace path,
coarse state, and timestamp. Cursor `workspaceOpen` records use a deterministic
path-derived key and contain no session identity or agent/model metadata;
Cursor lifecycle records may also contain bounded model and agent labels
supplied by Cursor's native hooks.
Antigravity activity records
may also include an optional closed model classification from hook `modelName`
or, for IDE hooks that omit it, the bounded current-model enum embedded in that
conversation's latest user-input step. Bounded execution metadata and the
IDE's last-selected-model preference remain compatibility fallbacks. Raw model
identifiers are discarded and never persisted. IDE records may also carry an
opaque SHA-256 model-signal revision so a new turn can be distinguished from
the preceding execution; the revision is not shown in the UI.
Zed monitoring opens Zed's local SQLite databases read-only and correlates
persisted workspace metadata with the live Zed process and current
session/window stack. It surfaces only validated workspace paths, timestamps,
the latest persisted agent activity, coarse native turn boundaries, and model
metadata where available. A bounded thread blob may be read selectively to
inspect only its last message variant, tool-use presence, and model
provider/name, then discarded. Thread titles, prompts, responses, tool
payloads, and source code are not retained or copied into VSParallel state.
Live provider usage handling and Claude Code's local status-line fallback
similarly retain only rate-limit percentages and reset times. Antigravity also
writes a path- and model-free hook-health receipt containing fixed
event/surface/outcome values, a timestamp, and validated workspace count so Setup can
distinguish configured from observed execution. See
[PRIVACY.md](PRIVACY.md) for the complete local-data and cleanup policy.

## Requirements and platform support

- VS Code 1.85 or newer, Cursor, Antigravity IDE, Zed, or any combination of them
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
2. Install the companion for **VS Code** or **Antigravity IDE** as needed. For
   Cursor, choose **Set up Cursor monitoring**; this installs or repairs the
   companion and Cursor activity hooks together. Zed is detected automatically
   and needs no companion or hook installation.
3. Optionally install **Antigravity activity hooks**, **Codex lifecycle hooks**,
   **Claude Code lifecycle hooks**, or any combination of them. A separate
   **Cursor hooks only** control is available when non-live fallback monitoring
   is preferred without the companion.
4. Reload editor windows that were already open and restart affected provider
   sessions.
5. Cursor IDE windows report live heartbeats after reload. Cursor exposes its
   experimental Desktop Bridge only to a limited server-controlled rollout. If
   Cursor shows **Settings > Beta > Desktop Bridge > Allow CLI to access desktop
   agents**, enable it, restart Cursor, and then enable **Cursor Agents Window
   (experimental)** in VSParallel. If the Desktop Bridge section is absent,
   live Agents Window monitoring is unavailable in that Cursor installation;
   keep the hooks enabled for recent hook-only status. VSParallel cannot enable
   Cursor's rollout flag and does not modify Cursor's internal feature storage.
6. For an Antigravity built-in model, open a saved **Project** in Antigravity
   2.0 or a workspace in Antigravity IDE and start a new agent turn. Merely
   opening or selecting it does not fire a lifecycle hook. If that workspace
   has `.agents/hooks.json`, its hooks take precedence over the global hook, so
   add the VSParallel handlers there as well or remove the override.
7. After installing Codex hooks, run `/hooks` in Codex, review the three
   VSParallel handlers, and trust them.

The final Codex review is an intentional security boundary. VSParallel reads
Codex's resulting user-level trust status but never attempts to approve user
hooks on your behalf. Workspace settings can still disable hooks. Opening Codex
or `/hooks` does not create an activity marker; after trusting the handlers,
submit a prompt from the monitored workspace.

If a Cursor IDE window is open but no live workspace appears, open **Setup & diagnostics**,
set up or repair **Cursor monitoring**, and reload every affected Cursor
window. Cursor IDE monitoring is fully supported. The separate Agents Window
falls back to recent, non-live native-hook observations unless its experimental
bridge is enabled as described above. Setup installs or repairs the companion
and all five managed Cursor handlers,
including `workspaceOpen`, together. Named-profile users must launch VSParallel
with the matching `VSPARALLEL_CURSOR_PROFILE` value.

The bundled companion extension reports workspace, focus, heartbeat,
provider-extension presence, and a closed editor identifier through compatible
VS Code APIs. Installed in VS Code, Cursor IDE, or Antigravity IDE, it provides
live window tracking and a trusted open target when the host exposes a local
workspace path. Cursor's separate Agents Window does not activate the
third-party companion. Its optional experimental bridge can add a conservative
live-running signal, while native hooks remain the source of workspace and
agent/model metadata. Optional lifecycle hooks add
coarse **Activity detected**, **Turn finished**, and
**Failed/interrupted** states. Before the first matching hook event, the UI says
**No activity yet**; lifecycle information older than 24 hours becomes
**Unknown**. Extension activation alone is never presented as active work.

Zed monitoring is a separate native, read-only adapter. On refresh it inspects
the local Zed stable, preview, nightly, and development channel databases and
checks for a live Zed GUI process. Because the portable process probe cannot
safely distinguish release channels, only Stable workspaces are eligible for
**Open**; other channel observations remain visible as **Recent**. A Stable
workspace appears in **Open** only when its persisted session matches Zed's
current session and its window ID is in the
current session's window stack. Otherwise a usable saved workspace appears in
**Recent**. This correlation does not reveal foreground focus, so Zed rows are
never marked focused. For the native Zed Agent, a submitted turn whose saved
thread ends at a user boundary is shown as **Activity detected** while the
workspace remains open; a newly saved final assistant boundary is shown as
**Turn finished**. The same bounded parse may expose the native model
provider/name. These are coarse persisted boundaries that can lag a fast turn,
not an exact live-generation feed. Zed does not persist enough status here to
distinguish success, cancellation, interruption, or failure. Unknown or older
thread structures retain the **Recent agent activity** fallback.
Remote Zed workspaces are omitted because this local adapter cannot safely
reconstruct or activate their connection context.

Cursor activity monitoring merges VSParallel-owned handlers into Cursor's
native user hook file at `~/.cursor/hooks.json`. The `workspaceOpen` event
records recent workspace evidence only. It creates no agent status or
agent/model label and never proves that a window is live, focused, or openable.
The `sessionStart`, `beforeSubmitPrompt`, `stop`, and `sessionEnd` events provide
coarse Cursor Agent **Activity detected**, **Turn finished**, and
failure/interruption status, plus bounded agent or model labels when Cursor
supplies them. In particular, `sessionStart` captures the agent kind from
Cursor's bounded composer-mode/background fields, but remains metadata-only
until a prompt is submitted. These hooks can run for local Cursor agent
surfaces, including the VS Code-based IDE and separate Agents Window. They do
not expose a native window identity, liveness, focus, exact open target, or
source-surface identifier. Only local paths in the `workspace_roots` values
supplied by Cursor can be associated; pathless events are omitted. An unmatched
`workspaceOpen` path appears in **Recent** as a generic **Cursor** workspace row
with no activity card. An unmatched lifecycle path appears as a generic recent
**Cursor Agent** row rather than being attributed to the IDE, Agents Window, or
Cursor CLI. Neither kind of hook-only row is live, focused, or openable. When
exactly one Cursor IDE heartbeat covers the path, that companion-backed window
owns the observation and the generic duplicate is suppressed; when multiple
windows cover it, the generic row avoids guessing. Parallel sessions are
reduced independently, so one session finishing does not conceal another
session's newer unresolved activity marker.
Prompts, responses, email fields, transcripts, token data, and all other
unselected hook payload fields are discarded. When Cursor launches a hook with
the exact, case-sensitive environment value `CURSOR_CODE_REMOTE=true`, the
handler persists no Cursor activity record; it still returns `{}` and exits
successfully so monitoring remains fail-open. User-level hooks do not cover
cloud agents. Cursor's managed `hooks.json` must be strict JSON; VSParallel
leaves JSON-with-comments or otherwise invalid configuration unchanged and
reports it in Setup. Setup requires all five current handlers; a four-handler
installation from an earlier VSParallel build is shown as needing an update or
repair. Reinstalling preserves unrelated hooks, and uninstall recognizes both
the current and legacy VSParallel-owned handler sets.

Experimental Cursor Agents Window monitoring is a separate, explicit opt-in
and limited by Cursor's server-controlled `desktop_bridge` rollout. When Cursor
shows **Allow CLI to access desktop agents**, that setting is enabled, and
Cursor is restarted, VSParallel reads Cursor's private local Desktop Bridge
discovery files and sends only `listThreads` over local inter-process
communication. It never sends an agent message. A returned thread ID is
immediately SHA-256 hashed and the raw ID is discarded; VSParallel also does
not retain the thread title, bridge token, socket path, Cursor user-data path,
prompt text, or response text. An exact thread hash-to-hook `sessionKey` match
is required before the thread can be associated with a local workspace or show
the hook's bounded agent/model label. Unmatched threads are not displayed.

Only a matched bridge status of `running` makes an experimental **Cursor agent
thread** row appear in **Open**. Observed `completed`, `error`, and `idle` threads remain
in **Recent** with coarse status, and every such row is non-focused and
non-openable. This private Cursor interface is undocumented and may change or
stop working after a Cursor update. It also does not distinguish the standalone
Agents Window from every other Cursor agent surface, so the label means that a
matched Cursor thread was observed through the bridge—not that VSParallel
proved which native surface owns it. If the Desktop Bridge section is absent
from Cursor Settings > Beta, Cursor has not made the rollout available to that
installation and VSParallel cannot activate it. In other cases, no discovery
file can mean Cursor is closed, the setting is disabled, or the bridge has not
started; VSParallel does not guess among those cases.

Antigravity activity monitoring installs a named `vsparallel` entry in
Antigravity's documented global `~/.gemini/config/hooks.json`. Its
`PreInvocation`, `PostToolUse`, and `Stop` handlers derive recent local
workspace activity from `workspacePaths`. Antigravity 2.0, Antigravity IDE, and
the Antigravity CLI share this hook file. VSParallel reduces the documented
`transcriptPath` or fallback `artifactDirectoryPath` root to a bounded product
label and immediately discards the path; CLI and unrecognized events are
ignored. A recognized hook `modelName` is reduced to a closed classification
before the raw value is discarded. Because the Antigravity IDE hook contract
omits that field, each `PreInvocation` reads the current model enum from the
latest user-input step in that hook's local IDE conversation database. That row
exists before the hook runs, so **Activity detected** shows a newly selected
model without waiting for completion. The incremental, size-bounded parser
reads bounded protobuf structural varints, uses only the queued flag and model
enum, and seeks over prompt and context bodies without reading or copying them.
The bounded model-name field from the latest `executor_metadata` row and the
IDE's last-selected preference remain compatibility fallbacks for an older
schema or absent user-input row. An unusable newest row instead preserves the
last correlated model. The desktop refresh repeats the narrow
per-conversation read but waits for the lifecycle hook before adopting a
different current-turn revision, avoiding a relabel in the brief interval
before `PreInvocation`. A decoded unknown model clears the qualifier. Raw step
data, generated responses, prompts, trajectory data, transcripts, executor
blobs, and model names are never retained. Tool-completion hooks are
state-neutral, so `PostToolUse` does not write
the activity file and cannot replace a completed `Stop` marker. An opaque
per-conversation model-signal revision prevents refreshes from restoring the
model associated with the preceding execution.
The workspace row keeps **Antigravity** as the provider and adds a family label,
such as **(Gemini)**, **(Claude)**, or **(GPT-OSS)**, when recognized; otherwise
it remains generic. **Auto** remains explicit rather than guessing the routed
model, and a label means only “latest model associated with this Antigravity
lifecycle record”—it does not prove that inference is live. These lifecycle
hooks begin with an agent/model invocation—they do not run when Antigravity
merely opens or selects a Project or workspace. Setup therefore distinguishes a
configured hook that is **awaiting agent turn** from one whose execution has
been observed in Antigravity 2.0 or Antigravity IDE. A
workspace-level `.agents/hooks.json` can override the global configuration. A
hook-only **Antigravity 2.0** or **Antigravity IDE** row is evidence of recent
activity only: it is never marked live or focused and cannot be opened by
VSParallel. If the same IDE path has a companion heartbeat, the activity is
associated with that exact window.

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

Use a companion-backed workspace card with a verified local target to ask its
reporting editor—VS Code, Cursor, or Antigravity IDE—to open or activate that
target. A validated Stable Zed card uses the locally configured Zed launcher:
an **Open** target uses Zed's `--existing` option, while a **Recent** target
uses Zed's `--new` option. Multi-root workspaces pass the complete validated,
ordered path list in one invocation. Hook-only rows are not openable.
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
independently through VS Code's, Cursor's, or Antigravity IDE's supported
extension command. Temporary installation files are removed after the
installed version is verified. All installations use the extension ID
`vsparallel.vsparallel-companion`.

Zed does not host this companion. Its automatic adapter reads Zed-owned local
metadata in read-only/query-only mode and writes nothing to Zed's databases or
configuration.

The primary **Set up Cursor monitoring** action installs or repairs the Cursor
companion first and then all five native activity hooks. A hooks-only action is
also available for users who want the recent, non-live fallback without the
companion. The experimental Cursor Agents Window bridge is configured
separately and stays off until explicitly enabled.

The Cursor, Antigravity, Codex, and Claude Code integrations merge VSParallel-owned
handlers into their user configuration. Existing unrelated settings and hooks
are preserved, writes are atomic, and a one-time backup is created before the
first change. Removing an integration removes only its recognized
VSParallel-owned handlers. Cursor preserves
`~/.cursor/hooks.json.vsparallel.bak`, and Antigravity preserves
`~/.gemini/config/hooks.json.vsparallel.bak`, for manual cleanup.

When no `statusLine` is configured, the Claude Code integration also installs a
privacy-minimal fallback usage capture command with a 60-second refresh
interval. An existing custom `statusLine` is unrelated user configuration and
is left unchanged. Lifecycle monitoring and the active Claude usage query can
still work; only VSParallel's managed refresh of the status-line fallback is
disabled.

To use VS Code Insiders or another installation, set
`VSPARALLEL_CODE_COMMAND` to its absolute executable path before launching
VSParallel.

To select a different Cursor installation, set `VSPARALLEL_CURSOR_COMMAND` to
its absolute executable path before launching VSParallel.

To select a different Antigravity IDE installation, set
`VSPARALLEL_ANTIGRAVITY_IDE_COMMAND` to its absolute executable path before
launching VSParallel. Companion heartbeats contain only the trusted `vscode`,
`cursor`, or `antigravity_ide` identifier, never an executable path.

To select a different Zed launcher, set `VSPARALLEL_ZED_COMMAND` to its
absolute executable path. To inspect a non-default Zed data root, set
`VSPARALLEL_ZED_DATA_DIR` to that absolute directory. Neither setting adds Zed
to the companion's closed editor list.

Named companion profiles can be selected with `VSPARALLEL_VSCODE_PROFILE`,
`VSPARALLEL_CURSOR_PROFILE`, and `VSPARALLEL_ANTIGRAVITY_IDE_PROFILE`.

For bundled Codex and Claude executable discovery, VSParallel tries the
configured editor launchers, then reads only the provider entries in the local
registries at `~/.vscode/extensions`, `~/.vscode-insiders/extensions`,
`~/.vscode-oss/extensions`, `~/.cursor/extensions`, and
`~/.antigravity-ide/extensions`.

Codex usage tries `codex` on `PATH` and the executable bundled with the locally
installed Codex extension in VS Code, Cursor, or Antigravity IDE. To select another
signed-in executable, set
`VSPARALLEL_CODEX_COMMAND` to its absolute path before launching VSParallel.

Claude Code usage can use either `claude` on `PATH` or the executable bundled
with the installed Claude extension in VS Code, Cursor, or Antigravity IDE, trying the
other source if the first query fails. To select another signed-in executable,
set
`VSPARALLEL_CLAUDE_COMMAND` to its absolute path before launching VSParallel.

## How it works

```text
VS Code / Cursor IDE / Antigravity IDE companion ─ live workspace heartbeat ──────┐
Zed read-only SQLite + process adapter ─ open/recent + saved agent metadata ───────┤
Cursor native user hooks ───── recent workspace-open/lifecycle metadata ──────────┤
Cursor Desktop Bridge (opt-in) ─ matched thread status, memory only ───────────────┤
Antigravity agent hooks ────── recent product/path lifecycle marker ──────────────┤
Codex hooks ─────────────────── coarse lifecycle marker ──────────────────────────┤─ local state ─┐
Claude Code hooks ───────────── coarse lifecycle marker ──────────────────────────┤               │
Claude Code statusLine ───────── fallback usage cache ────────────────────────────┘               │
Claude CLI control ───────────── live usage percentages/reset times ──────────────────────────────┤─ Rust core ─ Tauri UI
Codex app-server ─────────────── hook trust + live usage percentages/reset times ─────────────────┘             └─ native tray
```

The desktop application uses Tauri 2, a Rust backend, and a framework-free
HTML/CSS/TypeScript UI compiled to static JavaScript. The dependency-free
companion extension writes local workspace heartbeats. Optional provider hooks
write only coarse lifecycle records. The separate Zed adapter reads Zed-owned
SQLite and process metadata without writing a heartbeat or lifecycle record.
Claude and Codex limits are fetched live
through their provider-owned processes and are not written to the state
directory. The Claude Code status-line fallback writes a separate global
`usage/claude.json` record containing only captured percentages, reset times,
and a capture timestamp. The Rust backend validates these records and serves
UI-safe snapshots to the main window; the workspace snapshot also feeds the
native tray menu.

Lifecycle state is normally hook-derived rather than an internal provider
progress feed. When the experimental Cursor Desktop Bridge is enabled, its
coarse thread status can refine an exact hash-matched Cursor hook session; it
does not replace the hook's workspace or agent/model metadata.
Zed's displayed agent timestamp and model identify only the latest persisted
associated thread. Its **Activity detected** and **Turn finished** labels are
derived from saved user/assistant turn boundaries; they do not establish exact
live generation or a success, cancellation, interruption, or failure outcome.

Records are associated with workspaces by local path, and exact native-window
foregrounding remains subject to the companion-backed editor and
operating-system focus behavior. Cursor IDE heartbeats are fully supported;
Cursor's separate Agents Window otherwise remains hook-only. Cursor hooks
identify only the local `workspace_roots` Cursor supplies; they cover local
agent surfaces including the IDE and Agents Window but distinguish neither the
source surface nor a native window. A `workspaceOpen` observation can add a
generic recent workspace but cannot establish liveness, focus, an open target,
or agent activity. The opt-in bridge can mark only an exactly correlated
`running` thread as **Open**; its rows are never focused or openable, and it
cannot prove that the standalone Agents Window rather than another Cursor agent
surface owns the thread.
Antigravity hooks identify their product
surface only after an agent turn and do not expose exact live-window state, so
only a companion heartbeat can establish liveness and focus, and only one with
a verified local target can be opened. Antigravity 2.0's saved-project registry does not identify the
currently open Project and is not used as a presence signal.
Zed **Open** state is conservative: it requires a live GUI process, a matching
current session, and membership in that session's window stack. The adapter
does not report Zed focus, and a database-only observation remains **Recent**.
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
- for Zed, validated local workspace paths and timestamps, current
  session/window correlation, and the latest persisted associated agent ID,
  activity timestamp, structural user/assistant turn boundary, tool-use
  presence, and native model provider/name when available; these are read from
  Zed-owned databases and are not copied to the VSParallel state directory;
- whether the configured Codex and Claude Code extensions are installed and
  active in a VS Code, Cursor, or Antigravity IDE window/profile and, when known,
  whether they run in the local or remote extension host;
- coarse lifecycle state, a one-way hash of the provider session or
  conversation identifier, working directory, and timestamp when optional
  hooks are enabled, plus an optional closed Antigravity model classification
  when recognized; for IDE activity, this is derived from the bounded current
  model enum in the latest per-conversation user-input step, with bounded
  executor metadata and the last-selected-model preference as compatibility
  fallbacks; IDE records may contain an opaque SHA-256 model-signal revision;
- Cursor workspace-open observations containing a deterministic, path-derived
  one-way key, the local workspace path, `workspace_opened`, and a timestamp,
  but no session identity or agent/model label;
- Cursor Agent lifecycle records containing the same privacy-minimal core,
  plus optional bounded `modelName` and `agentKind` labels selected from the
  native hook payload;
- when explicitly enabled, an in-memory SHA-256 thread key and coarse status
  from Cursor's local Desktop Bridge, retained only long enough to match a
  Cursor hook session, plus a bridge-instance-scoped hash of Cursor's numeric window ID
  used only to avoid merging duplicate live observations; raw thread IDs,
  titles, bridge credentials and paths, prompts, and responses are not retained;
- Antigravity hook execution health containing fixed event, surface, and
  outcome values plus timestamp and validated workspace count, but no model; and
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

Set `VSPARALLEL_STATE_DIR` to the same absolute path for VSParallel, its
companion-backed editors, Cursor hooks, Codex, Claude Code, and Antigravity only
when overriding these defaults. Zed is read from its own data root; override
that separately with `VSPARALLEL_ZED_DATA_DIR`.

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

To use a different Cursor executable:

```bash
VSPARALLEL_CURSOR_COMMAND=/absolute/path/to/cursor ./scripts/run-dev.sh
```

To use a different Antigravity IDE executable:

```bash
VSPARALLEL_ANTIGRAVITY_IDE_COMMAND=/absolute/path/to/antigravity-ide ./scripts/run-dev.sh
```

To use a different Zed executable or data root:

```bash
VSPARALLEL_ZED_COMMAND=/absolute/path/to/zed ./scripts/run-dev.sh
VSPARALLEL_ZED_DATA_DIR=/absolute/path/to/zed-data ./scripts/run-dev.sh
```

If you use a named editor profile, select the same profile for companion setup:

```bash
VSPARALLEL_VSCODE_PROFILE=Work ./scripts/run-dev.sh
VSPARALLEL_CURSOR_PROFILE=Agents ./scripts/run-dev.sh
VSPARALLEL_ANTIGRAVITY_IDE_PROFILE=Agents ./scripts/run-dev.sh
```

To use a Codex executable that is not the `codex` found on `PATH`:

```bash
VSPARALLEL_CODEX_COMMAND=/absolute/path/to/codex ./scripts/run-dev.sh
```

To force a Claude executable instead of the `claude` found on `PATH` or the
binary bundled with the Claude extension in VS Code, Cursor, or Antigravity IDE:

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

Open `companion/` in VS Code, Cursor, or Antigravity IDE and start an Extension
Development Host. There is no dependency installation step. The optional
standalone packager is:

```bash
python3 companion/package_vsix.py
```

Production installation uses the deterministic VSIX assembled and tested by
the Rust backend.

## Uninstall and remove local data

1. Open **Setup & diagnostics** in VSParallel.
2. Uninstall each enabled lifecycle, Cursor activity, or Antigravity activity integration.
3. Uninstall each installed VS Code, Cursor, or Antigravity IDE companion.
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
