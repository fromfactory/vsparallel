//! Production Codex integration for VSParallel.
//!
//! The desktop UI uses the install/status/uninstall functions in this module.
//! Codex invokes the same installed executable with the `codex-hook` argument;
//! [`run_codex_hook_stdio`] is the corresponding fail-open entry point.

use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::{Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::Builder as TempFileBuilder;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

const EVENTS: [&str; 3] = ["UserPromptSubmit", "Stop", "SessionEnd"];
const HOOKS_FILENAME: &str = "hooks.json";
const BACKUP_FILENAME: &str = "hooks.json.vsparallel.bak";
const HOOK_TIMEOUT_SECONDS: u64 = 2;
const MANAGED_STATUS_MESSAGE: &str = "VSParallel: recording coarse local activity";
const SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HOOK_INPUT_BYTES: usize = 1024 * 1024;
const MAX_SESSION_ID_BYTES: usize = 16 * 1024;
const MAX_CWD_BYTES: usize = 32 * 1024;
const MAX_EVENT_NAME_BYTES: usize = 64;
const LEGACY_SCRIPT_SUFFIX: &str = "/integrations/codex/vsparallel_hook.py";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventState {
    Current,
    Stale,
    Missing,
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
pub struct CodexIntegrationStatus {
    pub state: String,
    pub installed: bool,
    pub config_path: String,
    pub backup_path: String,
    pub event_states: BTreeMap<String, String>,
    pub message: String,
}

/// Result of an install or uninstall request.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexIntegrationChange {
    pub changed: bool,
    pub migrated: bool,
    pub status: CodexIntegrationStatus,
}

/// Codex's runtime decision for the exact three handlers VSParallel owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexHookReviewStatus {
    Trusted,
    ReviewRequired,
}

#[derive(Debug, Default)]
struct HookPayload {
    hook_event_name: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    // All other input fields are deliberately ignored by serde while streaming.
    // Prompt, response, transcript, and tool content are never represented in
    // the in-memory model and can therefore never be serialized by this module.
}

/// Mutates the small payload model as fields are encountered. Keeping this
/// state outside serde's return value lets an oversized Stop request still
/// receive fail-open `{}` output even when parsing later sensitive fields is
/// deliberately aborted at the byte cap.
struct HookPayloadSeed<'a> {
    payload: &'a mut HookPayload,
}

impl<'de> DeserializeSeed<'de> for HookPayloadSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(HookPayloadVisitor {
            payload: self.payload,
        })
    }
}

struct HookPayloadVisitor<'a> {
    payload: &'a mut HookPayload,
}

impl<'de> Visitor<'de> for HookPayloadVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Codex hook JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut event_seen = false;
        let mut session_seen = false;
        let mut cwd_seen = false;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "hook_event_name" => {
                    if event_seen {
                        return Err(serde::de::Error::duplicate_field("hook_event_name"));
                    }
                    event_seen = true;
                    let value: Option<String> = map.next_value()?;
                    if value
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_EVENT_NAME_BYTES)
                    {
                        return Err(serde::de::Error::custom(
                            "Codex hook event name exceeds the safety limit",
                        ));
                    }
                    self.payload.hook_event_name = value;
                }
                "session_id" => {
                    if session_seen {
                        return Err(serde::de::Error::duplicate_field("session_id"));
                    }
                    session_seen = true;
                    let value: Option<String> = map.next_value()?;
                    if value
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_SESSION_ID_BYTES)
                    {
                        return Err(serde::de::Error::custom(
                            "Codex session identifier exceeds the safety limit",
                        ));
                    }
                    self.payload.session_id = value;
                }
                "cwd" => {
                    if cwd_seen {
                        return Err(serde::de::Error::duplicate_field("cwd"));
                    }
                    cwd_seen = true;
                    let value: Option<String> = map.next_value()?;
                    if value
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_CWD_BYTES)
                    {
                        return Err(serde::de::Error::custom(
                            "Codex working directory exceeds the safety limit",
                        ));
                    }
                    self.payload.cwd = value;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
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

/// Resolve the user Codex directory (`CODEX_HOME`, then the platform home).
pub fn codex_home_from_environment() -> Result<PathBuf, String> {
    if let Some(configured) = nonempty_env_path("CODEX_HOME") {
        return absolute_user_path(configured);
    }

    #[cfg(target_os = "windows")]
    let home = nonempty_env_path("USERPROFILE").or_else(|| {
        let drive = env::var_os("HOMEDRIVE")?;
        let path = env::var_os("HOMEPATH")?;
        Some(PathBuf::from(drive).join(path))
    });

    #[cfg(not(target_os = "windows"))]
    let home = nonempty_env_path("HOME");

    home.map(|path| path.join(".codex"))
        .ok_or_else(|| "could not determine the Codex home directory; set CODEX_HOME".to_string())
}

