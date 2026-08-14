# VSParallel privacy and local data

VSParallel contains no account system, telemetry, analytics, or advertising.
Workspace monitoring and provider usage handling operate locally. When Setup
status is checked, VSParallel starts the installed Codex `app-server` to read
whether its exact user-level handlers are enabled and trusted. Every 60 seconds,
and when the user explicitly refreshes the app, VSParallel also asks installed
Codex and Claude processes for current rate-limit percentages and reset times.
Each provider subprocess owns authentication and any provider connection.
VSParallel does not receive, read, or store either credential, and it does not
persist either live usage response. A recent, unexpired response may be retained
in application memory for up to 15 minutes and is visibly marked stale if a
later refresh fails.

Claude usage is requested through an evolving CLI/SDK control interface, not a
documented stable standalone command. Claude Code's current full-usage getter
also computes attribution summaries from the session-history directory it is
given. VSParallel prevents access to real history by launching this subprocess
with a new empty, owner-private configuration directory and no session
persistence. Claude's original secure-storage location remains provider-owned
so the subprocess can authenticate without VSParallel reading a credential.
The temporary directory is removed when the query ends.

VSParallel does not extract, log, store, or transmit prompts, responses, source
files, terminal contents, transcripts, or Git data. Optional hook integrations
receive documented provider event payloads and construct new records containing
only five core fields: a schema version, one-way key, local workspace path,
coarse state, and timestamp. Cursor `workspaceOpen` records derive their key
deterministically from the normalized path and contain no session identity or
agent/model metadata. Cursor lifecycle records may also include bounded
`modelName` and `agentKind` strings selected from Cursor's native user-hook
payload. Values that are empty or exceed 128 bytes are
omitted; model labels must use a narrow ASCII-safe character set and agent
labels must be one of four closed values. Antigravity activity records
may add one optional closed `modelKind` classification from hook `modelName` or
the bounded current-model enum in the IDE conversation's latest user-input
step, with bounded execution metadata and the last-selected-model preference as
compatibility fallbacks; raw model identifiers and unrecognized values are
immediately discarded. An IDE activity record may also contain an opaque
SHA-256 model-signal revision so a new turn can be distinguished from the
preceding execution. Antigravity hooks additionally
replace a product-specific, model-free execution-health record containing only
fixed event/surface/outcome values, a timestamp, and the number of validated
workspace associations. The live Claude response parser and status-line receiver
represent only percentage and reset fields; account, session,
behavior-attribution, and other unselected response fields are discarded and
never reach the UI or storage.
Provider stderr and raw failure messages are also discarded. When usage is
unavailable, the UI receives only a fixed source/category explanation such as
could not start, timed out, rejected, or incompatible response.

Zed monitoring is automatic and separate from those companions and hooks.
VSParallel opens Zed-owned SQLite databases in logical read-only/query-only
mode and correlates their persisted workspace metadata with a live Zed GUI
process and the current session/window stack. Within a small aggregate refresh
budget, it may parse size-bounded native thread blobs attached to displayed
workspaces. Only the last message's structural variant, whether that assistant
boundary contains a tool use, and safely joinable model provider/name may
survive into the snapshot. The blob and all other parsed data are then
discarded. Thread titles are not selected, and prompts, responses, tool
payloads, source code, and other thread data are not retained, logged, returned
to the UI, or copied into VSParallel's state directory.

To show its workspace overview, VSParallel uses the following metadata on the
current device. Zed-derived values remain snapshot-only; other items are
persisted only where stated:

- local workspace or `.code-workspace` paths, display names, the closed
  `vscode`, `cursor`, or `antigravity_ide` editor value, focus state, and
  heartbeat timestamps reported by the companion;
- whether the configured Codex and Claude Code extensions are installed and
  active in each VS Code, Cursor, or Antigravity IDE window, whether the window
  is remote, and, when known, whether an installed extension runs in the local
  or remote extension host;
- Zed's validated local workspace paths and timestamps, channel, current
  session/window-stack correlation, and the latest safely associated persisted
  agent identifier, activity timestamp, and native model provider/name when
  available; VSParallel does not persist these Zed-derived values;
- Codex and Claude coarse lifecycle state, a one-way hash of the provider
  session identifier, working directory, and timestamp when optional lifecycle
  hooks are installed;
- Antigravity hook-derived activity records under `antigravity/` or
  `antigravity-ide/`, containing the five core fields above and, when
  recognized, an optional closed `modelKind` and opaque `ideModelRevision`,
  with one record per documented local `workspacePaths` entry;
