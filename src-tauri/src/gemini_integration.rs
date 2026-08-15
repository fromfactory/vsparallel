//! Safe global Gemini CLI hook integration for usage capture.
//!
//! Gemini CLI loads user hooks from `~/.gemini/settings.json` (or the
//! equivalent directory rooted at `GEMINI_CLI_HOME`). VSParallel owns one
//! named `AfterModel` command hook in that file. This module changes only that
//! hook, preserves every unrelated setting and hook, and refuses unsafe or
//! malformed configuration files.

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::Builder as TempFileBuilder;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

const SETTINGS_FILENAME: &str = "settings.json";
const BACKUP_FILENAME: &str = "settings.json.vsparallel.bak";
const AFTER_MODEL_EVENT: &str = "AfterModel";
const HOOK_TIMEOUT_MS: u64 = 2_000;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

/// Stable CLI argument used by Gemini CLI's managed VSParallel hook.
pub const GEMINI_USAGE_ARGUMENT: &str = "gemini-usage";

/// The Gemini CLI hook name reserved for VSParallel's usage capture.
pub const GEMINI_USAGE_HOOK_NAME: &str = "vsparallel-usage";

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
enum HookState {
    Current,
    Stale,
    Missing,
    Conflict,
}

impl HookState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Missing => "missing",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct HooksDisableState {
    globally_disabled: bool,
    managed_hook_disabled: bool,
}

impl HooksDisableState {
    fn any(self) -> bool {
        self.globally_disabled || self.managed_hook_disabled
    }
}

/// Serializable status for Gemini CLI's global VSParallel usage hook.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeminiIntegrationStatus {
    pub state: String,
    pub installed: bool,
    pub config_path: String,
    pub backup_path: String,
    pub event_states: BTreeMap<String, String>,
    pub hooks_disabled: bool,
    pub message: String,
}

/// Result of installing, repairing, or uninstalling the Gemini CLI hook.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeminiIntegrationChange {
    pub changed: bool,
    pub migrated: bool,
    pub status: GeminiIntegrationStatus,
}

/// Resolve Gemini CLI's documented user configuration directory.
///
/// `GEMINI_CLI_HOME` is the user-home root used by Gemini CLI, so its value is
/// followed by `.gemini`. Otherwise this returns the platform user's
/// `~/.gemini` directory.
pub fn gemini_config_dir_from_environment() -> Result<PathBuf, String> {
    if let Some(root) = nonempty_env_path("GEMINI_CLI_HOME") {
        return absolute_user_path(root, "GEMINI_CLI_HOME").map(|path| path.join(".gemini"));
    }

    home_directory()
        .map(|path| path.join(".gemini"))
        .map_err(|_| {
            "could not determine the Gemini CLI configuration directory; set GEMINI_CLI_HOME"
                .to_string()
        })
}

/// Inspect VSParallel's global Gemini CLI usage hook without changing it.
pub fn gemini_integration_status(
    gemini_config_dir: &Path,
    executable: &Path,
) -> Result<GeminiIntegrationStatus, String> {
    let paths = IntegrationPaths::new(gemini_config_dir)?;
    let handler = managed_handler(executable)?;
    let (config, _) = read_config(&paths.config)?;
    status_from_config(&paths, &config, &handler)
}

/// Install or repair VSParallel's global Gemini CLI usage hook.
pub fn install_gemini_integration(
    gemini_config_dir: &Path,
    executable: &Path,
) -> Result<GeminiIntegrationChange, String> {
    let paths = IntegrationPaths::new(gemini_config_dir)?;
    validate_install_executable(executable)?;
    let handler = managed_handler(executable)?;
    let (mut config, original) = read_config(&paths.config)?;
    let state = hook_state(&config, &handler)?;

    if state == HookState::Conflict {
        return Err(format!(
            "the Gemini CLI hook name '{GEMINI_USAGE_HOOK_NAME}' is already used by a hook VSParallel does not own; it was left unchanged"
        ));
    }
    if state == HookState::Current {
        return Ok(GeminiIntegrationChange {
            changed: false,
            migrated: false,
            status: status_from_config(&paths, &config, &handler)?,
        });
    }

    // Validate the managed event container before any mutation, then rebuild
    // only its list while preserving unrelated groups and handlers.
    let existing = after_model_groups(&config)?.cloned().unwrap_or_default();
    let (mut filtered, removed) = without_owned_handlers(existing, &handler);
    filtered.push(canonical_group(handler.clone()));
    let hooks = hooks_map_mut(&mut config, true)?.expect("create=true returns a hooks object");
    hooks.insert(AFTER_MODEL_EVENT.to_string(), Value::Array(filtered));

    ensure_backup(&paths.backup, &original)?;
    atomic_write_json(&paths.config, &config)?;

    Ok(GeminiIntegrationChange {
        changed: true,
        migrated: removed,
        status: status_from_config(&paths, &config, &handler)?,
    })
}