/// Inspect the hooks configuration without changing it.
pub fn codex_integration_status(
    codex_home: &Path,
    executable: &Path,
) -> Result<CodexIntegrationStatus, String> {
    let paths = IntegrationPaths::new(codex_home)?;
    let handler = managed_handler(executable)?;
    let (config, _) = read_config(&paths.config)?;
    status_from_config(&paths, &config, &handler)
}

/// Ask Codex whether the installed VSParallel handlers are enabled and trusted.
///
/// Trust is recorded against Codex's own handler hash, so VSParallel uses the
/// app-server's `hooks/list` result instead of duplicating that private hash.
pub fn codex_hook_review_status(
    codex_home: &Path,
    executable: &Path,
    codex_command: &OsStr,
    allow_extension_fallback: bool,
) -> Result<CodexHookReviewStatus, String> {
    let paths = IntegrationPaths::new(codex_home)?;
    let handler = managed_handler(executable)?;
    let cwd = codex_home
        .to_str()
        .ok_or_else(|| "the Codex home path is not valid Unicode".to_string())?;
    let params = serde_json::json!({"cwds": [cwd]});
    let result = if allow_extension_fallback {
        crate::usage::codex_app_server_request_resolved(codex_command, "hooks/list", params)?
    } else {
        crate::usage::codex_app_server_request(codex_command, "hooks/list", params)?
    };
    hook_review_status_from_response(&paths, &handler, &result)
}

/// Install or migrate VSParallel's three lifecycle hooks.
pub fn install_codex_integration(
    codex_home: &Path,
    executable: &Path,
) -> Result<CodexIntegrationChange, String> {
    let paths = IntegrationPaths::new(codex_home)?;
    validate_install_executable(executable)?;
    let handler = managed_handler(executable)?;
    let (mut config, original) = read_config(&paths.config)?;

    let states = event_states(&config, &handler)?;
    if states.values().all(|state| *state == EventState::Current) {
        return Ok(CodexIntegrationChange {
            changed: false,
            migrated: false,
            status: status_from_states(&paths, states)?,
        });
    }

    // Validate all target event containers before mutating the value or disk.
    let existing = event_groups_for_all(&config)?;
    let hooks =
        hooks_map_mut(&mut config, true)?.expect("create=true always returns a hooks object");
    let mut migrated = false;

    for event in EVENTS {
        if states[event] == EventState::Current {
            continue;
        }
        let groups = existing[event].clone().unwrap_or_default();
        let (mut filtered, removed) = without_owned_handlers(groups);
        migrated |= removed;
        filtered.push(canonical_group(handler.clone()));
        hooks.insert(event.to_string(), Value::Array(filtered));
    }

    ensure_backup(&paths.backup, &original)?;
    atomic_write_json(&paths.config, &config)?;
    let status = codex_integration_status(codex_home, executable)?;
    Ok(CodexIntegrationChange {
        changed: true,
        migrated,
        status,
    })
}

/// Remove only handlers owned by VSParallel, preserving all unrelated config.
pub fn uninstall_codex_integration(
    codex_home: &Path,
    executable: &Path,
) -> Result<CodexIntegrationChange, String> {
    let paths = IntegrationPaths::new(codex_home)?;
    let handler = managed_handler(executable)?;
    let (mut config, original) = read_config(&paths.config)?;

    // Validate target containers first, so malformed input never yields a
    // partial uninstall.
    let existing = event_groups_for_all(&config)?;
    let mut changed = false;
    if let Some(hooks) = hooks_map_mut(&mut config, false)? {
        for event in EVENTS {
            let Some(groups) = existing[event].clone() else {
                continue;
            };
            let (filtered, removed) = without_owned_handlers(groups);
            if removed {
                hooks.insert(event.to_string(), Value::Array(filtered));
                changed = true;
            }
        }
    }

    if changed {
        ensure_backup(&paths.backup, &original)?;
        atomic_write_json(&paths.config, &config)?;
    }

    let status = status_from_config(&paths, &config, &handler)?;
    Ok(CodexIntegrationChange {
        changed,
        migrated: false,
        status,
    })
}

