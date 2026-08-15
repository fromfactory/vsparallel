//! Local Cursor lifecycle-hook integration for VSParallel.
//!
//! Cursor loads native user hooks from `~/.cursor/hooks.json` and Cursor Agent
//! CLI settings from `~/.cursor/cli-config.json`. This module merges one
//! VSParallel-owned command into each lifecycle event and, when no custom
//! status line exists, installs a context-capacity capture command. Every
//! unrelated setting and hook is preserved. Payloads are consumed through
//! capped streaming deserializers: prompt text, responses, transcripts,
//! attachments, user identity, and tool data are ignored without becoming
//! part of the persisted data model.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::Builder as TempFileBuilder;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

const HOOKS_FILENAME: &str = "hooks.json";
const BACKUP_FILENAME: &str = "hooks.json.vsparallel.bak";
const CLI_CONFIG_FILENAME: &str = "cli-config.json";
const CLI_BACKUP_FILENAME: &str = "cli-config.json.vsparallel.bak";
const HOOK_ARGUMENT: &str = "cursor-hook";
const USAGE_ARGUMENT: &str = crate::usage::CURSOR_USAGE_ARGUMENT;
const HOOK_TIMEOUT_SECONDS: u64 = 2;
const USAGE_UPDATE_INTERVAL_MS: u64 = 1_000;
const USAGE_TIMEOUT_MS: u64 = 2_000;
const CONFIG_VERSION: u64 = 1;
const SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HOOK_INPUT_BYTES: usize = 1024 * 1024;
const MAX_EXISTING_RECORD_BYTES: u64 = 64 * 1024;
const MAX_SESSION_ID_BYTES: usize = 16 * 1024;
const MAX_WORKSPACE_PATH_BYTES: usize = 32 * 1024;
const MAX_WORKSPACE_PATHS: usize = 64;
const MAX_MODEL_IDENTIFIER_BYTES: usize = 128;
const MAX_MODEL_PARAMETER_BYTES: usize = 32;
const MAX_MODEL_PARAMETERS: usize = 16;
const MAX_MODEL_DISPLAY_BYTES: usize = 128;
const MAX_COMPOSER_MODE_BYTES: usize = 32;
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
const RECORD_LOCK_ATTEMPTS: usize = 200;
const RECORD_LOCK_RETRY: Duration = Duration::from_millis(5);
const STALE_RECORD_LOCK_AGE: Duration = Duration::from_secs(5);
const WORKSPACE_SESSION_KEY_DOMAIN: &[u8] = b"vsparallel.cursor.workspace-open.session.v1\0";
const WORKSPACE_RECORD_KEY_DOMAIN: &[u8] = b"vsparallel.cursor.workspace-open.record.v1\0";

const EVENTS: [CursorHookEvent; 5] = [
    CursorHookEvent::WorkspaceOpen,
    CursorHookEvent::SessionStart,
    CursorHookEvent::BeforeSubmitPrompt,
    CursorHookEvent::Stop,
    CursorHookEvent::SessionEnd,
];

#[cfg(windows)]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
#[cfg(windows)]
const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
}

/// A native Cursor lifecycle event handled by VSParallel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CursorHookEvent {
    WorkspaceOpen,
    SessionStart,
    BeforeSubmitPrompt,
    Stop,
    SessionEnd,
}

impl CursorHookEvent {
    /// Parse the stable event argument following the `cursor-hook` subcommand.
    pub fn from_cli_argument(value: &str) -> Option<Self> {
        match value {
            "workspace-open" => Some(Self::WorkspaceOpen),
            "session-start" => Some(Self::SessionStart),
            "before-submit-prompt" => Some(Self::BeforeSubmitPrompt),
            "stop" => Some(Self::Stop),
            "session-end" => Some(Self::SessionEnd),
            _ => None,
        }
    }

    fn cli_argument(self) -> &'static str {
        match self {
            Self::WorkspaceOpen => "workspace-open",
            Self::SessionStart => "session-start",
            Self::BeforeSubmitPrompt => "before-submit-prompt",
            Self::Stop => "stop",
            Self::SessionEnd => "session-end",
        }
    }

    fn config_name(self) -> &'static str {
        match self {
            Self::WorkspaceOpen => "workspaceOpen",
            Self::SessionStart => "sessionStart",
            Self::BeforeSubmitPrompt => "beforeSubmitPrompt",
            Self::Stop => "stop",
            Self::SessionEnd => "sessionEnd",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventState {
    Current,
    Stale,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsageCaptureState {
    Current,
    Stale,
    Missing,
    Conflict,
}

impl UsageCaptureState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Missing => "missing",
            Self::Conflict => "conflict",
        }
    }
}

impl EventState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Missing => "missing",
        }
    }
}

/// Serializable setup status for Cursor's global native hook and CLI config.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CursorIntegrationStatus {
    pub state: String,
    pub installed: bool,
    pub config_path: String,
    pub backup_path: String,
    pub event_states: BTreeMap<String, String>,
    pub usage_capture_state: String,
    pub message: String,
}

/// Result of installing, repairing, or uninstalling Cursor monitoring.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CursorIntegrationChange {
    pub changed: bool,
    pub migrated: bool,
    pub status: CursorIntegrationStatus,
}

/// The deliberately small, non-content view of Cursor's hook input.
///
/// Serde streams every unknown field into `IgnoredAny`. In particular, this
/// type intentionally has no fields for `prompt`, `text`, `transcript_path`,
/// `attachments`, `user_email`, `error_message`, tool input/output, or agent
/// thoughts and responses.
#[derive(Debug, Default, Deserialize)]
struct HookPayload {
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_workspace_roots")]
    workspace_roots: Vec<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default, deserialize_with = "deserialize_model_parameters")]
    model_params: Vec<ModelParameter>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    composer_mode: Option<String>,
    #[serde(default)]
    is_background_agent: Option<bool>,
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ModelParameter {
    id: String,
    value: String,
}

fn deserialize_workspace_roots<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{IgnoredAny, SeqAccess, Visitor};
    use std::fmt;

    struct WorkspaceRootsVisitor;

    impl<'de> Visitor<'de> for WorkspaceRootsVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an array of Cursor workspace paths")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut roots = Vec::with_capacity(MAX_WORKSPACE_PATHS);
            while roots.len() < MAX_WORKSPACE_PATHS {
                let Some(root) = sequence.next_element::<String>()? else {
                    return Ok(roots);
                };
                roots.push(root);
            }
            while sequence.next_element::<IgnoredAny>()?.is_some() {}
            Ok(roots)
        }
    }

    deserializer.deserialize_seq(WorkspaceRootsVisitor)
}

fn deserialize_model_parameters<'de, D>(deserializer: D) -> Result<Vec<ModelParameter>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{IgnoredAny, SeqAccess, Visitor};
    use std::fmt;

    struct ModelParametersVisitor;

    impl<'de> Visitor<'de> for ModelParametersVisitor {
        type Value = Vec<ModelParameter>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an array of Cursor model parameters")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut parameters = Vec::with_capacity(MAX_MODEL_PARAMETERS);
            while parameters.len() < MAX_MODEL_PARAMETERS {
                let Some(parameter) = sequence.next_element::<ModelParameter>()? else {
                    return Ok(parameters);
                };
                parameters.push(parameter);
            }
            while sequence.next_element::<IgnoredAny>()?.is_some() {}
            Ok(parameters)
        }
    }

    deserializer.deserialize_seq(ModelParametersVisitor)
}

/// The only persisted representation of a Cursor hook payload.
///
/// Each validated local workspace root receives its own record. Conversation
/// identity is SHA-256 pseudonymized before it is used in either a field or a
/// file name. A `workspaceOpen` observation instead uses independently
/// domain-separated hashes of its normalized local root. Optional model and
/// agent labels are strictly bounded displays.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookRecord {
    schema_version: u32,
    session_key: String,
    cwd: String,
    state: String,
    changed_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExistingHookRecord {
    schema_version: u32,
    session_key: String,
    state: String,
    changed_at_ms: i64,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    agent_kind: Option<String>,
}

/// Resolve Cursor's documented global user-hook directory (`~/.cursor`).
pub fn cursor_config_dir_from_environment() -> Result<PathBuf, String> {
    home_directory()
        .map(|path| path.join(".cursor"))
        .map_err(|_| "could not determine the Cursor global configuration directory".to_string())
}

/// Inspect VSParallel's Cursor hook and context-capture entries without changes.
pub fn cursor_integration_status(
    config_dir: &Path,
    executable: &Path,
) -> Result<CursorIntegrationStatus, String> {
    let paths = IntegrationPaths::new(config_dir)?;
    let handlers = managed_handlers(executable)?;
    let usage_handler = managed_usage_status_line(executable)?;
    let (config, _) = read_config(&paths.config)?;
    let (cli_config, _) = read_cli_config(&paths.cli_config)?;
    status_from_config(
        &paths,
        &config,
        &handlers,
        usage_capture_state(&cli_config, &usage_handler),
    )
}

/// Install or repair only VSParallel-owned lifecycle and context entries.
pub fn install_cursor_integration(
    config_dir: &Path,
    executable: &Path,
) -> Result<CursorIntegrationChange, String> {
    let paths = IntegrationPaths::new(config_dir)?;
    validate_install_executable(executable)?;
    let handlers = managed_handlers(executable)?;
    let usage_handler = managed_usage_status_line(executable)?;
    let (mut config, original) = read_config(&paths.config)?;
    let (mut cli_config, cli_original) = read_cli_config(&paths.cli_config)?;
    let existing = event_entries_for_all(&config)?;
    let states = event_states_from_entries(&existing, &handlers);
    let usage_state = usage_capture_state(&cli_config, &usage_handler);
    let needs_version = !config.contains_key("version");
    let hooks_need_change =
        needs_version || states.values().any(|state| *state != EventState::Current);
    let usage_needs_change = matches!(
        usage_state,
        UsageCaptureState::Missing | UsageCaptureState::Stale
    );

    if !hooks_need_change && !usage_needs_change {
        return Ok(CursorIntegrationChange {
            changed: false,
            migrated: false,
            status: status_from_states(&paths, states, usage_state),
        });
    }

    // Both files are managed as one setup operation. Validate every backup
    // target that may be needed before the first external write so a bad CLI
    // backup cannot leave only the lifecycle half updated (or vice versa).
    if hooks_need_change {
        validate_backup_target(&paths.backup)?;
    }
    if usage_needs_change {
        validate_backup_target(&paths.cli_backup)?;
    }

    let mut migrated = false;
    if hooks_need_change {
        if needs_version {
            config.insert("version".to_string(), Value::from(CONFIG_VERSION));
        }
        let hooks = hooks_map_mut(&mut config, true)?.expect("create=true returns a hooks object");

        for event in EVENTS {
            if states[&event] == EventState::Current {
                continue;
            }
            let entries = existing[&event].clone().unwrap_or_default();
            let (mut filtered, removed) = without_owned_entries(entries, event, &handlers[&event]);
            migrated |= removed;
            filtered.push(handlers[&event].clone());
            hooks.insert(event.config_name().to_string(), Value::Array(filtered));
        }

        ensure_backup(&paths.backup, &original)?;
        atomic_write_json(&paths.config, &config)?;
    }

    if usage_needs_change {
        migrated |= usage_state == UsageCaptureState::Stale;
        cli_config.insert("statusLine".to_string(), usage_handler.clone());
        ensure_backup(&paths.cli_backup, &cli_original)?;
        atomic_write_json(&paths.cli_config, &cli_config)?;
    }

    Ok(CursorIntegrationChange {
        changed: true,
        migrated,
        status: status_from_config(
            &paths,
            &config,
            &handlers,
            usage_capture_state(&cli_config, &usage_handler),
        )?,
    })
}

