# VSParallel Companion

This local-only VS Code extension reports a small, versioned heartbeat for the
current VS Code window to the VSParallel desktop app. It reports workspace
names, local folder paths, an ephemeral opaque window identity, window focus
state, timestamps, and public extension presence for Codex (`openai.chatgpt`)
and Claude Code (`anthropic.claude-code`). Remote and virtual URI authorities
are not serialized.

Extension presence is reported as `available`, `installed`, and `active`
booleans under `agentExtensions`. `active` means the VS Code extension has
activated; it does not mean that an agent is processing a turn. Coarse turn
activity requires the separate, optional lifecycle integration managed by the
desktop app.

It does not read or report source files, prompts, Codex answers, terminal
contents, diffs, transcripts, extension exports, private extension state, or
credentials. It has no network code, dependencies, telemetry, commands, or
settings. A failed public extension lookup is reported as unavailable rather
than as a confirmed missing extension.

If `VSPARALLEL_STATE_DIR` is set, it must be an absolute path shared with the
desktop app and lifecycle hooks. Relative overrides are rejected because the
local processes can have different working directories.

Production users install, repair, and uninstall the companion from the settings
gear beside **Refresh** inside VSParallel. The app embeds this extension,
creates a temporary VSIX, installs it with the supported VS Code CLI, verifies
the version, and removes the temporary file. No Python or Node runtime is used.

Reload each already-open VS Code window after installation. The extension
activates after startup and refreshes its heartbeat every three seconds.

Opening this directory in an Extension Development Host is the supported
standalone development workflow. `package_vsix.py` is an optional developer
packager and is not called by the production application.