/// Fail-open hook entry point used by the production binary's `codex-hook` mode.
///
/// It always returns zero. A Stop hook gets `{}` on stdout even if persistence
/// fails, because non-empty Stop output must be a valid JSON object.
pub fn run_codex_hook_stdio() -> i32 {
    run_codex_hook(io::stdin().lock(), io::stdout().lock())
}

/// Testable streaming hook entry point. Production callers normally use
/// [`run_codex_hook_stdio`].
pub fn run_codex_hook<R: Read, W: Write>(reader: R, writer: W) -> i32 {
    let root = crate::state::state_dir_from_environment();
    run_codex_hook_with(reader, writer, root.as_deref(), unix_time_ms())
}

fn run_codex_hook_with<R: Read, W: Write>(
    reader: R,
    mut writer: W,
    state_root: Result<&Path, &String>,
    changed_at_ms: i64,
) -> i32 {
    let capped = CappedReader::new(reader, MAX_HOOK_INPUT_BYTES);
    let mut deserializer = serde_json::Deserializer::from_reader(capped);
    let mut payload = HookPayload::default();
    let parsed = HookPayloadSeed {
        payload: &mut payload,
    }
    .deserialize(&mut deserializer)
    .and_then(|()| deserializer.end());
    let is_stop = payload.hook_event_name.as_deref() == Some("Stop");

    if parsed.is_ok() {
        if let (Ok(root), Some(record)) = (state_root, record_from_payload(&payload, changed_at_ms))
        {
            // Persistence errors are intentionally swallowed. Monitoring must
            // never interrupt the user's Codex workflow.
            if crate::state::integration_source_is_enabled_at(
                root,
                crate::state::IntegrationSource::CodexHooks,
            ) {
                let _ = persist_record(root, &record);
            }
        }
    }

    if is_stop {
        let _ = writer.write_all(b"{}\n");
        let _ = writer.flush();
    }
    0
}

fn record_from_payload(payload: &HookPayload, changed_at_ms: i64) -> Option<HookRecord> {
    let state = match payload.hook_event_name.as_deref()? {
        "UserPromptSubmit" => "activity_detected",
        "Stop" => "turn_finished",
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
        return Err("invalid session key".to_string());
    }
    let directory = root.join("codex");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    set_private_directory_permissions(&directory);

    let target = directory.join(format!("{}.json", record.session_key));
    let mut bytes = serde_json::to_vec(record)
        .map_err(|error| format!("could not serialize Codex state: {error}"))?;
    bytes.push(b'\n');
    atomic_write_bytes(&target, &bytes, Some(0o600))
}

#[derive(Debug)]
struct IntegrationPaths {
    config: PathBuf,
    backup: PathBuf,
}

