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
other three states only. The UI derives `Unknown` when no usable recent record
can be associated with a workspace; `unknown` is not written as a lifecycle
state.

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

## Privacy invariant

No record may contain prompt text, assistant output, source text, terminal
content, Git diffs, tool inputs/output, transcript paths/content, credentials,
or machine identifiers. The companion records only the public extension
presence booleans shown above; it never reads extension exports or private
state. The lifecycle adapters receive richer documented hook payloads but
create new five-field objects and discard the input before writing. Automated
Rust and JavaScript tests assert these boundaries.