/// Remove only VSParallel-owned Gemini CLI usage hook handlers.
pub fn uninstall_gemini_integration(
    gemini_config_dir: &Path,
    executable: &Path,
) -> Result<GeminiIntegrationChange, String> {
    let paths = IntegrationPaths::new(gemini_config_dir)?;
    let handler = managed_handler(executable)?;
    let (mut config, original) = read_config(&paths.config)?;
    let existing = after_model_groups(&config)?.cloned().unwrap_or_default();
    let (filtered, changed) = without_owned_handlers(existing, &handler);

    if changed {
        let hooks = hooks_map_mut(&mut config, false)?
            .expect("an existing AfterModel array requires a hooks object");
        hooks.insert(AFTER_MODEL_EVENT.to_string(), Value::Array(filtered));
        ensure_backup(&paths.backup, &original)?;
        atomic_write_json(&paths.config, &config)?;
    }

    Ok(GeminiIntegrationChange {
        changed,
        migrated: false,
        status: status_from_config(&paths, &config, &handler)?,
    })
}

#[derive(Debug)]
struct IntegrationPaths {
    config: PathBuf,
    backup: PathBuf,
}

impl IntegrationPaths {
    fn new(config_dir: &Path) -> Result<Self, String> {
        if !config_dir.is_absolute() {
            return Err(
                "the Gemini CLI configuration directory must be an absolute path".to_string(),
            );
        }
        Ok(Self {
            config: config_dir.join(SETTINGS_FILENAME),
            backup: config_dir.join(BACKUP_FILENAME),
        })
    }
}

fn managed_handler(executable: &Path) -> Result<Value, String> {
    if !executable.is_absolute() {
        return Err("the VSParallel Gemini usage executable must be an absolute path".to_string());
    }
    let executable = executable.to_str().ok_or_else(|| {
        "the VSParallel Gemini usage executable path is not valid Unicode".to_string()
    })?;
    if executable.contains(['\0', '\n', '\r']) {
        return Err(
            "the VSParallel Gemini usage executable path contains unsafe characters".to_string(),
        );
    }

    #[cfg(windows)]
    let command = format!("{} {GEMINI_USAGE_ARGUMENT}", quote_windows(executable));
    #[cfg(not(windows))]
    let command = format!("{} {GEMINI_USAGE_ARGUMENT}", quote_posix(executable));

    Ok(serde_json::json!({
        "name": GEMINI_USAGE_HOOK_NAME,
        "type": "command",
        "command": command,
        "timeout": HOOK_TIMEOUT_MS,
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
            "the VSParallel Gemini usage executable is unavailable at {}: {error}",
            executable.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "the VSParallel Gemini usage executable is not a regular file: {}",
            executable.display()
        ));
    }
    Ok(())
}

