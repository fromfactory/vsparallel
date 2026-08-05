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
receive documented provider event payloads, extract only the event name,
session ID, and working directory, and immediately discard all unselected
fields. The live Claude response parser and status-line receiver represent only
percentage and reset fields; account, session, behavior-attribution, and other
unselected response fields are discarded and never reach the UI or storage.

To show its workspace overview, VSParallel stores the following metadata on the
current device:

- local workspace or `.code-workspace` paths, display names, focus state, and
  heartbeat timestamps reported by the VS Code companion;
- whether the configured Codex and Claude Code VS Code extensions are installed
  and active in each window;
- coarse lifecycle state, a one-way hash of the provider session identifier,
  working directory, and timestamp when optional lifecycle hooks are installed;
  and
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

Graphical Claude sessions in VS Code do not run the terminal `statusLine`, so
the active query supplies usage independently of those sessions and lifecycle
hooks. VSParallel installs its local fallback command only when `statusLine` is
absent. If a custom command exists, it is preserved exactly; only the fallback
cache's managed refresh is disabled, and an existing record may remain visible
as stale.

VSParallel runs `codex` from `PATH` by default. For Claude it can use either
`claude` from `PATH` or the executable bundled with the installed Claude VS Code
extension, trying the other source if the first query fails. To locate the
bundled executable, it asks the configured VS Code launcher or reads the bounded
local VS Code extension registry for the exact Anthropic extension path; that
path may be cached in process memory but is not persisted. Explicit executables
can be selected with `VSPARALLEL_CODEX_COMMAND` and
`VSPARALLEL_CLAUDE_COMMAND`.

The location of this state directory is shown in Setup diagnostics. Heartbeats
older than 60 seconds are hidden and lifecycle state older than 24 hours is
shown as unknown. VSParallel parses at most the newest 4,096 eligible record
bodies in each record directory and reports omissions in diagnostics. Directory
enumeration still considers every entry when selecting those records, and old
records are not deleted automatically.

Before VSParallel changes Codex or Claude Code hook configuration, or installs
its owned Claude Code status line, it creates a private, one-time backup of the
entire original configuration file. That backup can therefore contain unrelated
settings, custom status-line commands, environment values, or secrets that were
already present in the provider configuration. On Unix, VSParallel creates
these files with owner-only permissions. Integration removal preserves the
backup so that it cannot destroy user data.

To erase VSParallel data, first uninstall its integrations in the application,
quit VSParallel and VS Code, then delete the state directory shown in Setup
diagnostics and, if no longer needed, these backup files:

- `$CODEX_HOME/hooks.json.vsparallel.bak` (normally
  `~/.codex/hooks.json.vsparallel.bak`)
- `$CLAUDE_CONFIG_DIR/settings.json.vsparallel.bak` (normally
  `~/.claude/settings.json.vsparallel.bak`)

Deleting those files is optional and should be done only after confirming the
original provider configuration is working as expected.
