//! Production Claude Code lifecycle integration for VSParallel.
//!
//! The desktop UI installs, inspects, and removes four documented Claude Code
//! lifecycle hooks through this module. Claude invokes the installed
//! VSParallel executable directly with the `claude-hook` argument;
//! [`run_claude_hook_stdio`] is the corresponding fail-open entry point.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::Builder as TempFileBuilder;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

const EVENTS: [&str; 4] = ["UserPromptSubmit", "Stop", "StopFailure", "SessionEnd"];
const SETTINGS_FILENAME: &str = "settings.json";
const BACKUP_FILENAME: &str = "settings.json.vsparallel.bak";
const HOOK_ARGUMENT: &str = "claude-hook";
const USAGE_ARGUMENT: &str = crate::usage::CLAUDE_STATUSLINE_ARGUMENT;
const HOOK_TIMEOUT_SECONDS: u64 = 2;
const USAGE_REFRESH_SECONDS: u64 = 60;
const SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HOOK_INPUT_BYTES: usize = 1024 * 1024;
const MAX_SESSION_ID_BYTES: usize = 16 * 1024;
const MAX_CWD_BYTES: usize = 32 * 1024;

#[cfg(windows)]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
#[cfg(windows)]
const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
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

/// Serializable status returned to the Tauri setup UI.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeIntegrationStatus {
    pub state: String,
    pub installed: bool,
    pub config_path: String,
    pub backup_path: String,
    pub event_states: BTreeMap<String, String>,
    pub usage_capture_state: String,
    pub hooks_disabled: bool,
    pub message: String,
}

/// Result of an install or uninstall request.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeIntegrationChange {
    pub changed: bool,
    pub migrated: bool,
    pub status: ClaudeIntegrationStatus,
}