impl IntegrationPaths {
    fn new(codex_home: &Path) -> Result<Self, String> {
        if !codex_home.is_absolute() {
            return Err("the Codex home directory must be an absolute path".to_string());
        }
        Ok(Self {
            config: codex_home.join(HOOKS_FILENAME),
            backup: codex_home.join(BACKUP_FILENAME),
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
        "command": format!("{} codex-hook", quote_posix(executable)),
        "commandWindows": format!("{} codex-hook", quote_windows(executable)),
        "timeout": HOOK_TIMEOUT_SECONDS,
        "statusMessage": MANAGED_STATUS_MESSAGE,
    }))
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

fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

// CommandLineToArgvW-compatible quoting, matching Python's list2cmdline for a
// single argument. The argument is always quoted, including paths without spaces.
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
    Ok((object, raw))
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
            .filter(|candidate| is_owned_handler(candidate))
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

fn is_managed_handler(candidate: &Value) -> bool {
    candidate.as_object().is_some_and(|object| {
        object.get("type").and_then(Value::as_str) == Some("command")
            && object.get("statusMessage").and_then(Value::as_str) == Some(MANAGED_STATUS_MESSAGE)
    })
}

fn is_legacy_handler(candidate: &Value) -> bool {
    let Some(object) = candidate.as_object() else {
        return false;
    };
    let expected_keys = ["type", "command", "commandWindows", "timeout"];
    object.len() == expected_keys.len()
        && expected_keys.iter().all(|key| object.contains_key(*key))
        && object.get("type").and_then(Value::as_str) == Some("command")
        && object.get("timeout").and_then(Value::as_u64) == Some(HOOK_TIMEOUT_SECONDS)
        && object
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(command_targets_legacy_hook)
        && object
            .get("commandWindows")
            .and_then(Value::as_str)
            .is_some_and(command_targets_legacy_hook)
}

fn command_targets_legacy_hook(command: &str) -> bool {
    command
        .trim_end_matches([' ', '\t', '\r', '\n', '"', '\''])
        .replace('\\', "/")
        .to_lowercase()
        .ends_with(LEGACY_SCRIPT_SUFFIX)
}

fn is_owned_handler(candidate: &Value) -> bool {
    is_managed_handler(candidate) || is_legacy_handler(candidate)
}

fn without_owned_handlers(groups: Vec<Value>) -> (Vec<Value>, bool) {
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
            .filter(|handler| !is_owned_handler(handler))
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

fn status_from_config(
    paths: &IntegrationPaths,
    config: &Map<String, Value>,
    handler: &Value,
) -> Result<CodexIntegrationStatus, String> {
    status_from_states(paths, event_states(config, handler)?)
}

fn status_from_states(
    paths: &IntegrationPaths,
    states: BTreeMap<&'static str, EventState>,
) -> Result<CodexIntegrationStatus, String> {
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
    let message = match state {
        "installed" => {
            "Codex activity monitoring is installed. Review hook trust in Codex with /hooks."
        }
        "not_installed" => "Codex activity monitoring is not installed.",
        "stale" => "An older VSParallel Codex integration was found and can be migrated.",
        _ => "Codex activity monitoring is only partially installed.",
    };
    Ok(CodexIntegrationStatus {
        state: state.to_string(),
        installed: state == "installed",
        config_path: paths.config.to_string_lossy().into_owned(),
        backup_path: paths.backup.to_string_lossy().into_owned(),
        event_states: states
            .into_iter()
            .map(|(event, state)| (event.to_string(), state.as_str().to_string()))
            .collect(),
        message: message.to_string(),
    })
}

fn hook_review_status_from_response(
    paths: &IntegrationPaths,
    handler: &Value,
    result: &Value,
) -> Result<CodexHookReviewStatus, String> {
    let entries = result
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Codex returned no hook status data".to_string())?;
    let expected_commands: Vec<_> = ["command", "commandWindows"]
        .into_iter()
        .filter_map(|key| handler.get(key).and_then(Value::as_str))
        .collect();
    if expected_commands.len() != 2 {
        return Err("the VSParallel hook commands could not be compared".to_string());
    }
    let expected_source_path = comparable_hook_source_path(&paths.config)
        .ok_or_else(|| "the VSParallel hook source path could not be compared".to_string())?;

    let mut matching = BTreeMap::new();
    for hook in entries
        .iter()
        .filter_map(|entry| entry.get("hooks").and_then(Value::as_array))
        .flatten()
    {
        let Some(object) = hook.as_object() else {
            continue;
        };
        let Some(event) = object.get("eventName").and_then(Value::as_str) else {
            continue;
        };
        if !["userPromptSubmit", "stop", "sessionEnd"].contains(&event)
            || object.get("handlerType").and_then(Value::as_str) != Some("command")
            || object.get("source").and_then(Value::as_str) != Some("user")
            || object.get("statusMessage").and_then(Value::as_str) != Some(MANAGED_STATUS_MESSAGE)
            || object
                .get("command")
                .and_then(Value::as_str)
                .is_none_or(|command| !expected_commands.contains(&command))
            || object.get("timeoutSec").and_then(Value::as_u64) != Some(HOOK_TIMEOUT_SECONDS)
            || object
                .get("sourcePath")
                .and_then(Value::as_str)
                .and_then(|path| comparable_hook_source_path(Path::new(path)))
                .as_ref()
                != Some(&expected_source_path)
        {
            continue;
        }
        if matching.insert(event, object).is_some() {
            return Err(format!("Codex returned duplicate {event} hook status"));
        }
    }

    let mut review_required = false;
    for event in ["userPromptSubmit", "stop", "sessionEnd"] {
        let hook = matching
            .get(event)
            .ok_or_else(|| format!("Codex did not report the installed {event} handler"))?;
        let enabled = hook
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("Codex returned invalid {event} enablement status"))?;
        let trust = hook
            .get("trustStatus")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Codex returned invalid {event} trust status"))?;
        match trust {
            "trusted" | "managed" if enabled => {}
            "trusted" | "managed" | "untrusted" | "modified" => review_required = true,
            _ => return Err(format!("Codex returned an unknown {event} trust status")),
        }
    }

    Ok(if review_required {
        CodexHookReviewStatus::ReviewRequired
    } else {
        CodexHookReviewStatus::Trusted
    })
}

fn comparable_hook_source_path(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path)
        .ok()
        .or_else(|| lexical_normalize_absolute(path))
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
        .map_err(|error| format!("could not serialize Codex hooks: {error}"))?;
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

/// Refuse to replace links and non-files. The same-directory rename replaces a
/// link itself rather than following it, but rejecting it explicitly keeps the
/// configuration and state write policy easy to audit on every platform.
fn reject_unsafe_existing_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) => Err(format!(
            "refusing to replace symbolic link {}",
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
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    // Closing the file first avoids Windows sharing violations. TempPath keeps
    // ownership of failure cleanup; after success its source name is gone, so
    // dropping it cannot delete the destination.
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

fn absolute_user_path(path: PathBuf) -> Result<PathBuf, String> {
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
            .map_err(|error| format!("could not resolve CODEX_HOME: {error}"))
    }
}

fn home_directory() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let home = nonempty_env_path("USERPROFILE");
    #[cfg(not(target_os = "windows"))]
    let home = nonempty_env_path("HOME");
    home.ok_or_else(|| "could not expand '~' because the home directory is unavailable".to_string())
}

