# Claude Code lifecycle and usage integration

This optional integration adds coarse Claude Code activity and a local usage
fallback to VSParallel. Install, repair, or remove it from **Setup &
diagnostics** in the desktop app. Production users should not edit Claude
settings by hand.

## Activity

VSParallel recognizes these Claude Code events:

- `UserPromptSubmit` → `activity_detected`
- `Stop` → `turn_finished`
- `StopFailure` → `failed_or_interrupted`
- `SessionEnd` → `session_ended`

A user interruption does not always emit `StopFailure`. If no terminal event
arrives, the saved activity eventually appears as **Unknown** instead of being
reported as a failure.

Each local record contains five fields: schema version, SHA-256 session key,
working directory, coarse state, and timestamp. Prompt text, responses, source,
transcripts, tool data, and raw session IDs are discarded.

## Usage

While at least one usage card is visible, VSParallel periodically asks a
signed-in local Claude executable for five-hour and seven-day usage. The query
also runs when you select **Refresh**. This CLI/SDK control interface can change
between Claude versions, so VSParallel treats incompatible responses as
unavailable.

Claude's full-usage getter can inspect session history when run normally.
VSParallel starts it with a new empty, private configuration directory and
disables session persistence. It keeps only percentages and reset times in
memory, and removes the temporary directory after the query.

If Claude's top-level `statusLine` setting is unused, setup also installs a
60-second fallback capture. It writes only percentages, reset times, and a
capture timestamp to `usage/claude.json`. An existing custom status line is
left unchanged; only this fallback is then unavailable. Graphical Claude
sessions do not run a terminal status line, so the active query remains the
primary source.

Setup preserves unrelated handlers and settings and keeps a private one-time
backup of the original settings file. Because it is a complete copy, the backup
may include unrelated secrets already present in that file. See
[Privacy](../../PRIVACY.md) for storage and deletion details.

[`settings.example.json`](settings.example.json) is a reference example, not an
installer. Its placeholder executable path is intentionally not runnable.
