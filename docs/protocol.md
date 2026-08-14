# VSParallel local metadata protocol

VSParallel uses a deliberately small, versioned file protocol. Writers replace
records atomically; readers treat every file as untrusted and skip it if it is
missing, oversized, malformed, from another schema version, or contains an
invalid required timestamp. Unusable optional paths are ignored for display,
association, and opening without discarding an otherwise useful heartbeat.

The shared root is selected in this order:

1. Absolute `VSPARALLEL_STATE_DIR`
2. Windows: `%LOCALAPPDATA%\VSParallel`
3. macOS: `~/Library/Application Support/VSParallel`
4. Linux/other Unix: `$XDG_STATE_HOME/vsparallel`, otherwise
   `~/.local/state/vsparallel`

## Workspace heartbeat (schema version 1)

Stored as `instances/<safe-instance-id>.json`:

```json
{
  "schemaVersion": 1,
  "instanceId": "51adf7cb-d0ee-42a2-8d5d-dc8ef93d74f8",
  "editor": "cursor",
  "workspaceName": "example-workspace",
  "workspaceFolders": [
    {
      "name": "example-workspace",
      "index": 0,
      "path": "/work/example-workspace"
    }
  ],
  "workspaceFile": null,
  "primaryPath": "/work/example-workspace",
  "openTarget": "/work/example-workspace",
  "focused": true,
  "active": true,
  "remoteWindow": false,
  "agentExtensions": {
    "codex": {
      "available": true,
      "installed": true,
      "active": true,
      "remote": false
    },
    "claude": {
      "available": true,
      "installed": true,
      "active": false,
      "remote": false
    }
  },
  "lastSeenAtMs": 1785800000000,
  "startedAtMs": 1785799000000
}
```

`editor`, added in companion version 0.4.0 and extended in companion version
0.4.1, is the closed value `vscode`, `cursor`, or `antigravity_ide`. It selects
a locally configured launcher but never contains an executable or command
path. Older heartbeats without `editor` remain valid and use the historical VS
Code behavior. `antigravity_2`, `zed`, and a separate Cursor Agents Window
value are not accepted from a companion heartbeat. Neither Zed nor Cursor's
separate Agents Window activates the third-party companion. Zed uses the native
read-only adapter below. Cursor Agents Window is represented through the
hook-record protocol below, optionally refined in memory by the experimental
Cursor Desktop Bridge observation described below.

`active` is the VS Code-compatible host's recent-interaction hint; it is not
used as liveness. VSParallel derives liveness only from `lastSeenAtMs`.

`agentExtensions` is additive to schema version 1 and is always included by
companion version 0.2.0 and later. Each provider entry has these status fields:

- `available`: the public VS Code-compatible extension lookup succeeded
- `installed`: the extension was found for this editor window/profile; this
  does not by itself identify which extension host runs it
- `active`: the editor host reports that the extension has activated
- `remote`: added in companion 0.3.0; `true` means the installed extension runs
  in the remote/workspace extension host, `false` means the local/UI extension
  host, and `null` means placement is not known or not applicable

When lookup is unavailable or fails, the three booleans are false and `remote`
is `null`. A false `available` value distinguishes that case from a successful
lookup that confirmed the extension is absent. `remoteWindow`, also added in
companion 0.3.0, says only whether the editor host reports a remote window. It
never contains the remote name, authority, or host identity.
Older heartbeats without `agentExtensions`, and 0.2.x heartbeats without the
placement fields, are still valid. Installation and activation are not agent
lifecycle signals: in particular, `active: true` does not mean that Codex or
Claude is processing a turn.

Only local `file` paths are serialized. Remote or virtual workspace URIs and
their authorities are deliberately omitted; such windows remain listable by
their editor-provided display names but are not openable by VSParallel. The
desktop app, lifecycle hooks, and live usage subprocesses are machine-local;
this release has no bridge for reading provider state from a remote host.

Opening resolves both the target and editor from the validated heartbeat, not
from UI-supplied path or command data. `vscode` selects
`VSPARALLEL_CODE_COMMAND`; `cursor` selects `VSPARALLEL_CURSOR_COMMAND`;
`antigravity_ide` selects
`VSPARALLEL_ANTIGRAVITY_IDE_COMMAND`. An active heartbeat asks that editor to
prefer an existing exact-target window. A retained but inactive heartbeat uses
`--new-window` for the exact target. The target must still be an existing local
absolute path. Hook-only Cursor workspace, Cursor Agent, Antigravity 2.0, and
Antigravity IDE rows never produce an open target. Experimental bridge-refined
Cursor Agent rows are also non-openable.

