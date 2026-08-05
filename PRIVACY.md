# VSParallel privacy and local data

VSParallel contains no account system, telemetry, analytics, or advertising.
Workspace monitoring and Claude Code usage capture operate locally. When Setup
status is checked, VSParallel starts the installed Codex `app-server` to read
whether its exact user-level handlers are enabled and trusted. Every 60 seconds,
and when the user explicitly refreshes the app, it also requests current rate-limit
percentages and reset times. The Codex subprocess may contact the Codex service
using its own existing sign-in. VSParallel does not receive, read, or store that
credential, and it does not persist either app-server response. A recent,
unexpired usage response may be retained in application memory for up to 15
minutes and is visibly marked stale if a later refresh fails.

VSParallel does not extract, log, store, or transmit prompts, responses, source
files, terminal contents, transcripts, or Git data. Optional lifecycle hooks
receive documented provider event payloads, extract only the event name,
session ID, and working directory, and immediately discard all unselected
fields. The Claude Code status-line receiver represents only the percentage and
reset fields in memory and discards all other status-line input.

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
  and a local capture timestamp in `usage/claude.json` when managed status-line
  capture is available.

The Claude usage record is global rather than associated with a workspace or
session. It contains no account identifier, session identifier, working
directory, prompt, response, transcript path, cost, source data, or credential.
VSParallel derives the percentage remaining in memory and does not write that
derived value back to disk. A record is presented as stale after 15 minutes.
Windows that have passed their reset times are omitted; when none remain, the
Claude card is unavailable until Claude Code supplies a fresh value.

Claude Code has a single `statusLine` setting. VSParallel installs its local
capture command only when that setting is absent, and runs it at a 60-second
interval. If a custom command is already configured, VSParallel preserves it
exactly and Claude usage capture remains unavailable. The lifecycle hooks are
independent and may still be installed.

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