/// Remove only VSParallel-owned Cursor entries and preserve all other config.
pub fn uninstall_cursor_integration(
    config_dir: &Path,
    executable: &Path,
) -> Result<CursorIntegrationChange, String> {
    let paths = IntegrationPaths::new(config_dir)?;
    let handlers = managed_handlers(executable)?;
    let usage_handler = managed_usage_status_line(executable)?;
    let (mut config, original) = read_config(&paths.config)?;
    let (mut cli_config, cli_original) = read_cli_config(&paths.cli_config)?;
    let existing = event_entries_for_all(&config)?;
    let mut changed = false;
    let mut hooks_changed = false;
    let usage_changed = matches!(
        usage_capture_state(&cli_config, &usage_handler),
        UsageCaptureState::Current | UsageCaptureState::Stale
    );

    if let Some(hooks) = hooks_map_mut(&mut config, false)? {
        for event in EVENTS {
            let Some(entries) = existing[&event].clone() else {
                continue;
            };
            let (filtered, removed) = without_owned_entries(entries, event, &handlers[&event]);
            if removed {
                hooks.insert(event.config_name().to_string(), Value::Array(filtered));
                changed = true;
                hooks_changed = true;
            }
        }
    }

    if hooks_changed {
        validate_backup_target(&paths.backup)?;
    }
    if usage_changed {
        validate_backup_target(&paths.cli_backup)?;
    }

    if hooks_changed {
        ensure_backup(&paths.backup, &original)?;
        atomic_write_json(&paths.config, &config)?;
    }

    if usage_changed {
        cli_config.remove("statusLine");
        ensure_backup(&paths.cli_backup, &cli_original)?;
        atomic_write_json(&paths.cli_config, &cli_config)?;
        changed = true;
    }

    Ok(CursorIntegrationChange {
        changed,
        migrated: false,
        status: status_from_config(
            &paths,
            &config,
            &handlers,
            usage_capture_state(&cli_config, &usage_handler),
        )?,
    })
}

/// Fail-open stdio entry point used by the installed `cursor-hook` command.
pub fn run_cursor_hook_stdio(event: CursorHookEvent) -> i32 {
    run_cursor_hook(event, io::stdin().lock(), io::stdout().lock())
}

/// Testable Cursor hook entry point. It always writes `{}` and returns zero.
pub fn run_cursor_hook<R: Read, W: Write>(event: CursorHookEvent, reader: R, writer: W) -> i32 {
    let root = crate::state::state_dir_from_environment();
    run_cursor_hook_with(
        event,
        reader,
        writer,
        root.as_deref().ok(),
        cursor_remote_workspace_from_environment(),
        unix_time_ms(),
    )
}

fn run_cursor_hook_with<R: Read, W: Write>(
    event: CursorHookEvent,
    reader: R,
    mut writer: W,
    state_root: Option<&Path>,
    remote_workspace: bool,
    changed_at_ms: i64,
) -> i32 {
    let payload =
        serde_json::from_reader::<_, HookPayload>(CappedReader::new(reader, MAX_HOOK_INPUT_BYTES));
    if let (false, Ok(payload), Some(root)) = (remote_workspace, payload, state_root) {
        if !crate::state::integration_source_is_enabled_at(
            root,
            crate::state::IntegrationSource::CursorHooks,
        ) {
            let _ = writer.write_all(b"{}\n");
            let _ = writer.flush();
            return 0;
        }
        if event == CursorHookEvent::Stop {
            // Only these two counters cross into the usage recorder. Cache
            // reads/writes are input breakdowns and remain unrepresented;
            // lifecycle metadata is handled separately below and never enters
            // the usage record.
            let _ = crate::usage::capture_cursor_turn_usage(
                root,
                changed_at_ms,
                payload.input_tokens,
                payload.output_tokens,
            );
        }
        for (record_key, record) in records_from_payload(event, &payload, changed_at_ms) {
            // Monitoring is observational. Parsing, path, and persistence
            // failures must never interrupt or alter Cursor's agent loop.
            let _ = persist_record(root, &record_key, &record, event);
        }
    }

    let _ = writer.write_all(b"{}\n");
    let _ = writer.flush();
    0
}

fn cursor_remote_workspace_from_environment() -> bool {
    remote_workspace_value(env::var_os("CURSOR_CODE_REMOTE").as_deref())
}

fn remote_workspace_value(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("true"))
}

fn records_from_payload(
    event: CursorHookEvent,
    payload: &HookPayload,
    changed_at_ms: i64,
) -> Vec<(String, HookRecord)> {
    let Some(state) = activity_state(event, payload) else {
        return Vec::new();
    };
    let mut normalized = BTreeSet::new();
    for raw in &payload.workspace_roots {
        if let Some(path) = normalize_workspace_path(raw) {
            normalized.insert(path);
        }
    }

    if event == CursorHookEvent::WorkspaceOpen {
        return normalized
            .into_iter()
            .map(|cwd| {
                let cwd = cwd.to_string_lossy().into_owned();
                let session_key = workspace_observation_hash(WORKSPACE_SESSION_KEY_DOMAIN, &cwd);
                let record_key = workspace_observation_hash(WORKSPACE_RECORD_KEY_DOMAIN, &cwd);
                let record = HookRecord {
                    schema_version: SCHEMA_VERSION,
                    session_key,
                    cwd,
                    state: state.to_string(),
                    changed_at_ms,
                    // `workspaceOpen` has no conversation or agent context.
                    // Cursor's user_email and cursor_version fields are
                    // deliberately ignored by HookPayload.
                    model_name: None,
                    agent_kind: None,
                };
                (record_key, record)
            })
            .collect();
    }

    let Some(identity) = event_identity(event, payload) else {
        return Vec::new();
    };
    let session_key = cursor_identity_hash(identity);
    let model_name = cursor_model_display(payload);
    let agent_kind = matches!(
        event,
        CursorHookEvent::SessionStart | CursorHookEvent::SessionEnd
    )
    .then(|| cursor_agent_kind(payload))
    .flatten();
    normalized
        .into_iter()
        .map(|cwd| {
            let cwd = cwd.to_string_lossy().into_owned();
            let mut record_identity = Vec::with_capacity(identity.len() + cwd.len() + 1);
            record_identity.extend_from_slice(identity.as_bytes());
            record_identity.push(0);
            record_identity.extend_from_slice(cwd.as_bytes());
            let record_key = sha256_hex(&record_identity);
            let record = HookRecord {
                schema_version: SCHEMA_VERSION,
                session_key: session_key.clone(),
                cwd,
                state: state.to_string(),
                changed_at_ms,
                model_name: model_name.clone(),
                agent_kind: agent_kind.clone(),
            };
            (record_key, record)
        })
        .collect()
}

fn workspace_observation_hash(domain: &[u8], cwd: &str) -> String {
    let mut input = Vec::with_capacity(domain.len() + cwd.len());
    input.extend_from_slice(domain);
    input.extend_from_slice(cwd.as_bytes());
    sha256_hex(&input)
}

fn event_identity(event: CursorHookEvent, payload: &HookPayload) -> Option<&str> {
    let candidates = match event {
        CursorHookEvent::WorkspaceOpen => return None,
        CursorHookEvent::SessionStart
        | CursorHookEvent::BeforeSubmitPrompt
        | CursorHookEvent::Stop
        | CursorHookEvent::SessionEnd => [
            payload.conversation_id.as_deref(),
            payload.session_id.as_deref(),
        ],
    };
    candidates.into_iter().flatten().find(|value| {
        !value.is_empty() && value.len() <= MAX_SESSION_ID_BYTES && !value.contains('\0')
    })
}

fn activity_state(event: CursorHookEvent, payload: &HookPayload) -> Option<&'static str> {
    match event {
        CursorHookEvent::WorkspaceOpen => Some("workspace_opened"),
        CursorHookEvent::SessionStart => Some("session_started"),
        CursorHookEvent::BeforeSubmitPrompt => Some("activity_detected"),
        CursorHookEvent::Stop => match bounded_ascii_token(payload.status.as_deref()?, 32)? {
            "completed" => Some("turn_finished"),
            "aborted" => Some("interrupted"),
            "error" => Some("failed"),
            _ => None,
        },
        CursorHookEvent::SessionEnd => match bounded_ascii_token(payload.reason.as_deref()?, 32)? {
            "completed" | "window_close" | "user_close" => Some("session_ended"),
            "aborted" => Some("interrupted"),
            "error" => Some("failed"),
            _ => None,
        },
    }
}

fn bounded_ascii_token(value: &str, limit: usize) -> Option<&str> {
    (value.len() <= limit
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then_some(value)
}

fn cursor_model_display(payload: &HookPayload) -> Option<String> {
    let base = payload
        .model_id
        .as_deref()
        .and_then(sanitize_model_identifier)
        .or_else(|| payload.model.as_deref().and_then(sanitize_model_identifier))?;

    let mut qualifiers = Vec::new();
    let mut seen = BTreeSet::new();
    for parameter in &payload.model_params {
        if parameter.id.len() > MAX_MODEL_PARAMETER_BYTES
            || parameter.value.len() > MAX_MODEL_PARAMETER_BYTES
            || !seen.insert(parameter.id.as_str())
        {
            continue;
        }
        match parameter.id.as_str() {
            "thinking" if parameter.value == "true" => qualifiers.push("Thinking".to_string()),
            "context" => {
                if let Some(value) = sanitize_model_parameter(&parameter.value) {
                    qualifiers.push(format!("{value} context"));
                }
            }
            "effort" => {
                let label = match parameter.value.as_str() {
                    "low" => Some("Low effort"),
                    "medium" => Some("Medium effort"),
                    "high" => Some("High effort"),
                    "max" => Some("Max effort"),
                    _ => None,
                };
                if let Some(label) = label {
                    qualifiers.push(label.to_string());
                }
            }
            _ => {}
        }
    }

    let mut display = base;
    if !qualifiers.is_empty() {
        let suffix = format!(" ({})", qualifiers.join(", "));
        if display.len() + suffix.len() <= MAX_MODEL_DISPLAY_BYTES {
            display.push_str(&suffix);
        }
    }
    Some(display)
}

fn sanitize_model_identifier(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_MODEL_IDENTIFIER_BYTES
        || matches!(value.to_ascii_lowercase().as_str(), "default" | "unknown")
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'_' | b'.' | b'/' | b':' | b'+' | b'(' | b')' | b' '
                )
        })
    {
        return None;
    }
    Some(value.to_string())
}