Zed targets never come from this heartbeat protocol. The native adapter
validates a local path directly from Zed's read-only workspace database, and
the opener selects `VSPARALLEL_ZED_COMMAND` or the local `zed` launcher. An
**Open** Zed target adds `--existing`; a **Recent** Zed target adds `--new`.
For a multi-root workspace, every validated path is passed in Zed's saved
order. Only Stable-channel observations are openable because the portable
process probe cannot safely select a Preview, Nightly, or Dev launcher.
No executable or command value is accepted from Zed's databases or from the
UI.

## Zed native observation (read-only; memory only)

Zed does not host the VSParallel companion and does not expose a VSParallel
hook-record protocol. Instead, each workspace refresh invokes a native adapter
that reads bounded metadata from Zed-owned SQLite databases and correlates it
with local process state. The adapter writes no Zed heartbeat or activity
record and does not copy its observations into the shared VSParallel state
root.

Unless `VSPARALLEL_ZED_DATA_DIR` selects another absolute directory, the Zed
data root is:

| Platform | Zed data root |
| --- | --- |
| Linux/Unix | `$XDG_DATA_HOME/zed`, otherwise `~/.local/share/zed`; the community Flatpak root at `~/.var/app/dev.zed.Zed/data/zed` is also considered |
| macOS | `~/Library/Application Support/Zed` |
| Windows | `%LOCALAPPDATA%\Zed` |

Within that root, the adapter independently considers the stable, preview,
nightly, and development channel databases at
`db/0-{stable,preview,nightly,dev}/db.sqlite`. Missing channels are normal. A
candidate database is opened in logical read-only/query-only mode; schema
mismatches, missing tables or columns, invalid values, links, excessive input,
and query failures omit that candidate without changing it.

Workspace discovery admits only these Zed-owned fields:

- `workspaces.paths`, `paths_order`, `timestamp`, `session_id`, and `window_id`;
- `kv_store` values for the exact keys `session_id` and
  `session_window_stack`.

Rows with `remote_connection_id` are omitted; the adapter does not reconstruct
or activate Zed remote-connection context.

Rows, serialized path collections, strings, and path counts are bounded. Only
validated local absolute paths can reach the snapshot or opener; other values
are discarded. VSParallel classifies a Zed workspace as **Open** only when all
three independent conditions hold:

1. a live local Zed GUI process is present;
2. the workspace's persisted `session_id` equals the current Zed session; and
3. its `window_id` appears in that current session's window stack.

A usable workspace observation that does not satisfy every condition remains
**Recent**. Process presence or a database row alone is insufficient for
**Open**. The generic process signal is used only for Stable; Preview, Nightly,
and Dev observations fail closed to **Recent**. The adapter has no
foreground-window or interaction signal, so every Zed row has `focused: false`;
the **Open** classification must not be
interpreted as focus.

For optional native agent metadata, the adapter selects only bounded
`session_id`, `agent_id`, `updated_at`, folder/main workspace paths, `archived`,
and `interacted_at` values from `sidebar_threads`. It does not select the title
column. The newest eligible persisted thread is associated only when its
validated local paths exactly identify a discovered workspace.
Its timestamp remains latest saved activity evidence rather than an exact live
generation signal.

When a safely associated thread has a usable ID/session pair, the adapter may
join it to Zed's `threads/threads.db` and read bounded `updated_at`, `data_type`,
and `data` values. Only a supported, size-capped thread blob is parsed. The
selective parser retains the last message's structural variant, whether a final
assistant boundary contains a tool use, and the safely joinable native model
provider/name; it ignores message and tool contents.

For the native Zed Agent (`agent_id IS NULL`), a saved User or Resume boundary
can produce coarse `activity_detected` while the workspace is Open. A newer
`interacted_at` than the joined thread save is treated the same way to cover
asynchronous persistence. An Agent boundary containing a tool use remains
coarse activity because another model step may follow. A newly saved Agent
boundary without a tool use can produce `turn_finished`. A terminal manual
Compaction boundary is not considered activity.
These are persisted turn boundaries that may lag or skip a fast live turn;
they do not reveal whether the turn succeeded, was cancelled, was interrupted,
or failed. External-agent sidebar entries continue to use their own timestamp
as `recent_activity`. Unknown, malformed, unsupported, or ambiguously closed
structures also fail closed to `recent_activity`.

The blob buffer and all other parsed data are discarded immediately. Thread
titles are not selected; prompts, responses, source code, tool payloads, and
transcript content are never logged, persisted, or returned to the UI. A
displayed model therefore means “model in the latest persisted associated Zed
thread,” not “model currently generating.”