/// Deliberately minimal view of Claude Code's hook payload.
///
/// Serde ignores every other field. Prompt text, assistant responses,
/// transcript paths, tool inputs, and terminal contents are never represented
/// by this module's data model and are never persisted.
#[derive(Debug, Deserialize)]
struct HookPayload {
    hook_event_name: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookRecord {
    schema_version: u32,
    session_key: String,
    cwd: String,
    state: String,
    changed_at_ms: i64,
}

/// Resolve Claude Code's configuration directory.
///
/// `CLAUDE_CONFIG_DIR` wins when present. Otherwise this returns the platform
/// user's `~/.claude` directory.
pub fn claude_config_dir_from_environment() -> Result<PathBuf, String> {
    if let Some(configured) = nonempty_env_path("CLAUDE_CONFIG_DIR") {
        return absolute_user_path(configured, "CLAUDE_CONFIG_DIR");
    }

    home_directory()
        .map(|path| path.join(".claude"))
        .map_err(|_| {
            "could not determine the Claude configuration directory; set CLAUDE_CONFIG_DIR"
                .to_string()
        })
}

/// Inspect Claude Code's settings without changing them.
pub fn claude_integration_status(
    claude_config_dir: &Path,
    executable: &Path,
) -> Result<ClaudeIntegrationStatus, String> {
    let paths = IntegrationPaths::new(claude_config_dir)?;
    let handler = managed_handler(executable)?;
    let usage_handler = managed_usage_status_line(executable)?;
    let (config, _) = read_config(&paths.config)?;
    status_from_config(&paths, &config, &handler, &usage_handler)
}

/// Install or repair VSParallel's four Claude Code lifecycle hooks.
pub fn install_claude_integration(
    claude_config_dir: &Path,
    executable: &Path,
) -> Result<ClaudeIntegrationChange, String> {
    let paths = IntegrationPaths::new(claude_config_dir)?;
    validate_install_executable(executable)?;
    let handler = managed_handler(executable)?;
    let usage_handler = managed_usage_status_line(executable)?;
    let (mut config, original) = read_config(&paths.config)?;

    let states = event_states(&config, &handler)?;
    let usage_state = usage_capture_state(&config, &usage_handler);
    let hooks_disabled = hooks_disabled(&config)?;
    if states.values().all(|state| *state == EventState::Current)
        && matches!(
            usage_state,
            UsageCaptureState::Current | UsageCaptureState::Conflict
        )
    {
        return Ok(ClaudeIntegrationChange {
            changed: false,
            migrated: false,
            status: status_from_states(&paths, states, usage_state, hooks_disabled),
        });
    }

    // Validate all managed event containers before mutating either the value
    // or the settings file. A malformed target leaves the file untouched.
    let existing = event_groups_for_all(&config)?;
    let hooks =
        hooks_map_mut(&mut config, true)?.expect("create=true always returns a hooks object");
    let mut migrated = false;

    for event in EVENTS {
        if states[event] == EventState::Current {
            continue;
        }
        let groups = existing[event].clone().unwrap_or_default();
        let (mut filtered, removed) = without_owned_handlers(groups, &handler);
        migrated |= removed;
        filtered.push(canonical_group(handler.clone()));
        hooks.insert(event.to_string(), Value::Array(filtered));
    }

    if matches!(
        usage_state,
        UsageCaptureState::Missing | UsageCaptureState::Stale
    ) {
        migrated |= usage_state == UsageCaptureState::Stale;
        config.insert("statusLine".to_string(), usage_handler);
    }

    ensure_backup(&paths.backup, &original)?;
    atomic_write_json(&paths.config, &config)?;
    let status = claude_integration_status(claude_config_dir, executable)?;
    Ok(ClaudeIntegrationChange {
        changed: true,
        migrated,
        status,
    })
}

/// Remove only VSParallel-owned Claude Code handlers.
pub fn uninstall_claude_integration(
    claude_config_dir: &Path,
    executable: &Path,
) -> Result<ClaudeIntegrationChange, String> {
    let paths = IntegrationPaths::new(claude_config_dir)?;
    let handler = managed_handler(executable)?;
    let usage_handler = managed_usage_status_line(executable)?;
    let (mut config, original) = read_config(&paths.config)?;

    // Validate first, so malformed settings cannot cause a partial uninstall.
    let existing = event_groups_for_all(&config)?;
    let mut changed = false;
    if let Some(hooks) = hooks_map_mut(&mut config, false)? {
        for event in EVENTS {
            let Some(groups) = existing[event].clone() else {
                continue;
            };
            let (filtered, removed) = without_owned_handlers(groups, &handler);
            if removed {
                hooks.insert(event.to_string(), Value::Array(filtered));
                changed = true;
            }
        }
    }

    if matches!(
        usage_capture_state(&config, &usage_handler),
        UsageCaptureState::Current | UsageCaptureState::Stale
    ) {
        config.remove("statusLine");
        changed = true;
    }

    if changed {
        ensure_backup(&paths.backup, &original)?;
        atomic_write_json(&paths.config, &config)?;
    }

    let status = status_from_config(&paths, &config, &handler, &usage_handler)?;
    Ok(ClaudeIntegrationChange {
        changed,
        migrated: false,
        status,
    })
}

/// Fail-open hook entry point used by the production binary's `claude-hook`
/// mode. Claude hook processing must never be interrupted by monitoring.
pub fn run_claude_hook_stdio() -> i32 {
    run_claude_hook(io::stdin().lock(), io::stdout().lock())
}

/// Testable hook entry point. It always returns zero and writes no output.
pub fn run_claude_hook<R: Read, W: Write>(reader: R, writer: W) -> i32 {
    let root = crate::state::state_dir_from_environment();
    run_claude_hook_with(reader, writer, root.as_deref().ok(), unix_time_ms())
}

fn run_claude_hook_with<R: Read, W: Write>(
    reader: R,
    _writer: W,
    state_root: Option<&Path>,
    changed_at_ms: i64,
) -> i32 {
    let payload =
        serde_json::from_reader::<_, HookPayload>(CappedReader::new(reader, MAX_HOOK_INPUT_BYTES));
    if let (Ok(payload), Some(root)) = (payload, state_root) {
        if let Some(record) = record_from_payload(&payload, changed_at_ms) {
            // Persistence failures are intentionally swallowed. VSParallel is
            // an observer and must never block or alter a Claude Code turn.
            if crate::state::integration_source_is_enabled_at(
                root,
                crate::state::IntegrationSource::ClaudeHooks,
            ) {
                let _ = persist_record(root, &record);
            }
        }
    }
    0
}

fn record_from_payload(payload: &HookPayload, changed_at_ms: i64) -> Option<HookRecord> {
    let state = match payload.hook_event_name.as_deref()? {
        "UserPromptSubmit" => "activity_detected",
        "Stop" => "turn_finished",
        "StopFailure" => "failed_or_interrupted",
        "SessionEnd" => "session_ended",
        _ => return None,
    };
    let session_id = payload
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= MAX_SESSION_ID_BYTES)?;
    let raw_cwd = payload
        .cwd
        .as_deref()
        .filter(|value| value.len() <= MAX_CWD_BYTES)?;
    let cwd = normalize_cwd(raw_cwd)?;

    Some(HookRecord {
        schema_version: SCHEMA_VERSION,
        session_key: sha256_hex(session_id.as_bytes()),
        cwd: cwd.to_string_lossy().into_owned(),
        state: state.to_string(),
        changed_at_ms,
    })
}

fn persist_record(root: &Path, record: &HookRecord) -> Result<(), String> {
    if !is_session_key(&record.session_key) {
        return Err("invalid Claude session key".to_string());
    }
    let directory = root.join("claude");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    set_private_directory_permissions(&directory);

    let target = directory.join(format!("{}.json", record.session_key));
    let mut bytes = serde_json::to_vec(record)
        .map_err(|error| format!("could not serialize Claude state: {error}"))?;
    bytes.push(b'\n');
    atomic_write_bytes(&target, &bytes, Some(0o600))
}