fn sanitize_model_parameter(value: &str) -> Option<&str> {
    (!value.is_empty()
        && value.len() <= 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then_some(value)
}

fn cursor_agent_kind(payload: &HookPayload) -> Option<String> {
    if payload.is_background_agent == Some(true) {
        return Some("Background agent".to_string());
    }
    let mode = payload
        .composer_mode
        .as_deref()
        .filter(|value| value.len() <= MAX_COMPOSER_MODE_BYTES)?;
    let label = match mode {
        "agent" => "Agent",
        "ask" | "chat" => "Ask",
        "edit" => "Edit",
        _ => return None,
    };
    Some(label.to_string())
}

fn persist_record(
    root: &Path,
    record_key: &str,
    record: &HookRecord,
    event: CursorHookEvent,
) -> Result<(), String> {
    if !is_sha256_key(record_key) || !is_sha256_key(&record.session_key) {
        return Err("invalid Cursor record key".to_string());
    }
    ensure_private_directory(root)?;
    let directory = root.join("cursor");
    ensure_private_directory(&directory)?;
    let target = directory.join(format!("{record_key}.json"));
    let _lock = acquire_record_lock(&directory, record_key)?;

    let mut persisted = record.clone();
    if let Some(existing) = read_existing_record(&target, &record.session_key, record.changed_at_ms)
    {
        if event == CursorHookEvent::SessionStart {
            let adds_agent = existing.agent_kind.is_none() && record.agent_kind.is_some();
            if !adds_agent {
                return Ok(());
            }
            // sessionStart is fire-and-forget and can finish after the first
            // beforeSubmitPrompt (or even a very short Stop). It may enrich
            // missing session metadata, but must never regress lifecycle state
            // or replace metadata already attached to the newer event.
            persisted.state = existing.state;
            persisted.changed_at_ms = existing.changed_at_ms;
            // A delayed session-start model may predate the first prompt's
            // concrete choice. Only the session-scoped agent kind may enrich
            // an already-observed lifecycle record.
            persisted.model_name = existing.model_name;
            persisted.agent_kind = existing.agent_kind.or(persisted.agent_kind);
        } else if existing.changed_at_ms > record.changed_at_ms
            || (existing.changed_at_ms == record.changed_at_ms
                && cursor_state_precedence(&existing.state) > event_precedence(event))
        {
            return Ok(());
        } else {
            match event {
                CursorHookEvent::WorkspaceOpen => {
                    // Workspace observations are intentionally independent of
                    // all conversation, model, and agent metadata.
                    persisted.model_name = None;
                    persisted.agent_kind = None;
                }
                CursorHookEvent::SessionStart => unreachable!("handled above"),
                CursorHookEvent::BeforeSubmitPrompt => {
                    // Model selection is turn-specific. An absent or rejected
                    // value at a new prompt clears the preceding turn's label.
                    // Agent kind is session-scoped and comes from the
                    // documented sessionStart/sessionEnd lifecycle fields.
                    persisted.agent_kind = existing.agent_kind;
                }
                CursorHookEvent::Stop | CursorHookEvent::SessionEnd => {
                    persisted.model_name = persisted.model_name.or(existing.model_name);
                    persisted.agent_kind = persisted.agent_kind.or(existing.agent_kind);
                }
            }
        }
    }

    let mut bytes = serde_json::to_vec(&persisted)
        .map_err(|error| format!("could not serialize Cursor state: {error}"))?;
    bytes.push(b'\n');
    atomic_write_bytes(&target, &bytes, Some(0o600))
}

fn event_precedence(event: CursorHookEvent) -> u8 {
    match event {
        CursorHookEvent::WorkspaceOpen => 0,
        CursorHookEvent::SessionStart => 0,
        CursorHookEvent::BeforeSubmitPrompt => 1,
        CursorHookEvent::Stop => 2,
        CursorHookEvent::SessionEnd => 3,
    }
}

fn cursor_state_precedence(state: &str) -> u8 {
    match state {
        "workspace_opened" => 0,
        "session_started" => 0,
        "activity_detected" => 1,
        "turn_finished" | "failed" | "interrupted" => 2,
        "session_ended" => 3,
        _ => 0,
    }
}

struct RecordLock {
    path: PathBuf,
}

impl Drop for RecordLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            sync_parent(parent);
        }
    }
}

fn acquire_record_lock(directory: &Path, record_key: &str) -> Result<RecordLock, String> {
    if !is_sha256_key(record_key) {
        return Err("invalid Cursor record lock key".to_string());
    }
    let path = directory.join(format!(".{record_key}.lock"));
    for attempt in 0..RECORD_LOCK_ATTEMPTS {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&path) {
            Ok(file) => {
                file.sync_all().map_err(|error| {
                    let _ = fs::remove_file(&path);
                    format!("could not initialize {}: {error}", path.display())
                })?;
                set_private_file_permissions(&path, 0o600);
                return Ok(RecordLock { path });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&path).map_err(|inspect| {
                    format!(
                        "could not inspect Cursor record lock {}: {inspect}",
                        path.display()
                    )
                })?;
                if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                    return Err(format!("{} is not a regular lock file", path.display()));
                }
                let stale = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|age| age >= STALE_RECORD_LOCK_AGE);
                if stale {
                    match fs::remove_file(&path) {
                        Ok(()) => continue,
                        Err(remove) if remove.kind() == io::ErrorKind::NotFound => continue,
                        Err(remove) => {
                            return Err(format!(
                                "could not remove stale Cursor record lock {}: {remove}",
                                path.display()
                            ));
                        }
                    }
                }
                if attempt + 1 < RECORD_LOCK_ATTEMPTS {
                    std::thread::sleep(RECORD_LOCK_RETRY);
                }
            }
            Err(error) => {
                return Err(format!(
                    "could not create Cursor record lock {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "timed out waiting for Cursor record lock {}",
        path.display()
    ))
}

fn read_existing_record(
    path: &Path,
    expected_session_key: &str,
    event_time_ms: i64,
) -> Option<ExistingHookRecord> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if is_link_or_reparse_point(&metadata)
        || !metadata.is_file()
        || metadata.len() > MAX_EXISTING_RECORD_BYTES
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .ok()?
        .take(MAX_EXISTING_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_EXISTING_RECORD_BYTES {
        return None;
    }
    let record: ExistingHookRecord = serde_json::from_slice(&bytes).ok()?;
    if record.schema_version != SCHEMA_VERSION
        || record.session_key != expected_session_key
        || record.changed_at_ms < 0
        || record.changed_at_ms > event_time_ms.saturating_add(MAX_FUTURE_SKEW_MS)
        || !known_cursor_state(&record.state)
        || (record.state == "workspace_opened"
            && (record.model_name.is_some() || record.agent_kind.is_some()))
        || record
            .model_name
            .as_deref()
            .is_some_and(|value| !valid_persisted_model_display(value))
        || record
            .agent_kind
            .as_deref()
            .is_some_and(|value| !known_agent_kind(value))
    {
        return None;
    }
    Some(record)
}

fn known_cursor_state(state: &str) -> bool {
    matches!(
        state,
        "workspace_opened"
            | "session_started"
            | "activity_detected"
            | "turn_finished"
            | "session_ended"
            | "failed"
            | "interrupted"
    )
}

fn valid_persisted_model_display(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_MODEL_DISPLAY_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'_' | b'.' | b'/' | b':' | b'+' | b'(' | b')' | b' ' | b','
                )
        })
}

fn known_agent_kind(value: &str) -> bool {
    matches!(value, "Background agent" | "Agent" | "Ask" | "Edit")
}

#[derive(Debug)]
struct IntegrationPaths {
    config: PathBuf,
    backup: PathBuf,
    cli_config: PathBuf,
    cli_backup: PathBuf,
}

impl IntegrationPaths {
    fn new(config_dir: &Path) -> Result<Self, String> {
        if !config_dir.is_absolute() {
            return Err(
                "the Cursor global configuration directory must be an absolute path".into(),
            );
        }
        Ok(Self {
            config: config_dir.join(HOOKS_FILENAME),
            backup: config_dir.join(BACKUP_FILENAME),
            cli_config: config_dir.join(CLI_CONFIG_FILENAME),
            cli_backup: config_dir.join(CLI_BACKUP_FILENAME),
        })
    }
}

fn managed_handlers(executable: &Path) -> Result<BTreeMap<CursorHookEvent, Value>, String> {
    EVENTS
        .into_iter()
        .map(|event| managed_handler(executable, event).map(|handler| (event, handler)))
        .collect()
}

fn managed_handler(executable: &Path, event: CursorHookEvent) -> Result<Value, String> {
    if !executable.is_absolute() {
        return Err("the VSParallel hook executable must be an absolute path".to_string());
    }
    let executable = executable
        .to_str()
        .ok_or_else(|| "the VSParallel hook executable path is not valid Unicode".to_string())?;
    if executable.contains(['\0', '\n', '\r']) {
        return Err("the VSParallel hook executable path contains unsafe characters".to_string());
    }

    #[cfg(windows)]
    let command = format!(
        "{} {HOOK_ARGUMENT} {}",
        quote_windows(executable),
        event.cli_argument()
    );
    #[cfg(not(windows))]
    let command = format!(
        "{} {HOOK_ARGUMENT} {}",
        quote_posix(executable),
        event.cli_argument()
    );

    Ok(serde_json::json!({
        "type": "command",
        "command": command,
        "timeout": HOOK_TIMEOUT_SECONDS,
    }))
}

fn managed_usage_status_line(executable: &Path) -> Result<Value, String> {
    if !executable.is_absolute() {
        return Err("the VSParallel usage executable must be an absolute path".to_string());
    }
    let executable = executable
        .to_str()
        .ok_or_else(|| "the VSParallel usage executable path is not valid Unicode".to_string())?;
    if executable.contains(['\0', '\n', '\r']) {
        return Err("the VSParallel usage executable path contains unsafe characters".to_string());
    }
    #[cfg(windows)]
    let command = format!("{} {USAGE_ARGUMENT}", quote_windows(executable));
    #[cfg(not(windows))]
    let command = format!("{} {USAGE_ARGUMENT}", quote_posix(executable));
    Ok(serde_json::json!({
        "type": "command",
        "command": command,
        "padding": 0,
        "updateIntervalMs": USAGE_UPDATE_INTERVAL_MS,
        "timeoutMs": USAGE_TIMEOUT_MS,
    }))
}

#[cfg(not(windows))]
fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(windows)]
fn quote_windows(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    let mut backslashes = 0usize;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            result.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            result.push('"');
        } else {
            result.extend(std::iter::repeat_n('\\', backslashes));
            result.push(character);
        }
        backslashes = 0;
    }
    result.extend(std::iter::repeat_n('\\', backslashes * 2));
    result.push('"');
    result
}

fn validate_install_executable(executable: &Path) -> Result<(), String> {
    let metadata = fs::metadata(executable).map_err(|error| {
        format!(
            "the VSParallel hook executable is unavailable at {}: {error}",
            executable.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "the VSParallel hook executable is not a regular file: {}",
            executable.display()
        ));
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<(Map<String, Value>, Vec<u8>), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Map::new(), b"{}\n".to_vec()));
        }
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    };
    if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "{} exceeds the {} byte safety limit; it was left unchanged",
            path.display(),
            MAX_CONFIG_BYTES
        ));
    }
    let raw =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let json_bytes = raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&raw);
    let value: Value = serde_json::from_slice(json_bytes).map_err(|error| {
        format!(
            "{} is not valid UTF-8 JSON; it was left unchanged: {error}",
            path.display()
        )
    })?;
    let object = value.as_object().cloned().ok_or_else(|| {
        format!(
            "{} must contain a JSON object; it was left unchanged",
            path.display()
        )
    })?;
    match object.get("version") {
        None => {}
        Some(value) if value.as_u64() == Some(CONFIG_VERSION) => {}
        Some(_) => {
            return Err(format!(
                "{} has an unsupported Cursor hook configuration version; it was left unchanged",
                path.display()
            ));
        }
    }
    hooks_map(&object)?;
    Ok((object, raw))
}

fn read_cli_config(path: &Path) -> Result<(Map<String, Value>, Vec<u8>), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Map::new(), b"{}\n".to_vec()));
        }
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    };
    if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "{} exceeds the {} byte safety limit; it was left unchanged",
            path.display(),
            MAX_CONFIG_BYTES
        ));
    }
    let raw =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let json_bytes = raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&raw);
    let value: Value = serde_json::from_slice(json_bytes).map_err(|error| {
        format!(
            "{} is not valid UTF-8 JSON; it was left unchanged: {error}",
            path.display()
        )
    })?;
    value
        .as_object()
        .cloned()
        .map(|object| (object, raw))
        .ok_or_else(|| {
            format!(
                "{} must contain a JSON object; it was left unchanged",
                path.display()
            )
        })
}

