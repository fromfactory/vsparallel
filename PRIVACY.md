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
files, terminal contents, transcripts, or Git data. Optional lifecycle hooks
receive documented provider event payloads and construct new records containing
only five core fields: a schema version, one-way session/conversation key,
local workspace path, coarse state, and timestamp. Antigravity activity records
may add one optional closed `modelKind` classification derived from documented
`modelName`; the raw model identifier and unrecognized values are immediately
discarded. Antigravity hooks additionally replace a product-specific,
model-free execution-health record containing only fixed event/surface/outcome
values, a timestamp, and the number of workspace records written. The live
Claude response parser and status-line receiver represent only percentage and
reset fields; account, session, behavior-attribution, and other unselected
response fields are discarded and never reach the UI or storage.
Provider stderr and raw failure messages are also discarded. When usage is
unavailable, the UI receives only a fixed source/category explanation such as
could not start, timed out, rejected, or incompatible response.

To show its workspace overview, VSParallel stores the following metadata on the
current device:

- local workspace or `.code-workspace` paths, display names, the closed
  `vscode` or `antigravity_ide` editor value, focus state, and heartbeat
  timestamps reported by the companion;
- whether the configured Codex and Claude Code extensions are installed and
  active in each VS Code or Antigravity IDE window, whether the window is
  remote, and, when known, whether an installed extension runs in the local or
  remote extension host;
- Codex and Claude coarse lifecycle state, a one-way hash of the provider
  session identifier, working directory, and timestamp when optional lifecycle
  hooks are installed;
- Antigravity hook-derived activity records under `antigravity/` or
  `antigravity-ide/`, containing the five core fields above and, when
  recognized, an optional closed `modelKind`, with one record per documented
  local `workspacePaths` entry;
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

The bundled companion gives VS Code and Antigravity IDE the same exact-window
tracking. A heartbeat's editor field is a closed value and cannot inject an
executable path; opening uses the corresponding command configured locally in
VSParallel. Antigravity 2.0 does not host this companion. Its documented global
hooks at `~/.gemini/config/hooks.json` are shared with Antigravity IDE and the
Antigravity CLI. Their documented transcript roots identify the product
surface, but do not expose a native window identity, liveness, or focus, and no
event fires merely because a Project was opened or selected. VSParallel reduces
the transcript path—or documented artifact-directory fallback—to a bounded
product label and immediately discards it. CLI, conflicting, and unrecognized
surfaces are ignored; supported hook-only paths remain recent and non-openable
rather than claiming a live window. A Project-level `.agents/hooks.json` can
take precedence over the global hook.

The Antigravity adapter admits documented `conversationId`, `workspacePaths`,
`transcriptPath`, `artifactDirectoryPath`, and `modelName` values, plus the
minimum `error`, `terminationReason`, and `fullyIdle` fields needed to select a
coarse state for applicable events. It reduces a recognized model to one of the
closed `modelKind` values documented in the
[metadata protocol](docs/protocol.md), and retains neither the raw model value,
product path, nor those error/reason values. The stored `sessionKey` is a
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
bundled with the corresponding locally installed VS Code or Antigravity IDE
extension. To locate a bundled executable, it first asks the configured
`VSPARALLEL_CODE_COMMAND` and `VSPARALLEL_ANTIGRAVITY_IDE_COMMAND` launchers.
As a bounded fallback, it reads only the exact provider entries in
`~/.vscode/extensions/extensions.json`,
`~/.vscode-insiders/extensions/extensions.json`,
`~/.vscode-oss/extensions/extensions.json`, and
`~/.antigravity-ide/extensions/extensions.json`. A resolved path may be cached
in process memory but is not persisted. Explicit executables selected with
`VSPARALLEL_CODEX_COMMAND` or `VSPARALLEL_CLAUDE_COMMAND` are used literally.

The location of this state directory is shown in Setup diagnostics. Heartbeats
older than 60 seconds are hidden and lifecycle state older than 24 hours is
shown as unknown. VSParallel parses at most the newest 4,096 eligible record
bodies in each record directory and reports omissions in diagnostics. Directory
enumeration still considers every entry when selecting those records, and old
records are not deleted automatically.

Before VSParallel changes Antigravity, Codex, or Claude Code hook configuration,
or installs its owned Claude Code status line, it creates a private, one-time
backup of the entire original configuration file. That backup can therefore
contain unrelated settings, custom status-line commands, environment values, or
secrets that were already present in the provider configuration. On Unix,
VSParallel creates these files with owner-only permissions. Antigravity
installation merges one top-level entry named `vsparallel`; removal deletes
only an entry it recognizes as its own. Integration removal preserves every
backup so that it cannot destroy user data.

To erase VSParallel data, first uninstall its integrations in the application,
quit VSParallel and the supported editors, then delete the state directory
shown in Setup diagnostics and, if no longer needed, these backup files:

- `$CODEX_HOME/hooks.json.vsparallel.bak` (normally
  `~/.codex/hooks.json.vsparallel.bak`)
- `$CLAUDE_CONFIG_DIR/settings.json.vsparallel.bak` (normally
  `~/.claude/settings.json.vsparallel.bak`)
- `~/.gemini/config/hooks.json.vsparallel.bak`

Deleting those files is optional and should be done only after confirming the
original provider configuration is working as expected.