#[derive(Debug)]
struct IntegrationPaths {
    config: PathBuf,
    backup: PathBuf,
}

impl IntegrationPaths {
    fn new(claude_config_dir: &Path) -> Result<Self, String> {
        if !claude_config_dir.is_absolute() {
            return Err("the Claude configuration directory must be an absolute path".to_string());
        }
        Ok(Self {
            config: claude_config_dir.join(SETTINGS_FILENAME),
            backup: claude_config_dir.join(BACKUP_FILENAME),
        })
    }
}

fn managed_handler(executable: &Path) -> Result<Value, String> {
    if !executable.is_absolute() {
        return Err("the VSParallel hook executable must be an absolute path".to_string());
    }
    let executable = executable
        .to_str()
        .ok_or_else(|| "the VSParallel hook executable path is not valid Unicode".to_string())?;
    if executable.contains(['\0', '\n', '\r']) {
        return Err("the VSParallel hook executable path contains unsafe characters".to_string());
    }

    Ok(serde_json::json!({
        "type": "command",
        "command": executable,
        "args": [HOOK_ARGUMENT],
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

    Ok(serde_json::json!({
        "type": "command",
        "command": usage_status_line_command(executable),
        "padding": 0,
        "refreshInterval": USAGE_REFRESH_SECONDS,
    }))
}

#[cfg(not(windows))]
fn usage_status_line_command(executable: &str) -> String {
    format!("{} {USAGE_ARGUMENT}", quote_posix(executable))
}

#[cfg(not(windows))]
fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(windows)]
fn usage_status_line_command(executable: &str) -> String {
    let executable = executable.replace('\\', "/").replace('\'', "''");
    format!("powershell -NoProfile -NonInteractive -Command \"& '{executable}' {USAGE_ARGUMENT}\"")
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

fn canonical_group(handler: Value) -> Value {
    serde_json::json!({ "hooks": [handler] })
}

fn read_config(path: &Path) -> Result<(Map<String, Value>, Vec<u8>), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Map::new(), b"{}\n".to_vec()));
        }
        Err(error) => {
            return Err(format!("could not inspect {}: {error}", path.display()));
        }
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
    // Validate this setting when present because it materially changes whether
    // successfully installed hooks can run.
    hooks_disabled(&object)?;
    Ok((object, raw))
}

fn hooks_disabled(config: &Map<String, Value>) -> Result<bool, String> {
    match config.get("disableAllHooks") {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err("the top-level 'disableAllHooks' value must be a boolean".to_string()),
    }
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

fn event_groups<'a>(
    hooks: &'a Map<String, Value>,
    event: &str,
) -> Result<Option<&'a Vec<Value>>, String> {
    match hooks.get(event) {
        None => Ok(None),
        Some(Value::Array(groups)) => Ok(Some(groups)),
        Some(_) => Err(format!("hooks.{event} must be a JSON array")),
    }
}

fn event_groups_for_all(
    config: &Map<String, Value>,
) -> Result<BTreeMap<&'static str, Option<Vec<Value>>>, String> {
    let Some(hooks) = hooks_map(config)? else {
        return Ok(EVENTS.into_iter().map(|event| (event, None)).collect());
    };
    EVENTS
        .into_iter()
        .map(|event| event_groups(hooks, event).map(|groups| (event, groups.cloned())))
        .collect()
}

fn event_states(
    config: &Map<String, Value>,
    current: &Value,
) -> Result<BTreeMap<&'static str, EventState>, String> {
    let Some(hooks) = hooks_map(config)? else {
        return Ok(EVENTS
            .into_iter()
            .map(|event| (event, EventState::Missing))
            .collect());
    };
    EVENTS
        .into_iter()
        .map(|event| event_state(event_groups(hooks, event)?, current).map(|state| (event, state)))
        .collect()
}

fn event_state(groups: Option<&Vec<Value>>, current: &Value) -> Result<EventState, String> {
    let Some(groups) = groups else {
        return Ok(EventState::Missing);
    };
    let mut owned_count = 0usize;
    let mut canonical = false;
    for group in groups {
        let Some(object) = group.as_object() else {
            continue;
        };
        let Some(handlers) = object.get("hooks").and_then(Value::as_array) else {
            continue;
        };
        owned_count += handlers
            .iter()
            .filter(|candidate| is_owned_handler(candidate, current))
            .count();
        canonical |=
            object.len() == 1 && object.get("hooks") == Some(&Value::Array(vec![current.clone()]));
    }
    Ok(if owned_count == 1 && canonical {
        EventState::Current
    } else if owned_count > 0 {
        EventState::Stale
    } else {
        EventState::Missing
    })
}