fn hooks_map(config: &Map<String, Value>) -> Result<Option<&Map<String, Value>>, String> {
    match config.get("hooks") {
        None => Ok(None),
        Some(Value::Object(hooks)) => Ok(Some(hooks)),
        Some(_) => Err("the top-level 'hooks' value must be a JSON object".to_string()),
    }
}

fn hooks_map_mut(
    config: &mut Map<String, Value>,
    create: bool,
) -> Result<Option<&mut Map<String, Value>>, String> {
    if !config.contains_key("hooks") {
        if !create {
            return Ok(None);
        }
        config.insert("hooks".to_string(), Value::Object(Map::new()));
    }
    config
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .map(Some)
        .ok_or_else(|| "the top-level 'hooks' value must be a JSON object".to_string())
}

fn event_entries(
    hooks: &Map<String, Value>,
    event: CursorHookEvent,
) -> Result<Option<&Vec<Value>>, String> {
    match hooks.get(event.config_name()) {
        None => Ok(None),
        Some(Value::Array(entries)) => Ok(Some(entries)),
        Some(_) => Err(format!(
            "hooks.{} must be a JSON array",
            event.config_name()
        )),
    }
}

fn event_entries_for_all(
    config: &Map<String, Value>,
) -> Result<BTreeMap<CursorHookEvent, Option<Vec<Value>>>, String> {
    let Some(hooks) = hooks_map(config)? else {
        return Ok(EVENTS.into_iter().map(|event| (event, None)).collect());
    };
    EVENTS
        .into_iter()
        .map(|event| event_entries(hooks, event).map(|entries| (event, entries.cloned())))
        .collect()
}

fn event_states(
    config: &Map<String, Value>,
    handlers: &BTreeMap<CursorHookEvent, Value>,
) -> Result<BTreeMap<CursorHookEvent, EventState>, String> {
    let entries = event_entries_for_all(config)?;
    Ok(event_states_from_entries(&entries, handlers))
}

fn event_states_from_entries(
    entries: &BTreeMap<CursorHookEvent, Option<Vec<Value>>>,
    handlers: &BTreeMap<CursorHookEvent, Value>,
) -> BTreeMap<CursorHookEvent, EventState> {
    EVENTS
        .into_iter()
        .map(|event| {
            let Some(entries) = entries[&event].as_ref() else {
                return (event, EventState::Missing);
            };
            let owned: Vec<_> = entries
                .iter()
                .filter(|candidate| is_owned_handler(candidate, event, &handlers[&event]))
                .collect();
            let state = if owned.len() == 1 && owned[0] == &handlers[&event] {
                EventState::Current
            } else if !owned.is_empty() {
                EventState::Stale
            } else {
                EventState::Missing
            };
            (event, state)
        })
        .collect()
}

fn is_owned_handler(candidate: &Value, event: CursorHookEvent, current: &Value) -> bool {
    candidate == current || historical_vsparallel_handler(candidate, event)
}

fn historical_vsparallel_handler(candidate: &Value, event: CursorHookEvent) -> bool {
    let Some(object) = candidate.as_object() else {
        return false;
    };
    const ALLOWED_KEYS: [&str; 3] = ["type", "command", "timeout"];
    if object.is_empty()
        || object.len() > ALLOWED_KEYS.len()
        || object
            .keys()
            .any(|key| !ALLOWED_KEYS.contains(&key.as_str()))
        || object
            .get("type")
            .is_some_and(|value| value.as_str() != Some("command"))
        || object.get("timeout").is_some_and(|value| !value.is_u64())
    {
        return false;
    }
    let Some(command) = object.get("command").and_then(Value::as_str) else {
        return false;
    };
    historical_command_targets_vsparallel(command, event)
}

fn historical_command_targets_vsparallel(command: &str, event: CursorHookEvent) -> bool {
    if command.contains(['\0', '\n', '\r']) {
        return false;
    }
    let suffix = format!(" {HOOK_ARGUMENT} {}", event.cli_argument());
    let Some(prefix) = command.trim().strip_suffix(&suffix) else {
        return false;
    };
    let prefix = prefix.trim();
    let executable = if prefix.starts_with('\'') {
        parse_posix_single_quoted_word(prefix)
    } else if prefix.len() >= 2 && prefix.starts_with('"') && prefix.ends_with('"') {
        let inner = &prefix[1..prefix.len() - 1];
        (!inner.contains('"')).then(|| inner.to_string())
    } else if !prefix.chars().any(char::is_whitespace)
        && !prefix.contains([';', '&', '|', '`', '$', '<', '>', '(', ')'])
    {
        Some(prefix.to_string())
    } else {
        None
    };
    executable
        .as_deref()
        .is_some_and(historical_vsparallel_executable)
}

fn parse_posix_single_quoted_word(value: &str) -> Option<String> {
    let mut remaining = value.strip_prefix('\'')?;
    let mut decoded = String::with_capacity(value.len());
    loop {
        let closing = remaining.find('\'')?;
        decoded.push_str(&remaining[..closing]);
        remaining = &remaining[closing + 1..];
        if remaining.is_empty() {
            return Some(decoded);
        }
        // This is the one escape sequence emitted by quote_posix: close the
        // single-quoted word, emit one apostrophe in double quotes, and reopen
        // the single-quoted word. Accepting only that grammar avoids treating
        // arbitrary shell expressions as VSParallel-owned commands.
        remaining = remaining.strip_prefix("\"'\"'")?;
        decoded.push('\'');
    }
}

fn historical_vsparallel_executable(executable: &str) -> bool {
    let path = Path::new(executable);
    if !path.is_absolute() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    name == "vsparallel"
        || name == "vsparallel.exe"
        || (name.starts_with("vsparallel") && name.ends_with(".appimage"))
}

fn without_owned_entries(
    entries: Vec<Value>,
    event: CursorHookEvent,
    current: &Value,
) -> (Vec<Value>, bool) {
    let original_len = entries.len();
    let filtered: Vec<_> = entries
        .into_iter()
        .filter(|candidate| !is_owned_handler(candidate, event, current))
        .collect();
    let removed = filtered.len() != original_len;
    (filtered, removed)
}

fn usage_capture_state(config: &Map<String, Value>, current: &Value) -> UsageCaptureState {
    let Some(candidate) = config.get("statusLine") else {
        return UsageCaptureState::Missing;
    };
    if candidate == current {
        return UsageCaptureState::Current;
    }
    if is_stale_usage_status_line(candidate) {
        UsageCaptureState::Stale
    } else {
        UsageCaptureState::Conflict
    }
}

fn is_stale_usage_status_line(candidate: &Value) -> bool {
    let Some(object) = candidate.as_object() else {
        return false;
    };
    const ALLOWED_KEYS: [&str; 5] = [
        "type",
        "command",
        "padding",
        "updateIntervalMs",
        "timeoutMs",
    ];
    if object.len() > ALLOWED_KEYS.len()
        || object
            .keys()
            .any(|key| !ALLOWED_KEYS.contains(&key.as_str()))
        || object.get("type").and_then(Value::as_str) != Some("command")
    {
        return false;
    }
    object
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(command_targets_vsparallel_usage)
}

fn command_targets_vsparallel_usage(command: &str) -> bool {
    if command.is_empty()
        || command.len() > MAX_WORKSPACE_PATH_BYTES
        || command.contains(['\0', '\n', '\r'])
    {
        return false;
    }
    let suffix = format!(" {USAGE_ARGUMENT}");
    let Some(executable_word) = command.strip_suffix(&suffix) else {
        return false;
    };
    let executable = parse_exact_executable_word(executable_word);
    executable
        .as_deref()
        .is_some_and(historical_vsparallel_executable)
}

fn parse_exact_executable_word(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }

    #[cfg(not(windows))]
    if value.starts_with('\'') {
        return parse_posix_single_quoted_word(value);
    }

    #[cfg(windows)]
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        return (!inner.contains('"')).then(|| inner.to_string());
    }

    (!value.chars().any(char::is_whitespace)
        && !value.contains(['\'', '"', ';', '&', '|', '`', '$', '<', '>', '(', ')']))
    .then(|| value.to_string())
}

fn status_from_config(
    paths: &IntegrationPaths,
    config: &Map<String, Value>,
    handlers: &BTreeMap<CursorHookEvent, Value>,
    usage_state: UsageCaptureState,
) -> Result<CursorIntegrationStatus, String> {
    Ok(status_from_states(
        paths,
        event_states(config, handlers)?,
        usage_state,
    ))
}

fn status_from_states(
    paths: &IntegrationPaths,
    states: BTreeMap<CursorHookEvent, EventState>,
    usage_state: UsageCaptureState,
) -> CursorIntegrationStatus {
    let current = states
        .values()
        .filter(|state| **state == EventState::Current)
        .count();
    let stale = states
        .values()
        .filter(|state| **state == EventState::Stale)
        .count();
    let hook_state = if current == EVENTS.len() {
        "installed"
    } else if current == 0 && stale == 0 {
        "not_installed"
    } else if current == 0 && stale > 0 {
        "stale"
    } else {
        "partial"
    };
    let state = match (hook_state, usage_state) {
        ("installed", UsageCaptureState::Current | UsageCaptureState::Conflict) => "installed",
        ("installed", UsageCaptureState::Missing | UsageCaptureState::Stale) => "partial",
        ("not_installed", UsageCaptureState::Current | UsageCaptureState::Stale) => "partial",
        _ => hook_state,
    };
    let mut message = match state {
        "installed" => "Cursor workspace and agent monitoring is installed.",
        "not_installed" => "Cursor workspace and agent monitoring is not installed.",
        "stale" => "An older VSParallel Cursor integration can be repaired.",
        _ => "Cursor workspace and agent monitoring is only partially installed.",
    }
    .to_string();
    match usage_state {
        UsageCaptureState::Current => {
            message.push_str(" Cursor Agent CLI context capture is installed.");
        }
        UsageCaptureState::Stale => {
            message.push_str(" Cursor Agent CLI context capture needs repair.");
        }
        UsageCaptureState::Missing => {
            message.push_str(" Cursor Agent CLI context capture is not installed.");
        }
        UsageCaptureState::Conflict => {
            message.push_str(
                " An existing custom Cursor Agent status line was kept; context capture is disabled.",
            );
        }
    }
    CursorIntegrationStatus {
        state: state.to_string(),
        installed: state == "installed",
        config_path: paths.config.to_string_lossy().into_owned(),
        backup_path: paths.backup.to_string_lossy().into_owned(),
        event_states: states
            .into_iter()
            .map(|(event, state)| (event.config_name().to_string(), state.as_str().to_string()))
            .collect(),
        usage_capture_state: usage_state.as_str().to_string(),
        message,
    }
}

fn ensure_backup(path: &Path, original: &[u8]) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                return Err(format!("{} is not a regular file", path.display()));
            }
            return Ok(false);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    ensure_private_directory(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Ok(mut file) => {
            if let Err(error) = file.write_all(original).and_then(|_| file.sync_all()) {
                drop(file);
                let _ = fs::remove_file(path);
                return Err(format!("could not write {}: {error}", path.display()));
            }
            set_private_file_permissions(path, 0o600);
            sync_parent(parent);
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)
                .map_err(|inspect| format!("could not inspect {}: {inspect}", path.display()))?;
            if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                Err(format!("{} is not a regular file", path.display()))
            } else {
                Ok(false)
            }
        }
        Err(error) => Err(format!("could not create {}: {error}", path.display())),
    }
}