- Cursor native-hook activity records under `cursor/`, containing the five
  core fields and optional bounded `modelName` and `agentKind`, with records
  created only for usable local paths in the `workspace_roots` values Cursor
  supplies;
- Cursor `workspaceOpen` observations under the same directory, containing a
  deterministic path-derived one-way key, one normalized local workspace path,
  fixed state `workspace_opened`, and a timestamp, but no conversation/session
  identity or agent/model label;
- the local on/off preference for experimental Cursor Agents Window monitoring;
- local display preferences for VS Code, Cursor, Antigravity, and Zed workspace
  rows and for usage-limit percentages, stored in the state root with an
  equivalent webview-local fallback cache; every display is enabled by default,
  and these preferences contain no workspace or provider-account data;
- app-owned integration suppression markers written after verified per-source
  uninstall, and for every installable integration source requested by
  **Uninstall all**, so a
  still-running editor or provider process cannot make removed-source records
  visible before it is reloaded; reinstalling that source removes its marker;
- for IDE hook activity only, metadata for the latest `USER_INPUT` row in the
  local Antigravity IDE conversation database selected directly from the
  validated hook `conversationId`, or hash-matched in memory during a desktop
  refresh; VSParallel opens its at-most-1-MiB `step_payload` through SQLite's
  incremental BLOB API and reads bounded protobuf tags, lengths, and scalar
  varints encountered while locating fixed current-model enum path
  `19 → 12 → 1 → 15 → 1`; it immediately discards unrelated scalar values,
  uses only the queued flag and model enum, and seeks over all unrelated
  length-delimited bodies without reading or copying their bytes;
- as compatibility fallbacks, the latest at-most-64-KiB `executor_metadata`
  row's fixed model-name field and one bounded `ItemTable` last-selected-model
  preference from the local editor `state.vscdb`; none of these queries reads
  generation metadata, response bodies, trajectory data, transcripts, OAuth, or
  user-status data. SQLite is opened in logical read-only/query-only mode; for
  WAL databases, SQLite itself may maintain its normal reader-coordination
  sidecar;
- Antigravity hook execution health under `antigravity-hook-health/`, containing
  only schema version, fixed event/surface/outcome values, timestamp, and
  workspace count—never a model name or classification; and
- the Claude Code five-hour and weekly percentages used, optional reset times,
  and a local capture timestamp in `usage/claude.json` only when managed
  status-line fallback capture is available.

The on-disk Claude fallback record is global rather than associated with a
workspace or session. It contains no account identifier, session identifier,
working directory, prompt, response, transcript path, cost, source data, or
credential. VSParallel derives the percentage remaining in memory and does not
write that derived value back to disk. A record is presented as stale after 15
minutes. Windows that have passed their reset times are omitted. The active
Claude response is never written to this record or elsewhere in the state
directory.

Remote-placement metadata is boolean only. VSParallel never stores a remote
name, authority, hostname, address, or account identity, and the desktop app
does not connect to the remote host to read provider state. Lifecycle hooks and
live usage queries remain local to the machine on which they run.

The bundled companion gives VS Code, Cursor IDE, and Antigravity IDE live
window tracking. A heartbeat's editor field is a closed value and cannot inject
an executable path; opening uses the corresponding command configured locally
in VSParallel. Cursor's separate Agents Window does not activate the third-party
companion. Its experimental Desktop Bridge integration is a separate opt-in and
does not provide focus or an open target. Antigravity 2.0 does not host this
companion. Zed also does not host this companion and is never added to the
heartbeat protocol's closed `vscode`, `cursor`, and `antigravity_ide` editor
list.

Display preferences affect presentation, not collection or provider
configuration. Disabling an editor hides that editor's rows in both the main
workspace list and native tray; disabling usage percentages hides the global
usage display. All are enabled by default, and changing them does not install,
uninstall, or disable an integration.

Zed data is discovered under `$XDG_DATA_HOME/zed` (or
`~/.local/share/zed`) and the community Flatpak root
`~/.var/app/dev.zed.Zed/data/zed` on Linux,
`~/Library/Application Support/Zed` on macOS, and `%LOCALAPPDATA%\Zed` on
Windows. `VSPARALLEL_ZED_DATA_DIR` can select a
different absolute data root. The adapter considers the stable, preview,
nightly, and development channel databases at
`db/0-{stable,preview,nightly,dev}/db.sqlite` when present. It never creates,
updates, checkpoints, repairs, or deletes these databases or Zed configuration.

