# VSParallel Companion

The VSParallel Companion is a dependency-free extension for VS Code, Cursor
IDE, and Antigravity IDE. It writes a local heartbeat every three seconds so the
VSParallel desktop app can show live editor windows.

## Data it reports

Each heartbeat contains only:

- the editor type and an ephemeral window ID;
- workspace names and validated local paths;
- focus and heartbeat times;
- whether the window is remote; and
- whether the Codex and Claude Code extensions are installed, active, and
  local or remote when that placement is known.

Remote host names and virtual URI authorities are omitted. Extension activation
does not mean an agent is running; coarse turn activity comes from separate
optional lifecycle hooks.

The companion does not read or report prompts, responses, source files,
terminal contents, diffs, transcripts, extension exports, private extension
state, or credentials. It has no network code, dependencies, telemetry,
commands, or settings.

Cursor's separate Agents Window, Antigravity 2.0, and Zed do not run this
extension. See the [main README](../README.md#what-it-supports) for how those
surfaces are monitored.

## Install

Production users should open **Setup & diagnostics** in VSParallel and install
or repair the editor integration there. Cursor and Antigravity setup install the
companion and their activity hooks together. The app embeds the VSIX, installs
it with the editor's command-line interface, verifies the version, and removes
the temporary package.

Reload editor windows that were open during installation. Named profiles can be
selected before launching VSParallel with:

- `VSPARALLEL_VSCODE_PROFILE`
- `VSPARALLEL_CURSOR_PROFILE`
- `VSPARALLEL_ANTIGRAVITY_IDE_PROFILE`

If `VSPARALLEL_STATE_DIR` is set, it must be the same absolute path used by the
desktop app and hooks.

## Development

Open `companion/` in a supported editor and start an Extension Development Host.
There is no dependency installation step. To build a standalone development
VSIX, run:

```bash
python3 package_vsix.py
```

The release workflow uses this script for the standalone VSIX. The desktop
backend separately assembles and tests the embedded package used for in-app
installation. See the
[metadata protocol](../docs/protocol.md#workspace-heartbeat-schema-version-1)
for the complete heartbeat schema and [Privacy](../PRIVACY.md) for local-data
handling.