fn normalize_cwd(raw: &str) -> Option<PathBuf> {
    if raw.trim().is_empty() || raw.contains('\0') {
        return None;
    }
    let mut path = PathBuf::from(raw.trim());
    if path == Path::new("~") || path.starts_with("~/") || path.starts_with("~\\") {
        path = absolute_user_path(path).ok()?;
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

/// A streaming read adapter that stops after a strict payload limit. It probes
/// at most one additional byte to distinguish an exact-limit payload from an
/// oversized one, so malformed input cannot cause unbounded processing.
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
                "Codex hook payload exceeds the safety limit",
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
                        "Codex hook payload exceeds the safety limit",
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

// Small dependency-free SHA-256 implementation. Keeping this here avoids
// shipping a Python runtime or adding another application dependency solely to
// pseudonymize a session identifier.
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
    use std::cell::Cell;
    use std::rc::Rc;
    use tempfile::TempDir;

    struct MeteredReader<'a> {
        input: &'a [u8],
        position: usize,
        consumed: Rc<Cell<usize>>,
    }

    impl Read for MeteredReader<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let remaining = &self.input[self.position..];
            let count = remaining.len().min(buffer.len());
            buffer[..count].copy_from_slice(&remaining[..count]);
            self.position += count;
            self.consumed.set(self.position);
            Ok(count)
        }
    }

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
        serde_json::from_slice(&fs::read(home.join(HOOKS_FILENAME)).unwrap()).unwrap()
    }

    fn hook_with_root(input: &str, root: &Path, now: i64) -> (i32, String) {
        let mut output = Vec::new();
        let state_root = Ok(root);
        let code = run_codex_hook_with(input.as_bytes(), &mut output, state_root, now);
        (code, String::from_utf8(output).unwrap())
    }

    fn hooks_list_result(home: &Path, executable: &Path) -> Value {
        let handler = managed_handler(executable).unwrap();
        #[cfg(windows)]
        let command = handler["commandWindows"].clone();
        #[cfg(not(windows))]
        let command = handler["command"].clone();
        let hooks: Vec<_> = ["userPromptSubmit", "stop", "sessionEnd"]
            .into_iter()
            .map(|event| {
                json!({
                    "eventName": event,
                    "handlerType": "command",
                    "command": command.clone(),
                    "timeoutSec": HOOK_TIMEOUT_SECONDS,
                    "statusMessage": MANAGED_STATUS_MESSAGE,
                    "sourcePath": home.join(HOOKS_FILENAME),
                    "source": "user",
                    "enabled": true,
                    "trustStatus": "trusted"
                })
            })
            .collect();
        json!({"data": [{"cwd": home, "hooks": hooks, "warnings": [], "errors": []}]})
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
            "transcript_path": "/secret/transcript"
        })
        .to_string();
        let (code, output) = hook_with_root(&input, temp.path(), 1_700_000_000_123);
        assert_eq!(code, 0);
        assert!(output.is_empty());

        let entries: Vec<_> = fs::read_dir(temp.path().join("codex"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries.len(), 1);
        let record: Value = serde_json::from_slice(&fs::read(&entries[0]).unwrap()).unwrap();
        assert_eq!(record.as_object().unwrap().len(), 5);
        assert_eq!(record["schemaVersion"], 1);
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["changedAtMs"], 1_700_000_000_123i64);
        assert_eq!(record["sessionKey"].as_str().unwrap().len(), 64);
        let saved = String::from_utf8(fs::read(&entries[0]).unwrap()).unwrap();
        assert!(!saved.contains("SECRET"));
        assert!(!saved.contains("private-session-id"));
        assert!(!saved.contains("transcript"));
    }

    #[test]
    fn stop_is_fail_open_and_atomically_replaces_session_state() {
        let temp = TempDir::new().unwrap();
        let cwd = temp.path().join("project");
        fs::create_dir(&cwd).unwrap();
        let start = json!({
            "hook_event_name": "UserPromptSubmit", "session_id": "s", "cwd": cwd
        })
        .to_string();
        hook_with_root(&start, temp.path(), 10);
        let stop = json!({
            "hook_event_name": "Stop", "session_id": "s", "cwd": cwd,
            "last_assistant_message": "must not persist"
        })
        .to_string();
        let (_, output) = hook_with_root(&stop, temp.path(), 20);
        assert_eq!(output, "{}\n");
        let records: Vec<_> = fs::read_dir(temp.path().join("codex"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(records.len(), 1);
        let record: Value = serde_json::from_slice(&fs::read(&records[0]).unwrap()).unwrap();
        assert_eq!(record["state"], "turn_finished");
        assert_eq!(record["changedAtMs"], 20);
    }

    #[test]
    fn stop_outputs_valid_object_even_when_record_is_invalid() {
        let temp = TempDir::new().unwrap();
        let input = r#"{"hook_event_name":"Stop","prompt":"ignored"}"#;
        let (_, output) = hook_with_root(input, temp.path(), 20);
        assert_eq!(output, "{}\n");
        assert!(!temp.path().join("codex").exists());
    }

    #[test]
    fn malformed_hook_input_never_fails_codex() {
        let temp = TempDir::new().unwrap();
        let (code, output) = hook_with_root("not json", temp.path(), 20);
        assert_eq!(code, 0);
        assert!(output.is_empty());
        assert!(!temp.path().join("codex").exists());
    }

    #[test]
    fn oversized_stop_is_capped_fails_open_and_is_not_persisted() {
        let temp = TempDir::new().unwrap();
        let cwd = serde_json::to_string(&temp.path().to_string_lossy()).unwrap();
        let input = format!(
            "{{\"hook_event_name\":\"Stop\",\"session_id\":\"s\",\"cwd\":{cwd},\"prompt\":\"{}\"}}",
            "sensitive".repeat(MAX_HOOK_INPUT_BYTES / 9 + 1)
        );
        assert!(input.len() > MAX_HOOK_INPUT_BYTES);
        let consumed = Rc::new(Cell::new(0));
        let reader = MeteredReader {
            input: input.as_bytes(),
            position: 0,
            consumed: Rc::clone(&consumed),
        };
        let mut output = Vec::new();

        let code = run_codex_hook_with(reader, &mut output, Ok(temp.path()), 20);

        assert_eq!(code, 0);
        assert_eq!(output, b"{}\n");
        assert!(consumed.get() <= MAX_HOOK_INPUT_BYTES + 1);
        assert!(!temp.path().join("codex").exists());
    }

    #[test]
    fn oversized_session_and_cwd_fields_are_rejected_but_stop_stays_fail_open() {
        let temp = TempDir::new().unwrap();
        let oversized_session = format!(
            "{{\"hook_event_name\":\"Stop\",\"session_id\":\"{}\",\"cwd\":\"/tmp\"}}",
            "s".repeat(MAX_SESSION_ID_BYTES + 1)
        );
        let (_, session_output) = hook_with_root(&oversized_session, temp.path(), 20);
        assert_eq!(session_output, "{}\n");

        let oversized_cwd = format!(
            "{{\"hook_event_name\":\"Stop\",\"session_id\":\"s\",\"cwd\":\"/{}\"}}",
            "c".repeat(MAX_CWD_BYTES + 1)
        );
        let (_, cwd_output) = hook_with_root(&oversized_cwd, temp.path(), 20);
        assert_eq!(cwd_output, "{}\n");
        assert!(!temp.path().join("codex").exists());
    }

    #[test]
    fn session_end_uses_expected_state() {
        let temp = TempDir::new().unwrap();
        let cwd = temp.path().join("project");
        fs::create_dir(&cwd).unwrap();
        let input = json!({
            "hook_event_name": "SessionEnd", "session_id": "s", "cwd": cwd
        })
        .to_string();
        hook_with_root(&input, temp.path(), 30);
        let path = fs::read_dir(temp.path().join("codex"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let record: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(record["state"], "session_ended");
    }

    #[test]
    fn install_creates_current_hooks_and_one_time_backup() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let config = home.join(HOOKS_FILENAME);
        write_json(
            &config,
            &json!({
                "unrelated": {"kept": true},
                "hooks": {"Notification": [{"hooks": [{"type": "command", "command": "notify"}]}]}
            }),
        );
        let original = fs::read(&config).unwrap();
        let result = install_codex_integration(&home, &executable(temp.path())).unwrap();
        assert!(result.changed);
        assert!(!result.migrated);
        assert!(result.status.installed);
        assert_eq!(fs::read(home.join(BACKUP_FILENAME)).unwrap(), original);

        let installed = parse_config(&home);
        assert_eq!(installed["unrelated"]["kept"], true);
        assert_eq!(
            installed["hooks"]["Notification"][0]["hooks"][0]["command"],
            "notify"
        );
        for event in EVENTS {
            let handler = &installed["hooks"][event][0]["hooks"][0];
            assert_eq!(handler["statusMessage"], MANAGED_STATUS_MESSAGE);
            assert!(handler["command"]
                .as_str()
                .unwrap()
                .ends_with(" codex-hook"));
            assert!(handler["commandWindows"]
                .as_str()
                .unwrap()
                .ends_with(" codex-hook"));
        }

        let backup = fs::read(home.join(BACKUP_FILENAME)).unwrap();
        let second = install_codex_integration(&home, &executable(temp.path())).unwrap();
        assert!(!second.changed);
        assert_eq!(fs::read(home.join(BACKUP_FILENAME)).unwrap(), backup);
    }

    #[test]
    fn install_migrates_python_managed_and_legacy_handlers() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let python_managed = json!({
            "type": "command",
            "command": "python3 /old/vsparallel_hook.py",
            "commandWindows": "python.exe C:\\old\\vsparallel_hook.py",
            "timeout": 2,
            "statusMessage": MANAGED_STATUS_MESSAGE
        });
        let legacy = json!({
            "type": "command",
            "command": "python3 /repo/integrations/codex/vsparallel_hook.py",
            "commandWindows": "python.exe C:\\repo\\integrations\\codex\\vsparallel_hook.py",
            "timeout": 2
        });
        write_json(
            &home.join(HOOKS_FILENAME),
            &json!({"hooks": {
                "UserPromptSubmit": [{"hooks": [python_managed, {"type":"command","command":"keep"}]}],
                "Stop": [{"hooks": [legacy]}]
            }}),
        );
        let result = install_codex_integration(&home, &executable(temp.path())).unwrap();
        assert!(result.migrated);
        assert!(result.status.installed);
        let text = String::from_utf8(fs::read(home.join(HOOKS_FILENAME)).unwrap()).unwrap();
        assert!(!text.contains("python3"));
        assert!(!text.contains("python.exe"));
        assert!(text.contains("\"command\": \"keep\""));
    }

    #[test]
    fn stale_binary_path_is_migrated_by_stable_marker() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let old = managed_handler(Path::new("/old/VSParallel")).unwrap();
        write_json(
            &home.join(HOOKS_FILENAME),
            &json!({"hooks": {
                "UserPromptSubmit": [{"hooks": [old.clone()]}],
                "Stop": [{"hooks": [old.clone()]}],
                "SessionEnd": [{"hooks": [old]}]
            }}),
        );
        let before = codex_integration_status(&home, &executable(temp.path())).unwrap();
        assert_eq!(before.state, "stale");
        let result = install_codex_integration(&home, &executable(temp.path())).unwrap();
        assert!(result.changed);
        assert!(result.migrated);
        assert!(result.status.installed);
    }

    #[test]
    fn uninstall_removes_only_owned_handlers_and_keeps_backup() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        install_codex_integration(&home, &executable(temp.path())).unwrap();
        let backup_before = fs::read(home.join(BACKUP_FILENAME)).unwrap();

        let mut config = parse_config(&home);
        config["hooks"]["Stop"].as_array_mut().unwrap().insert(
            0,
            json!({"matcher":"keep metadata","hooks":[{"type":"command","command":"keep"}]}),
        );
        write_json(&home.join(HOOKS_FILENAME), &config);

        let result = uninstall_codex_integration(&home, &executable(temp.path())).unwrap();
        assert!(result.changed);
        assert!(!result.status.installed);
        assert_eq!(fs::read(home.join(BACKUP_FILENAME)).unwrap(), backup_before);
        let remaining = parse_config(&home);
        assert_eq!(remaining["hooks"]["Stop"][0]["matcher"], "keep metadata");
        assert_eq!(remaining["hooks"]["Stop"][0]["hooks"][0]["command"], "keep");
        let text = remaining.to_string();
        assert!(!text.contains(MANAGED_STATUS_MESSAGE));
    }

    #[test]
    fn malformed_config_is_never_modified() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(&home).unwrap();
        let config = home.join(HOOKS_FILENAME);
        fs::write(&config, b"{not json").unwrap();
        let before = fs::read(&config).unwrap();
        assert!(install_codex_integration(&home, &executable(temp.path())).is_err());
        assert_eq!(fs::read(&config).unwrap(), before);
        assert!(!home.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn malformed_target_event_prevents_partial_install() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let config = home.join(HOOKS_FILENAME);
        write_json(
            &config,
            &json!({"hooks": {"UserPromptSubmit": [], "Stop": "broken"}}),
        );
        let before = fs::read(&config).unwrap();
        assert!(install_codex_integration(&home, &executable(temp.path())).is_err());
        assert_eq!(fs::read(&config).unwrap(), before);
        assert!(!home.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn status_reports_partial_installation() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let handler = managed_handler(&executable(temp.path())).unwrap();
        write_json(
            &home.join(HOOKS_FILENAME),
            &json!({"hooks": {"Stop": [{"hooks": [handler]}]}}),
        );
        let status = codex_integration_status(&home, &executable(temp.path())).unwrap();
        assert_eq!(status.state, "partial");
        assert!(!status.installed);
        assert_eq!(status.event_states["Stop"], "current");
        assert_eq!(status.event_states["SessionEnd"], "missing");
    }

    #[test]
    fn app_server_status_confirms_all_current_hooks_are_trusted() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let executable = executable(temp.path());
        let paths = IntegrationPaths::new(&home).unwrap();
        let handler = managed_handler(&executable).unwrap();
        let response = hooks_list_result(&home, &executable);

        assert_eq!(
            hook_review_status_from_response(&paths, &handler, &response).unwrap(),
            CodexHookReviewStatus::Trusted
        );

        let unnormalized_paths =
            IntegrationPaths::new(&temp.path().join("nested/../codex")).unwrap();
        assert_eq!(
            hook_review_status_from_response(&unnormalized_paths, &handler, &response).unwrap(),
            CodexHookReviewStatus::Trusted
        );
    }

    #[test]
    fn app_server_status_requires_review_for_untrusted_modified_or_disabled_hook() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let executable = executable(temp.path());
        let paths = IntegrationPaths::new(&home).unwrap();
        let handler = managed_handler(&executable).unwrap();

        for (field, value) in [
            ("trustStatus", json!("untrusted")),
            ("trustStatus", json!("modified")),
            ("enabled", json!(false)),
        ] {
            let mut response = hooks_list_result(&home, &executable);
            response["data"][0]["hooks"][1][field] = value;
            assert_eq!(
                hook_review_status_from_response(&paths, &handler, &response).unwrap(),
                CodexHookReviewStatus::ReviewRequired
            );
        }
    }

    #[test]
    fn app_server_status_rejects_unknown_or_incomplete_hook_metadata() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("codex");
        let executable = executable(temp.path());
        let paths = IntegrationPaths::new(&home).unwrap();
        let handler = managed_handler(&executable).unwrap();

        let mut unknown = hooks_list_result(&home, &executable);
        unknown["data"][0]["hooks"][0]["trustStatus"] = json!("future-status");
        assert!(hook_review_status_from_response(&paths, &handler, &unknown).is_err());

        let mut incomplete = hooks_list_result(&home, &executable);
        incomplete["data"][0]["hooks"].as_array_mut().unwrap().pop();
        assert!(hook_review_status_from_response(&paths, &handler, &incomplete).is_err());
    }

    #[test]
    fn command_quoting_handles_spaces_quotes_and_backslashes() {
        assert_eq!(quote_posix("/a b/it's/app"), "'/a b/it'\"'\"'s/app'");
        assert_eq!(
            quote_windows(r#"C:\Program Files\App\app.exe"#),
            r#""C:\Program Files\App\app.exe""#
        );
        assert_eq!(quote_windows(r#"C:\a\"quoted"#), r#""C:\a\\\"quoted""#);
    }

    #[test]
    fn executable_and_codex_home_must_be_absolute() {
        let temp = TempDir::new().unwrap();
        assert!(codex_integration_status(Path::new("relative"), &executable(temp.path())).is_err());
        assert!(codex_integration_status(temp.path(), Path::new("relative")).is_err());
        assert!(install_codex_integration(temp.path(), &temp.path().join("missing")).is_err());
    }
}