## Hook observations and lifecycle records (schema version 1)

Codex records are stored as `codex/<sha256-of-session-id>.json`; Claude records
use the same five-core-field structure at
`claude/<sha256-of-session-id>.json`. Antigravity 2.0 records are stored under
`antigravity/` and Antigravity IDE records under `antigravity-ide/`, each using
`<sha256-of-conversation-id-NUL-normalized-workspace-path>.json`:

```json
{
  "schemaVersion": 1,
  "sessionKey": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "cwd": "/work/example-workspace",
  "state": "activity_detected",
  "changedAtMs": 1785800000000
}
```

Cursor native hook records use the same five core fields under `cursor/`.
Only events containing a valid local path in Cursor's `workspace_roots` can
produce a record; pathless events are omitted rather than creating an
unassociated row. For `workspaceOpen`, `sessionKey` and the filename are the
same deterministic, domain-separated SHA-256 key derived from the normalized
workspace path. This key carries no session or conversation identity. Each
valid root produces an independent `workspace_opened` record. Such records
contain no `modelName` or `agentKind`.

Cursor records may add optional camel-case `modelName` and `agentKind` string
fields selected from the native hook payload. `modelName` is trimmed, must be
nonempty and at most 128 bytes, must be ASCII, and may
contain only alphanumerics, spaces, and `- _ . / : + ( ) ,`; `agentKind` must
be exactly `Background agent`, `Agent`, `Ask`, or `Edit` and is otherwise
omitted. These labels are
bounded metadata supplied by Cursor, not a claim that a particular model is
currently inferring. Prompt and response bodies, email fields, transcript
material, token data, and every other unselected payload field are discarded
before the record is written.

Antigravity records may add an optional `modelKind` field when a hook supplies
`modelName` or Antigravity IDE exposes a recognized current-model enum in its
latest bounded per-conversation user-input step. Bounded execution metadata and
the selected-model preference are compatibility fallbacks, for example
`"modelKind": "gemini_3_6_flash_medium"`.

Antigravity IDE records may also add `ideModelRevision`, an opaque lowercase
SHA-256 revision derived from the selected bounded model signal. It is used
only to tell a new turn or execution row from the previous one and is never
presented to the UI.

The allowed values are `automatic`, `gemini`,
`gemini_3_6_flash_medium`, `gemini_3_6_flash_high`,
`gemini_3_5_flash`, `gemini_3_1_pro_high`, `gemini_3_1_pro_low`,
`gemini_3_flash`, `claude`, `claude_sonnet_4_6_thinking`,
`claude_opus_4_6_thinking`, `gpt_oss`, `gpt_oss_120b`, and
`gpt_oss_120b_medium`. Specific recognized names use their specific token;
other recognized Gemini, Claude, or GPT-OSS family names use the corresponding
family token. `auto` is stored as `automatic` and displayed honestly as **Auto
model** rather than guessing the routed model. Unrecognized, invalid, and
oversized model names are omitted. The raw `modelName` is never stored.

These optional fields are additive, backward-compatible parts of schema version
1. Codex and Claude records, Cursor records without bounded metadata, older
Antigravity records, and Antigravity events without IDE model metadata continue
to contain only the five core fields.

Allowed on-disk states across the four adapters are `workspace_opened`,
`session_started`, `activity_detected`, `turn_finished`, `session_ended`,
`failed_or_interrupted`, `failed`, and `interrupted`. Each adapter writes only
its subset shown below. The snapshot ignores both Cursor-only non-activity
states when deriving an activity card: `workspace_opened` may create recent
workspace evidence, while `session_started` remains metadata-only. It
normalizes the last three failure states to **Failed/interrupted**, and derives
`unknown` when no usable recent record can be associated with a workspace;
`unknown` is not written. The main UI labels an initial absence **No activity
yet** and reserves **Unknown** for a previous lifecycle signal that has become
stale.

The adapters map only documented hook events:

| Provider | Event | Recorded state |
| --- | --- | --- |
| Cursor | `workspaceOpen` | `workspace_opened` (recent workspace evidence only; not displayed as activity) |
| Cursor | `sessionStart` | `session_started` (metadata only; not displayed as activity) |
| Cursor | `beforeSubmitPrompt` | `activity_detected` |
| Cursor | `stop` with completed status | `turn_finished` |
| Cursor | `stop` with aborted status | `interrupted` |
| Cursor | `stop` with error status | `failed` |
| Cursor | `sessionEnd` with `completed`, `window_close`, or `user_close` outcome | `session_ended` |
| Cursor | `sessionEnd` with aborted outcome | `interrupted` |
| Cursor | `sessionEnd` with error outcome | `failed` |
| Codex | `UserPromptSubmit` | `activity_detected` |
| Codex | `Stop` | `turn_finished` |
| Codex | `SessionEnd` | `session_ended` |
| Claude | `UserPromptSubmit` | `activity_detected` |
| Claude | `Stop` | `turn_finished` |
| Claude | `StopFailure` | `failed_or_interrupted` |
| Claude | `SessionEnd` | `session_ended` |
| Antigravity | `PreInvocation` | `activity_detected` |
| Antigravity | `PostToolUse` | no activity-record write |
| Antigravity | `Stop` with an error or error termination | `failed` |
| Antigravity | Otherwise `Stop`, interrupted/cancelled | `interrupted` |
| Antigravity | Otherwise `Stop`, `fullyIdle: false` | `activity_detected` |
| Antigravity | Otherwise `Stop` | `turn_finished` |

The Cursor adapter admits only `conversation_id`, `session_id`,
`workspace_roots`, `model_id`, `model`, bounded `model_params`, `status`,
`reason`, `composer_mode`, and `is_background_agent`. Hook input is capped at
1 MiB and all other fields are streamed past without representation.
`workspaceOpen` admits only `workspace_roots`; it ignores every other field and
requires no session or conversation identity. For the four lifecycle events,
raw conversation/session identity is required, capped at 16 KiB, hashed to the
stored `sessionKey`, and then discarded. Prompt and stop events prefer
`conversation_id`; `sessionStart` and `sessionEnd` prefer `session_id`; each
can fall back to the other identifier. Their filename hashes the raw identity
together with the normalized workspace path so one multi-root event produces
independent records without exposing the identity.

At most 64 unique `workspace_roots` are considered. Each must be a local
absolute path no longer than 32 KiB; URI-like, UNC-style non-local, and
lexically escaping paths are rejected. Existing paths are canonicalized and
other absolute paths are lexically normalized. An event with no valid path is
omitted. The hook always fails open and emits no record for malformed,
oversized, pathless, or unsupported/missing terminal-status events. Lifecycle
events with missing or oversized identity are also omitted; `workspaceOpen`
does not require one. When the hook subprocess receives the exact,
case-sensitive environment value `CURSOR_CODE_REMOTE=true`, the adapter
suppresses all Cursor hook-record persistence because this release has no
remote-host bridge. It still writes `{}` to standard output and exits
successfully, preserving fail-open behavior.

For `modelName`, the adapter selects `model_id` with `model` as a fallback,
omits the non-specific `default` and `unknown` sentinels, admits only a trimmed
ASCII-safe token up to 128 bytes, and may append only
closed thinking/context/effort parameters while keeping the entire result
within 128 bytes. `agentKind` is one of **Background agent**, **Agent**,
**Ask**, or **Edit**, derived from the bounded background and composer-mode
fields supplied by the session lifecycle hooks. Invalid metadata is omitted;
the session-scoped agent kind survives later events for the same hashed session,
and terminal events preserve the current turn's earlier valid model label when
their payload omits it. A new prompt without a concrete model clears the prior
turn's model label.

Cursor's user-level hooks run across local Cursor agent surfaces, including its
VS Code-based IDE and separate Agents Window. The payload does not provide a
trustworthy source-surface or native-window identity, liveness, focus, or exact
open target, so an unmatched local event cannot be attributed specifically to
the IDE, Agents Window, or Cursor CLI. A hook path covered by exactly one
retained Cursor companion heartbeat is associated with that IDE workspace and
the generic duplicate is suppressed. A fresh (at most 24 hours old) unmatched
`workspace_opened` record is synthesized in **Recent** as one generic
**Cursor** workspace row. It is non-live, non-focused, non-openable, has
`recentlyActive: false`, and has no Cursor activity view or agent/model
information. A fresh unmatched displayable lifecycle record is instead
synthesized as a generic recent **Cursor Agent** row that is also non-live,
non-focused, and non-openable; `session_started` by itself remains completely
hidden. Zero or multiple matching heartbeats retain the appropriate generic
row rather than guessing ownership.
Lifecycle records are first reduced to the newest marker per hashed session;
any remaining fresh active session takes priority over another session's
terminal marker.
Stale and pathless hook records do not produce hook-only rows. No Cursor process
or native-window scraping is performed. User-level hooks do not cover cloud
agents, and remote hook executions are suppressed as described above.

### Cursor Desktop Bridge observation (experimental; memory only)