For workspace discovery, the adapter selects only `paths`, `paths_order`,
`timestamp`, `session_id`, and `window_id` from Zed's `workspaces` table and
the `session_id` and `session_window_stack` values from `kv_store`. Every path
must pass the same bounded local-path validation used before display or
opening. A Stable workspace is classified **Open** only when a live Zed GUI
process exists, its stored session matches Zed's current session, and its window ID is
present in the current session's window stack. All other usable observations
are **Recent**. Preview, Nightly, and Dev observations always fail closed to
**Recent** because the portable process probe cannot safely identify their
release channel. This is a conservative liveness correlation, not foreground or
keyboard focus: Zed rows are always non-focused, and **Open** does not mean an
agent is running.
Zed rows with a remote connection are omitted rather than exposing or guessing
their connection context.

For native agent metadata, the adapter selects the bounded association fields
`session_id`, `agent_id`, `updated_at`, folder/main workspace paths, `archived`,
and `interacted_at` from `sidebar_threads`; it never selects the thread title.
When the newest eligible persisted native thread can be associated with one
discovered workspace by its exact local paths, VSParallel may join its session
to Zed's separate `threads/threads.db` and inspect bounded `updated_at`,
`data_type`, and `data` values.
The selective thread-data parser keeps only the last message variant, the
presence of a tool-use boundary, and the native model provider/name when safely
joinable and parsable. It ignores message and tool contents. A saved user
boundary (or a newer `interacted_at` racing the blob write) can produce coarse
**Activity detected** while that workspace is open; a later saved assistant
boundary without a tool use can produce **Turn finished**. These signals can
lag live generation, and Zed does not persist enough information here to
distinguish success, cancellation, interruption, or failure. Unknown,
malformed, or unsupported structures remain **Recent agent activity**. The
bounded input buffer and all other parsed values are discarded after snapshot
construction.

A validated Zed target is opened only through the locally configured `zed`
launcher (or `VSPARALLEL_ZED_COMMAND`). Current **Open** workspaces use
`--existing`; **Recent** workspaces use `--new`, and multi-root workspaces pass
the complete validated, ordered path vector. VSParallel does not
derive an executable path or command from either database. Zed monitoring
creates no companion heartbeat, hook record, configuration backup, or other
Zed-specific file in the VSParallel state directory.

Cursor's native `workspaceOpen`, `sessionStart`, `beforeSubmitPrompt`, `stop`,
and `sessionEnd` user hooks in `~/.cursor/hooks.json` can execute for local
Cursor surfaces, including the Cursor IDE and Agents Window. `workspaceOpen`
admits only valid local paths from `workspace_roots`; it requires no
conversation/session identity, ignores all other fields, and stores one
path-derived `workspace_opened` observation per normalized root. `sessionStart`
may supply the bounded composer/background fields used for the closed agent
label, but its record is metadata-only and does not itself appear as activity.
These hooks do not identify a native window or prove liveness, focus, or an open
target, and they do not identify whether an unmatched event came from the IDE,
Agents Window, or Cursor CLI. For the four lifecycle events, it admits a bounded
conversation/session identity only long enough to hash it, usable local paths
from `workspace_roots`, the minimum fields required to choose a coarse state,
and optional bounded model/agent labels. The raw identity is discarded.
Pathless events are omitted. Prompts, responses, email fields, transcripts,
token data, error text, and all other unselected payload fields are discarded.
An unmatched workspace-open observation appears in **Recent** as a generic
non-live, non-focused, non-openable Cursor workspace row with no activity card.
An unmatched lifecycle record appears as a generic recent **Cursor Agent** row
with the same non-live limitations. Exactly one matching Cursor IDE companion
heartbeat owns either observation instead; multiple matching windows leave it
generic rather than guessing an owner.

The single **Cursor** integration action installs or repairs both the Cursor
IDE companion and these native hooks. Its per-editor uninstall removes both
components together after verifying their removal.

Experimental Cursor Agents Window monitoring is off by default and depends on
Cursor's limited, server-controlled `desktop_bridge` rollout. Only when Cursor
shows **Settings > Beta > Desktop Bridge > Allow CLI to access desktop agents**
can the user enable that option, restart Cursor, and then enable the separate
option in VSParallel. If Cursor hides that section, VSParallel cannot activate
the bridge and continues to offer only recent, hook-derived fallback status.
VSParallel does not edit Cursor's internal feature-gate storage. When available,
VSParallel reads Cursor's private local Desktop
Bridge discovery files and sends only `listThreads` over local inter-process
communication. It never invokes a send-message operation.

This experimental preference is separate from the core Cursor integration: it
is excluded from setup and per-editor Cursor uninstall. **Uninstall all** turns
it off in addition to attempting to remove the normal integrations.