fn is_owned_handler(candidate: &Value, current: &Value) -> bool {
    candidate == current || is_stale_vsparallel_handler(candidate)
}

fn is_stale_vsparallel_handler(candidate: &Value) -> bool {
    let Some(object) = candidate.as_object() else {
        return false;
    };
    const EXPECTED_KEYS: [&str; 4] = ["type", "command", "args", "timeout"];
    object.len() == EXPECTED_KEYS.len()
        && EXPECTED_KEYS.iter().all(|key| object.contains_key(*key))
        && object.get("type").and_then(Value::as_str) == Some("command")
        && object.get("args")
            == Some(&Value::Array(vec![Value::String(
                HOOK_ARGUMENT.to_string(),
            )]))
        && object.get("timeout").and_then(Value::as_u64).is_some()
        && object
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(command_targets_vsparallel)
}

fn command_targets_vsparallel(command: &str) -> bool {
    if command.contains(['\0', '\n', '\r']) {
        return false;
    }
    let path = Path::new(command);
    if !path.is_absolute() {
        return false;
    }
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "vsparallel" | "vsparallel.exe" | "vsparallel.appimage"
    ) || ((name.starts_with("vsparallel-") || name.starts_with("vsparallel_"))
        && name.ends_with(".appimage"))
}

fn without_owned_handlers(groups: Vec<Value>, current: &Value) -> (Vec<Value>, bool) {
    let mut filtered_groups = Vec::with_capacity(groups.len());
    let mut changed = false;
    for group in groups {
        let Some(object) = group.as_object() else {
            filtered_groups.push(group);
            continue;
        };
        let Some(old_handlers) = object.get("hooks").and_then(Value::as_array) else {
            filtered_groups.push(group);
            continue;
        };
        let new_handlers: Vec<_> = old_handlers
            .iter()
            .filter(|handler| !is_owned_handler(handler, current))
            .cloned()
            .collect();
        if new_handlers.len() == old_handlers.len() {
            filtered_groups.push(group);
            continue;
        }
        changed = true;
        if new_handlers.is_empty() && object.len() == 1 {
            continue;
        }
        let mut updated = object.clone();
        updated.insert("hooks".to_string(), Value::Array(new_handlers));
        filtered_groups.push(Value::Object(updated));
    }
    (filtered_groups, changed)
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
    const ALLOWED_KEYS: [&str; 4] = ["type", "command", "padding", "refreshInterval"];
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
        || command.len() > MAX_CWD_BYTES
        || command.contains(['\0', '\n', '\r', '|', ';'])
        || !command.contains(USAGE_ARGUMENT)
    {
        return false;
    }

    let lower = command.to_ascii_lowercase();
    let targets_binary = [
        "/vsparallel ",
        "/vsparallel' ",
        "/vsparallel\" ",
        "/vsparallel.exe ",
        "/vsparallel.exe' ",
        "/vsparallel.exe\" ",
        "/vsparallel.appimage ",
        "/vsparallel.appimage' ",
        "/vsparallel.appimage\" ",
        "\\vsparallel.exe' ",
        "\\vsparallel.exe\" ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || lower.contains("/vsparallel-")
        || lower.contains("/vsparallel_");
    targets_binary && lower.trim_end_matches('"').ends_with(USAGE_ARGUMENT)
}

fn status_from_config(
    paths: &IntegrationPaths,
    config: &Map<String, Value>,
    handler: &Value,
    usage_handler: &Value,
) -> Result<ClaudeIntegrationStatus, String> {
    Ok(status_from_states(
        paths,
        event_states(config, handler)?,
        usage_capture_state(config, usage_handler),
        hooks_disabled(config)?,
    ))
}

fn status_from_states(
    paths: &IntegrationPaths,
    states: BTreeMap<&'static str, EventState>,
    usage_state: UsageCaptureState,
    hooks_disabled: bool,
) -> ClaudeIntegrationStatus {
    let current = states
        .values()
        .filter(|state| **state == EventState::Current)
        .count();
    let stale = states
        .values()
        .filter(|state| **state == EventState::Stale)
        .count();
    let state = if current == EVENTS.len() {
        "installed"
    } else if current == 0 && stale == 0 {
        "not_installed"
    } else if stale > 0 && current == 0 {
        "stale"
    } else {
        "partial"
    };
    let mut message = match state {
        "installed" => "Claude Code activity monitoring is installed.".to_string(),
        "not_installed" => "Claude Code activity monitoring is not installed.".to_string(),
        "stale" => {
            "An older VSParallel Claude Code integration was found and can be migrated.".to_string()
        }
        _ => "Claude Code activity monitoring is only partially installed.".to_string(),
    };
    match usage_state {
        UsageCaptureState::Current => {
            message.push_str(
                " The terminal usage fallback is installed; live usage is checked separately.",
            );
        }
        UsageCaptureState::Stale => {
            message.push_str(" The terminal usage fallback needs repair.");
        }
        UsageCaptureState::Missing => {
            message.push_str(
                " The terminal usage fallback is not installed; live CLI usage does not require it.",
            );
        }
        UsageCaptureState::Conflict => {
            message.push_str(
                " An existing custom Claude Code status line was kept. Live CLI usage may still be available; terminal fallback capture is disabled.",
            );
        }
    }
    if hooks_disabled {
        message.push_str(
            " Claude Code's disableAllHooks setting is true, so no lifecycle activity can be received.",
        );
    }

    ClaudeIntegrationStatus {
        state: state.to_string(),
        installed: state == "installed",
        config_path: paths.config.to_string_lossy().into_owned(),
        backup_path: paths.backup.to_string_lossy().into_owned(),
        event_states: states
            .into_iter()
            .map(|(event, state)| (event.to_string(), state.as_str().to_string()))
            .collect(),
        usage_capture_state: usage_state.as_str().to_string(),
        hooks_disabled,
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
        Err(error) => {
            return Err(format!("could not inspect {}: {error}", path.display()));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    set_private_directory_permissions(parent);
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

fn atomic_write_json(path: &Path, config: &Map<String, Value>) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("could not serialize Claude Code settings: {error}"))?;
    bytes.push(b'\n');
    let mode = existing_mode(path).unwrap_or(0o600);
    atomic_write_bytes(path, &bytes, Some(mode))
}

fn atomic_write_bytes(path: &Path, content: &[u8], mode: Option<u32>) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    set_private_directory_permissions(parent);
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

/// Refuse to overwrite links or non-files. This is checked immediately before
/// replacement in addition to the settings parser's earlier validation, so a
/// stale or malicious reparse target is never followed by the write path.
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
    // Close the temporary file before replacement. The TempPath still owns
    // cleanup on failure; after a successful move its old path no longer
    // exists, so dropping it cannot remove the destination.
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
        if let Ok(directory) = fs::File::open(path) {
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

fn absolute_user_path(path: PathBuf, source: &str) -> Result<PathBuf, String> {
    let expanded = if path == Path::new("~") {
        home_directory()?
    } else if let Ok(suffix) = path.strip_prefix("~") {
        home_directory()?.join(suffix)
    } else {
        path
    };
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(expanded))
            .map_err(|error| format!("could not resolve {source}: {error}"))
    }
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
    home.ok_or_else(|| "could not determine the platform home directory".to_string())
}

fn normalize_cwd(raw: &str) -> Option<PathBuf> {
    if raw.trim().is_empty() || raw.contains('\0') {
        return None;
    }
    let mut path = PathBuf::from(raw.trim());
    if path == Path::new("~") || path.starts_with("~/") || path.starts_with("~\\") {
        path = absolute_user_path(path, "hook cwd").ok()?;
    } else if !path.is_absolute() {
        path = env::current_dir().ok()?.join(path);
    }
    if let Ok(canonical) = fs::canonicalize(&path) {
        return Some(canonical);
    }
    lexical_normalize_absolute(&path)
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

fn is_session_key(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A read adapter that returns an error if its input exceeds a strict byte
/// limit. This lets serde stream only the three selected fields without first
/// buffering an unbounded, potentially sensitive payload.
struct CappedReader<R> {
    inner: R,
    remaining: usize,
}

impl<R> CappedReader<R> {
    fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            remaining: limit,
        }
    }
}

impl<R: Read> Read for CappedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Claude hook payload exceeds the safety limit",
                )),
            };
        }
        let allowed = buffer.len().min(self.remaining);
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.remaining -= read;
        Ok(read)
    }
}