This integration is disabled by default and has no public or stable Cursor
protocol guarantee. Cursor renders **Settings > Beta > Desktop Bridge > Allow
CLI to access desktop agents** only for installations included in its limited,
server-controlled `desktop_bridge` rollout. If present, the user can enable it,
restart Cursor, and then enable the separate experimental option in VSParallel.
If absent, VSParallel cannot activate or bypass the rollout and remains on the
recent, hook-only fallback. Bridge discovery absence is not treated as proof
that Cursor or its Agents Window is closed: the private feature may be gated,
the visible user setting may be disabled, or the bridge may not have started.

When enabled, VSParallel reads Cursor's private local Desktop Bridge discovery
files and uses the discovered local IPC endpoint to send this read-only request:

```json
{"type":"listThreads"}
```

VSParallel does not call the bridge's send-message operation. Discovery tokens,
socket paths, Cursor user-data paths, app details, and process details are used
only transiently to validate and reach the local endpoint; they are not copied
into the metadata protocol, logs, diagnostics, or UI. Response size, field
shape, identifiers, timestamps, source, and status are bounded and validated.

For each accepted response item, VSParallel immediately computes
`sha256(thread.id)` and discards the raw ID and title. The hash, closed source,
coarse status (`running`, `completed`, `error`, `idle`, or unknown), update
time, and a bridge-instance-scoped hash of Cursor's numeric window ID exist in process
memory only. The last value prevents distinct observations from being merged
and is never persisted or exposed. A thread is eligible for display only when
that hash exactly equals an existing Cursor hook record's `sessionKey`; the
hook record supplies the normalized workspace and any bounded agent/model
metadata. There is deliberately no path or time heuristic, and unmatched
bridge threads are not shown.

Bridge matching does not override a Cursor IDE workspace already covered by a
companion heartbeat. For an otherwise unmatched hook workspace, a matched
`running` thread produces a non-focused, non-openable experimental **Cursor
agent thread** row in **Open**. Matched `completed`, `error`, and `idle` observations
remain in **Recent** with a coarse lifecycle label. The bridge does not expose
a trustworthy native-window identity or exact surface discriminator, so this
row indicates a correlated Cursor agent thread, not proof that the standalone
Agents Window rather than another Cursor agent surface owns it.

### Cursor hook installation

The primary **Set up Cursor monitoring** action installs or repairs the Cursor
companion and these native hooks together. A separate hooks-only action manages
the same hook configuration without installing the companion and therefore
provides only recent, non-live fallback observations.

VSParallel manages one fail-open command handler in each native flat hook array
`workspaceOpen`, `sessionStart`, `beforeSubmitPrompt`, `stop`, and `sessionEnd`
in `~/.cursor/hooks.json`. The `workspaceOpen` handler invokes the absolute
VSParallel executable as `cursor-hook workspace-open`; `sessionStart` uses
`cursor-hook session-start`, and the other handlers use the corresponding
kebab-case event. Cursor's hook timeout is set to two seconds.
Existing unrelated top-level configuration and handlers are preserved.
The managed file must parse as strict UTF-8 JSON; if it contains comments,
trailing commas, or otherwise invalid JSON, VSParallel leaves it unchanged and
reports that the integration is unavailable. As with Cursor itself, avoid
editing `hooks.json` concurrently with an install, repair, or uninstall action.

Before the first change, VSParallel makes the one-time exact-byte backup
`~/.cursor/hooks.json.vsparallel.bak`. Installation and removal recognize only
an exact current handler or a strict historical VSParallel handler with no
unexpected keys and a safely parsed absolute executable/event command.
Modified lookalikes remain user-owned. Malformed managed hook arrays, an
unsupported top-level version, oversized configuration, or link/reparse-point
configuration aborts without writing. Updates are atomic; uninstall removes
only recognized VSParallel-owned handlers and retains the backup. Existing
records under `cursor/` are not deleted automatically. Status and repair
require all five current handlers, so an older four-handler installation is
reported as update- or repair-needed. Reinstall adds the managed
`workspaceOpen` handler while preserving unrelated hooks. Uninstall recognizes
and removes both the current set and strictly recognized legacy VSParallel
handlers.

### Codex, Claude, and Antigravity lifecycle reduction

