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
  "agentExtensions": {
    "codex": {
      "available": true,
      "installed": true,
      "active": true
    },
    "claude": {
      "available": true,
      "installed": true,
      "active": false
    }
  },
  "lastSeenAtMs": 1785800000000,
  "startedAtMs": 1785799000000
}
```

`active` is VS Code's recent-interaction hint; it is not used as liveness.
VSParallel derives liveness only from `lastSeenAtMs`.

`agentExtensions` is additive to schema version 1 and is always included by
companion version 0.2.0 and later. Each provider entry has exactly three
booleans:

- `available`: the public VS Code extension lookup succeeded
- `installed`: the extension was found in this VS Code extension host
- `active`: VS Code reports that the extension has activated

When lookup is unavailable or fails, all three values are false. A false
`available` value distinguishes that case from a successful lookup that
confirmed the extension is absent.
Older heartbeats without `agentExtensions` are still valid. Installation and
activation are not agent lifecycle signals: in particular, `active: true` does
not mean that Codex or Claude is processing a turn.

Only local `file` paths are serialized. Remote or virtual workspace URIs and
their authorities are deliberately omitted; such windows remain listable by
their VS Code-provided display names but are not openable by VSParallel.

## Lifecycle record (schema version 1)

Codex records are stored as `codex/<sha256-of-session-id>.json`; Claude records
use the same five-field structure at
`claude/<sha256-of-session-id>.json`:

```json
{
  "schemaVersion": 1,
  "sessionKey": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "cwd": "/work/example-workspace",
  "state": "activity_detected",
  "changedAtMs": 1785800000000
}
```

Allowed on-disk states are `activity_detected`, `turn_finished`,
`session_ended`, and `failed_or_interrupted`. The Claude adapter writes
`failed_or_interrupted` for its documented `StopFailure` event. Codex does not
currently expose a corresponding documented hook, so its adapter produces the
other three states only. The snapshot derives `unknown` when no usable recent
record can be associated with a workspace; `unknown` is not written as a
lifecycle state. The main UI labels an initial absence **No activity yet** and
reserves **Unknown** for a previous lifecycle signal that has become stale.

Both adapters map only documented lifecycle events:

| Provider | Event | Recorded state |
| --- | --- | --- |
| Codex | `UserPromptSubmit` | `activity_detected` |
| Codex | `Stop` | `turn_finished` |
| Codex | `SessionEnd` | `session_ended` |
| Claude | `UserPromptSubmit` | `activity_detected` |
| Claude | `Stop` | `turn_finished` |
| Claude | `StopFailure` | `failed_or_interrupted` |
| Claude | `SessionEnd` | `session_ended` |

Each adapter hashes the provider session ID and does not retain the raw session
ID or turn ID.

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
Claude sessions in VS Code do not run `statusLine`; the active query is what
makes usage available for those sessions. `VSPARALLEL_CLAUDE_COMMAND` can
select a different executable; otherwise VSParallel can use either `claude`
from `PATH` or the executable bundled with the installed Claude VS Code
extension, trying the other source if the first query fails. It locates the
bundled source through the VS Code launcher or, when the launcher is
unavailable, a bounded read of the local extension registry for the exact
Anthropic extension entry.

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
VSParallel runs `codex` from `PATH`.

## Privacy invariant

No record may contain prompt text, assistant output, source text, terminal
content, Git diffs, tool inputs/output, transcript paths/content, credentials,
or machine identifiers. The companion records only the public extension
presence booleans shown above; it never reads extension exports or private
state. The lifecycle adapters receive richer documented hook payloads but
create new five-field objects and discard the input before writing. The Claude
status-line adapter likewise creates the minimal usage record shown above and
does not represent or persist the accompanying session ID, working directory,
model, cost, repository data, or transcript path. Live Codex and Claude usage
remains in memory only, and VSParallel never reads or stores provider
credentials. Only the minimal Claude status-line fallback record is persisted.
Automated Rust and JavaScript tests assert these boundaries.