fn validate_backup_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) || !metadata.is_file() => {
            Err(format!("{} is not a regular file", path.display()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn atomic_write_json(path: &Path, config: &Map<String, Value>) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("could not serialize Cursor hooks: {error}"))?;
    bytes.push(b'\n');
    let mode = existing_mode(path).unwrap_or(0o600);
    atomic_write_bytes(path, &bytes, Some(mode))
}

fn atomic_write_bytes(path: &Path, content: &[u8], mode: Option<u32>) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    ensure_private_directory(parent)?;
    reject_unsafe_existing_target(path)?;

    let target_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("vsparallel");
    let prefix = format!(".{target_name}.");
    let mut temporary = TempFileBuilder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| {
            format!(
                "could not create a temporary file in {}: {error}",
                parent.display()
            )
        })?;
    if let Some(mode) = mode {
        set_private_file_permissions(temporary.path(), mode);
    }
    temporary
        .write_all(content)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| {
            format!(
                "could not write temporary file {}: {error}",
                temporary.path().display()
            )
        })?;
    replace_temporary_file(temporary, path)?;
    if let Some(mode) = mode {
        set_private_file_permissions(path, mode);
    }
    sync_parent(parent);
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) => {
            return Err(format!("refusing to use symbolic link {}", path.display()));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(format!("{} is not a directory", path.display()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)
                .map_err(|create| format!("could not create {}: {create}", path.display()))?;
            let metadata = fs::symlink_metadata(path)
                .map_err(|inspect| format!("could not inspect {}: {inspect}", path.display()))?;
            if is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
                return Err(format!("{} is not a safe directory", path.display()));
            }
        }
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    }
    set_private_directory_permissions(path);
    Ok(())
}

fn reject_unsafe_existing_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) => Err(format!(
            "refusing to replace link or reparse point {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => {
            Err(format!("{} is not a regular file", path.display()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(not(windows))]
fn replace_temporary_file(temporary: tempfile::NamedTempFile, target: &Path) -> Result<(), String> {
    temporary.persist(target).map_err(|error| {
        format!(
            "could not atomically replace {}: {}",
            target.display(),
            error.error
        )
    })?;
    Ok(())
}

#[cfg(windows)]
fn replace_temporary_file(temporary: tempfile::NamedTempFile, target: &Path) -> Result<(), String> {
    let temporary = temporary.into_temp_path();
    let source_wide = nul_terminated_wide_path(temporary.as_ref())?;
    let target_wide = nul_terminated_wide_path(target)?;
    let replaced = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        return Err(format!(
            "could not atomically replace {}: {}",
            target.display(),
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn nul_terminated_wide_path(path: &Path) -> Result<Vec<u16>, String> {
    let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    if encoded.contains(&0) {
        return Err(format!("{} contains an embedded NUL", path.display()));
    }
    encoded.push(0);
    Ok(encoded)
}

fn existing_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        fs::metadata(path)
            .ok()
            .map(|metadata| metadata.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

fn set_private_file_permissions(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

fn set_private_directory_permissions(path: &Path) {
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn sync_parent(path: &Path) {
    #[cfg(unix)]
    {
        if let Ok(directory) = File::open(path) {
            let _ = directory.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn nonempty_env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn home_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let home = nonempty_env_path("USERPROFILE").or_else(|| {
        let drive = env::var_os("HOMEDRIVE")?;
        let path = env::var_os("HOMEPATH")?;
        Some(PathBuf::from(drive).join(path))
    });
    #[cfg(not(target_os = "windows"))]
    let home = nonempty_env_path("HOME");
    home.ok_or_else(|| "the home directory is unavailable".to_string())
}

fn normalize_workspace_path(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.len() > MAX_WORKSPACE_PATH_BYTES
        || raw.contains('\0')
        || raw.starts_with("//")
    {
        return None;
    }

    #[cfg(windows)]
    let path = cursor_windows_path(raw)?;
    #[cfg(not(windows))]
    let path = PathBuf::from(raw);

    if !path.is_absolute() {
        return None;
    }
    #[cfg(windows)]
    if windows_path_is_nonlocal(&path) {
        return None;
    }
    if let Ok(canonical) = fs::canonicalize(&path) {
        #[cfg(windows)]
        if windows_path_is_nonlocal(&canonical) {
            return None;
        }
        return Some(canonical);
    }
    lexical_normalize_absolute(&path)
}

#[cfg(windows)]
fn cursor_windows_path(raw: &str) -> Option<PathBuf> {
    let bytes = raw.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && matches!(bytes[3], b'/' | b'\\')
    {
        return Some(PathBuf::from(&raw[1..]));
    }
    Some(PathBuf::from(raw))
}

#[cfg(windows)]
fn windows_path_is_nonlocal(path: &Path) -> bool {
    use std::path::Prefix;
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(
                prefix.kind(),
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(..)
            )
    )
}

fn lexical_normalize_absolute(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Some(normalized)
}

fn unix_time_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn is_sha256_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A streaming adapter that consumes at most the limit plus one probe byte.
struct CappedReader<R> {
    inner: R,
    remaining: usize,
    exceeded: bool,
}

impl<R> CappedReader<R> {
    fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            remaining: limit,
            exceeded: false,
        }
    }
}

impl<R: Read> Read for CappedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.exceeded {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Cursor hook payload exceeds the safety limit",
            ));
        }
        if self.remaining == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => {
                    self.exceeded = true;
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Cursor hook payload exceeds the safety limit",
                    ))
                }
            };
        }
        let allowed = buffer.len().min(self.remaining);
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.remaining -= read;
        Ok(read)
    }
}

// Dependency-free SHA-256 used only to pseudonymize conversation identities.
pub(crate) fn cursor_identity_hash(identity: &str) -> String {
    sha256_hex(identity.as_bytes())
}