Codex and Claude hash the provider session ID and do not retain the raw session
or turn ID. The Antigravity adapter selects documented `conversationId`,
`workspacePaths`, `transcriptPath`, and `artifactDirectoryPath` fields, plus the
optional `modelName` when supplied, plus `error`, `terminationReason`, and
`fullyIdle` for `Stop`. It reduces the transcript path, or
the artifact directory as a fallback, to the bounded surface `antigravity` or
`antigravity-ide` and immediately discards both paths; CLI, conflicting, and
unrecognized surfaces are not recorded. It independently reduces a recognized
model name to the closed `modelKind` token above and immediately discards the
raw value. Because the IDE hook contract omits `modelName`, each IDE
`PreInvocation` and `Stop` first open that conversation's local IDE database
read-only and select metadata for the newest `steps` row whose `step_type` is
`USER_INPUT`. The newest row is rejected rather than bypassed when its
`step_payload` is absent, non-BLOB, queued, malformed, or larger than 1 MiB.
For a usable row, SQLite's incremental BLOB API follows protobuf field path
`19 → 12 → 1 → 15 → 1` to the current model enum. The streaming scanner reads
and immediately discards bounded tags, lengths, and scalar varints encountered
while locating the path; it uses only the queued flag and model enum and seeks
over all unrelated length-delimited bodies without reading or copying prompt or
context bytes. It never materializes the full step and deliberately ignores the
prior config in field 13. Because Antigravity commits this row before
`PreInvocation`, the closed model classification is available when **Activity
detected** begins.

When the current-turn table or row is absent, the adapter may select the latest
`executor_metadata.data` value by `idx` as a compatibility fallback. An
unusable newest user-input row instead preserves the last correlated model and
is never bypassed for an older row. The executor blob is capped at 64 KiB; only
protobuf field path `10 → 1 → 28` (the bounded model name) is selected, reduced
to a closed token, and immediately discarded. At `PreInvocation`, the fixed
`antigravityUnifiedStateSync.modelPreferences` key in the local editor
`state.vscdb` is a final bounded compatibility fallback. A successfully decoded
unknown current model clears the qualifier rather than borrowing an older
classification. The adapter does not read generation metadata, response bodies,
trajectory blobs, transcripts, OAuth, or user-status data.

Snapshot generation repeats the same narrow per-conversation read: it hashes
conversation database filename stems in memory, matches them to the stored
`sessionKey`, and exposes only the resulting closed classification. The
hook-time read instead uses the validated documented `conversationId` directly
to select that conversation's database. The raw filename stem, conversation ID,
executor blob, step payload, and model name or enum are not retained. An opaque
revision of the selected model signal is stored with the closed model. A
snapshot does not adopt a differing current-turn revision until `PreInvocation`
has correlated it with the lifecycle record; matching revisions preserve the
hook's model, so refresh cannot revert **Activity detected** to the preceding
execution's model.
`PostToolUse` events do not write an activity record and therefore cannot race a
newer or terminal `Stop`; terminal error/interruption classification comes from
`Stop`. A delayed event timestamp cannot replace a newer record.
Events without a trustworthy model stay generic Antigravity activity. The
adapter hashes `conversationId`, writes one record for each distinct valid local
path in `workspacePaths`, and discards all other payload fields.
Its `sessionKey` is the conversation hash; including the normalized path in the
filename prevents one multi-folder project path from replacing another.

Antigravity 2.0, Antigravity IDE, and the Antigravity CLI share the documented
global hook configuration at `~/.gemini/config/hooks.json`. Their documented
transcript roots distinguish the product surface, but no hook identifies a
native window, liveness, focus, or an exact open target. A supported path
without a matching companion heartbeat is therefore presented as recent and
non-openable under its identified Antigravity product. These events start in
the agent execution loop: opening or selecting an Antigravity 2.0 Project alone
does not produce a record. A workspace `.agents/hooks.json` can take precedence
over the global configuration. Any model label is the latest model reported on
the lifecycle record currently selected for display; it is not proof that the
model is actively inferring at that moment.

### Antigravity hook installation

VSParallel manages one top-level entry named `vsparallel` in the documented
global `~/.gemini/config/hooks.json`. It installs fail-open command handlers for
`PreInvocation`, `PostToolUse` with matcher `*`, and `Stop`; a recording failure
never blocks or changes the Antigravity action. Existing unrelated top-level
entries are preserved. If an unrelated entry already owns the name
`vsparallel`, installation reports a conflict and leaves the file unchanged.

Before its first change, VSParallel makes the one-time full-file backup
`~/.gemini/config/hooks.json.vsparallel.bak`. Uninstall removes only a
recognized VSParallel-owned entry and deliberately retains the backup. After
uninstalling the integration and confirming the remaining hook configuration,
the user may delete that backup manually. Existing files under the shared
state root's `antigravity/`, `antigravity-ide/`, and
`antigravity-hook-health/` directories
are likewise not deleted automatically.

### Antigravity hook execution health

Because a valid installation proves only that the global JSON was written,
each invocation also atomically replaces one product-specific record in
`antigravity-hook-health/`. Antigravity 2.0 uses
`antigravity-hook-health/antigravity-2.json`; Antigravity IDE uses
`antigravity-hook-health/antigravity-ide.json`. Both have the same shape:

```json
{
  "schemaVersion": 1,
  "event": "pre-invocation",
  "surface": "antigravity_2",
  "outcome": "recorded",
  "observedAtMs": 1785800000000,
  "workspaceCount": 1
}
```

The fixed outcomes are `recorded`, `invalid_payload`, `unsupported_surface`,
`missing_conversation`, `no_workspace`, and `persist_failed`. Health records
never contain a conversation identifier, workspace path, transcript or
artifact path, model name or `modelKind`, error text, prompt, or response.
`workspaceCount` is the number of validated workspace associations observed;
for a state-neutral `PostToolUse`, it does not imply an activity-file write.
Setup reads both supported surface records, and diagnostics reports them
separately. This distinguishes **awaiting agent turn** from a hook that executed
but could not produce an activity row, including for IDE-only use. If the
shared state root itself is unavailable, the hook remains fail-open and cannot
write either the activity or health record.

## Claude Code status-line fallback record (schema version 1)

The optional on-disk Claude Code usage cache is provider-global rather than
workspace- or session-specific. It is a fallback for terminal Claude sessions
and older CLI versions when the live query described below is unavailable.
When the Claude integration owns the user's `statusLine` setting, Claude Code
runs VSParallel's local capture command every 60 seconds and supplies its
documented status-line JSON on standard input. VSParallel extracts only the
five-hour and weekly percentage/reset fields and atomically replaces
`usage/claude.json`:

```json
{
  "schemaVersion": 1,
  "capturedAtMs": 1785800000000,
  "fiveHour": {
    "usedPercent": 23.5,
    "resetsAtMs": 1785803600000
  },
  "sevenDay": {
    "usedPercent": 41.2,
    "resetsAtMs": 1786404800000
  }
}
```

`fiveHour` and `sevenDay` are independently optional because Claude Code can
omit either window. `resetsAtMs` is also optional. At least one valid window is
required before the record is replaced. `usedPercent` is bounded to 0–100; the
UI derives `remainingPercent` as `100 - usedPercent` and uses the lowest
remaining value as the provider's compact summary.

Records older than 15 minutes are presented as stale. Individual windows that
have passed their reset times are omitted; when no unexpired window remains,
the fallback cannot supply usage until a fresh value is captured. Missing,
oversized, malformed, more than five minutes future-dated, wrong-version,
symlinked, or otherwise unusable records are ignored. If the user already has
a custom Claude Code
`statusLine`, VSParallel does not replace or wrap it, so this file is not
refreshed. The live query remains independent and can still supply current
Claude usage.

## Live Claude Code usage (not persisted)

Every 60 seconds, and on an explicit refresh, VSParallel starts the installed
Claude executable and asks the signed-in account for its five-hour and
seven-day usage through the CLI/SDK control channel. This usage getter is an
evolving Claude CLI compatibility interface, not a documented stable standalone
command.

Claude Code's current full-usage getter also attempts to compute behavior,
agent, skill, plugin, and MCP attribution from its configured recent session
history. VSParallel launches the subprocess with `CLAUDE_CONFIG_DIR` pointing
to a new empty, owner-private temporary directory and disables session
persistence, so that calculation cannot enumerate the user's real transcripts.
The provider's original secure-storage root is passed separately so Claude can
use its own existing sign-in without VSParallel reading a credential. The
temporary directory is removed after the query.

For each valid window, VSParallel keeps only the percentage used, the derived
percentage remaining, and the optional reset time. The compact card uses the
lowest remaining percentage so it cannot overstate available capacity. The
Claude subprocess owns authentication and any provider connection. A narrow
response type admits only `rate_limits`; account, session, behavior-attribution,
and other fields are discarded. VSParallel never reads Claude credentials or
writes the live response to the state directory. It may retain a recent,
unexpired last-known value in memory for up to 15 minutes and marks it as stale.

If the executable is absent, signed out, times out, exposes an incompatible
control interface, or returns no valid windows, VSParallel falls back to the
managed status-line record above when that record is usable. Native graphical
Claude sessions in VS Code-compatible editors do not run `statusLine`; the
active query is what makes usage available for those sessions.
`VSPARALLEL_CLAUDE_COMMAND` can
select a different executable; otherwise VSParallel can use either `claude`
from `PATH` or the executable bundled with the installed Claude extension in VS
Code, Cursor, or Antigravity IDE, trying the other source if the first query fails. It
locates the bundled source through either configured editor launcher or, when
the launchers are unavailable, a bounded read of the local extension registries
described below.

## Codex hook review status (not persisted)