// Small dependency-free SHA-256 implementation. The digest pseudonymizes a
// session identifier before it can become a file name or persisted field.
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
        let executable = root.join("VSParallel app").join("vsparallel");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        if !executable.exists() {
            fs::write(&executable, b"test executable").unwrap();
        }
        executable
    }

    fn write_json(path: &Path, value: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn parse_config(home: &Path) -> Value {
        serde_json::from_slice(&fs::read(home.join(SETTINGS_FILENAME)).unwrap()).unwrap()
    }

    fn hook_with_root(input: &str, root: &Path, now: i64) -> (i32, Vec<u8>) {
        let mut output = Vec::new();
        let code = run_claude_hook_with(input.as_bytes(), &mut output, Some(root), now);
        (code, output)
    }

    fn only_record(root: &Path) -> Value {
        let entries: Vec<_> = fs::read_dir(root.join("claude"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 1);
        serde_json::from_slice(&fs::read(&entries[0]).unwrap()).unwrap()
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn environment_config_directory_is_absolute_when_available() {
        let directory = claude_config_dir_from_environment().unwrap();
        assert!(directory.is_absolute());
    }

    #[test]
    fn atomic_write_replaces_an_existing_regular_file() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("state.json");
        fs::write(&target, b"old\n").unwrap();

        atomic_write_bytes(&target, b"new\n", Some(0o600)).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new\n");
        let leftovers: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_refuses_a_symbolic_link_target() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let victim = temp.path().join("victim.json");
        let target = temp.path().join("state.json");
        fs::write(&victim, b"private\n").unwrap();
        symlink(&victim, &target).unwrap();

        let result = atomic_write_bytes(&target, b"replacement\n", Some(0o600));

        assert!(result.is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"private\n");
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn hook_persists_only_five_privacy_safe_fields() {
        let temp = TempDir::new().unwrap();
        let cwd = temp.path().join("project");
        fs::create_dir(&cwd).unwrap();
        let input = json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "private-session-id",
            "cwd": cwd,
            "prompt": "SECRET PROMPT",
            "last_assistant_message": "SECRET ANSWER",
            "transcript_path": "/secret/transcript",
            "tool_input": {"command": "SECRET TERMINAL"}
        })
        .to_string();
        let (code, output) = hook_with_root(&input, temp.path(), 1_700_000_000_123);
        assert_eq!(code, 0);
        assert!(output.is_empty());

        let record = only_record(temp.path());
        assert_eq!(record.as_object().unwrap().len(), 5);
        assert_eq!(record["schemaVersion"], 1);
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["changedAtMs"], 1_700_000_000_123i64);
        assert_eq!(record["sessionKey"].as_str().unwrap().len(), 64);
        let saved = fs::read_to_string(
            fs::read_dir(temp.path().join("claude"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(!saved.contains("SECRET"));
        assert!(!saved.contains("private-session-id"));
        assert!(!saved.contains("transcript"));
    }

    #[test]
    fn all_four_lifecycle_events_map_to_expected_states() {
        let cases = [
            ("UserPromptSubmit", "activity_detected"),
            ("Stop", "turn_finished"),
            ("StopFailure", "failed_or_interrupted"),
            ("SessionEnd", "session_ended"),
        ];
        for (index, (event, expected)) in cases.into_iter().enumerate() {
            let temp = TempDir::new().unwrap();
            let cwd = temp.path().join("project");
            fs::create_dir(&cwd).unwrap();
            let input = json!({
                "hook_event_name": event,
                "session_id": format!("session-{index}"),
                "cwd": cwd,
                "prompt": "not retained",
                "last_assistant_message": "not retained"
            })
            .to_string();
            let (code, output) = hook_with_root(&input, temp.path(), index as i64);
            assert_eq!(code, 0);
            assert!(output.is_empty());
            assert_eq!(only_record(temp.path())["state"], expected);
        }
    }

    #[test]
    fn malformed_oversized_and_unknown_hook_input_fail_open() {
        let temp = TempDir::new().unwrap();
        for input in [
            "not json".to_string(),
            json!({"hook_event_name":"Unknown","session_id":"s","cwd":temp.path()}).to_string(),
            json!({"hook_event_name":"Stop","cwd":temp.path()}).to_string(),
            format!("{{\"padding\":\"{}\"}}", "x".repeat(MAX_HOOK_INPUT_BYTES)),
        ] {
            let (code, output) = hook_with_root(&input, temp.path(), 20);
            assert_eq!(code, 0);
            assert!(output.is_empty());
        }
        assert!(!temp.path().join("claude").exists());
    }

    #[test]
    fn install_preserves_settings_and_unrelated_hooks_with_one_time_backup() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let config = home.join(SETTINGS_FILENAME);
        write_json(
            &config,
            &json!({
                "model": "claude-test",
                "permissions": {"allow": ["Read"]},
                "hooks": {
                    "Notification": [{"matcher":"idle_prompt", "hooks":[{"type":"command","command":"notify"}]}],
                    "Stop": [{"matcher":"preserve", "hooks":[{"type":"command","command":"other"}]}]
                }
            }),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());
        let result = install_claude_integration(&home, &executable).unwrap();
        assert!(result.changed);
        assert!(!result.migrated);
        assert!(result.status.installed);
        assert_eq!(result.status.usage_capture_state, "current");
        assert_eq!(fs::read(home.join(BACKUP_FILENAME)).unwrap(), original);

        let installed = parse_config(&home);
        assert_eq!(installed["model"], "claude-test");
        assert_eq!(installed["permissions"]["allow"][0], "Read");
        assert_eq!(
            installed["hooks"]["Notification"][0]["hooks"][0]["command"],
            "notify"
        );
        assert_eq!(installed["hooks"]["Stop"][0]["matcher"], "preserve");
        for event in EVENTS {
            let groups = installed["hooks"][event].as_array().unwrap();
            let handler = &groups.last().unwrap()["hooks"][0];
            assert_eq!(handler["type"], "command");
            assert_eq!(handler["command"], executable.to_string_lossy().as_ref());
            assert_eq!(handler["args"], json!(["claude-hook"]));
            assert_eq!(handler["timeout"], 2);
            assert_eq!(handler.as_object().unwrap().len(), 4);
        }
        assert_eq!(installed["statusLine"]["type"], "command");
        assert!(installed["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("claude-usage"));
        assert_eq!(installed["statusLine"]["padding"], 0);
        assert_eq!(installed["statusLine"]["refreshInterval"], 60);

        let backup = fs::read(home.join(BACKUP_FILENAME)).unwrap();
        let second = install_claude_integration(&home, &executable).unwrap();
        assert!(!second.changed);
        assert_eq!(fs::read(home.join(BACKUP_FILENAME)).unwrap(), backup);
    }

    #[test]
    fn stale_vsparallel_handlers_are_migrated_conservatively() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let old_executable = temp.path().join("old install").join("vsparallel");
        let similar_executable = temp.path().join("similar install").join("vsparallel");
        let old = json!({
            "type": "command",
            "command": old_executable,
            "args": ["claude-hook"],
            "timeout": 5
        });
        let similar_but_not_owned = json!({
            "type": "command",
            "command": similar_executable,
            "args": ["different-mode"],
            "timeout": 2
        });
        write_json(
            &home.join(SETTINGS_FILENAME),
            &json!({"hooks": {
                "UserPromptSubmit": [{"hooks": [old.clone(), similar_but_not_owned.clone()]}],
                "Stop": [{"hooks": [old.clone()]}],
                "StopFailure": [{"hooks": [old.clone()]}],
                "SessionEnd": [{"hooks": [old]}]
            }}),
        );
        let executable = executable(temp.path());
        let before = claude_integration_status(&home, &executable).unwrap();
        assert_eq!(before.state, "stale");
        let result = install_claude_integration(&home, &executable).unwrap();
        assert!(result.changed);
        assert!(result.migrated);
        assert!(result.status.installed);

        let installed = parse_config(&home);
        let text = installed.to_string();
        let old_command = old_executable.to_string_lossy();
        for event in EVENTS {
            assert!(installed["hooks"][event]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| group["hooks"].as_array().unwrap())
                .all(|handler| handler["command"].as_str() != Some(old_command.as_ref())));
        }
        assert!(!text.contains("\"timeout\":5"));
        assert!(text.contains("different-mode"));
    }

    #[test]
    fn custom_status_line_is_preserved_without_claiming_usage_capture() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let custom = json!({
            "type": "command",
            "command": "/usr/local/bin/my-status",
            "padding": 2
        });
        write_json(
            &home.join(SETTINGS_FILENAME),
            &json!({"statusLine": custom}),
        );
        let executable = executable(temp.path());

        let installed = install_claude_integration(&home, &executable).unwrap();
        assert!(installed.changed);
        assert!(installed.status.installed);
        assert_eq!(installed.status.usage_capture_state, "conflict");
        assert!(installed
            .status
            .message
            .contains("Live CLI usage may still be available"));
        assert_eq!(parse_config(&home)["statusLine"], custom);

        let second = install_claude_integration(&home, &executable).unwrap();
        assert!(!second.changed);
        assert_eq!(parse_config(&home)["statusLine"], custom);

        let removed = uninstall_claude_integration(&home, &executable).unwrap();
        assert!(removed.changed);
        assert_eq!(parse_config(&home)["statusLine"], custom);
    }

    #[test]
    fn stale_owned_usage_status_line_is_repaired() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let old_executable = temp.path().join("old install").join("vsparallel");
        let mut stale = managed_usage_status_line(&old_executable).unwrap();
        stale["refreshInterval"] = json!(30);
        write_json(&home.join(SETTINGS_FILENAME), &json!({"statusLine": stale}));
        let executable = executable(temp.path());
        let result = install_claude_integration(&home, &executable).unwrap();
        assert!(result.changed);
        assert!(result.migrated);
        assert_eq!(result.status.usage_capture_state, "current");
        assert_eq!(
            parse_config(&home)["statusLine"],
            managed_usage_status_line(&executable).unwrap()
        );
    }

    #[test]
    fn handlers_with_extra_fields_or_non_vsparallel_commands_are_not_owned() {
        let temp = TempDir::new().unwrap();
        let current = managed_handler(&executable(temp.path())).unwrap();
        let old_executable = temp.path().join("old install").join("vsparallel");
        let other_executable = temp.path().join("other install").join("monitor");
        let extra = json!({
            "type":"command", "command":old_executable,
            "args":["claude-hook"], "timeout":2, "extra":true
        });
        let other = json!({
            "type":"command", "command":other_executable,
            "args":["claude-hook"], "timeout":2
        });
        assert!(!is_owned_handler(&extra, &current));
        assert!(!is_owned_handler(&other, &current));
        assert!(is_owned_handler(&current, &current));
    }

    #[test]
    fn uninstall_removes_only_owned_handlers() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let executable = executable(temp.path());
        install_claude_integration(&home, &executable).unwrap();
        let backup_before = fs::read(home.join(BACKUP_FILENAME)).unwrap();

        let mut config = parse_config(&home);
        config["hooks"]["Stop"].as_array_mut().unwrap().insert(
            0,
            json!({"matcher":"keep", "hooks":[{"type":"command","command":"other"}]}),
        );
        write_json(&home.join(SETTINGS_FILENAME), &config);

        let result = uninstall_claude_integration(&home, &executable).unwrap();
        assert!(result.changed);
        assert!(!result.status.installed);
        assert_eq!(fs::read(home.join(BACKUP_FILENAME)).unwrap(), backup_before);
        let remaining = parse_config(&home);
        assert!(remaining.get("statusLine").is_none());
        assert_eq!(remaining["hooks"]["Stop"][0]["matcher"], "keep");
        assert_eq!(
            remaining["hooks"]["Stop"][0]["hooks"][0]["command"],
            "other"
        );
        assert!(!remaining.to_string().contains("claude-hook"));
    }

    #[test]
    fn malformed_settings_are_never_modified() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        fs::create_dir_all(&home).unwrap();
        let config = home.join(SETTINGS_FILENAME);
        let executable = executable(temp.path());

        for malformed in [
            b"{not json".to_vec(),
            serde_json::to_vec(&json!([])).unwrap(),
            serde_json::to_vec(&json!({"hooks": []})).unwrap(),
            serde_json::to_vec(&json!({"disableAllHooks": "yes"})).unwrap(),
        ] {
            fs::write(&config, &malformed).unwrap();
            assert!(install_claude_integration(&home, &executable).is_err());
            assert_eq!(fs::read(&config).unwrap(), malformed);
            assert!(!home.join(BACKUP_FILENAME).exists());
        }
    }

    #[test]
    fn malformed_managed_event_prevents_partial_install_or_uninstall() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let config = home.join(SETTINGS_FILENAME);
        let executable = executable(temp.path());
        write_json(
            &config,
            &json!({"hooks": {"UserPromptSubmit": [], "Stop": "broken"}}),
        );
        let before = fs::read(&config).unwrap();
        assert!(install_claude_integration(&home, &executable).is_err());
        assert!(uninstall_claude_integration(&home, &executable).is_err());
        assert_eq!(fs::read(&config).unwrap(), before);
        assert!(!home.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn status_reports_partial_and_disable_all_hooks() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let executable = executable(temp.path());
        let handler = managed_handler(&executable).unwrap();
        write_json(
            &home.join(SETTINGS_FILENAME),
            &json!({"disableAllHooks": true, "hooks": {
                "Stop": [{"hooks": [handler]}]
            }}),
        );
        let status = claude_integration_status(&home, &executable).unwrap();
        assert_eq!(status.state, "partial");
        assert!(!status.installed);
        assert!(status.hooks_disabled);
        assert_eq!(status.event_states["Stop"], "current");
        assert_eq!(status.event_states["StopFailure"], "missing");
        assert_eq!(status.usage_capture_state, "missing");
        assert!(status.message.contains("disableAllHooks"));
    }

    #[test]
    fn oversized_config_is_rejected_without_changes() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        fs::create_dir_all(&home).unwrap();
        let config = home.join(SETTINGS_FILENAME);
        let bytes = vec![b' '; MAX_CONFIG_BYTES as usize + 1];
        fs::write(&config, &bytes).unwrap();
        let executable = executable(temp.path());
        assert!(install_claude_integration(&home, &executable).is_err());
        assert_eq!(fs::metadata(&config).unwrap().len(), MAX_CONFIG_BYTES + 1);
        assert!(!home.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn executable_and_config_directory_must_be_absolute() {
        let temp = TempDir::new().unwrap();
        assert!(
            claude_integration_status(Path::new("relative"), &executable(temp.path())).is_err()
        );
        assert!(claude_integration_status(temp.path(), Path::new("relative")).is_err());
        assert!(install_claude_integration(temp.path(), &temp.path().join("missing")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn new_settings_backup_and_state_are_private() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("claude");
        let executable = executable(temp.path());
        install_claude_integration(&home, &executable).unwrap();
        assert_eq!(
            fs::metadata(home.join(SETTINGS_FILENAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(home.join(BACKUP_FILENAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let cwd = temp.path().join("project");
        fs::create_dir(&cwd).unwrap();
        hook_with_root(
            &json!({"hook_event_name":"Stop", "session_id":"s", "cwd":cwd}).to_string(),
            temp.path(),
            10,
        );
        let state_file = fs::read_dir(temp.path().join("claude"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::metadata(state_file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