fn canonical_group(handler: Value) -> Value {
    serde_json::json!({
        "matcher": "*",
        "hooks": [handler],
    })
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

    // Both settings materially determine whether an installed hook can run.
    hooks_disable_state(&object)?;
    hooks_map(&object)?;
    after_model_groups(&object)?;
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

fn after_model_groups(config: &Map<String, Value>) -> Result<Option<&Vec<Value>>, String> {
    let Some(hooks) = hooks_map(config)? else {
        return Ok(None);
    };
    match hooks.get(AFTER_MODEL_EVENT) {
        None => Ok(None),
        Some(Value::Array(groups)) => Ok(Some(groups)),
        Some(_) => Err("hooks.AfterModel must be a JSON array".to_string()),
    }
}

fn hooks_disable_state(config: &Map<String, Value>) -> Result<HooksDisableState, String> {
    let Some(value) = config.get("hooksConfig") else {
        return Ok(HooksDisableState::default());
    };
    let settings = value
        .as_object()
        .ok_or_else(|| "the top-level 'hooksConfig' value must be a JSON object".to_string())?;

    let globally_disabled = match settings.get("enabled") {
        None => false,
        Some(Value::Bool(enabled)) => !enabled,
        Some(_) => return Err("hooksConfig.enabled must be a boolean".to_string()),
    };
    let managed_hook_disabled = match settings.get("disabled") {
        None => false,
        Some(Value::Array(names)) => {
            if names.iter().any(|name| !name.is_string()) {
                return Err("hooksConfig.disabled must be an array of strings".to_string());
            }
            names
                .iter()
                .any(|name| name.as_str() == Some(GEMINI_USAGE_HOOK_NAME))
        }
        Some(_) => return Err("hooksConfig.disabled must be an array of strings".to_string()),
    };

    Ok(HooksDisableState {
        globally_disabled,
        managed_hook_disabled,
    })
}

fn hook_state(config: &Map<String, Value>, current: &Value) -> Result<HookState, String> {
    if managed_name_is_used_outside_after_model(config)? {
        return Ok(HookState::Conflict);
    }
    let Some(groups) = after_model_groups(config)? else {
        return Ok(HookState::Missing);
    };
    let canonical = canonical_group(current.clone());
    let mut owned_count = 0usize;
    let mut canonical_found = false;
    let mut conflict = false;

    for group in groups {
        let Some(handlers) = group
            .as_object()
            .and_then(|object| object.get("hooks"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for handler in handlers {
            if is_owned_handler(handler, current) {
                owned_count += 1;
            } else if handler_name(handler) == Some(GEMINI_USAGE_HOOK_NAME) {
                conflict = true;
            }
        }
        canonical_found |= group == &canonical;
    }

    Ok(if conflict {
        HookState::Conflict
    } else if owned_count == 1 && canonical_found {
        HookState::Current
    } else if owned_count > 0 {
        HookState::Stale
    } else {
        HookState::Missing
    })
}

fn managed_name_is_used_outside_after_model(config: &Map<String, Value>) -> Result<bool, String> {
    let Some(hooks) = hooks_map(config)? else {
        return Ok(false);
    };
    Ok(hooks.iter().any(|(event, value)| {
        event != AFTER_MODEL_EVENT
            && value.as_array().is_some_and(|groups| {
                groups.iter().any(|group| {
                    group
                        .as_object()
                        .and_then(|object| object.get("hooks"))
                        .and_then(Value::as_array)
                        .is_some_and(|handlers| {
                            handlers.iter().any(|handler| {
                                handler_name(handler) == Some(GEMINI_USAGE_HOOK_NAME)
                            })
                        })
                })
            })
    }))
}

fn handler_name(handler: &Value) -> Option<&str> {
    handler.as_object()?.get("name").and_then(Value::as_str)
}

fn is_owned_handler(candidate: &Value, current: &Value) -> bool {
    candidate == current || is_historical_vsparallel_handler(candidate)
}

fn is_historical_vsparallel_handler(candidate: &Value) -> bool {
    let Some(object) = candidate.as_object() else {
        return false;
    };
    const EXPECTED_KEYS: [&str; 4] = ["name", "type", "command", "timeout"];
    object.len() == EXPECTED_KEYS.len()
        && EXPECTED_KEYS.iter().all(|key| object.contains_key(*key))
        && object.get("name").and_then(Value::as_str) == Some(GEMINI_USAGE_HOOK_NAME)
        && object.get("type").and_then(Value::as_str) == Some("command")
        && object.get("timeout").and_then(Value::as_u64).is_some()
        && object
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(command_targets_vsparallel_usage)
}

fn command_targets_vsparallel_usage(command: &str) -> bool {
    if command.is_empty() || command.contains(['\0', '\n', '\r']) {
        return false;
    }
    let suffix = format!(" {GEMINI_USAGE_ARGUMENT}");
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
        return decode_posix_single_quoted(value);
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

/// Decode the exact segmented single-quote form emitted by `quote_posix`.
///
/// An apostrophe is represented by closing the single-quoted segment,
/// emitting a double-quoted apostrophe, then opening the next segment:
/// `'/path/O'"'"'Brien/vsparallel'`. Restricting recognition to that grammar
/// avoids treating shell operators or substitutions as a managed command.
#[cfg(not(windows))]
fn decode_posix_single_quoted(value: &str) -> Option<String> {
    let mut remaining = value;
    let mut decoded = String::new();

    loop {
        remaining = remaining.strip_prefix('\'')?;
        let segment_end = remaining.find('\'')?;
        decoded.push_str(&remaining[..segment_end]);
        remaining = &remaining[segment_end + 1..];
        if remaining.is_empty() {
            return Some(decoded);
        }

        remaining = remaining.strip_prefix("\"'\"")?;
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
        let has_only_group_fields = object.keys().all(|key| key == "matcher" || key == "hooks");
        if new_handlers.is_empty() && has_only_group_fields {
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
) -> Result<GeminiIntegrationStatus, String> {
    let hook_state = hook_state(config, handler)?;
    let disabled = hooks_disable_state(config)?;
    let state = match hook_state {
        HookState::Conflict => "conflict",
        HookState::Current if disabled.any() => "disabled",
        HookState::Current => "installed",
        HookState::Stale => "stale",
        HookState::Missing => "not_installed",
    };

    let mut message = match state {
        "installed" => "Gemini CLI usage capture is installed.".to_string(),
        "disabled" => {
            "Gemini CLI usage capture is installed, but Gemini CLI settings disable it. VSParallel preserves those user-controlled settings; enable hooks manually to start capture."
                .to_string()
        }
        "conflict" => format!(
            "The Gemini CLI hook name '{GEMINI_USAGE_HOOK_NAME}' is used by another command. Rename or remove that hook before installing VSParallel usage capture."
        ),
        "stale" => "An older VSParallel Gemini CLI usage hook can be repaired.".to_string(),
        _ => "Gemini CLI usage capture is not installed.".to_string(),
    };
    if disabled.globally_disabled {
        message
            .push_str(" Gemini CLI's hooksConfig.enabled setting is false, so no hooks can run.");
    }
    if disabled.managed_hook_disabled {
        message.push_str(&format!(
            " Gemini CLI's hooksConfig.disabled list contains '{GEMINI_USAGE_HOOK_NAME}', so this hook cannot run."
        ));
    }

    Ok(GeminiIntegrationStatus {
        state: state.to_string(),
        // `installed` describes ownership of the managed handler. A user's
        // hooksConfig settings can prevent that structurally current handler
        // from running, which is reported separately by `state` and
        // `hooks_disabled` without inviting a repair that cannot change it.
        installed: hook_state == HookState::Current,
        config_path: paths.config.to_string_lossy().into_owned(),
        backup_path: paths.backup.to_string_lossy().into_owned(),
        event_states: BTreeMap::from([(
            AFTER_MODEL_EVENT.to_string(),
            hook_state.as_str().to_string(),
        )]),
        hooks_disabled: disabled.any(),
        message,
    })
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

fn atomic_write_json(path: &Path, config: &Map<String, Value>) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("could not serialize Gemini CLI settings: {error}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn executable(root: &Path) -> PathBuf {
        let path = root.join(if cfg!(windows) {
            "vsparallel.exe"
        } else {
            "vsparallel"
        });
        fs::write(&path, b"test executable").unwrap();
        path
    }

    fn write_json(path: &Path, value: &Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = serde_json::to_vec_pretty(value).unwrap();
        bytes.push(b'\n');
        fs::write(path, bytes).unwrap();
    }

    fn parse_config(config_dir: &Path) -> Value {
        serde_json::from_slice(&fs::read(config_dir.join(SETTINGS_FILENAME)).unwrap()).unwrap()
    }

    #[test]
    fn install_preserves_settings_and_unrelated_hooks_with_one_time_backup() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let config = config_dir.join(SETTINGS_FILENAME);
        write_json(
            &config,
            &json!({
                "model": "gemini-test",
                "security": {"folderTrust": {"enabled": true}},
                "hooks": {
                    "BeforeTool": [{"matcher":"write_file","hooks":[{"name":"guard","type":"command","command":"guard"}]}],
                    "AfterModel": [{"matcher":"*","hooks":[{"name":"metrics","type":"command","command":"metrics"}]}]
                }
            }),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());

        let result = install_gemini_integration(&config_dir, &executable).unwrap();
        assert!(result.changed);
        assert!(!result.migrated);
        assert!(result.status.installed);
        assert_eq!(result.status.event_states[AFTER_MODEL_EVENT], "current");
        assert_eq!(
            fs::read(config_dir.join(BACKUP_FILENAME)).unwrap(),
            original
        );

        let installed = parse_config(&config_dir);
        assert_eq!(installed["model"], "gemini-test");
        assert_eq!(installed["security"]["folderTrust"]["enabled"], true);
        assert_eq!(installed["hooks"]["BeforeTool"][0]["matcher"], "write_file");
        assert_eq!(
            installed["hooks"]["AfterModel"][0]["hooks"][0]["name"],
            "metrics"
        );
        let managed = installed["hooks"]["AfterModel"]
            .as_array()
            .unwrap()
            .last()
            .unwrap();
        assert_eq!(managed["matcher"], "*");
        assert_eq!(managed["hooks"][0]["name"], GEMINI_USAGE_HOOK_NAME);
        assert_eq!(managed["hooks"][0]["type"], "command");
        assert_eq!(managed["hooks"][0]["timeout"], HOOK_TIMEOUT_MS);
        assert!(managed["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with(GEMINI_USAGE_ARGUMENT));

        let backup = fs::read(config_dir.join(BACKUP_FILENAME)).unwrap();
        let second = install_gemini_integration(&config_dir, &executable).unwrap();
        assert!(!second.changed);
        assert_eq!(fs::read(config_dir.join(BACKUP_FILENAME)).unwrap(), backup);
    }

    #[test]
    fn disabled_settings_are_preserved_and_reported_separately() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let executable = executable(temp.path());
        let handler = managed_handler(&executable).unwrap();
        let config_path = config_dir.join(SETTINGS_FILENAME);
        write_json(
            &config_path,
            &json!({
                "hooksConfig": {
                    "enabled": false,
                    "disabled": ["keep", GEMINI_USAGE_HOOK_NAME],
                    "notifications": false
                },
                "hooks": {AFTER_MODEL_EVENT: [canonical_group(handler)]}
            }),
        );
        let original = fs::read(&config_path).unwrap();

        let status = gemini_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(status.state, "disabled");
        assert!(status.installed);
        assert!(status.hooks_disabled);
        assert!(status.message.contains("enable hooks manually"));
        assert!(status.message.contains("hooksConfig.enabled"));
        assert!(status.message.contains("hooksConfig.disabled"));
        assert!(status.message.contains(GEMINI_USAGE_HOOK_NAME));

        let install = install_gemini_integration(&config_dir, &executable).unwrap();
        assert!(!install.changed);
        assert!(install.status.installed);
        assert_eq!(install.status.state, "disabled");
        assert_eq!(fs::read(&config_path).unwrap(), original);
        assert!(!config_dir.join(BACKUP_FILENAME).exists());
        let unchanged = parse_config(&config_dir);
        assert_eq!(unchanged["hooksConfig"]["enabled"], false);
        assert_eq!(unchanged["hooksConfig"]["disabled"][0], "keep");
    }

    #[test]
    fn stale_owned_handler_is_repaired_without_removing_custom_handlers() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let executable = executable(temp.path());
        let old_command = if cfg!(windows) {
            r#""C:\old\vsparallel.exe" gemini-usage"#
        } else {
            "'/old/vsparallel' gemini-usage"
        };
        write_json(
            &config_dir.join(SETTINGS_FILENAME),
            &json!({"hooks": {AFTER_MODEL_EVENT: [{
                "matcher": "*",
                "hooks": [
                    {"name":GEMINI_USAGE_HOOK_NAME,"type":"command","command":old_command,"timeout":5000},
                    {"name":"keep","type":"command","command":"keep"}
                ]
            }]}}),
        );

        let before = gemini_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(before.state, "stale");
        let repaired = install_gemini_integration(&config_dir, &executable).unwrap();
        assert!(repaired.changed);
        assert!(repaired.migrated);
        assert!(repaired.status.installed);
        let text = parse_config(&config_dir).to_string();
        assert!(!text.contains("/old/vsparallel"));
        assert!(text.contains("keep"));
    }

    #[cfg(not(windows))]
    #[test]
    fn historical_handler_with_apostrophe_path_is_owned_but_shell_syntax_is_not() {
        let historical_executable = "/opt/O'Brien/VSParallel/vsparallel";
        let quoted = quote_posix(historical_executable);
        assert_eq!(
            decode_posix_single_quoted(&quoted).as_deref(),
            Some(historical_executable)
        );
        let owned_command = format!("{quoted} {GEMINI_USAGE_ARGUMENT}");
        assert!(command_targets_vsparallel_usage(&owned_command));

        for custom in [
            format!("\"/opt/$APP/vsparallel\" {GEMINI_USAGE_ARGUMENT}"),
            format!("$(resolve-vsparallel) {GEMINI_USAGE_ARGUMENT}"),
            format!("{owned_command}; echo unsafe"),
            format!("{owned_command} "),
            format!("render && {owned_command}"),
        ] {
            assert!(
                !command_targets_vsparallel_usage(&custom),
                "unexpectedly owned custom command: {custom}"
            );
        }

        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let executable = executable(temp.path());
        write_json(
            &config_dir.join(SETTINGS_FILENAME),
            &json!({"hooks": {AFTER_MODEL_EVENT: [{
                "matcher": "*",
                "hooks": [{
                    "name": GEMINI_USAGE_HOOK_NAME,
                    "type": "command",
                    "command": owned_command,
                    "timeout": 5000
                }]
            }]}}),
        );

        let before = gemini_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(before.state, "stale");
        let repaired = install_gemini_integration(&config_dir, &executable).unwrap();
        assert!(repaired.changed);
        assert!(repaired.migrated);
        assert!(repaired.status.installed);
        assert!(!parse_config(&config_dir).to_string().contains("O'Brien"));
    }

    #[test]
    fn same_name_collision_is_reported_and_never_modified_or_removed() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let config = config_dir.join(SETTINGS_FILENAME);
        write_json(
            &config,
            &json!({"hooks": {AFTER_MODEL_EVENT: [{
                "matcher": "*",
                "hooks": [{"name":GEMINI_USAGE_HOOK_NAME,"type":"command","command":"my-recorder","timeout":2000}]
            }]}}),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());

        let status = gemini_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(status.state, "conflict");
        assert!(status.message.contains("Rename or remove"));
        assert!(install_gemini_integration(&config_dir, &executable).is_err());
        let removed = uninstall_gemini_integration(&config_dir, &executable).unwrap();
        assert!(!removed.changed);
        assert_eq!(removed.status.state, "conflict");
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!config_dir.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn uninstall_removes_owned_handler_and_preserves_foreign_same_name_handler() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let config_path = config_dir.join(SETTINGS_FILENAME);
        let executable = executable(temp.path());
        let owned = managed_handler(&executable).unwrap();
        let foreign = json!({
            "name": GEMINI_USAGE_HOOK_NAME,
            "type": "command",
            "command": "my-recorder",
            "timeout": 2000
        });
        write_json(
            &config_path,
            &json!({"hooks": {AFTER_MODEL_EVENT: [{
                "matcher": "*",
                "hooks": [owned, foreign]
            }]}}),
        );
        let original = fs::read(&config_path).unwrap();

        let before = gemini_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(before.state, "conflict");

        let removed = uninstall_gemini_integration(&config_dir, &executable).unwrap();
        assert!(removed.changed);
        assert!(!removed.status.installed);
        assert_eq!(removed.status.state, "conflict");

        let remaining = parse_config(&config_dir);
        let handlers = remaining["hooks"][AFTER_MODEL_EVENT][0]["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0]["command"], "my-recorder");
        assert_eq!(
            fs::read(config_dir.join(BACKUP_FILENAME)).unwrap(),
            original
        );
    }

    #[test]
    fn reserved_name_in_another_event_is_also_a_conflict() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let config = config_dir.join(SETTINGS_FILENAME);
        write_json(
            &config,
            &json!({"hooks": {
                "BeforeTool": [{
                    "matcher": "*",
                    "hooks": [{"name":GEMINI_USAGE_HOOK_NAME,"type":"command","command":"custom"}]
                }]
            }}),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());

        let status = gemini_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(status.state, "conflict");
        assert!(install_gemini_integration(&config_dir, &executable).is_err());
        assert_eq!(fs::read(&config).unwrap(), original);
    }

    #[test]
    fn uninstall_removes_only_owned_handler_and_keeps_group_metadata() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let executable = executable(temp.path());
        install_gemini_integration(&config_dir, &executable).unwrap();

        let mut config = parse_config(&config_dir);
        config["hooks"][AFTER_MODEL_EVENT]
            .as_array_mut()
            .unwrap()
            .insert(
                0,
                json!({
                    "matcher":"*",
                    "description":"keep metadata",
                    "hooks":[
                        managed_handler(&executable).unwrap(),
                        {"name":"keep","type":"command","command":"keep"}
                    ]
                }),
            );
        write_json(&config_dir.join(SETTINGS_FILENAME), &config);

        let removed = uninstall_gemini_integration(&config_dir, &executable).unwrap();
        assert!(removed.changed);
        let remaining = parse_config(&config_dir);
        assert_eq!(
            remaining["hooks"][AFTER_MODEL_EVENT][0]["description"],
            "keep metadata"
        );
        assert_eq!(
            remaining["hooks"][AFTER_MODEL_EVENT][0]["hooks"][0]["name"],
            "keep"
        );
        assert!(!remaining.to_string().contains(GEMINI_USAGE_HOOK_NAME));
    }

    #[test]
    fn malformed_settings_are_never_modified() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join(SETTINGS_FILENAME);
        let executable = executable(temp.path());

        for malformed in [
            b"{not json".to_vec(),
            serde_json::to_vec(&json!([])).unwrap(),
            serde_json::to_vec(&json!({"hooks": []})).unwrap(),
            serde_json::to_vec(&json!({"hooks": {AFTER_MODEL_EVENT: "broken"}})).unwrap(),
            serde_json::to_vec(&json!({"hooksConfig": []})).unwrap(),
            serde_json::to_vec(&json!({"hooksConfig": {"enabled": "yes"}})).unwrap(),
            serde_json::to_vec(&json!({"hooksConfig": {"disabled": [1]}})).unwrap(),
        ] {
            fs::write(&config, &malformed).unwrap();
            assert!(install_gemini_integration(&config_dir, &executable).is_err());
            assert!(uninstall_gemini_integration(&config_dir, &executable).is_err());
            assert_eq!(fs::read(&config).unwrap(), malformed);
            assert!(!config_dir.join(BACKUP_FILENAME).exists());
        }
    }

    #[test]
    fn oversized_config_and_relative_paths_are_rejected() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join(SETTINGS_FILENAME);
        fs::write(&config, vec![b' '; MAX_CONFIG_BYTES as usize + 1]).unwrap();
        let executable = executable(temp.path());

        assert!(install_gemini_integration(&config_dir, &executable).is_err());
        assert_eq!(fs::metadata(&config).unwrap().len(), MAX_CONFIG_BYTES + 1);
        assert!(!config_dir.join(BACKUP_FILENAME).exists());
        assert!(gemini_integration_status(Path::new("relative"), &executable).is_err());
        assert!(gemini_integration_status(temp.path(), Path::new("relative")).is_err());
        assert!(install_gemini_integration(temp.path(), &temp.path().join("missing")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_settings_and_backup_are_refused() {
        let temp = TempDir::new().unwrap();
        let executable = executable(temp.path());
        let config_dir = temp.path().join(".gemini");
        fs::create_dir_all(&config_dir).unwrap();
        let victim = temp.path().join("victim.json");
        fs::write(&victim, b"{}\n").unwrap();
        symlink(&victim, config_dir.join(SETTINGS_FILENAME)).unwrap();

        assert!(install_gemini_integration(&config_dir, &executable).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"{}\n");

        fs::remove_file(config_dir.join(SETTINGS_FILENAME)).unwrap();
        fs::write(config_dir.join(SETTINGS_FILENAME), b"{}\n").unwrap();
        symlink(&victim, config_dir.join(BACKUP_FILENAME)).unwrap();
        assert!(install_gemini_integration(&config_dir, &executable).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"{}\n");
        assert_eq!(
            fs::read(config_dir.join(SETTINGS_FILENAME)).unwrap(),
            b"{}\n"
        );

        let linked_target = temp.path().join("linked-config-target");
        let linked_config = temp.path().join("linked-config");
        fs::create_dir_all(&linked_target).unwrap();
        symlink(&linked_target, &linked_config).unwrap();
        assert!(install_gemini_integration(&linked_config, &executable).is_err());
        assert_eq!(fs::read_dir(&linked_target).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn newly_created_settings_and_backup_are_private() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini");
        let executable = executable(temp.path());
        install_gemini_integration(&config_dir, &executable).unwrap();

        assert_eq!(
            fs::metadata(config_dir.join(SETTINGS_FILENAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(config_dir.join(BACKUP_FILENAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&config_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