When Setup status is checked, VSParallel starts the configured Codex
`app-server` and calls `hooks/list` from `CODEX_HOME` to isolate the user hook
layer. It matches only the three exact VSParallel handlers and reads their
`enabled` and `trustStatus` fields. The response is used only to distinguish
**Installed · trusted** from **Installed · review required** and is not
persisted. This confirms the user-layer decision; workspace configuration can
still disable hooks. If the installed Codex version cannot provide the status,
the hooks remain **Installed** with neutral guidance to check `/hooks`;
VSParallel never approves a handler.

## Live Codex usage (not persisted)

Codex usage does not have an on-disk record. Every 60 seconds, and on an
explicit refresh, VSParallel starts the installed Codex executable's
`app-server` and requests the signed-in account's documented rate-limit view.
For each valid primary or secondary window, the UI-safe response contains the
percentage used, the percentage remaining, the provider-defined window
duration, and the optional reset time. The compact card uses the lowest
remaining percentage so it cannot overstate available capacity.

The Codex subprocess owns authentication and any provider connection.
VSParallel never reads Codex credentials or writes the response to the state
directory. If the executable is not installed, does not support `app-server`,
is signed out, times out, or returns no valid windows, Codex usage is reported
as unavailable. The UI may retain a recent, unexpired last-known value in memory
for up to 15 minutes and marks it as stale; it is never written to disk.
`VSPARALLEL_CODEX_COMMAND` can select a different executable; otherwise
VSParallel tries `codex` from `PATH` and the executable bundled with a locally
installed Codex extension in VS Code, Cursor, or Antigravity IDE. An explicit
`VSPARALLEL_CODEX_COMMAND` is used literally and does not enable
bundled-extension fallback.

For bundled-provider discovery, VSParallel first invokes the configured
`VSPARALLEL_CODE_COMMAND`, `VSPARALLEL_CURSOR_COMMAND`, and
`VSPARALLEL_ANTIGRAVITY_IDE_COMMAND` launchers with the supported
extension-location argument. If that fails, it reads only
the exact provider entries in these local registry files:

- `~/.vscode/extensions/extensions.json`
- `~/.vscode-insiders/extensions/extensions.json`
- `~/.vscode-oss/extensions/extensions.json`
- `~/.cursor/extensions/extensions.json`
- `~/.antigravity-ide/extensions/extensions.json`

The resulting extension path may be cached in process memory but is not
persisted.

## Privacy invariant

No record may contain prompt text, assistant output, source text, terminal
content, Git diffs, tool inputs/output, transcript paths/content, credentials,
or machine identifiers. The companion records only the public extension status
and placement fields shown above; it never reads extension exports or private
state. The hook adapters receive richer documented payloads but
create new objects with the five core fields and discard the input before
writing. Antigravity may add only the optional closed `modelKind` described
above; its IDE reconciliation incrementally reads only the structural bytes and
fixed current-model enum from the bounded latest user-input step, seeking over
prompt and context bodies, with the bounded executor field as a fallback. It
never writes the raw conversation filename, model identifier, or step payload.
Only the closed `modelKind` and an opaque SHA-256 model-signal revision may
survive that reduction. The
Cursor adapter may add only the optional bounded `modelName` and `agentKind`
strings described above for lifecycle records. A `workspaceOpen` record admits
only normalized local workspace roots and a timestamp, uses a path-derived
one-way key, and contains no agent/model metadata. The adapter discards prompts,
responses, email fields, transcripts, token data, and other unselected
native-hook fields, and does not write a record when Cursor supplies no usable
local workspace root. The experimental Cursor Desktop Bridge writes no thread
record: it hashes each bounded raw thread ID immediately, discards the raw ID
and title, and retains only the hash and validated coarse fields in process
memory for exact hook-session matching. Discovery tokens, socket paths, Cursor
user-data paths, prompt text, and response text never enter the metadata
protocol. The Zed adapter creates no record at all. Its workspace, process, and
agent correlation exists only in the current snapshot; its optional bounded
thread-blob parser exposes only the last structural message variant, tool-use
presence, and model provider/name, then immediately discards the input buffer.
It does not retain or expose thread titles, prompts, responses, source, tool
payloads, or other thread fields. The Claude status-line adapter
likewise creates the minimal usage record shown above and does not represent or
persist the accompanying session ID, working directory, model, cost,
repository data, or transcript path. Live Codex and Claude usage remains in
memory only, and VSParallel never reads or stores provider credentials. Of the
usage responses, only the minimal Claude status-line fallback record is
persisted; live usage responses remain memory-only.
Automated Rust and JavaScript tests assert these boundaries.
