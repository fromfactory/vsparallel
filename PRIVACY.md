# VSParallel privacy and local data

Last updated: August 15, 2026

VSParallel is designed to keep workspace monitoring on your device. It has no
VSParallel account system or advertising, and it sends no product analytics or
monitoring records to the project maintainers.

VSParallel does not send prompts, responses, source code, terminal contents,
transcripts, tool payloads, Git data, or provider account credentials to the
project. It also does not retain those content fields in monitoring records.
Some editor and provider interfaces supply richer input to a local monitoring
process; that process selects only the fields described below and discards the
rest. Complete configuration backups are a separate local-only exception
described later in this notice.

This notice covers the desktop app, its companion extension and optional hooks,
and the project website. Your editors, coding-agent providers, operating system,
and GitHub have their own privacy practices.

## Data used on your device

VSParallel uses the following local data to build its dashboard:

| Category | Data used | Storage |
| --- | --- | --- |
| Workspace status | Local workspace paths and names, editor type, focus and heartbeat times, and whether a window or provider extension is local or remote | Local VSParallel state |
| Agent activity | Local workspace path, one-way session or conversation key, coarse state, and timestamp | Local VSParallel state when an optional hook is installed |
| Bounded labels | Recognized Antigravity model family; bounded Cursor model and agent labels; fixed hook-health values | Local VSParallel state when available |
| Usage | Quota percentages and reset times, latest-call or latest-thread token totals, or Cursor context remaining | In memory or a small local record, depending on the source |
| Preferences | Editor and card visibility, appearance, integration suppression, and the experimental Cursor bridge setting | Local VSParallel state and WebView preference storage |

Remote placement is stored as a boolean only. VSParallel does not store a
remote hostname, address, authority, or account identity.

Optional lifecycle hooks can receive provider events that contain more data
than VSParallel needs. The hook adapters create a new bounded record and do not
copy prompt text, responses, source, terminal output, transcripts, raw session
IDs, error text, or unselected fields. Raw session and conversation IDs are
replaced with SHA-256 keys before a record is written.

Cursor records may include a short model label and one of a fixed set of agent
types when Cursor supplies them. Antigravity records may include a recognized
model family and an opaque revision used to distinguish turns. A Cursor
`workspaceOpen` record contains a path-derived key, local path, state, and
timestamp, but no session, model, or agent label.

For exact schemas and validation limits, see the
[metadata protocol](docs/protocol.md).

## Read-only editor data

Zed discovery is automatic while VSParallel runs. The app opens Zed's local
SQLite databases in read-only/query-only mode. It uses validated local
workspace paths and timestamps, saved session and window information, and
limited metadata from the newest eligible Zed Agent thread. That thread
metadata can include a saved turn boundary, tool-use presence, model name, and
cumulative token counters.

VSParallel does not select thread titles or keep prompt, response, source, or
tool content. Selected Zed values are used for the current in-memory snapshot
and are not copied into the VSParallel state directory. VSParallel does not
change or delete Zed data. Hiding Zed in **Visibility** hides its rows but does
not stop automatic discovery.

For Antigravity IDE activity, VSParallel may read a small set of fields from the
latest user-input metadata for the conversation named by the hook. It keeps only
the queued flag and a recognized model family. It skips prompt and context
bodies and does not keep the raw model value or conversation ID. Older metadata
and the editor's last-selected-model setting can be used as compatibility
fallbacks.

## Provider usage

While at least one usage card is visible, VSParallel can ask installed provider
processes for current usage every 60 seconds and when you select **Refresh**:

- Codex rate limits through the local Codex `app-server`;
- Claude Code rate limits through the local Claude CLI; and
- Antigravity quota through `agy -p "/usage" --output-format json`.

These provider processes normally handle authentication and any network
connection. The usage collectors select only percentages, reset times, and
bounded quota labels. They do not extract provider account credentials or write
the live response to disk. A last-known live value can remain in memory for up
to 15 minutes and is marked stale after a failed refresh.

Claude's usage interface can also inspect session history when run normally.
VSParallel starts it with a new empty, private configuration directory and
disables session persistence so it cannot inspect the user's real Claude
transcripts. The temporary directory is removed after the query.

The following optional local usage records can be written:

- `usage/claude.json`: Claude percentages and reset times from VSParallel's
  managed status-line fallback;
- `usage/gemini.json`: the latest Gemini model call's total token count and
  capture time;
- `usage/cursor.json`: the latest Cursor Agent CLI context percentage remaining
  and capture time; and
- `usage/cursor-turn.json`: the latest local Cursor agent turn's input/output
  token total and capture time.

These records contain no account ID, session ID, workspace, model, prompt,
response, transcript path, cost, source data, or credential. Gemini and Cursor
records are marked stale after 15 minutes and ignored after 24 hours. Expired
Claude windows are omitted. Zed token data remains in memory only.

Gemini's `AfterModel` payload can contain the complete request and response. The
opt-in receiver selects only `usageMetadata.totalTokenCount` and a local
timestamp, then discards the payload. The count is local activity data, not
Gemini subscription quota.

## Experimental Cursor Desktop Bridge

Cursor Agents Window monitoring is off by default and is available only when
Cursor exposes its experimental private Desktop Bridge. This is not the
separately documented Cursor SDK Bridge. When enabled, VSParallel reads Cursor's
local discovery file and sends only `listThreads` over local inter-process
communication. It never sends an agent message.

