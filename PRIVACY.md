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

To show its workspace overview, VSParallel stores the following metadata on the
current device:

- local workspace or `.code-workspace` paths, display names, the closed
  `vscode`, `cursor`, or `antigravity_ide` editor value, focus state, and
  heartbeat timestamps reported by the companion;
- whether the configured Codex and Claude Code extensions are installed and
  active in each VS Code, Cursor, or Antigravity IDE window, whether the window
  is remote, and, when known, whether an installed extension runs in the local
  or remote extension host;
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
companion and remains hook-only. Antigravity 2.0 does not host this companion.

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
Agents Window, or Cursor CLI. VSParallel does not scan Cursor processes or
native-window internals; live state comes only from companion heartbeats. For
the four lifecycle events, it admits a bounded
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

The primary **Set up Cursor monitoring** action installs or repairs both the
Cursor IDE companion and these native hooks. The separate hooks-only action
changes only `~/.cursor/hooks.json` and provides no live-window monitoring.

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
bodies in each record directory and reports omissions in diagnostics. Directory
enumeration still considers every entry when selecting those records, and old
records are not deleted automatically.

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
backup so that it cannot destroy user data.

To erase VSParallel data, first uninstall its integrations in the application,
quit VSParallel and the supported editors, then delete the state directory
shown in Setup diagnostics and, if no longer needed, these backup files:

- `$CODEX_HOME/hooks.json.vsparallel.bak` (normally
  `~/.codex/hooks.json.vsparallel.bak`)
- `$CLAUDE_CONFIG_DIR/settings.json.vsparallel.bak` (normally
  `~/.claude/settings.json.vsparallel.bak`)
- `~/.cursor/hooks.json.vsparallel.bak`
- `~/.gemini/config/hooks.json.vsparallel.bak`

Deleting those files is optional and should be done only after confirming the
original provider configuration is working as expected.
