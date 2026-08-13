# VSParallel Companion

This local-only VS Code-compatible extension reports a small, versioned
heartbeat for the current VS Code, Cursor, or Antigravity IDE window to the
VSParallel desktop app. It reports workspace names, local folder paths, an ephemeral
opaque window identity, window focus state, timestamps, and public extension
presence for Codex (`openai.chatgpt`) and Claude Code
(`anthropic.claude-code`). It also reports whether the window is remote and,
when known, whether each installed provider extension runs in the local/UI or
remote/workspace extension host. Remote names, host identities, and virtual URI
authorities are not serialized.

Each heartbeat identifies its editor with the bounded `editor` value `vscode`,
`cursor`, or `antigravity_ide`. The companion derives this classification from
the public `vscode.env.uriScheme` field, with `vscode.env.appName` as a compatibility
fallback. Raw URI schemes and application names are never serialized. Older or
partially compatible hosts without a recognized signal retain the historical
`vscode` classification.

The heartbeat covers Cursor's fully supported VS Code-based IDE window.
Cursor's separate Agents Window does not activate the third-party companion and
remains hook-only. Optional documented native Cursor user hooks can add a recent
generic workspace observation from `workspaceOpen` and coarse agent activity
when Cursor supplies a usable workspace root. Unmatched hook activity remains
generic because hook payloads do not identify a native window or source
surface. A workspace-open observation has no agent status or agent/model label,
and hooks alone cannot establish window liveness, focus, or an open target.

Extension presence is reported as `available`, `installed`, `active`, and
nullable `remote` fields under `agentExtensions`. `installed` means the
extension was found for the current editor window/profile; `remote` identifies
its extension-host placement when installed. `active` means the editor reports
that the extension has activated; it does not mean that an agent is processing
a turn. Coarse turn activity requires the separate, optional lifecycle
integration managed by the desktop app.

It does not read or report source files, prompts, Codex answers, terminal
contents, diffs, transcripts, extension exports, private extension state, or
credentials. It has no network code, dependencies, telemetry, commands, or
settings. A failed public extension lookup is reported as unavailable rather
than as a confirmed missing extension.

All files are written on the machine running the desktop app. The companion
does not connect to a remote host, and this release has no bridge for remote
lifecycle hooks or provider usage queries.

If `VSPARALLEL_STATE_DIR` is set, it must be an absolute path shared with the
desktop app and lifecycle hooks. Relative overrides are rejected because the
local processes can have different working directories.

Production users install, repair, and uninstall the companion from the settings
gear beside **Refresh** inside VSParallel. **Set up Cursor monitoring** installs
or repairs this companion and Cursor's native activity hooks together; the
hooks-only control remains available for a non-live fallback setup. The app
embeds this extension, creates a temporary VSIX, installs it with the supported
editor CLI, verifies the version, and removes the temporary file. No Python or
Node runtime is used.
Named profiles can be selected with `VSPARALLEL_VSCODE_PROFILE` or
`VSPARALLEL_CURSOR_PROFILE` or `VSPARALLEL_ANTIGRAVITY_IDE_PROFILE`; status,
installation, verification, and
uninstallation all use the selected profile.

Reload each already-open editor window after installation. The extension
activates after startup and refreshes its heartbeat every three seconds.

If Cursor launches but no live workspace appears in VSParallel, use **Setup &
diagnostics** to set up or repair Cursor monitoring for the selected profile,
then reload the affected Cursor IDE windows. Cursor IDE heartbeats are fully
supported; the separate Agents Window remains hook-only. Cursor hooks can
provide only recent non-live fallback rows. A Cursor hook installation that
predates the managed `workspaceOpen` handler is intentionally reported as
needing **Update** or **Repair**; reinstalling the hook integration brings all
five handlers up to date without replacing unrelated Cursor hooks.

Opening this directory in an Extension Development Host is the supported
standalone development workflow. `package_vsix.py` is an optional developer
packager and is not called by the production application.