pub(crate) fn cursor_bytes_hash(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

fn sha256_hex(input: &[u8]) -> String {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity((input.len() + 72) & !63);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut hash = INITIAL;
    for chunk in padded.chunks_exact(64) {
        let mut schedule = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            schedule[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(big_s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(schedule[index]);
            let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = big_s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
        hash[5] = hash[5].wrapping_add(f);
        hash[6] = hash[6].wrapping_add(g);
        hash[7] = hash[7].wrapping_add(h);
    }

    let mut output = String::with_capacity(64);
    for word in hash {
        use std::fmt::Write as _;
        let _ = write!(output, "{word:08x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn executable(root: &Path) -> PathBuf {
        let executable = root.join("VSParallel app").join(if cfg!(windows) {
            "vsparallel.exe"
        } else {
            "vsparallel"
        });
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, b"test executable").unwrap();
        executable
    }

    fn write_json(path: &Path, value: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn parse_config(config_dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(config_dir.join(HOOKS_FILENAME)).unwrap()).unwrap()
    }

    fn parse_cli_config(config_dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(config_dir.join(CLI_CONFIG_FILENAME)).unwrap()).unwrap()
    }

    fn hook_with_root(
        event: CursorHookEvent,
        input: &str,
        root: &Path,
        now: i64,
    ) -> (i32, Vec<u8>) {
        let mut output = Vec::new();
        let code =
            run_cursor_hook_with(event, input.as_bytes(), &mut output, Some(root), false, now);
        (code, output)
    }

    fn records(root: &Path) -> Vec<(PathBuf, Value)> {
        let mut records: Vec<_> = fs::read_dir(root.join("cursor"))
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                let value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                (path, value)
            })
            .collect();
        records.sort_by(|left, right| left.0.cmp(&right.0));
        records
    }

    #[test]
    fn cli_event_arguments_are_stable_and_closed() {
        for (argument, event) in [
            ("workspace-open", CursorHookEvent::WorkspaceOpen),
            ("session-start", CursorHookEvent::SessionStart),
            ("before-submit-prompt", CursorHookEvent::BeforeSubmitPrompt),
            ("stop", CursorHookEvent::Stop),
            ("session-end", CursorHookEvent::SessionEnd),
        ] {
            assert_eq!(CursorHookEvent::from_cli_argument(argument), Some(event));
            assert_eq!(event.cli_argument(), argument);
        }
        assert_eq!(
            CursorHookEvent::from_cli_argument("beforeSubmitPrompt"),
            None
        );
        assert_eq!(CursorHookEvent::from_cli_argument("unknown"), None);
    }

    #[test]
    fn workspace_open_records_path_derived_privacy_safe_observations() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first workspace");
        let second = temp.path().join("second-workspace");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let input = json!({
            "hook_event_name":"workspaceOpen",
            "cursor_version":"3.15.19-private",
            "workspace_roots":[first.clone(), second, first.clone()],
            "user_email":"private@example.invalid"
        });

        let (code, output) = hook_with_root(
            CursorHookEvent::WorkspaceOpen,
            &input.to_string(),
            temp.path(),
            42,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");

        let saved = records(temp.path());
        assert_eq!(saved.len(), 2);
        for (path, record) in &saved {
            let cwd = record["cwd"].as_str().unwrap();
            let expected_session = workspace_observation_hash(WORKSPACE_SESSION_KEY_DOMAIN, cwd);
            let expected_record = workspace_observation_hash(WORKSPACE_RECORD_KEY_DOMAIN, cwd);
            assert_eq!(path.file_stem().unwrap().to_string_lossy(), expected_record);
            assert_eq!(record.as_object().unwrap().len(), 5);
            assert_eq!(record["schemaVersion"], SCHEMA_VERSION);
            assert_eq!(record["sessionKey"], expected_session);
            assert_eq!(record["state"], "workspace_opened");
            assert_eq!(record["changedAtMs"], 42);
            assert_ne!(record["sessionKey"], expected_record);
            assert!(record.get("modelName").is_none());
            assert!(record.get("agentKind").is_none());
            let serialized = record.to_string();
            for secret in [
                "private@example.invalid",
                "3.15.19-private",
                "workspaceOpen",
            ] {
                assert!(!serialized.contains(secret), "persisted {secret}");
            }
        }

        // The same normalized path owns the same two domain-separated keys,
        // independent of user identity, and an older competing hook cannot
        // move the observation timestamp backwards.
        let repeated = json!({
            "workspace_roots":[first],
            "user_email":"another-private@example.invalid"
        });
        hook_with_root(
            CursorHookEvent::WorkspaceOpen,
            &repeated.to_string(),
            temp.path(),
            84,
        );
        hook_with_root(
            CursorHookEvent::WorkspaceOpen,
            &repeated.to_string(),
            temp.path(),
            21,
        );
        let saved = records(temp.path());
        assert_eq!(saved.len(), 2);
        let first_canonical = fs::canonicalize(first).unwrap();
        let first_record = saved
            .iter()
            .map(|(_, record)| record)
            .find(|record| record["cwd"] == first_canonical.to_string_lossy().as_ref())
            .unwrap();
        assert_eq!(first_record["changedAtMs"], 84);
        assert!(!first_record
            .to_string()
            .contains("another-private@example.invalid"));
    }

    #[test]
    fn session_start_records_metadata_without_claiming_turn_activity() {
        let temp = TempDir::new().unwrap();
        let input = json!({
            "session_id":"private-session-id",
            "conversation_id":"fallback-conversation-id",
            "workspace_roots":[temp.path().join("workspace")],
            "composer_mode":"edit",
            "is_background_agent":false
        });
        let (code, output) = hook_with_root(
            CursorHookEvent::SessionStart,
            &input.to_string(),
            temp.path(),
            9,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");
        let record = &records(temp.path())[0].1;
        assert_eq!(record["state"], "session_started");
        assert_eq!(cursor_state_precedence("session_started"), 0);
        assert_eq!(record["agentKind"], "Edit");
        assert_eq!(
            record["sessionKey"],
            sha256_hex(b"fallback-conversation-id")
        );
        let saved = record.to_string();
        assert!(!saved.contains("private-session-id"));
        assert!(!saved.contains("fallback-conversation-id"));
    }

    #[test]
    fn hook_persists_one_privacy_safe_record_per_workspace() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first project");
        let second = temp.path().join("second-project");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let input = json!({
            "conversation_id": "private-conversation-id",
            "generation_id": "private-generation-id",
            "hook_event_name": "beforeSubmitPrompt",
            "workspace_roots": [first, second],
            "model": "legacy-private-model",
            "model_id": "claude-opus-4-7",
            "model_params": [
                {"id":"thinking", "value":"true"},
                {"id":"context", "value":"1m"},
                {"id":"effort", "value":"max"},
                {"id":"secret", "value":"private-value"}
            ],
            "composer_mode": "agent",
            "is_background_agent": false,
            "prompt": "extremely private prompt",
            "text": "private assistant response",
            "transcript_path": "/private/transcript.jsonl",
            "user_email": "private@example.invalid",
            "attachments": [{"private":"attachment"}]
        })
        .to_string();

        let (code, output) = hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &input,
            temp.path(),
            1_700_000_000_123,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");
        let records = records(temp.path());
        assert_eq!(records.len(), 2);
        for (path, record) in records {
            assert_eq!(path.file_stem().unwrap().to_string_lossy().len(), 64);
            assert_eq!(record.as_object().unwrap().len(), 6);
            assert_eq!(record["schemaVersion"], 1);
            assert_eq!(record["state"], "activity_detected");
            assert_eq!(record["changedAtMs"], 1_700_000_000_123i64);
            assert_eq!(record["sessionKey"].as_str().unwrap().len(), 64);
            assert_eq!(
                record["modelName"],
                "claude-opus-4-7 (Thinking, 1m context, Max effort)"
            );
            assert!(record.get("agentKind").is_none());
            let saved = serde_json::to_string(&record).unwrap();
            for secret in [
                "private-conversation-id",
                "private-generation-id",
                "extremely private prompt",
                "private assistant response",
                "transcript.jsonl",
                "private@example.invalid",
                "attachment",
                "legacy-private-model",
                "private-value",
            ] {
                assert!(!saved.contains(secret), "persisted {secret}");
            }
        }
    }

    #[test]
    fn lifecycle_events_map_documented_outcomes() {
        for (index, event, field, value, expected) in [
            (
                0,
                CursorHookEvent::Stop,
                "status",
                "completed",
                "turn_finished",
            ),
            (1, CursorHookEvent::Stop, "status", "aborted", "interrupted"),
            (2, CursorHookEvent::Stop, "status", "error", "failed"),
            (
                3,
                CursorHookEvent::SessionEnd,
                "reason",
                "completed",
                "session_ended",
            ),
            (
                4,
                CursorHookEvent::SessionEnd,
                "reason",
                "window_close",
                "session_ended",
            ),
            (
                5,
                CursorHookEvent::SessionEnd,
                "reason",
                "user_close",
                "session_ended",
            ),
            (
                6,
                CursorHookEvent::SessionEnd,
                "reason",
                "aborted",
                "interrupted",
            ),
            (7, CursorHookEvent::SessionEnd, "reason", "error", "failed"),
        ] {
            let temp = TempDir::new().unwrap();
            let mut input = Map::new();
            input.insert(
                if event == CursorHookEvent::SessionEnd {
                    "session_id"
                } else {
                    "conversation_id"
                }
                .to_string(),
                Value::String(format!("session-{index}")),
            );
            input.insert(
                "workspace_roots".to_string(),
                json!([temp.path().join("workspace")]),
            );
            input.insert(field.to_string(), Value::String(value.to_string()));
            let (code, output) =
                hook_with_root(event, &Value::Object(input).to_string(), temp.path(), index);
            assert_eq!(code, 0);
            assert_eq!(output, b"{}\n");
            assert_eq!(records(temp.path())[0].1["state"], expected);
        }
    }

    #[test]
    fn stop_hook_captures_turn_tokens_without_content_or_cache_breakdowns() {
        let temp = TempDir::new().unwrap();
        let input = json!({
            "conversation_id":"private-conversation",
            "generation_id":"private-generation",
            "workspace_roots":[temp.path().join("workspace")],
            "model":"private-model",
            "status":"completed",
            "input_tokens":191_000,
            "output_tokens":2_345,
            "cache_read_tokens":176_000,
            "cache_write_tokens":12_000,
            "text":"SECRET RESPONSE CONTENT"
        });

        let (code, output) = hook_with_root(
            CursorHookEvent::Stop,
            &input.to_string(),
            temp.path(),
            20_000,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");

        let path = temp.path().join("usage").join("cursor-turn.json");
        let persisted = fs::read_to_string(path).unwrap();
        let record: Value = serde_json::from_str(&persisted).unwrap();
        assert_eq!(record.as_object().unwrap().len(), 3);
        assert_eq!(record["schemaVersion"], 1);
        assert_eq!(record["capturedAtMs"], 20_000);
        assert_eq!(record["totalTokens"], 193_345);
        for secret in [
            "private-conversation",
            "private-generation",
            "private-model",
            "SECRET RESPONSE CONTENT",
            "cache_read_tokens",
            "cache_write_tokens",
        ] {
            assert!(!persisted.contains(secret), "persisted {secret}");
        }
    }

    #[test]
    fn disabled_cursor_source_does_not_capture_turn_tokens() {
        let temp = TempDir::new().unwrap();
        crate::state::set_integration_source_enabled_at(
            temp.path(),
            crate::state::IntegrationSource::CursorHooks,
            false,
        )
        .unwrap();
        let input = json!({
            "conversation_id":"private-conversation",
            "workspace_roots":[temp.path().join("workspace")],
            "status":"completed",
            "input_tokens":100,
            "output_tokens":25
        });

        let (code, output) = hook_with_root(
            CursorHookEvent::Stop,
            &input.to_string(),
            temp.path(),
            20_000,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");
        assert!(!temp.path().join("usage").join("cursor-turn.json").exists());
        assert!(!temp.path().join("cursor").exists());
    }

    #[test]
    fn terminal_events_preserve_prior_model_and_agent_labels() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        let start = json!({
            "session_id":"same-session",
            "workspace_roots":[workspace],
            "model_id":"gpt-5.6-codex",
            "composer_mode":"chat"
        });
        hook_with_root(
            CursorHookEvent::SessionStart,
            &start.to_string(),
            temp.path(),
            10,
        );
        let stop = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace],
            "status":"completed"
        });
        hook_with_root(CursorHookEvent::Stop, &stop.to_string(), temp.path(), 11);
        let record = &records(temp.path())[0].1;
        assert_eq!(record["state"], "turn_finished");
        assert_eq!(record["modelName"], "gpt-5.6-codex");
        assert_eq!(record["agentKind"], "Ask");
    }

    #[test]
    fn new_prompt_clears_stale_model_but_retains_session_agent_kind() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        let first = json!({
            "session_id":"same-session",
            "workspace_roots":[workspace],
            "model_id":"first-turn-model",
            "composer_mode":"agent"
        });
        hook_with_root(
            CursorHookEvent::SessionStart,
            &first.to_string(),
            temp.path(),
            10,
        );
        let next_prompt = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace],
            "model":"default"
        });
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &next_prompt.to_string(),
            temp.path(),
            11,
        );
        let stop = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace],
            "status":"completed"
        });
        hook_with_root(CursorHookEvent::Stop, &stop.to_string(), temp.path(), 12);
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &next_prompt.to_string(),
            temp.path(),
            13,
        );
        let record = &records(temp.path())[0].1;
        assert_eq!(record["state"], "activity_detected");
        assert!(record.get("modelName").is_none());
        assert_eq!(record["agentKind"], "Agent");
    }

    #[test]
    fn before_submit_mode_is_not_treated_as_documented_session_metadata() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        let prior_prompt = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace],
            "composer_mode":"edit"
        });
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &prior_prompt.to_string(),
            temp.path(),
            10,
        );
        let next_prompt = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace]
        });
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &next_prompt.to_string(),
            temp.path(),
            11,
        );
        let record = &records(temp.path())[0].1;
        assert!(record.get("agentKind").is_none());
    }

    #[test]
    fn delayed_session_start_only_enriches_a_newer_terminal_record() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        let prompt = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace],
            "model_id":"turn-model"
        });
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &prompt.to_string(),
            temp.path(),
            20,
        );
        let stop = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace],
            "status":"completed"
        });
        hook_with_root(CursorHookEvent::Stop, &stop.to_string(), temp.path(), 21);
        let delayed_start = json!({
            "session_id":"same-session",
            "workspace_roots":[workspace],
            "model_id":"stale-session-model",
            "is_background_agent":true
        });
        hook_with_root(
            CursorHookEvent::SessionStart,
            &delayed_start.to_string(),
            temp.path(),
            30,
        );
        let record = &records(temp.path())[0].1;
        assert_eq!(record["state"], "turn_finished");
        assert_eq!(record["changedAtMs"], 21);
        assert_eq!(record["modelName"], "turn-model");
        assert_eq!(record["agentKind"], "Background agent");
    }

    #[test]
    fn delayed_session_start_never_supplies_a_stale_turn_model() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        let prompt = json!({
            "conversation_id":"same-session",
            "workspace_roots":[workspace]
        });
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &prompt.to_string(),
            temp.path(),
            20,
        );
        let delayed_start = json!({
            "session_id":"same-session",
            "workspace_roots":[workspace],
            "model_id":"session-start-model",
            "composer_mode":"ask"
        });
        hook_with_root(
            CursorHookEvent::SessionStart,
            &delayed_start.to_string(),
            temp.path(),
            30,
        );
        let record = &records(temp.path())[0].1;
        assert_eq!(record["changedAtMs"], 20);
        assert!(record.get("modelName").is_none());
        assert_eq!(record["agentKind"], "Ask");
    }

    #[test]
    fn model_and_agent_metadata_are_strictly_bounded_and_closed() {
        let payload: HookPayload = serde_json::from_value(json!({
            "model_id":"private<script>",
            "model":"safe-model",
            "model_params":[
                {"id":"thinking","value":"false"},
                {"id":"context","value":"1m; secret"},
                {"id":"effort","value":"unlimited"}
            ],
            "composer_mode":"private-mode",
            "is_background_agent":false
        }))
        .unwrap();
        assert_eq!(
            cursor_model_display(&payload).as_deref(),
            Some("safe-model")
        );
        assert_eq!(cursor_agent_kind(&payload), None);

        let background: HookPayload = serde_json::from_value(json!({
            "model_id":"x".repeat(MAX_MODEL_IDENTIFIER_BYTES + 1),
            "composer_mode":"ask",
            "is_background_agent":true
        }))
        .unwrap();
        assert_eq!(cursor_model_display(&background), None);
        assert_eq!(
            cursor_agent_kind(&background).as_deref(),
            Some("Background agent")
        );

        for sentinel in ["default", "DEFAULT", "unknown", "Unknown"] {
            let payload: HookPayload = serde_json::from_value(json!({"model":sentinel})).unwrap();
            assert_eq!(cursor_model_display(&payload), None);
        }
    }

    #[test]
    fn workspace_roots_are_absolute_local_deduplicated_and_bounded() {
        let temp = TempDir::new().unwrap();
        let valid = temp.path().join("workspace");
        fs::create_dir_all(&valid).unwrap();
        let mut roots = vec![
            valid.to_string_lossy().into_owned(),
            valid.join(".").to_string_lossy().into_owned(),
            "relative/project".to_string(),
            "file:///private/project".to_string(),
            "https://example.invalid/project".to_string(),
        ];
        roots.extend((0..MAX_WORKSPACE_PATHS).map(|index| {
            temp.path()
                .join(format!("bounded-{index}"))
                .to_string_lossy()
                .into_owned()
        }));
        let input = json!({
            "conversation_id":"session",
            "workspace_roots":roots
        });
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &input.to_string(),
            temp.path(),
            1,
        );
        // The streaming adapter retains at most the first 64 candidates; the
        // duplicate and three non-local paths reduce the persisted count.
        assert_eq!(records(temp.path()).len(), MAX_WORKSPACE_PATHS - 4);
    }

    #[cfg(windows)]
    #[test]
    fn windows_cursor_paths_convert_drive_form_and_reject_network_prefixes() {
        assert_eq!(
            cursor_windows_path("/c:/Users/Test/project").unwrap(),
            PathBuf::from("c:/Users/Test/project")
        );
        assert!(normalize_workspace_path("C:\\Users\\Test\\project").is_some());
        for raw in [
            r"\\server\share\project",
            r"\\?\UNC\server\share\project",
            r"\\.\UNC\server\share\project",
        ] {
            let path = PathBuf::from(raw);
            assert!(windows_path_is_nonlocal(&path));
            assert!(normalize_workspace_path(raw).is_none());
        }
    }

    #[test]
    fn malformed_oversized_and_unknown_inputs_fail_open() {
        let cases = [
            (CursorHookEvent::Stop, "{".to_string()),
            (
                CursorHookEvent::Stop,
                json!({
                    "conversation_id":"s",
                    "workspace_roots":["/workspace"],
                    "status":"future-status"
                })
                .to_string(),
            ),
            (
                CursorHookEvent::BeforeSubmitPrompt,
                json!({"workspace_roots":["/workspace"]}).to_string(),
            ),
            (
                CursorHookEvent::WorkspaceOpen,
                json!({
                    "workspace_roots":["relative/workspace"],
                    "user_email":"private@example.invalid"
                })
                .to_string(),
            ),
            (
                CursorHookEvent::BeforeSubmitPrompt,
                "x".repeat(MAX_HOOK_INPUT_BYTES + 1),
            ),
        ];
        for (event, input) in cases {
            let temp = TempDir::new().unwrap();
            let (code, output) = hook_with_root(event, &input, temp.path(), 20);
            assert_eq!(code, 0);
            assert_eq!(output, b"{}\n");
            assert!(!temp.path().join("cursor").exists());
        }
    }

    #[test]
    fn exact_remote_environment_value_prevents_local_persistence() {
        assert!(remote_workspace_value(Some(std::ffi::OsStr::new("true"))));
        for value in ["TRUE", "True", "1", " true", "true ", ""] {
            assert!(!remote_workspace_value(Some(std::ffi::OsStr::new(value))));
        }
        assert!(!remote_workspace_value(None));

        let temp = TempDir::new().unwrap();
        let input = json!({
            "session_id":"remote-session",
            "workspace_roots":[temp.path().join("remote-workspace")],
            "composer_mode":"agent",
            "is_background_agent":true
        })
        .to_string();
        let mut output = Vec::new();
        let code = run_cursor_hook_with(
            CursorHookEvent::SessionStart,
            input.as_bytes(),
            &mut output,
            Some(temp.path()),
            true,
            10,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");
        assert!(!temp.path().join("cursor").exists());
    }

    #[test]
    fn record_lock_serializes_competing_hook_process_writes() {
        use std::sync::mpsc;

        let temp = TempDir::new().unwrap();
        let directory = temp.path().join("cursor");
        ensure_private_directory(&directory).unwrap();
        let record_key = sha256_hex(b"record");
        let held = acquire_record_lock(&directory, &record_key).unwrap();
        let (finished_tx, finished_rx) = mpsc::channel();
        let root = temp.path().to_path_buf();
        let key = record_key.clone();
        let worker = std::thread::spawn(move || {
            let record = HookRecord {
                schema_version: SCHEMA_VERSION,
                session_key: sha256_hex(b"session"),
                cwd: "/workspace".to_string(),
                state: "activity_detected".to_string(),
                changed_at_ms: 10,
                model_name: None,
                agent_kind: None,
            };
            let result = persist_record(&root, &key, &record, CursorHookEvent::BeforeSubmitPrompt);
            finished_tx.send(result).unwrap();
        });
        assert!(matches!(
            finished_rx.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(held);
        assert!(finished_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_ok());
        worker.join().unwrap();
        assert!(directory.join(format!("{record_key}.json")).is_file());
        assert!(!directory.join(format!(".{record_key}.lock")).exists());
    }

    #[test]
    fn install_preserves_config_and_unrelated_hooks_with_one_time_backup() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        let original_value = json!({
            "version":1,
            "theme":"private-custom-theme",
            "hooks":{
                "sessionStart":[{"command":"custom-session-start"}],
                "afterFileEdit":[{"command":"format-project"}],
                "stop":[{"command":"custom-stop","timeout":30}],
                "beforeSubmitPrompt":[{"type":"prompt","prompt":"custom policy"}]
            }
        });
        write_json(&config_dir.join(HOOKS_FILENAME), &original_value);
        let original = fs::read(config_dir.join(HOOKS_FILENAME)).unwrap();

        let installed = install_cursor_integration(&config_dir, &executable).unwrap();
        assert!(installed.changed);
        assert!(!installed.migrated);
        assert!(installed.status.installed);
        assert_eq!(installed.status.usage_capture_state, "current");
        assert_eq!(installed.status.event_states["workspaceOpen"], "current");
        assert_eq!(installed.status.event_states["sessionEnd"], "current");
        let config = parse_config(&config_dir);
        assert_eq!(config["theme"], "private-custom-theme");
        assert_eq!(
            config["hooks"]["afterFileEdit"],
            original_value["hooks"]["afterFileEdit"]
        );
        assert_eq!(
            config["hooks"]["sessionStart"][0]["command"],
            "custom-session-start"
        );
        assert_eq!(config["hooks"]["stop"][0]["command"], "custom-stop");
        assert_eq!(config["hooks"]["beforeSubmitPrompt"][0]["type"], "prompt");
        for event in EVENTS {
            let entries = config["hooks"][event.config_name()].as_array().unwrap();
            let handler = entries.last().unwrap();
            assert_eq!(handler["type"], "command");
            assert_eq!(handler["timeout"], HOOK_TIMEOUT_SECONDS);
            let command = handler["command"].as_str().unwrap();
            assert!(command.contains("cursor-hook"));
            assert!(command.ends_with(event.cli_argument()));
        }
        assert_eq!(
            fs::read(config_dir.join(BACKUP_FILENAME)).unwrap(),
            original
        );
        let cli_config = parse_cli_config(&config_dir);
        assert_eq!(
            cli_config["statusLine"],
            managed_usage_status_line(&executable).unwrap()
        );
        assert_eq!(
            fs::read(config_dir.join(CLI_BACKUP_FILENAME)).unwrap(),
            b"{}\n"
        );

        let backup = fs::read(config_dir.join(BACKUP_FILENAME)).unwrap();
        let second = install_cursor_integration(&config_dir, &executable).unwrap();
        assert!(!second.changed);
        assert_eq!(fs::read(config_dir.join(BACKUP_FILENAME)).unwrap(), backup);
    }

    #[test]
    fn custom_cursor_status_line_is_reported_and_never_overwritten() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        let custom = json!({
            "type":"command",
            "command":"custom-context-renderer",
            "updateIntervalMs":5_000,
            "privateSetting":"keep-me"
        });
        write_json(
            &config_dir.join(CLI_CONFIG_FILENAME),
            &json!({"theme":"custom","statusLine":custom.clone()}),
        );
        let before = fs::read(config_dir.join(CLI_CONFIG_FILENAME)).unwrap();

        let installed = install_cursor_integration(&config_dir, &executable).unwrap();
        assert!(installed.status.installed);
        assert_eq!(installed.status.usage_capture_state, "conflict");
        assert!(installed.status.message.contains("was kept"));
        assert_eq!(
            fs::read(config_dir.join(CLI_CONFIG_FILENAME)).unwrap(),
            before
        );
        assert!(!config_dir.join(CLI_BACKUP_FILENAME).exists());

        let removed = uninstall_cursor_integration(&config_dir, &executable).unwrap();
        assert_eq!(removed.status.usage_capture_state, "conflict");
        assert_eq!(parse_cli_config(&config_dir)["statusLine"], custom);
    }

    #[test]
    fn compound_status_line_that_mentions_vsparallel_is_preserved_as_custom() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        let owned_command = managed_usage_status_line(&executable).unwrap()["command"]
            .as_str()
            .unwrap()
            .to_string();
        let custom = json!({
            "type":"command",
            "command":format!("render-custom-context && {owned_command}"),
            "padding":0,
            "updateIntervalMs":1_000,
            "timeoutMs":2_000
        });
        write_json(
            &config_dir.join(CLI_CONFIG_FILENAME),
            &json!({"statusLine":custom.clone()}),
        );
        let before = fs::read(config_dir.join(CLI_CONFIG_FILENAME)).unwrap();

        let installed = install_cursor_integration(&config_dir, &executable).unwrap();
        assert_eq!(installed.status.usage_capture_state, "conflict");
        assert_eq!(
            fs::read(config_dir.join(CLI_CONFIG_FILENAME)).unwrap(),
            before
        );
        assert!(!config_dir.join(CLI_BACKUP_FILENAME).exists());

        let removed = uninstall_cursor_integration(&config_dir, &executable).unwrap();
        assert_eq!(removed.status.usage_capture_state, "conflict");
        assert_eq!(parse_cli_config(&config_dir)["statusLine"], custom);
    }

    #[test]
    fn missing_context_capture_makes_current_hooks_repairable() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        install_cursor_integration(&config_dir, &executable).unwrap();
        write_json(
            &config_dir.join(CLI_CONFIG_FILENAME),
            &json!({"theme":"keep-me"}),
        );

        let status = cursor_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(status.state, "partial");
        assert!(!status.installed);
        assert_eq!(status.usage_capture_state, "missing");

        let repaired = install_cursor_integration(&config_dir, &executable).unwrap();
        assert!(repaired.changed);
        assert!(repaired.status.installed);
        assert_eq!(repaired.status.usage_capture_state, "current");
        assert_eq!(parse_cli_config(&config_dir)["theme"], "keep-me");
    }

    #[test]
    fn status_and_repair_add_workspace_open_to_a_four_hook_installation() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        let handlers = managed_handlers(&executable).unwrap();
        write_json(
            &config_dir.join(HOOKS_FILENAME),
            &json!({"version":1,"hooks":{
                "sessionStart":[handlers[&CursorHookEvent::SessionStart].clone()],
                "beforeSubmitPrompt":[handlers[&CursorHookEvent::BeforeSubmitPrompt].clone()],
                "stop":[handlers[&CursorHookEvent::Stop].clone()],
                "sessionEnd":[handlers[&CursorHookEvent::SessionEnd].clone()]
            }}),
        );

        let before = cursor_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(before.state, "partial");
        assert!(!before.installed);
        assert_eq!(before.event_states["workspaceOpen"], "missing");
        for name in ["sessionStart", "beforeSubmitPrompt", "stop", "sessionEnd"] {
            assert_eq!(before.event_states[name], "current");
        }

        let repaired = install_cursor_integration(&config_dir, &executable).unwrap();
        assert!(repaired.changed);
        assert!(!repaired.migrated);
        assert!(repaired.status.installed);
        assert_eq!(repaired.status.event_states["workspaceOpen"], "current");
        let config = parse_config(&config_dir);
        for event in EVENTS {
            assert_eq!(
                config["hooks"][event.config_name()],
                Value::Array(vec![handlers[&event].clone()])
            );
        }
    }

    #[test]
    fn install_repairs_historical_handlers_but_preserves_similar_custom_entries() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        let old_executable = temp.path().join(if cfg!(windows) {
            "vsparallel.exe"
        } else {
            "vsparallel"
        });
        let old_handlers = managed_handlers(&old_executable).unwrap();
        let similar = json!({
            "type":"command",
            "command":old_handlers[&CursorHookEvent::Stop]["command"],
            "timeout":2,
            "custom":true
        });
        write_json(
            &config_dir.join(HOOKS_FILENAME),
            &json!({"version":1,"hooks":{
                "sessionStart":[old_handlers[&CursorHookEvent::SessionStart].clone()],
                "beforeSubmitPrompt":[old_handlers[&CursorHookEvent::BeforeSubmitPrompt].clone()],
                "stop":[old_handlers[&CursorHookEvent::Stop].clone(), similar.clone()],
                "sessionEnd":[old_handlers[&CursorHookEvent::SessionEnd].clone()]
            }}),
        );

        let result = install_cursor_integration(&config_dir, &executable).unwrap();
        assert!(result.changed);
        assert!(result.migrated);
        assert!(result.status.installed);
        let config = parse_config(&config_dir);
        assert!(config["hooks"]["stop"]
            .as_array()
            .unwrap()
            .contains(&similar));
        assert_eq!(
            config.to_string().matches("cursor-hook").count(),
            EVENTS.len() + 1
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn historical_ownership_parses_the_exact_posix_apostrophe_escape() {
        let executable = "/tmp/owner's app/vsparallel";
        let quoted = quote_posix(executable);
        assert_eq!(
            parse_posix_single_quoted_word(&quoted).as_deref(),
            Some(executable)
        );
        let command = format!(
            "{quoted} {HOOK_ARGUMENT} {}",
            CursorHookEvent::SessionStart.cli_argument()
        );
        assert!(historical_command_targets_vsparallel(
            &command,
            CursorHookEvent::SessionStart
        ));
        assert!(!historical_command_targets_vsparallel(
            &format!("{command}; echo unsafe"),
            CursorHookEvent::SessionStart
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn usage_ownership_requires_one_exact_posix_command() {
        let executable = "/tmp/owner's app/vsparallel";
        let owned = format!("{} {USAGE_ARGUMENT}", quote_posix(executable));
        assert!(command_targets_vsparallel_usage(&owned));

        for custom in [
            format!("render-custom && {owned}"),
            format!("{} --compact {USAGE_ARGUMENT}", quote_posix(executable)),
            format!(
                "{} > /tmp/context {USAGE_ARGUMENT}",
                quote_posix(executable)
            ),
            format!("{owned} && {owned}"),
            format!("{owned} extra"),
            format!("{owned} "),
        ] {
            assert!(
                !command_targets_vsparallel_usage(&custom),
                "unexpectedly owned: {custom}"
            );
        }
    }

    #[test]
    fn uninstall_removes_only_owned_entries() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        install_cursor_integration(&config_dir, &executable).unwrap();
        let mut config = parse_config(&config_dir);
        config["hooks"]["stop"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"command":"keep-me"}));
        config["hooks"]["unrelated"] = json!([{"command":"also-keep-me"}]);
        write_json(&config_dir.join(HOOKS_FILENAME), &config);

        let removed = uninstall_cursor_integration(&config_dir, &executable).unwrap();
        assert!(removed.changed);
        assert!(!removed.status.installed);
        let remaining = parse_config(&config_dir);
        assert_eq!(remaining["hooks"]["stop"], json!([{"command":"keep-me"}]));
        assert_eq!(
            remaining["hooks"]["unrelated"],
            json!([{"command":"also-keep-me"}])
        );
        assert!(!remaining.to_string().contains("cursor-hook"));
        let cli_config = parse_cli_config(&config_dir);
        assert!(cli_config.get("statusLine").is_none());
        assert_eq!(removed.status.usage_capture_state, "missing");
    }

    #[test]
    fn malformed_cli_config_prevents_any_installation_changes() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        let original_hooks = json!({"version":1,"theme":"keep-me","hooks":{}});
        write_json(&config_dir.join(HOOKS_FILENAME), &original_hooks);
        fs::write(config_dir.join(CLI_CONFIG_FILENAME), b"not json").unwrap();
        let hooks_before = fs::read(config_dir.join(HOOKS_FILENAME)).unwrap();

        assert!(install_cursor_integration(&config_dir, &executable).is_err());
        assert_eq!(
            fs::read(config_dir.join(HOOKS_FILENAME)).unwrap(),
            hooks_before
        );
        assert!(!config_dir.join(BACKUP_FILENAME).exists());
        assert!(!config_dir.join(CLI_BACKUP_FILENAME).exists());
    }

    #[test]
    fn malformed_managed_event_prevents_partial_changes() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".cursor");
        let executable = executable(temp.path());
        write_json(
            &config_dir.join(HOOKS_FILENAME),
            &json!({"version":1,"hooks":{
                "beforeSubmitPrompt":[],
                "stop":"broken"
            }}),
        );
        let before = fs::read(config_dir.join(HOOKS_FILENAME)).unwrap();
        assert!(install_cursor_integration(&config_dir, &executable).is_err());
        assert!(uninstall_cursor_integration(&config_dir, &executable).is_err());
        assert!(cursor_integration_status(&config_dir, &executable).is_err());
        assert_eq!(fs::read(config_dir.join(HOOKS_FILENAME)).unwrap(), before);
        assert!(!config_dir.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn oversized_and_unsupported_config_are_left_unchanged() {
        let temp = TempDir::new().unwrap();
        let executable = executable(temp.path());
        for (index, bytes) in [
            vec![b' '; MAX_CONFIG_BYTES as usize + 1],
            serde_json::to_vec(&json!({"version":2,"hooks":{}})).unwrap(),
        ]
        .into_iter()
        .enumerate()
        {
            let config_dir = temp.path().join(format!("cursor-{index}"));
            fs::create_dir_all(&config_dir).unwrap();
            fs::write(config_dir.join(HOOKS_FILENAME), &bytes).unwrap();
            assert!(install_cursor_integration(&config_dir, &executable).is_err());
            assert_eq!(fs::read(config_dir.join(HOOKS_FILENAME)).unwrap(), bytes);
            assert!(!config_dir.join(BACKUP_FILENAME).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn config_and_backup_symbolic_links_are_refused() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let executable = executable(temp.path());
        let config_dir = temp.path().join("config-link");
        fs::create_dir_all(&config_dir).unwrap();
        let victim = temp.path().join("victim.json");
        fs::write(&victim, b"{\"victim\":true}\n").unwrap();
        symlink(&victim, config_dir.join(HOOKS_FILENAME)).unwrap();
        assert!(install_cursor_integration(&config_dir, &executable).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"{\"victim\":true}\n");

        let backup_dir = temp.path().join("backup-link");
        fs::create_dir_all(&backup_dir).unwrap();
        write_json(&backup_dir.join(HOOKS_FILENAME), &json!({"version":1}));
        symlink(&victim, backup_dir.join(BACKUP_FILENAME)).unwrap();
        let before = fs::read(backup_dir.join(HOOKS_FILENAME)).unwrap();
        assert!(install_cursor_integration(&backup_dir, &executable).is_err());
        assert_eq!(fs::read(backup_dir.join(HOOKS_FILENAME)).unwrap(), before);
        assert_eq!(fs::read(&victim).unwrap(), b"{\"victim\":true}\n");

        let cli_config_dir = temp.path().join("cli-config-link");
        fs::create_dir_all(&cli_config_dir).unwrap();
        write_json(
            &cli_config_dir.join(HOOKS_FILENAME),
            &json!({"version":1,"hooks":{}}),
        );
        symlink(&victim, cli_config_dir.join(CLI_CONFIG_FILENAME)).unwrap();
        let hooks_before = fs::read(cli_config_dir.join(HOOKS_FILENAME)).unwrap();
        assert!(install_cursor_integration(&cli_config_dir, &executable).is_err());
        assert_eq!(
            fs::read(cli_config_dir.join(HOOKS_FILENAME)).unwrap(),
            hooks_before
        );
        assert_eq!(fs::read(&victim).unwrap(), b"{\"victim\":true}\n");

        let cli_backup_dir = temp.path().join("cli-backup-link");
        fs::create_dir_all(&cli_backup_dir).unwrap();
        write_json(
            &cli_backup_dir.join(HOOKS_FILENAME),
            &json!({"version":1,"hooks":{}}),
        );
        symlink(&victim, cli_backup_dir.join(CLI_BACKUP_FILENAME)).unwrap();
        let hooks_before = fs::read(cli_backup_dir.join(HOOKS_FILENAME)).unwrap();
        assert!(install_cursor_integration(&cli_backup_dir, &executable).is_err());
        assert_eq!(
            fs::read(cli_backup_dir.join(HOOKS_FILENAME)).unwrap(),
            hooks_before
        );
        assert!(!cli_backup_dir.join(CLI_CONFIG_FILENAME).exists());
        assert_eq!(fs::read(&victim).unwrap(), b"{\"victim\":true}\n");
    }

    #[cfg(unix)]
    #[test]
    fn state_directory_and_record_symbolic_links_fail_open() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let root = temp.path().join("state");
        let victim_dir = temp.path().join("victim-directory");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&victim_dir).unwrap();
        symlink(&victim_dir, root.join("cursor")).unwrap();
        let input = json!({
            "conversation_id":"session",
            "workspace_roots":[temp.path().join("workspace")]
        });
        let (code, output) = hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &input.to_string(),
            &root,
            1,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");
        assert_eq!(fs::read_dir(&victim_dir).unwrap().count(), 0);

        fs::remove_file(root.join("cursor")).unwrap();
        fs::create_dir(root.join("cursor")).unwrap();
        let workspace =
            normalize_workspace_path(temp.path().join("workspace").to_string_lossy().as_ref())
                .unwrap();
        let mut identity = b"session\0".to_vec();
        identity.extend_from_slice(workspace.to_string_lossy().as_bytes());
        let target = root
            .join("cursor")
            .join(format!("{}.json", sha256_hex(&identity)));
        let victim = temp.path().join("victim-state.json");
        fs::write(&victim, b"victim\n").unwrap();
        symlink(&victim, &target).unwrap();
        let (code, output) = hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &input.to_string(),
            &root,
            2,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");
        assert_eq!(fs::read(&victim).unwrap(), b"victim\n");
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn paths_and_install_executable_are_validated_before_writes() {
        let temp = TempDir::new().unwrap();
        let executable = executable(temp.path());
        assert!(cursor_integration_status(Path::new("relative"), &executable).is_err());
        assert!(install_cursor_integration(temp.path(), &temp.path().join("missing")).is_err());
        assert!(!temp.path().join(HOOKS_FILENAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn newly_created_config_backup_and_state_are_private() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join("new-cursor");
        let executable = executable(temp.path());
        install_cursor_integration(&config_dir, &executable).unwrap();
        let config_mode = fs::metadata(config_dir.join(HOOKS_FILENAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let backup_mode = fs::metadata(config_dir.join(BACKUP_FILENAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let cli_config_mode = fs::metadata(config_dir.join(CLI_CONFIG_FILENAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let cli_backup_mode = fs::metadata(config_dir.join(CLI_BACKUP_FILENAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(config_mode, 0o600);
        assert_eq!(backup_mode, 0o600);
        assert_eq!(cli_config_mode, 0o600);
        assert_eq!(cli_backup_mode, 0o600);

        let input = json!({
            "conversation_id":"session",
            "workspace_roots":[temp.path().join("workspace")]
        });
        hook_with_root(
            CursorHookEvent::BeforeSubmitPrompt,
            &input.to_string(),
            temp.path(),
            1,
        );
        let record_mode = fs::metadata(&records(temp.path())[0].0)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(record_mode, 0o600);
    }

    #[test]
    fn sha256_matches_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
