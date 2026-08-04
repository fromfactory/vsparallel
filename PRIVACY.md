# VSParallel privacy and local data

VSParallel operates locally. The application contains no account system,
telemetry, analytics, advertising, or application-initiated network requests.
It does not extract, log, store, or transmit prompts, responses, source files,
terminal contents, transcripts, or Git data. Optional lifecycle hooks receive
documented provider event payloads, extract only the event name, session ID, and
working directory, and immediately discard all unselected fields.

To show its workspace overview, VSParallel stores the following metadata on the
current device:

- local workspace or `.code-workspace` paths, display names, focus state, and
  heartbeat timestamps reported by the VS Code companion;
- whether the configured Codex and Claude Code VS Code extensions are installed
  and active in each window; and
- coarse lifecycle state, a one-way hash of the provider session identifier,
  working directory, and timestamp when optional lifecycle hooks are installed.

The location of this state directory is shown in Setup diagnostics. Heartbeats
older than 60 seconds are hidden and lifecycle state older than 24 hours is
shown as unknown. VSParallel parses at most the newest 4,096 eligible record
bodies in each record directory and reports omissions in diagnostics. Directory
enumeration still considers every entry when selecting those records, and old
records are not deleted automatically.

Before VSParallel changes Codex or Claude Code hook configuration, it creates a
private, one-time backup of the entire original configuration file. That backup
can therefore contain unrelated settings, environment values, or secrets that
were already present in the provider configuration. On Unix, VSParallel creates
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