The discovery response includes more than VSParallel keeps. Each raw thread ID
is immediately SHA-256 hashed and discarded. Thread titles, bridge tokens,
socket paths, Cursor user-data paths, raw thread IDs, prompt text, and response
text are not logged, persisted, or exposed to the UI. The bridge-derived thread
hash, a bridge-instance-scoped hash of Cursor's numeric window ID, coarse status, source
category, and update time remain in process memory only. VSParallel displays a
bridge observation only when its thread hash exactly
matches a Cursor hook `sessionKey`, which supplies the local workspace and any
bounded model or agent label. An unmatched bridge thread is discarded rather
than guessed onto a workspace.

On Unix-like systems, VSParallel requires the bridge discovery directory,
files, and socket to be owned by the current user and rejects links or public
permissions. On Windows it rejects reparse points, validates the named-pipe
shape and live Cursor process, and relies on the user's profile and named-pipe
access controls for account isolation.

Only a matched `running` status is treated as **Open**. Matched `completed`,
`error`, and `idle` observations remain **Recent** with coarse status. These
rows are never marked focused and cannot be opened or activated by VSParallel.
Cursor's bridge is private and undocumented, and it does not prove that a
thread belongs to the standalone Agents Window rather than another Cursor agent
surface. A missing discovery file is also ambiguous: Cursor may be closed, the
Cursor setting or private feature may be unavailable, or the bridge may not
have started. VSParallel reports the absence without choosing one explanation.

The exact, case-sensitive environment value `CURSOR_CODE_REMOTE=true`
suppresses all Cursor hook-record persistence because this release has no
remote-host bridge. The adapter still returns `{}` and exits successfully, so
it remains fail-open. Cursor user hooks are not available to cloud agents, so
cloud-only activity is not represented.

Antigravity 2.0's documented global
hooks at `~/.gemini/config/hooks.json` are shared with Antigravity IDE and the
Antigravity CLI. Their documented transcript roots identify the product
surface, but do not expose a native window identity, liveness, or focus, and no
event fires merely because a Project was opened or selected. VSParallel reduces
the transcript path—or documented artifact-directory fallback—to a bounded
product label and immediately discards it. CLI, conflicting, and unrecognized
surfaces are ignored; supported hook-only paths remain recent and non-openable
rather than claiming a live window. A workspace-level `.agents/hooks.json` can
take precedence over the global hook.

The single **Antigravity** integration action manages the Antigravity IDE
companion and these shared activity hooks together. Its per-editor uninstall
removes both components after verifying their removal.

The Antigravity adapter admits documented `conversationId`, `workspacePaths`,
`transcriptPath`, and `artifactDirectoryPath` values, optional `modelName` when
supplied, plus the minimum `error`, `terminationReason`, and `fullyIdle` fields
needed to select a coarse state for `Stop`. It reduces a recognized
model to one of the closed `modelKind` values documented in the
[metadata protocol](docs/protocol.md), and retains neither the raw model value,
product path, nor those error/reason values. When an IDE hook omits `modelName`,
VSParallel opens only that validated conversation's local IDE database in
logical read-only/query-only mode and selects metadata for its newest
`USER_INPUT` step. It rejects an absent, non-BLOB, queued, or larger-than-1-MiB
newest row rather than scanning backward. For a usable row, SQLite's incremental
BLOB API and a streaming protobuf parser read only structural varints, the
queued flag, and the current model enum at `19 → 12 → 1 → 15 → 1`. It
immediately discards unrelated scalar values; all unrelated length-delimited
bodies—including the user input and context—are crossed with seeks, so their
bytes are never read or copied. The full step payload is never materialized.
This row is committed before `PreInvocation`, allowing the model shown with
**Activity detected** to change immediately.

If the current-turn table or row is absent, VSParallel may parse only the fixed
model-name field in the latest bounded `executor_metadata` blob or, as a final
compatibility fallback, the fixed
`antigravityUnifiedStateSync.modelPreferences` key in the local editor
`state.vscdb`. A decoded unknown current model clears the qualifier. The desktop
snapshot repeats the narrow per-conversation read; raw conversation identifiers
are hashed in memory while matching database filenames and are never retained.
It does not adopt a differing current-turn revision until the lifecycle hook
has correlated that row with `PreInvocation`.
Only an opaque SHA-256 revision derived from the selected bounded model signal
may be stored in the activity record. These queries do not inspect generation
metadata, response bodies, trajectory data, transcripts, OAuth state, or user
status. `PostToolUse` events do not write the
activity record, so they cannot replace a terminal state; terminal failure and
interruption classification comes from `Stop`. All other hook fields are
discarded. The stored `sessionKey` is a
SHA-256 hash of `conversationId`; the filename additionally hashes the
normalized workspace path so a multi-folder project gets one independent
record per path. Its execution-health record does not contain either
identifier, any workspace path, or model information.