VSParallel temporarily reads Cursor's local bridge token solely to authenticate
each local `listThreads` poll. It does not persist or log the token. Raw thread
IDs are hashed immediately; raw IDs, thread titles, socket paths, Cursor data
paths, prompts, and responses are discarded. A thread is displayed only when
its hash matches a Cursor hook record for a local workspace.

## Network connections

VSParallel has no project-operated backend. The following connections can still
occur:

- Installed release builds request the update manifest from GitHub
  Releases shortly after startup and when you select **Check for updates**.
  Update files are downloaded only after you choose to install an available
  update.
- Codex, Claude, and Antigravity usage checks start provider-owned processes.
  Those processes may connect to their provider under the provider's terms and
  privacy policy.
- The project website is hosted on GitHub Pages and requests public latest-
  release metadata from the GitHub API when the page loads. The site uses the
  browser's platform information locally to highlight a suitable download.

These requests expose normal connection information, such as an IP address and
request headers, to GitHub or the relevant provider. VSParallel does not add
workspace or activity records to them. VSParallel's site code adds no analytics,
advertising, forms, or cookies; GitHub hosting has its own behavior. GitHub's
handling of site and API requests is covered by the
[GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).

## Local storage and retention

The default VSParallel state directory is:

| Platform | Default directory |
| --- | --- |
| Linux and other Unix systems | `$XDG_STATE_HOME/vsparallel` or `~/.local/state/vsparallel` |
| macOS | `~/Library/Application Support/VSParallel` |
| Windows | `%LOCALAPPDATA%\VSParallel` |

`VSPARALLEL_STATE_DIR` can select a different absolute directory. The desktop
app, companions, and hooks must use the same value. Zed data is read from Zed's
own data directory; `VSPARALLEL_ZED_DATA_DIR` can select a different absolute
Zed data root.

Heartbeats older than 60 seconds are hidden, and activity older than 24 hours is
shown as unknown. Hiding or aging out a record does not by itself delete its
file. An orderly companion shutdown removes its heartbeat, but other old files
may remain until the related integration is removed, **Uninstall all** purges
them, or you delete the state directory.

Display settings are also cached under the WebView keys
`vsparallel.appearance` and `vsparallel.visibility`. These contain only theme
and visibility choices, not workspace or provider data. The platform WebView
controls their exact on-disk location, which is separate from the VSParallel
state directory.

Changing a **Visibility** switch changes presentation; it does not uninstall or
disable an integration. All six usage cards are visible by default. While any
card is visible, the shared usage snapshot reads all six sources, including
sources whose individual cards are hidden. Hiding every card stops that
periodic snapshot, but automatic Zed workspace discovery continues and
installed hooks can still write their local records.

## Provider configuration and backups

At startup and when Setup status is refreshed, VSParallel reads the relevant
local provider configuration files to report whether its integrations are
installed and usable. These status checks do not change the files. Gemini
settings can also be re-read during a usage snapshot to report whether its hook
is available.

When you install or repair an integration, VSParallel preserves unrelated
settings and writes the updated configuration atomically. Before the first
change, it saves a complete one-time backup:

- `$CODEX_HOME/hooks.json.vsparallel.bak` (normally
  `~/.codex/hooks.json.vsparallel.bak`)
- `$CLAUDE_CONFIG_DIR/settings.json.vsparallel.bak` (normally
  `~/.claude/settings.json.vsparallel.bak`)
- `~/.cursor/hooks.json.vsparallel.bak`
- `~/.cursor/cli-config.json.vsparallel.bak`
- `~/.gemini/config/hooks.json.vsparallel.bak`
- `$GEMINI_CLI_HOME/.gemini/settings.json.vsparallel.bak` when
  `GEMINI_CLI_HOME` is set, or `~/.gemini/settings.json.vsparallel.bak`

Because each backup is an exact copy, it may contain unrelated environment
values, tokens, passwords, or other secrets that were already in the provider
configuration. Backups stay on the current device, use owner-only permissions
on Unix, and are never uploaded by VSParallel. Uninstalling an integration does
not remove its backup.

The optional Claude and Cursor status lines are installed only if the setting is
unused. VSParallel does not replace a custom status line. Integration removal
deletes only handlers and settings recognized as VSParallel-owned.

## Delete local data

To stop managed integrations and remove VSParallel data:

1. Choose **Uninstall all** in **Setup & diagnostics**. This disables the
   experimental Cursor bridge, suppresses managed sources, and purges their
   app-owned records even if an external editor cannot be reached for physical
   removal.
2. Quit VSParallel and all supported editors and provider sessions.
3. Delete the state directory shown in **Setup & diagnostics**.
4. Uninstall the VSParallel desktop package through your operating system.
5. If your operating system keeps application data after uninstall, use its
   application-data cleanup to remove retained VSParallel/WebView data and clear
   the two preference keys described above. The exact location is
   platform-dependent.
6. Optionally delete the `.vsparallel.bak` files listed in the previous section
   after confirming the original provider configuration works as expected.

**Uninstall all** does not disable automatic Zed discovery or provider quota
checks while VSParallel remains installed and running. Quitting and uninstalling
the app stops those reads. VSParallel never deletes or changes Zed's own data;
use Zed's controls if you also want to remove it.

## Questions and changes

Policy updates are published in this repository and reflected by the date at
the top of this file. For questions or corrections, open a
[GitHub issue](https://github.com/fromfactory/vsparallel/issues). Do not include
secrets, private paths, prompts, source code, or other sensitive information in
a public issue.