Graphical Claude sessions in VS Code-compatible editors do not run the terminal
`statusLine`, so the active query supplies usage independently of those sessions
and lifecycle hooks. VSParallel installs its local fallback command only when `statusLine` is
absent. If a custom command exists, it is preserved exactly; only the fallback
cache's managed refresh is disabled, and an existing record may remain visible
as stale.

For Codex and Claude, VSParallel can try the executable on `PATH` and the binary
bundled with the corresponding locally installed VS Code, Cursor, or
Antigravity IDE extension. To locate a bundled executable, it first asks the configured
`VSPARALLEL_CODE_COMMAND`, `VSPARALLEL_CURSOR_COMMAND`, and
`VSPARALLEL_ANTIGRAVITY_IDE_COMMAND` launchers.
As a bounded fallback, it reads only the exact provider entries in
`~/.vscode/extensions/extensions.json`,
`~/.vscode-insiders/extensions/extensions.json`,
`~/.vscode-oss/extensions/extensions.json`,
`~/.cursor/extensions/extensions.json`, and
`~/.antigravity-ide/extensions/extensions.json`. A resolved path may be cached
in process memory but is not persisted. Explicit executables selected with
`VSPARALLEL_CODEX_COMMAND` or `VSPARALLEL_CLAUDE_COMMAND` are used literally.

The location of this state directory is shown in Setup diagnostics. Heartbeats
older than 60 seconds are hidden and lifecycle state older than 24 hours is
shown as unknown. VSParallel parses at most the newest 4,096 eligible record
bodies in each record directory and reports omissions in diagnostics. During
normal retention, directory enumeration still considers every entry when
selecting those records and old records are not deleted automatically. An
integration uninstall is different: VSParallel purges that source's app-owned
records and suppresses it until reinstall. **Uninstall all** applies this local
cleanup to every installable integration source even when an unavailable
external editor CLI prevents physical removal.

Before VSParallel changes Cursor, Antigravity, Codex, or Claude Code hook
configuration, or installs its owned Claude Code status line, it creates a private, one-time
backup of the entire original configuration file. That backup can therefore
contain unrelated settings, custom status-line commands, environment values, or
secrets that were already present in the provider configuration. On Unix,
VSParallel creates these files with owner-only permissions. Antigravity
installation merges one top-level entry named `vsparallel`; removal deletes
only an entry it recognizes as its own. Cursor installation merges one exact
VSParallel-owned handler into each of the native `workspaceOpen`,
`sessionStart`, `beforeSubmitPrompt`, `stop`, and `sessionEnd` arrays; removal
recognizes only exact current handlers or strict, safely parsed historical
VSParallel handlers, including the prior four-handler set, leaving modified
lookalikes untouched. Setup reports that prior set as needing an update or
repair so reinstalling can add `workspaceOpen`. A Cursor configuration that is
not strict UTF-8 JSON is left unchanged. Integration removal preserves every
unrelated setting and backup so that it cannot destroy user data. A normal
per-integration uninstall purges and suppresses its source after external
removal is verified. **Uninstall all** is also a global stop-tracking control
for installable integration sources: it suppresses and purges each one even if
external removal cannot be verified. An unavailable CLI or failed removal is
reported instead of being presented as complete physical removal; an unavailable
optional editor is a warning, while an attempted removal failure remains an
error. This action does not disable automatic read-only Zed discovery or live
provider usage checks.

To erase VSParallel data, first use **Uninstall all** in the application, quit
VSParallel and the supported editors, then delete the state directory shown in
Setup diagnostics and, if no longer needed, these backup files:

- `$CODEX_HOME/hooks.json.vsparallel.bak` (normally
  `~/.codex/hooks.json.vsparallel.bak`)
- `$CLAUDE_CONFIG_DIR/settings.json.vsparallel.bak` (normally
  `~/.claude/settings.json.vsparallel.bak`)
- `~/.cursor/hooks.json.vsparallel.bak`
- `~/.gemini/config/hooks.json.vsparallel.bak`

Deleting those files is optional and should be done only after confirming the
original provider configuration is working as expected. VSParallel never
deletes or modifies Zed's own data during integration removal or VSParallel
cleanup; manage that data through Zed if it must also be erased.
