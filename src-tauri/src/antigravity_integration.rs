//! Local Antigravity lifecycle-hook integration for VSParallel.
//!
//! Antigravity 2.0, Antigravity IDE, and the Antigravity CLI discover the
//! same global customization file at `~/.gemini/config/hooks.json`. This
//! module owns one named entry in that file and records only coarse lifecycle
//! state plus local workspace paths. For IDE hooks that omit `modelName`, a
//! turn-start hook incrementally scans protobuf structure in the conversation's
//! latest user-input step and uses only its current model enum, while seeking
//! past all prompt and context bodies. Bounded execution metadata and editor
//! preference values are compatibility fallbacks; raw model identifiers and
//! conversation content are never persisted.

use rusqlite::{Connection, OpenFlags, MAIN_DB};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::Builder as TempFileBuilder;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(windows)]
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
};

const HOOKS_FILENAME: &str = "hooks.json";
const BACKUP_FILENAME: &str = "hooks.json.vsparallel.bak";
const MANAGED_HOOK_NAME: &str = "vsparallel";
const HOOK_ARGUMENT: &str = "antigravity-hook";
const HOOK_HEALTH_DIRECTORY: &str = "antigravity-hook-health";
const HOOK_TIMEOUT_SECONDS: u64 = 2;
const SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_HOOK_HEALTH_BYTES: u64 = 8 * 1024;
const MAX_HOOK_INPUT_BYTES: usize = 1024 * 1024;
const MAX_IDE_STATE_DB_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IDE_MODEL_PREFERENCES_BYTES: usize = 64 * 1024;
const MAX_IDE_CONVERSATION_DB_BYTES: u64 = 256 * 1024 * 1024;
const MAX_IDE_CONVERSATION_AUX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IDE_USER_INPUT_BYTES: u64 = 1024 * 1024;
const MAX_IDE_PROTOBUF_STRUCTURE_BYTES: usize = 4 * 1024;
const MAX_IDE_EXECUTOR_METADATA_BYTES: usize = 64 * 1024;
const MAX_IDE_CONVERSATION_DATABASES: usize = 4_096;
const IDE_DB_BUSY_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_EXISTING_HOOK_RECORD_BYTES: u64 = 64 * 1024;
const MAX_CONVERSATION_ID_BYTES: usize = 16 * 1024;
const MAX_MODEL_NAME_BYTES: usize = 128;
const MAX_WORKSPACE_PATH_BYTES: usize = 32 * 1024;
const MAX_WORKSPACE_PATHS: usize = 64;
const MAX_TRANSCRIPT_PATH_BYTES: usize = 64 * 1024;
const MAX_TERMINATION_REASON_BYTES: usize = 256;
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
const RECORD_LOCK_ATTEMPTS: usize = 200;
const RECORD_LOCK_RETRY: Duration = Duration::from_millis(5);

const EVENTS: [AntigravityHookEvent; 3] = [
    AntigravityHookEvent::PreInvocation,
    AntigravityHookEvent::PostToolUse,
    AntigravityHookEvent::Stop,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AntigravityHookEvent {
    PreInvocation,
    PostToolUse,
    Stop,
}

impl AntigravityHookEvent {
    /// Parse the stable subcommand passed after `antigravity-hook`.
    pub fn from_cli_argument(value: &str) -> Option<Self> {
        match value {
            "pre-invocation" => Some(Self::PreInvocation),
            "post-tool-use" => Some(Self::PostToolUse),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    fn cli_argument(self) -> &'static str {
        match self {
            Self::PreInvocation => "pre-invocation",
            Self::PostToolUse => "post-tool-use",
            Self::Stop => "stop",
        }
    }

    fn observation_name(self) -> &'static str {
        self.cli_argument()
    }

    fn config_name(self) -> &'static str {
        match self {
            Self::PreInvocation => "PreInvocation",
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
        }
    }

    fn fail_open_output(self) -> &'static [u8] {
        match self {
            // Both fields accepted by PreInvocation are optional. PostToolUse
            // explicitly expects an empty JSON object.
            Self::PreInvocation | Self::PostToolUse => b"{}\n",
            // Stop documents `decision` as required and treats every value
            // other than `continue` as permission to stop.
            Self::Stop => b"{\"decision\":\"allow\"}\n",
        }
    }
}

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

/// Serializable setup status for future Tauri command wiring.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityIntegrationStatus {
    pub state: String,
    pub installed: bool,
    pub config_path: String,
    pub backup_path: String,
    pub event_states: BTreeMap<String, String>,
    pub hooks_disabled: bool,
    pub message: String,
}

/// Result of an install or uninstall request.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityIntegrationChange {
    pub changed: bool,
    pub migrated: bool,
    pub status: AntigravityIntegrationStatus,
}

/// The only persisted view of an Antigravity hook payload.
///
/// A multi-folder Antigravity project produces one record per workspace path,
/// keeping the on-disk schema compatible with VSParallel's existing coarse
/// activity records. The raw conversation ID is replaced with a SHA-256 key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookRecord {
    schema_version: u32,
    session_key: String,
    cwd: String,
    state: String,
    changed_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_kind: Option<AntigravityModelKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ide_model_revision: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExistingHookRecord {
    schema_version: u32,
    session_key: String,
    state: String,
    changed_at_ms: i64,
    #[serde(default)]
    model_kind: Option<AntigravityModelKind>,
    #[serde(default)]
    ide_model_revision: Option<String>,
}

#[derive(Debug, Default)]
struct HookPayload {
    conversation_id: Option<String>,
    workspace_paths: Vec<String>,
    surface: AntigravitySurface,
    surface_conflict: bool,
    model_name_present: bool,
    model_kind: Option<AntigravityModelKind>,
    had_error: bool,
    termination_reason: Option<String>,
    fully_idle: Option<bool>,
    ide_model_revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdeExecutionModel {
    pub(crate) revision: String,
    pub(crate) preference: IdeSelectedModelPreference,
    pub(crate) source: IdeModelSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdeModelSource {
    CurrentTurn,
    Execution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IdeCurrentTurnModel {
    Missing,
    Available(IdeExecutionModel),
    Unusable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HookModelUpdate {
    Preserve,
    Replace {
        model_kind: Option<AntigravityModelKind>,
        ide_model_revision: Option<String>,
    },
    Ide {
        current_turn_model: Option<IdeExecutionModel>,
        execution_model: Option<IdeExecutionModel>,
        selected_model: Option<IdeSelectedModelPreference>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdeSelectedModelPreference {
    Recognized(AntigravityModelKind),
    Unrecognized,
}

/// Closed, privacy-safe model classifications derived from Antigravity's raw
/// `modelName`. Raw identifiers are never written to VSParallel state.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AntigravityModelKind {
    Automatic,
    Gemini,
    #[serde(rename = "gemini_3_6_flash_medium")]
    Gemini36FlashMedium,
    #[serde(rename = "gemini_3_6_flash_high")]
    Gemini36FlashHigh,
    #[serde(rename = "gemini_3_5_flash")]
    Gemini35Flash,
    #[serde(rename = "gemini_3_1_pro_high")]
    Gemini31ProHigh,
    #[serde(rename = "gemini_3_1_pro_low")]
    Gemini31ProLow,
    #[serde(rename = "gemini_3_flash")]
    Gemini3Flash,
    Claude,
    #[serde(rename = "claude_sonnet_4_6_thinking")]
    ClaudeSonnet46Thinking,
    #[serde(rename = "claude_opus_4_6_thinking")]
    ClaudeOpus46Thinking,
    GptOss,
    #[serde(rename = "gpt_oss_120b")]
    GptOss120b,
    #[serde(rename = "gpt_oss_120b_medium")]
    GptOss120bMedium,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum AntigravitySurface {
    Two,
    Ide,
    Cli,
    #[default]
    Unknown,
}

impl AntigravitySurface {
    fn state_directory(self) -> Option<&'static str> {
        match self {
            Self::Two => Some("antigravity"),
            Self::Ide => Some("antigravity-ide"),
            // The global hook also runs in the CLI, which is outside this
            // workspace UI's supported editor surfaces. Do not mislabel it.
            Self::Cli | Self::Unknown => None,
        }
    }

    fn observation_file(self) -> &'static str {
        match self {
            Self::Two => "antigravity-2.json",
            Self::Ide => "antigravity-ide.json",
            Self::Cli => "antigravity-cli.json",
            Self::Unknown => "unknown.json",
        }
    }

    fn observation_name(self) -> &'static str {
        match self {
            Self::Two => "antigravity_2",
            Self::Ide => "antigravity_ide",
            Self::Cli => "antigravity_cli",
            Self::Unknown => "unknown",
        }
    }
}

impl HookPayload {
    fn observe_surface(&mut self, surface: AntigravitySurface) {
        if surface == AntigravitySurface::Unknown || self.surface_conflict {
            return;
        }
        if self.surface == AntigravitySurface::Unknown {
            self.surface = surface;
        } else if self.surface != surface {
            self.surface = AntigravitySurface::Unknown;
            self.surface_conflict = true;
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AntigravityHookOutcome {
    Recorded,
    InvalidPayload,
    UnsupportedSurface,
    MissingConversation,
    NoWorkspace,
    PersistFailed,
}

impl AntigravityHookOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::InvalidPayload => "invalid_payload",
            Self::UnsupportedSurface => "unsupported_surface",
            Self::MissingConversation => "missing_conversation",
            Self::NoWorkspace => "no_workspace",
            Self::PersistFailed => "persist_failed",
        }
    }

    pub(crate) fn user_message(self) -> &'static str {
        match self {
            Self::Recorded => "the latest agent event produced workspace activity",
            Self::InvalidPayload => "the latest event payload was not valid documented JSON",
            Self::UnsupportedSurface => {
                "the latest event did not identify a supported Antigravity surface"
            }
            Self::MissingConversation => {
                "the latest event did not include a conversation identifier"
            }
            Self::NoWorkspace => {
                "the latest event did not include a usable local Project workspace path"
            }
            Self::PersistFailed => "VSParallel could not save the latest workspace activity",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AntigravityHookObservation {
    schema_version: u32,
    pub(crate) event: String,
    pub(crate) surface: String,
    pub(crate) outcome: AntigravityHookOutcome,
    pub(crate) observed_at_ms: i64,
    pub(crate) workspace_count: u32,
}

/// Streaming payload parser. The transcript path is reduced immediately to a
/// bounded product enum; its value, model output, and tool arguments are never
/// represented in the persisted record.
struct HookPayloadSeed<'a> {
    event: AntigravityHookEvent,
    payload: &'a mut HookPayload,
}

impl<'de> DeserializeSeed<'de> for HookPayloadSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(HookPayloadVisitor {
            event: self.event,
            payload: self.payload,
        })
    }
}

struct HookPayloadVisitor<'a> {
    event: AntigravityHookEvent,
    payload: &'a mut HookPayload,
}

impl<'de> Visitor<'de> for HookPayloadVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an Antigravity hook JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut conversation_seen = false;
        let mut workspaces_seen = false;
        let mut error_seen = false;
        let mut reason_seen = false;
        let mut idle_seen = false;
        let mut transcript_seen = false;
        let mut artifact_directory_seen = false;
        let mut model_seen = false;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "conversationId" => {
                    if conversation_seen {
                        return Err(serde::de::Error::duplicate_field("conversationId"));
                    }
                    conversation_seen = true;
                    let value: Option<String> = map.next_value()?;
                    if value
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_CONVERSATION_ID_BYTES)
                    {
                        return Err(serde::de::Error::custom(
                            "Antigravity conversation identifier exceeds the safety limit",
                        ));
                    }
                    self.payload.conversation_id = value;
                }
                "workspacePaths" => {
                    if workspaces_seen {
                        return Err(serde::de::Error::duplicate_field("workspacePaths"));
                    }
                    workspaces_seen = true;
                    self.payload.workspace_paths = map.next_value_seed(WorkspacePathsSeed)?;
                }
                "transcriptPath" => {
                    if transcript_seen {
                        return Err(serde::de::Error::duplicate_field("transcriptPath"));
                    }
                    transcript_seen = true;
                    let value: Option<String> = map.next_value()?;
                    if value
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_TRANSCRIPT_PATH_BYTES)
                    {
                        return Err(serde::de::Error::custom(
                            "Antigravity transcript path exceeds the safety limit",
                        ));
                    }
                    if let Some(value) = value.as_deref() {
                        self.payload
                            .observe_surface(classify_antigravity_surface(value));
                    }
                }
                "artifactDirectoryPath" => {
                    if artifact_directory_seen {
                        return Err(serde::de::Error::duplicate_field("artifactDirectoryPath"));
                    }
                    artifact_directory_seen = true;
                    let value: Option<String> = map.next_value()?;
                    if value
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_TRANSCRIPT_PATH_BYTES)
                    {
                        return Err(serde::de::Error::custom(
                            "Antigravity artifact directory path exceeds the safety limit",
                        ));
                    }
                    if let Some(value) = value.as_deref() {
                        self.payload
                            .observe_surface(classify_antigravity_surface(value));
                    }
                }
                "modelName" => {
                    if model_seen {
                        return Err(serde::de::Error::duplicate_field("modelName"));
                    }
                    model_seen = true;
                    let value: Option<String> = map.next_value()?;
                    self.payload.model_name_present = value
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty());
                    self.payload.model_kind =
                        value.as_deref().and_then(classify_antigravity_model_name);
                }
                "error" if self.event == AntigravityHookEvent::Stop => {
                    if error_seen {
                        return Err(serde::de::Error::duplicate_field("error"));
                    }
                    error_seen = true;
                    let value: Option<String> = map.next_value()?;
                    self.payload.had_error = value.is_some_and(|value| !value.trim().is_empty());
                }
                "terminationReason" if self.event == AntigravityHookEvent::Stop => {
                    if reason_seen {
                        return Err(serde::de::Error::duplicate_field("terminationReason"));
                    }
                    reason_seen = true;
                    let value: Option<String> = map.next_value()?;
                    if value
                        .as_ref()
                        .is_some_and(|value| value.len() > MAX_TERMINATION_REASON_BYTES)
                    {
                        return Err(serde::de::Error::custom(
                            "Antigravity termination reason exceeds the safety limit",
                        ));
                    }
                    self.payload.termination_reason = value;
                }
                "fullyIdle" if self.event == AntigravityHookEvent::Stop => {
                    if idle_seen {
                        return Err(serde::de::Error::duplicate_field("fullyIdle"));
                    }
                    idle_seen = true;
                    self.payload.fully_idle = map.next_value()?;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

struct WorkspacePathsSeed;

impl<'de> DeserializeSeed<'de> for WorkspacePathsSeed {
    type Value = Vec<String>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(WorkspacePathsVisitor)
    }
}

struct WorkspacePathsVisitor;

impl<'de> Visitor<'de> for WorkspacePathsVisitor {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of Antigravity workspace paths")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut paths = Vec::new();
        while let Some(path) = sequence.next_element::<String>()? {
            if paths.len() == MAX_WORKSPACE_PATHS {
                return Err(serde::de::Error::custom(
                    "Antigravity workspace path count exceeds the safety limit",
                ));
            }
            if path.len() > MAX_WORKSPACE_PATH_BYTES {
                return Err(serde::de::Error::custom(
                    "Antigravity workspace path exceeds the safety limit",
                ));
            }
            paths.push(path);
        }
        Ok(paths)
    }
}

/// Resolve Antigravity's documented global customization directory.
pub fn antigravity_config_dir_from_environment() -> Result<PathBuf, String> {
    home_directory()
        .map(|path| path.join(".gemini").join("config"))
        .map_err(|_| "could not determine the Antigravity global configuration directory".into())
}

/// Inspect VSParallel's named Antigravity hook without changing it.
pub fn antigravity_integration_status(
    config_dir: &Path,
    executable: &Path,
) -> Result<AntigravityIntegrationStatus, String> {
    let paths = IntegrationPaths::new(config_dir)?;
    let handlers = managed_handlers(executable)?;
    let (config, _) = read_config(&paths.config)?;
    status_from_config(&paths, &config, &handlers)
}

/// Install or repair VSParallel's named global Antigravity hook entry.
pub fn install_antigravity_integration(
    config_dir: &Path,
    executable: &Path,
) -> Result<AntigravityIntegrationChange, String> {
    let paths = IntegrationPaths::new(config_dir)?;
    validate_install_executable(executable)?;
    let handlers = managed_handlers(executable)?;
    let (mut config, original) = read_config(&paths.config)?;
    let states = event_states(&config, &handlers);
    let disabled = hooks_disabled(&config);
    if !disabled && states.values().all(|state| *state == EventState::Current) {
        return Ok(AntigravityIntegrationChange {
            changed: false,
            migrated: false,
            status: status_from_states(&paths, states, false, false),
        });
    }

    let migrated = match config.get(MANAGED_HOOK_NAME) {
        Some(entry) if managed_entry_is_owned(entry, &handlers) => true,
        Some(_) => {
            return Err(format!(
                "the Antigravity hook name '{MANAGED_HOOK_NAME}' is already used by an entry VSParallel does not own; it was left unchanged"
            ));
        }
        None => false,
    };
    config.insert(
        MANAGED_HOOK_NAME.to_string(),
        canonical_managed_entry(&handlers),
    );
    ensure_backup(&paths.backup, &original)?;
    atomic_write_json(&paths.config, &config)?;

    Ok(AntigravityIntegrationChange {
        changed: true,
        migrated,
        status: status_from_config(&paths, &config, &handlers)?,
    })
}

/// Remove only VSParallel's named global Antigravity hook entry.
pub fn uninstall_antigravity_integration(
    config_dir: &Path,
    executable: &Path,
) -> Result<AntigravityIntegrationChange, String> {
    let paths = IntegrationPaths::new(config_dir)?;
    let handlers = managed_handlers(executable)?;
    let (mut config, original) = read_config(&paths.config)?;
    let changed = config
        .get(MANAGED_HOOK_NAME)
        .is_some_and(|entry| managed_entry_is_owned(entry, &handlers));
    if changed {
        config.remove(MANAGED_HOOK_NAME);
        ensure_backup(&paths.backup, &original)?;
        atomic_write_json(&paths.config, &config)?;
    }

    Ok(AntigravityIntegrationChange {
        changed,
        migrated: false,
        status: status_from_config(&paths, &config, &handlers)?,
    })
}

/// Fail-open stdio entry point used by the installed executable.
pub fn run_antigravity_hook_stdio(event: AntigravityHookEvent) -> i32 {
    run_antigravity_hook(event, io::stdin().lock(), io::stdout().lock())
}

/// Testable hook entry point. It always exits successfully and always emits a
/// valid event-specific JSON response, even if parsing or persistence fails.
pub fn run_antigravity_hook<R: Read, W: Write>(
    event: AntigravityHookEvent,
    reader: R,
    writer: W,
) -> i32 {
    let root = crate::state::state_dir_from_environment();
    let ide_state_database = antigravity_ide_state_database_from_environment().ok();
    let ide_conversations_directory =
        antigravity_ide_conversations_directory_from_environment().ok();
    run_antigravity_hook_with(
        event,
        reader,
        writer,
        root.as_deref(),
        ide_state_database.as_deref(),
        ide_conversations_directory.as_deref(),
        unix_time_ms(),
    )
}

fn run_antigravity_hook_with<R: Read, W: Write>(
    event: AntigravityHookEvent,
    reader: R,
    mut writer: W,
    state_root: Result<&Path, &String>,
    ide_state_database: Option<&Path>,
    ide_conversations_directory: Option<&Path>,
    changed_at_ms: i64,
) -> i32 {
    let capped = CappedReader::new(reader, MAX_HOOK_INPUT_BYTES);
    let mut deserializer = serde_json::Deserializer::from_reader(capped);
    let mut payload = HookPayload::default();
    let parsed = HookPayloadSeed {
        event,
        payload: &mut payload,
    }
    .deserialize(&mut deserializer)
    .and_then(|()| deserializer.end());

    let model_update = if parsed.is_err() {
        HookModelUpdate::Preserve
    } else if payload.model_name_present {
        // A hook-supplied identifier is authoritative, including an explicit
        // but unrecognized value, which clears any stale closed classification.
        let ide_model_revision = (payload.surface == AntigravitySurface::Ide)
            .then(|| {
                payload
                    .conversation_id
                    .as_deref()
                    .and_then(|conversation_id| {
                        ide_conversations_directory.and_then(|directory| {
                            read_antigravity_ide_conversation_model(directory, conversation_id)
                                .ok()
                                .flatten()
                                .map(|model| model.revision)
                        })
                    })
            })
            .flatten();
        payload.ide_model_revision = ide_model_revision.clone();
        HookModelUpdate::Replace {
            model_kind: payload.model_kind,
            ide_model_revision,
        }
    } else if payload.surface == AntigravitySurface::Ide
        && matches!(
            event,
            AntigravityHookEvent::PreInvocation | AntigravityHookEvent::Stop
        )
    {
        // Antigravity commits the current user-input step immediately before
        // the first PreInvocation. Its embedded user_config therefore gives
        // Activity detected the model for this turn without waiting for the
        // executor row written near completion.
        let current_turn_signal = match (
            payload.conversation_id.as_deref(),
            ide_conversations_directory,
        ) {
            (Some(conversation_id), Some(directory)) => {
                read_antigravity_ide_current_turn_model(directory, conversation_id)
                    .unwrap_or(IdeCurrentTurnModel::Unusable)
            }
            _ => IdeCurrentTurnModel::Missing,
        };
        let current_turn_model = match &current_turn_signal {
            IdeCurrentTurnModel::Available(model) => Some(model.clone()),
            IdeCurrentTurnModel::Missing | IdeCurrentTurnModel::Unusable => None,
        };
        let allow_compatibility_fallback = current_turn_signal == IdeCurrentTurnModel::Missing;
        // Execution metadata remains a compatibility fallback for older IDE
        // schemas or a transient failure to read the current step.
        let execution_model = allow_compatibility_fallback
            .then(|| {
                payload
                    .conversation_id
                    .as_deref()
                    .and_then(|conversation_id| {
                        ide_conversations_directory.and_then(|directory| {
                            read_antigravity_ide_execution_model(directory, conversation_id)
                                .ok()
                                .flatten()
                        })
                    })
            })
            .flatten();
        // The global preference can lag model switches, so consult it only as
        // a compatibility fallback when the per-turn signal is unavailable.
        let selected_model = allow_compatibility_fallback
            .then(|| {
                (event == AntigravityHookEvent::PreInvocation || execution_model.is_none())
                    .then(|| {
                        ide_state_database.and_then(|database| {
                            read_antigravity_ide_selected_model(database).ok().flatten()
                        })
                    })
                    .flatten()
            })
            .flatten();
        HookModelUpdate::Ide {
            current_turn_model,
            execution_model,
            selected_model,
        }
    } else {
        HookModelUpdate::Preserve
    };

    let surface = payload.surface;
    let persistence_enabled = state_root.as_ref().is_ok_and(|root| {
        crate::state::integration_source_is_enabled_at(
            root,
            crate::state::IntegrationSource::AntigravityHooks,
        )
    });
    let (outcome, workspace_count) = if parsed.is_err() {
        (AntigravityHookOutcome::InvalidPayload, 0)
    } else if surface.state_directory().is_none() {
        (AntigravityHookOutcome::UnsupportedSurface, 0)
    } else if payload
        .conversation_id
        .as_deref()
        .is_none_or(|value| value.is_empty())
    {
        (AntigravityHookOutcome::MissingConversation, 0)
    } else {
        let records = records_from_payload(event, &payload, changed_at_ms);
        if records.is_empty() {
            (AntigravityHookOutcome::NoWorkspace, 0)
        } else if event == AntigravityHookEvent::PostToolUse {
            // Tool completion is not a terminal lifecycle transition. Do not
            // launch a competing read/merge/write that could race a Stop
            // process and reopen or replace a completed turn.
            (AntigravityHookOutcome::Recorded, records.len() as u32)
        } else if persistence_enabled {
            let (Ok(root), Some(directory)) = (state_root, surface.state_directory()) else {
                unreachable!("enabled Antigravity persistence has a state root and directory")
            };
            let attempted = records.len();
            let recorded = records
                .into_iter()
                .filter(|(record_key, record)| {
                    persist_record(
                        root,
                        directory,
                        record_key,
                        record,
                        event,
                        model_update.clone(),
                    )
                    .is_ok()
                })
                .count();
            if recorded == attempted {
                (AntigravityHookOutcome::Recorded, recorded as u32)
            } else {
                (AntigravityHookOutcome::PersistFailed, recorded as u32)
            }
        } else {
            (AntigravityHookOutcome::PersistFailed, 0)
        }
    };

    if persistence_enabled {
        let Ok(root) = state_root else {
            unreachable!("enabled Antigravity persistence has a state root")
        };
        let observation = AntigravityHookObservation {
            schema_version: SCHEMA_VERSION,
            event: event.observation_name().to_string(),
            surface: surface.observation_name().to_string(),
            outcome,
            observed_at_ms: changed_at_ms,
            workspace_count,
        };
        // Hook execution health contains fixed enums, a count, and a timestamp
        // only. Its own write remains fail-open like activity persistence.
        let _ = persist_hook_observation(root, surface, &observation);
    }

    let _ = writer.write_all(event.fail_open_output());
    let _ = writer.flush();
    0
}

fn records_from_payload(
    event: AntigravityHookEvent,
    payload: &HookPayload,
    changed_at_ms: i64,
) -> Vec<(String, HookRecord)> {
    let Some(conversation_id) = payload
        .conversation_id
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= MAX_CONVERSATION_ID_BYTES)
    else {
        return Vec::new();
    };
    let state = activity_state(event, payload);
    let session_key = sha256_hex(conversation_id.as_bytes());
    let mut normalized = BTreeSet::new();
    for raw in &payload.workspace_paths {
        if let Some(path) = normalize_workspace_path(raw) {
            normalized.insert(path);
        }
    }

    normalized
        .into_iter()
        .map(|cwd| {
            let cwd = cwd.to_string_lossy().into_owned();
            let mut identity = Vec::with_capacity(conversation_id.len() + cwd.len() + 1);
            identity.extend_from_slice(conversation_id.as_bytes());
            identity.push(0);
            identity.extend_from_slice(cwd.as_bytes());
            let record_key = sha256_hex(&identity);
            let record = HookRecord {
                schema_version: SCHEMA_VERSION,
                session_key: session_key.clone(),
                cwd,
                state: state.to_string(),
                changed_at_ms,
                model_kind: payload.model_kind,
                ide_model_revision: payload.ide_model_revision.clone(),
            };
            (record_key, record)
        })
        .collect()
}

fn activity_state(event: AntigravityHookEvent, payload: &HookPayload) -> &'static str {
    match event {
        AntigravityHookEvent::PreInvocation => "activity_detected",
        AntigravityHookEvent::PostToolUse => "activity_detected",
        AntigravityHookEvent::Stop if payload.had_error => "failed",
        AntigravityHookEvent::Stop => {
            let reason = payload
                .termination_reason
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if reason == "error" || reason.ends_with("_error") {
                "failed"
            } else if reason.contains("interrupt") || reason.contains("cancel") {
                "interrupted"
            } else if payload.fully_idle == Some(false) {
                "activity_detected"
            } else {
                "turn_finished"
            }
        }
    }
}

fn persist_record(
    root: &Path,
    directory_name: &str,
    record_key: &str,
    record: &HookRecord,
    event: AntigravityHookEvent,
    model_update: HookModelUpdate,
) -> Result<(), String> {
    if !is_sha256_key(record_key) || !is_sha256_key(&record.session_key) {
        return Err("invalid Antigravity record key".to_string());
    }
    if !matches!(directory_name, "antigravity" | "antigravity-ide") {
        return Err("invalid Antigravity state directory".to_string());
    }
    ensure_private_directory(root)?;
    let directory = root.join(directory_name);
    ensure_private_directory(&directory)?;

    let target = directory.join(format!("{record_key}.json"));
    let _lock = acquire_record_lock(&directory, record_key)?;
    let mut persisted = record.clone();
    if let Some(existing) = read_existing_hook_record(&target, &record.session_key, unix_time_ms())
    {
        // Hook commands run as separate processes. Never let a delayed older
        // command, or an equal-time lower-precedence command, replace a
        // lifecycle event that has already been recorded.
        if existing.changed_at_ms > record.changed_at_ms
            || (existing.changed_at_ms == record.changed_at_ms
                && antigravity_state_precedence(&existing.state)
                    > antigravity_state_precedence(&record.state))
        {
            return Ok(());
        }

        apply_hook_model_update(&mut persisted, Some(&existing), event, model_update);
    } else {
        apply_hook_model_update(&mut persisted, None, event, model_update);
    }
    let mut bytes = serde_json::to_vec(&persisted)
        .map_err(|error| format!("could not serialize Antigravity state: {error}"))?;
    bytes.push(b'\n');
    atomic_write_bytes(&target, &bytes, Some(0o600))
}

fn antigravity_state_precedence(state: &str) -> u8 {
    match state {
        "activity_detected" => 0,
        "turn_finished" => 1,
        "failed_or_interrupted" | "interrupted" => 2,
        "failed" => 3,
        "session_ended" => 4,
        _ => 0,
    }
}

struct RecordLock {
    _file: File,
}

#[cfg(unix)]
fn acquire_record_lock(directory: &Path, record_key: &str) -> Result<RecordLock, String> {
    if !is_sha256_key(record_key) {
        return Err("invalid Antigravity record lock key".to_string());
    }
    let path = directory.join(format!(".{record_key}.lock"));
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let file = options.open(&path).map_err(|error| {
        format!(
            "could not open Antigravity record lock {}: {error}",
            path.display()
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "could not inspect Antigravity record lock {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular lock file", path.display()));
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| {
            format!(
                "could not secure Antigravity record lock {}: {error}",
                path.display()
            )
        })?;

    for attempt in 0..RECORD_LOCK_ATTEMPTS {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            return Ok(RecordLock { _file: file });
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::Interrupted => continue,
            io::ErrorKind::WouldBlock if attempt + 1 < RECORD_LOCK_ATTEMPTS => {
                std::thread::sleep(RECORD_LOCK_RETRY);
            }
            io::ErrorKind::WouldBlock => break,
            _ => {
                return Err(format!(
                    "could not lock Antigravity record lock {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "timed out waiting for Antigravity record lock {}",
        path.display()
    ))
}

#[cfg(windows)]
fn acquire_record_lock(directory: &Path, record_key: &str) -> Result<RecordLock, String> {
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;

    if !is_sha256_key(record_key) {
        return Err("invalid Antigravity record lock key".to_string());
    }
    let path = directory.join(format!(".{record_key}.lock"));
    for attempt in 0..RECORD_LOCK_ATTEMPTS {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        match options.open(&path) {
            Ok(file) => {
                let metadata = file.metadata().map_err(|error| {
                    format!(
                        "could not inspect Antigravity record lock {}: {error}",
                        path.display()
                    )
                })?;
                if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                    return Err(format!("{} is not a regular lock file", path.display()));
                }
                return Ok(RecordLock { _file: file });
            }
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
                ) && attempt + 1 < RECORD_LOCK_ATTEMPTS =>
            {
                std::thread::sleep(RECORD_LOCK_RETRY);
            }
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
                ) =>
            {
                break;
            }
            Err(error) => {
                return Err(format!(
                    "could not open Antigravity record lock {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "timed out waiting for Antigravity record lock {}",
        path.display()
    ))
}

fn apply_hook_model_update(
    persisted: &mut HookRecord,
    existing: Option<&ExistingHookRecord>,
    event: AntigravityHookEvent,
    update: HookModelUpdate,
) {
    match update {
        HookModelUpdate::Preserve => {
            persisted.model_kind = existing.and_then(|record| record.model_kind);
            persisted.ide_model_revision =
                existing.and_then(|record| record.ide_model_revision.clone());
        }
        HookModelUpdate::Replace {
            model_kind,
            ide_model_revision,
        } => {
            persisted.model_kind = model_kind;
            persisted.ide_model_revision = ide_model_revision
                .or_else(|| existing.and_then(|record| record.ide_model_revision.clone()));
        }
        HookModelUpdate::Ide {
            current_turn_model,
            execution_model,
            selected_model,
        } => {
            if let Some(current_turn_model) = current_turn_model {
                // This revision belongs to the user input that triggered the
                // hook, so it takes precedence over both the previous turn's
                // executor row and the IDE's eventually-consistent global
                // preference.
                persisted.model_kind = model_kind_from_preference(current_turn_model.preference);
                persisted.ide_model_revision = Some(current_turn_model.revision);
                return;
            }
            let existing_revision =
                existing.and_then(|record| record.ide_model_revision.as_deref());
            let execution_is_new = execution_model
                .as_ref()
                .is_some_and(|model| Some(model.revision.as_str()) != existing_revision);
            if event == AntigravityHookEvent::PreInvocation
                && existing.is_none_or(|record| terminal_antigravity_state(&record.state))
            {
                // At a new turn boundary the latest committed execution row
                // normally still belongs to the turn that just finished. Use
                // the current selection for the active label, while recording
                // that latest revision as the baseline. A later executor row
                // can then confirm or correct the model without reverting to
                // the previous turn during the pending window.
                if let Some(selected_model) = selected_model {
                    persisted.model_kind = model_kind_from_preference(selected_model);
                    persisted.ide_model_revision = execution_model
                        .map(|model| model.revision)
                        .or_else(|| existing.and_then(|record| record.ide_model_revision.clone()));
                } else if let Some(execution_model) = execution_model {
                    persisted.model_kind = model_kind_from_preference(execution_model.preference);
                    persisted.ide_model_revision = Some(execution_model.revision);
                } else if let Some(existing) = existing {
                    persisted.model_kind = existing.model_kind;
                    persisted.ide_model_revision = existing.ide_model_revision.clone();
                }
            } else if execution_is_new {
                let execution_model = execution_model.expect("checked as present");
                persisted.model_kind = model_kind_from_preference(execution_model.preference);
                persisted.ide_model_revision = Some(execution_model.revision);
            } else if let Some(selected_model) = selected_model {
                // This path is reached only when the conversation database has
                // no USER_INPUT schema. It preserves compatibility with older
                // IDE builds; an unusable or contended current schema does not
                // permit this eventually-consistent fallback.
                persisted.model_kind = model_kind_from_preference(selected_model);
                persisted.ide_model_revision =
                    existing.and_then(|record| record.ide_model_revision.clone());
            } else if let Some(existing) = existing {
                // Repeated invocations and Stop for the same turn retain the
                // model already chosen at its first invocation.
                persisted.model_kind = existing.model_kind;
                persisted.ide_model_revision = existing.ide_model_revision.clone();
            } else {
                persisted.model_kind = selected_model.and_then(model_kind_from_preference);
                persisted.ide_model_revision = None;
            }
        }
    }
}

fn model_kind_from_preference(
    preference: IdeSelectedModelPreference,
) -> Option<AntigravityModelKind> {
    match preference {
        IdeSelectedModelPreference::Recognized(kind) => Some(kind),
        IdeSelectedModelPreference::Unrecognized => None,
    }
}

fn read_existing_hook_record(
    path: &Path,
    expected_session_key: &str,
    now_ms: i64,
) -> Option<ExistingHookRecord> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if is_link_or_reparse_point(&metadata)
        || !metadata.is_file()
        || metadata.len() > MAX_EXISTING_HOOK_RECORD_BYTES
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .ok()?
        .take(MAX_EXISTING_HOOK_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_EXISTING_HOOK_RECORD_BYTES {
        return None;
    }
    let mut record: ExistingHookRecord = serde_json::from_slice(&bytes).ok()?;
    if record.schema_version != SCHEMA_VERSION
        || record.session_key != expected_session_key
        || record.changed_at_ms < 0
        || record.changed_at_ms > now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
        || !known_antigravity_state(&record.state)
        || record
            .ide_model_revision
            .as_deref()
            .is_some_and(|revision| !is_sha256_key(revision))
    {
        return None;
    }
    if record.model_kind == Some(AntigravityModelKind::Unknown) {
        record.model_kind = None;
    }
    Some(record)
}

fn known_antigravity_state(state: &str) -> bool {
    matches!(
        state,
        "activity_detected"
            | "turn_finished"
            | "session_ended"
            | "failed_or_interrupted"
            | "failed"
            | "interrupted"
    )
}

fn terminal_antigravity_state(state: &str) -> bool {
    matches!(
        state,
        "turn_finished" | "session_ended" | "failed_or_interrupted" | "failed" | "interrupted"
    )
}

fn persist_hook_observation(
    root: &Path,
    surface: AntigravitySurface,
    observation: &AntigravityHookObservation,
) -> Result<(), String> {
    ensure_private_directory(root)?;
    let directory = root.join(HOOK_HEALTH_DIRECTORY);
    ensure_private_directory(&directory)?;
    let target = directory.join(surface.observation_file());
    let lock_key = sha256_hex(surface.observation_file().as_bytes());
    let _lock = acquire_record_lock(&directory, &lock_key)?;
    if read_hook_observation(root, surface, unix_time_ms())
        .ok()
        .flatten()
        .is_some_and(|existing| {
            existing.observed_at_ms > observation.observed_at_ms
                || (existing.observed_at_ms == observation.observed_at_ms
                    && hook_observation_precedence(&existing.event)
                        > hook_observation_precedence(&observation.event))
        })
    {
        return Ok(());
    }
    let mut bytes = serde_json::to_vec(observation)
        .map_err(|error| format!("could not serialize Antigravity hook health: {error}"))?;
    bytes.push(b'\n');
    atomic_write_bytes(&target, &bytes, Some(0o600))
}

fn hook_observation_precedence(event: &str) -> u8 {
    match event {
        "pre-invocation" => 0,
        "post-tool-use" => 1,
        "stop" => 2,
        _ => 0,
    }
}

pub(crate) fn antigravity_two_hook_observation(
    root: &Path,
    now_ms: i64,
) -> Result<Option<AntigravityHookObservation>, String> {
    read_hook_observation(root, AntigravitySurface::Two, now_ms)
}

pub(crate) fn antigravity_ide_hook_observation(
    root: &Path,
    now_ms: i64,
) -> Result<Option<AntigravityHookObservation>, String> {
    read_hook_observation(root, AntigravitySurface::Ide, now_ms)
}

fn read_hook_observation(
    root: &Path,
    surface: AntigravitySurface,
    now_ms: i64,
) -> Result<Option<AntigravityHookObservation>, String> {
    let directory = root.join(HOOK_HEALTH_DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) || !metadata.is_dir() => {
            return Err(format!("{} is not a safe directory", directory.display()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not inspect {}: {error}",
                directory.display()
            ));
        }
    }

    let path = directory.join(surface.observation_file());
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    };
    if is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if metadata.len() > MAX_HOOK_HEALTH_BYTES {
        return Err(format!(
            "{} exceeds the Antigravity hook health safety limit",
            path.display()
        ));
    }
    let file =
        File::open(&path).map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_HOOK_HEALTH_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_HOOK_HEALTH_BYTES {
        return Err(format!(
            "{} exceeds the Antigravity hook health safety limit",
            path.display()
        ));
    }
    let observation: AntigravityHookObservation = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not valid hook health JSON: {error}", path.display()))?;
    if observation.schema_version != SCHEMA_VERSION
        || observation.surface != surface.observation_name()
        || !EVENTS
            .iter()
            .any(|event| event.observation_name() == observation.event)
        || observation.observed_at_ms < 0
        || observation.observed_at_ms > now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
        || observation.workspace_count > MAX_WORKSPACE_PATHS as u32
        || (observation.outcome == AntigravityHookOutcome::Recorded
            && observation.workspace_count == 0)
    {
        return Err(format!(
            "{} contains invalid hook health data",
            path.display()
        ));
    }
    Ok(Some(observation))
}

fn classify_antigravity_surface(transcript_path: &str) -> AntigravitySurface {
    let mut components = transcript_path
        .split(['/', '\\'])
        .filter(|component| !component.is_empty());
    while let Some(component) = components.next() {
        if component == ".gemini" {
            return match components.next() {
                Some("antigravity") => AntigravitySurface::Two,
                Some("antigravity-ide") => AntigravitySurface::Ide,
                Some("antigravity-cli") => AntigravitySurface::Cli,
                _ => AntigravitySurface::Unknown,
            };
        }
    }
    AntigravitySurface::Unknown
}

fn antigravity_ide_state_database_from_environment() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let config = nonempty_env_path("APPDATA")
        .or_else(|| nonempty_env_path("LOCALAPPDATA"))
        .ok_or_else(|| "the Windows application-data directory is unavailable".to_string())?;
    #[cfg(target_os = "macos")]
    let config = home_directory()?
        .join("Library")
        .join("Application Support");
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let config = nonempty_env_path("XDG_CONFIG_HOME")
        .filter(|path| path.is_absolute())
        .unwrap_or(home_directory()?.join(".config"));

    Ok(config
        .join("Antigravity IDE")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb"))
}

pub(crate) fn antigravity_ide_conversations_directory_from_environment() -> Result<PathBuf, String>
{
    Ok(home_directory()?
        .join(".gemini")
        .join("antigravity-ide")
        .join("conversations"))
}

/// Resolve the latest current-turn model, with execution metadata as a
/// compatibility fallback, for each requested opaque conversation hash.
/// Database filename stems are hashed in memory and discarded; callers never
/// receive or persist Antigravity's raw conversation identifiers.
pub(crate) fn antigravity_ide_execution_models(
    directory: &Path,
    session_keys: &BTreeSet<String>,
) -> BTreeMap<String, IdeExecutionModel> {
    let mut models = BTreeMap::new();
    if session_keys.is_empty() || !bounded_directory(directory) {
        return models;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return models;
    };

    let mut database_count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("db") {
            continue;
        }
        database_count = database_count.saturating_add(1);
        if database_count > MAX_IDE_CONVERSATION_DATABASES {
            break;
        }
        let Some(conversation_id) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if !valid_ide_conversation_id(conversation_id) {
            continue;
        }
        let session_key = sha256_hex(conversation_id.as_bytes());
        if !session_keys.contains(&session_key) {
            continue;
        }
        if let Ok(Some(model)) = read_antigravity_ide_conversation_model_path(&path) {
            models.insert(session_key, model);
        }
    }
    models
}

fn read_antigravity_ide_conversation_model(
    directory: &Path,
    conversation_id: &str,
) -> Result<Option<IdeExecutionModel>, String> {
    if !bounded_directory(directory) || !valid_ide_conversation_id(conversation_id) {
        return Ok(None);
    }
    read_antigravity_ide_conversation_model_path(&directory.join(format!("{conversation_id}.db")))
}

fn read_antigravity_ide_current_turn_model(
    directory: &Path,
    conversation_id: &str,
) -> Result<IdeCurrentTurnModel, String> {
    if !bounded_directory(directory) || !valid_ide_conversation_id(conversation_id) {
        return Ok(IdeCurrentTurnModel::Missing);
    }
    read_antigravity_ide_current_turn_model_path(&directory.join(format!("{conversation_id}.db")))
}

fn read_antigravity_ide_execution_model(
    directory: &Path,
    conversation_id: &str,
) -> Result<Option<IdeExecutionModel>, String> {
    if !bounded_directory(directory) || !valid_ide_conversation_id(conversation_id) {
        return Ok(None);
    }
    read_antigravity_ide_execution_model_path(&directory.join(format!("{conversation_id}.db")))
}

fn read_antigravity_ide_conversation_model_path(
    database: &Path,
) -> Result<Option<IdeExecutionModel>, String> {
    let Some(connection) = open_antigravity_ide_conversation_database(database)? else {
        return Ok(None);
    };
    let revision_context = ide_model_revision_context(database);
    match read_antigravity_ide_current_turn_model_from_connection(&connection, &revision_context)? {
        IdeCurrentTurnModel::Available(model) => return Ok(Some(model)),
        IdeCurrentTurnModel::Unusable => return Ok(None),
        IdeCurrentTurnModel::Missing => {}
    }
    read_antigravity_ide_execution_model_from_connection(&connection, &revision_context)
}

fn read_antigravity_ide_current_turn_model_path(
    database: &Path,
) -> Result<IdeCurrentTurnModel, String> {
    let Some(connection) = open_antigravity_ide_conversation_database(database)? else {
        return Ok(IdeCurrentTurnModel::Missing);
    };
    read_antigravity_ide_current_turn_model_from_connection(
        &connection,
        &ide_model_revision_context(database),
    )
}

fn read_antigravity_ide_execution_model_path(
    database: &Path,
) -> Result<Option<IdeExecutionModel>, String> {
    let Some(connection) = open_antigravity_ide_conversation_database(database)? else {
        return Ok(None);
    };
    read_antigravity_ide_execution_model_from_connection(
        &connection,
        &ide_model_revision_context(database),
    )
}

fn ide_model_revision_context(database: &Path) -> Vec<u8> {
    database
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned().into_bytes())
        .unwrap_or_default()
}

fn open_antigravity_ide_conversation_database(
    database: &Path,
) -> Result<Option<Connection>, String> {
    if !bounded_regular_file(database, MAX_IDE_CONVERSATION_DB_BYTES, false, false)? {
        return Ok(None);
    }
    for suffix in ["-wal", "-shm"] {
        let Some(filename) = database.file_name().and_then(|value| value.to_str()) else {
            return Ok(None);
        };
        let auxiliary = database.with_file_name(format!("{filename}{suffix}"));
        if !bounded_regular_file(&auxiliary, MAX_IDE_CONVERSATION_AUX_BYTES, true, true)? {
            return Ok(None);
        }
    }

    open_bounded_read_only_database(database).map(Some)
}

/// Locate the model enum in the newest user-input step. SQLite's incremental
/// BLOB API reads bounded structural varints, while the streaming scanner seeks
/// over every unrelated length-delimited body, including user-authored text.
fn read_antigravity_ide_current_turn_model_from_connection(
    connection: &Connection,
    revision_context: &[u8],
) -> Result<IdeCurrentTurnModel, String> {
    let has_steps_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema \
             WHERE type = 'table' AND name = 'steps')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("could not inspect the IDE turn-model schema: {error}"))?;
    if !has_steps_table {
        return Ok(IdeCurrentTurnModel::Missing);
    }
    let (row_id, step_index, payload_type, payload_length) = {
        let mut statement = connection
            .prepare(
                "SELECT rowid, idx, typeof(step_payload), length(step_payload) \
                 FROM steps WHERE step_type = 14 ORDER BY idx DESC LIMIT 1",
            )
            .map_err(|error| format!("could not prepare the IDE turn-model query: {error}"))?;
        let mut rows = statement
            .query([])
            .map_err(|error| format!("could not query the IDE turn model: {error}"))?;
        let Some(row) = rows
            .next()
            .map_err(|error| format!("could not read the IDE turn model: {error}"))?
        else {
            return Ok(IdeCurrentTurnModel::Missing);
        };
        let row_id: i64 = row
            .get(0)
            .map_err(|error| format!("the IDE turn-model row is invalid: {error}"))?;
        let step_index: i64 = row
            .get(1)
            .map_err(|error| format!("the IDE turn-model index is invalid: {error}"))?;
        let payload_type: String = row
            .get(2)
            .map_err(|error| format!("the IDE turn-model type is invalid: {error}"))?;
        let payload_length: Option<i64> = row
            .get(3)
            .map_err(|error| format!("the IDE turn-model length is invalid: {error}"))?;
        (row_id, step_index, payload_type, payload_length)
    };

    let Some(payload_length) = payload_length else {
        return Ok(IdeCurrentTurnModel::Unusable);
    };
    if payload_type != "blob"
        || payload_length <= 0
        || payload_length as u64 > MAX_IDE_USER_INPUT_BYTES
    {
        // The newest row is unusable; never skip backwards to an older input.
        return Ok(IdeCurrentTurnModel::Unusable);
    }

    let mut payload = connection
        .blob_open(MAIN_DB, "steps", "step_payload", row_id, true)
        .map_err(|error| format!("could not open the IDE turn-model BLOB: {error}"))?;
    if i64::from(payload.size()) != payload_length {
        return Ok(IdeCurrentTurnModel::Unusable);
    }
    let Some((preference, model_enum)) =
        decode_ide_user_input_model(&mut payload, payload_length as u64)
    else {
        return Ok(IdeCurrentTurnModel::Unusable);
    };

    let mut revision_source = Vec::with_capacity(64 + revision_context.len());
    revision_source.extend_from_slice(b"antigravity-ide-user-input-model-v1\0");
    revision_source.extend_from_slice(revision_context);
    revision_source.push(0);
    revision_source.extend_from_slice(&step_index.to_le_bytes());
    revision_source.extend_from_slice(&model_enum.to_le_bytes());
    Ok(IdeCurrentTurnModel::Available(IdeExecutionModel {
        revision: sha256_hex(&revision_source),
        preference,
        source: IdeModelSource::CurrentTurn,
    }))
}

fn read_antigravity_ide_execution_model_from_connection(
    connection: &Connection,
    revision_context: &[u8],
) -> Result<Option<IdeExecutionModel>, String> {
    let mut statement = connection
        .prepare(
            "SELECT idx, CAST(substr(data, 1, ?1) AS BLOB), length(data) \
             FROM executor_metadata ORDER BY idx DESC LIMIT 1",
        )
        .map_err(|error| format!("could not prepare the IDE execution-model query: {error}"))?;
    let mut rows = statement
        .query([MAX_IDE_EXECUTOR_METADATA_BYTES.saturating_add(1) as i64])
        .map_err(|error| format!("could not query the IDE execution model: {error}"))?;
    let Some(row) = rows
        .next()
        .map_err(|error| format!("could not read the IDE execution model: {error}"))?
    else {
        return Ok(None);
    };
    let execution_index: i64 = row
        .get(0)
        .map_err(|error| format!("the IDE execution-model index is invalid: {error}"))?;
    let encoded: Vec<u8> = row
        .get(1)
        .map_err(|error| format!("the IDE execution model is invalid: {error}"))?;
    let encoded_length: i64 = row
        .get(2)
        .map_err(|error| format!("the IDE execution-model length is invalid: {error}"))?;
    if encoded_length <= 0
        || encoded_length > MAX_IDE_EXECUTOR_METADATA_BYTES as i64
        || encoded.len() != encoded_length as usize
    {
        return Ok(None);
    }
    Ok(decode_ide_executor_model(&encoded).map(|preference| {
        let mut revision_source = Vec::with_capacity(64 + revision_context.len());
        revision_source.extend_from_slice(b"antigravity-ide-execution-model-v1\0");
        revision_source.extend_from_slice(revision_context);
        revision_source.push(0);
        revision_source.extend_from_slice(&execution_index.to_le_bytes());
        revision_source.extend_from_slice(ide_model_preference_revision_token(preference));
        IdeExecutionModel {
            revision: sha256_hex(&revision_source),
            preference,
            source: IdeModelSource::Execution,
        }
    }))
}

fn decode_ide_user_input_model<R: Read + Seek>(
    reader: &mut R,
    payload_length: u64,
) -> Option<(IdeSelectedModelPreference, u64)> {
    if payload_length == 0 || payload_length > MAX_IDE_USER_INPUT_BYTES {
        return None;
    }
    let mut budget = MAX_IDE_PROTOBUF_STRUCTURE_BYTES;
    let user_input =
        protobuf_stream_length_delimited_field(reader, (0, payload_length), 19, &mut budget)
            .ok()
            .flatten()?;
    let queued = protobuf_stream_varint_field(reader, user_input, 6, &mut budget).ok()?;
    if queued.unwrap_or(0) != 0 {
        // A queued input is not necessarily the invocation currently running.
        return None;
    }
    let user_config = protobuf_stream_length_delimited_field(reader, user_input, 12, &mut budget)
        .ok()
        .flatten()?;
    let planner_config =
        protobuf_stream_length_delimited_field(reader, user_config, 1, &mut budget)
            .ok()
            .flatten()?;
    let requested_model =
        protobuf_stream_length_delimited_field(reader, planner_config, 15, &mut budget)
            .ok()
            .flatten()?;
    if requested_model.1.saturating_sub(requested_model.0) > 4 * 1024 {
        return None;
    }
    if protobuf_stream_field(reader, requested_model, 2, &mut budget)
        .ok()?
        .is_some()
    {
        // ModelOrAlias is a oneof. An alias is not a stable closed model enum,
        // and competing oneof members make the value ambiguous.
        return None;
    }
    let model_enum = protobuf_stream_varint_field(reader, requested_model, 1, &mut budget)
        .ok()
        .flatten()?;
    Some((ide_model_preference_from_enum(model_enum), model_enum))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtobufStreamValue {
    Varint(u64),
    LengthDelimited((u64, u64)),
}

fn protobuf_stream_length_delimited_field<R: Read + Seek>(
    reader: &mut R,
    window: (u64, u64),
    wanted: u64,
    budget: &mut usize,
) -> Result<Option<(u64, u64)>, ()> {
    match protobuf_stream_field(reader, window, wanted, budget)? {
        Some(ProtobufStreamValue::LengthDelimited(value)) => Ok(Some(value)),
        Some(ProtobufStreamValue::Varint(_)) => Err(()),
        None => Ok(None),
    }
}

fn protobuf_stream_varint_field<R: Read + Seek>(
    reader: &mut R,
    window: (u64, u64),
    wanted: u64,
    budget: &mut usize,
) -> Result<Option<u64>, ()> {
    match protobuf_stream_field(reader, window, wanted, budget)? {
        Some(ProtobufStreamValue::Varint(value)) => Ok(Some(value)),
        Some(ProtobufStreamValue::LengthDelimited(_)) => Err(()),
        None => Ok(None),
    }
}

fn protobuf_stream_field<R: Read + Seek>(
    reader: &mut R,
    (start, end): (u64, u64),
    wanted: u64,
    budget: &mut usize,
) -> Result<Option<ProtobufStreamValue>, ()> {
    if start > end || wanted == 0 || reader.seek(SeekFrom::Start(start)).map_err(|_| ())? != start {
        return Err(());
    }
    let mut cursor = start;
    let mut found = None;
    while cursor < end {
        let key = protobuf_stream_varint(reader, &mut cursor, end, budget)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        if field == 0 || field > 0x1fff_ffff {
            return Err(());
        }
        match wire_type {
            0 => {
                let value = protobuf_stream_varint(reader, &mut cursor, end, budget)?;
                if field == wanted && found.replace(ProtobufStreamValue::Varint(value)).is_some() {
                    return Err(());
                }
            }
            1 => {
                if field == wanted {
                    return Err(());
                }
                protobuf_stream_skip(reader, &mut cursor, end, 8)?;
            }
            2 => {
                let length = protobuf_stream_varint(reader, &mut cursor, end, budget)?;
                let body_end = cursor
                    .checked_add(length)
                    .filter(|value| *value <= end)
                    .ok_or(())?;
                if field == wanted
                    && found
                        .replace(ProtobufStreamValue::LengthDelimited((cursor, body_end)))
                        .is_some()
                {
                    return Err(());
                }
                protobuf_stream_seek(reader, &mut cursor, body_end)?;
            }
            5 => {
                if field == wanted {
                    return Err(());
                }
                protobuf_stream_skip(reader, &mut cursor, end, 4)?;
            }
            _ => return Err(()),
        }
    }
    Ok(found)
}

fn protobuf_stream_varint<R: Read>(
    reader: &mut R,
    cursor: &mut u64,
    end: u64,
    budget: &mut usize,
) -> Result<u64, ()> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        if *cursor >= end {
            return Err(());
        }
        *budget = budget.checked_sub(1).ok_or(())?;
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).map_err(|_| ())?;
        *cursor += 1;
        if shift == 63 && byte[0] & 0xfe != 0 {
            return Err(());
        }
        value |= u64::from(byte[0] & 0x7f) << shift;
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(())
}

fn protobuf_stream_skip<R: Seek>(
    reader: &mut R,
    cursor: &mut u64,
    end: u64,
    length: u64,
) -> Result<(), ()> {
    let target = cursor
        .checked_add(length)
        .filter(|value| *value <= end)
        .ok_or(())?;
    protobuf_stream_seek(reader, cursor, target)
}

fn protobuf_stream_seek<R: Seek>(reader: &mut R, cursor: &mut u64, target: u64) -> Result<(), ()> {
    if reader.seek(SeekFrom::Start(target)).map_err(|_| ())? != target {
        return Err(());
    }
    *cursor = target;
    Ok(())
}

fn decode_ide_executor_model(encoded: &[u8]) -> Option<IdeSelectedModelPreference> {
    if encoded.is_empty() || encoded.len() > MAX_IDE_EXECUTOR_METADATA_BYTES {
        return None;
    }
    let cascade_config = protobuf_length_delimited_field(encoded, 10)?;
    let planner_config = protobuf_length_delimited_field(cascade_config, 1)?;
    let model_name = protobuf_length_delimited_field(planner_config, 28)?;
    let model_name = std::str::from_utf8(model_name).ok()?;
    Some(match classify_antigravity_model_name(model_name) {
        Some(kind) => IdeSelectedModelPreference::Recognized(kind),
        None => IdeSelectedModelPreference::Unrecognized,
    })
}

fn valid_ide_conversation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn bounded_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| !is_link_or_reparse_point(&metadata) && metadata.is_dir())
}

fn bounded_regular_file(
    path: &Path,
    maximum_bytes: u64,
    missing_is_valid: bool,
    empty_is_valid: bool,
) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(missing_is_valid),
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    };
    Ok(!is_link_or_reparse_point(&metadata)
        && metadata.is_file()
        && (empty_is_valid || metadata.len() > 0)
        && metadata.len() <= maximum_bytes)
}

fn open_bounded_read_only_database(database: &Path) -> Result<Connection, String> {
    // SQLITE_OPEN_NOFOLLOW rejects a filename when any component traverses a
    // symbolic link. macOS exposes its real temporary directory through the
    // standard `/var` -> `/private/var` alias, so an otherwise regular,
    // bounded database below `temp_dir()` cannot be opened through the path
    // returned by the OS. Resolve only the parent directory aliases, append
    // the original filename, and retain NOFOLLOW for SQLite's final open. Not
    // canonicalizing the last component preserves protection against a file
    // replaced with a link between the bounded metadata check and this open.
    let parent = database
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", database.display()))?;
    let filename = database
        .file_name()
        .ok_or_else(|| format!("{} has no database filename", database.display()))?;
    let resolved_parent = fs::canonicalize(parent).map_err(|error| {
        format!(
            "could not resolve the parent of {} read-only: {error}",
            database.display(),
        )
    })?;
    let resolved = resolved_parent.join(filename);
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let connection = Connection::open_with_flags(&resolved, flags)
        .map_err(|error| format!("could not open {} read-only: {error}", database.display()))?;
    connection
        .busy_timeout(IDE_DB_BUSY_TIMEOUT)
        .map_err(|error| format!("could not bound the IDE database query: {error}"))?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|error| format!("could not enforce a read-only IDE query: {error}"))?;
    Ok(connection)
}

/// Compatibility model signal for the IDE hook contract, which does not
/// currently include `modelName`. This query reads one bounded preference value
/// from Antigravity IDE's editor state and immediately reduces its closed enum;
/// it does not inspect conversation storage or credential state.
fn read_antigravity_ide_selected_model(
    database: &Path,
) -> Result<Option<IdeSelectedModelPreference>, String> {
    if !bounded_regular_file(database, MAX_IDE_STATE_DB_BYTES, false, false)? {
        return Ok(None);
    }

    let connection = open_bounded_read_only_database(database)?;
    let mut statement = connection
        .prepare(
            "SELECT CAST(value AS BLOB) FROM ItemTable \
             WHERE key = 'antigravityUnifiedStateSync.modelPreferences' \
             AND length(value) BETWEEN 1 AND ?1 LIMIT 1",
        )
        .map_err(|error| format!("could not prepare the IDE model-preference query: {error}"))?;
    let mut rows = statement
        .query([MAX_IDE_MODEL_PREFERENCES_BYTES as i64])
        .map_err(|error| format!("could not query the IDE model preference: {error}"))?;
    let Some(row) = rows
        .next()
        .map_err(|error| format!("could not read the IDE model preference: {error}"))?
    else {
        return Ok(None);
    };
    let encoded: Vec<u8> = row
        .get(0)
        .map_err(|error| format!("the IDE model preference is invalid: {error}"))?;
    decode_ide_model_preferences(&encoded)
        .map(Some)
        .ok_or_else(|| "the IDE model preference has an invalid bounded encoding".to_string())
}

#[cfg(test)]
fn model_kind_from_ide_preferences(encoded: &[u8]) -> Option<AntigravityModelKind> {
    match decode_ide_model_preferences(encoded)? {
        IdeSelectedModelPreference::Recognized(kind) => Some(kind),
        IdeSelectedModelPreference::Unrecognized => None,
    }
}

fn decode_ide_model_preferences(encoded: &[u8]) -> Option<IdeSelectedModelPreference> {
    if encoded.len() > MAX_IDE_MODEL_PREFERENCES_BYTES {
        return None;
    }
    let preferences = decode_base64(encoded)?;
    let wrapped = protobuf_map_value(&preferences, 1, b"last_selected_agent_model_sentinel_key")?;
    let encoded_model = protobuf_length_delimited_field(wrapped, 1)?;
    let model = decode_base64(encoded_model)?;
    Some(ide_model_preference_from_enum(protobuf_varint_field(
        &model, 2,
    )?))
}

fn ide_model_preference_from_enum(model: u64) -> IdeSelectedModelPreference {
    let kind = match model {
        // Observed stable Antigravity IDE 2.x enum values. Unknown future values
        // deliberately fall back to an unqualified Antigravity label.
        1071 => AntigravityModelKind::Gemini36FlashHigh,
        1072 => AntigravityModelKind::Gemini36FlashMedium,
        1073 => AntigravityModelKind::Gemini,
        1084 | 1020 | 1187 => AntigravityModelKind::Gemini35Flash,
        1016 => AntigravityModelKind::Gemini31ProHigh,
        1036 => AntigravityModelKind::Gemini31ProLow,
        1035 => AntigravityModelKind::ClaudeSonnet46Thinking,
        1026 => AntigravityModelKind::ClaudeOpus46Thinking,
        342 => AntigravityModelKind::GptOss120bMedium,
        _ => return IdeSelectedModelPreference::Unrecognized,
    };
    IdeSelectedModelPreference::Recognized(kind)
}

fn ide_model_preference_revision_token(preference: IdeSelectedModelPreference) -> &'static [u8] {
    match preference {
        IdeSelectedModelPreference::Recognized(kind) => match kind {
            AntigravityModelKind::Automatic => b"automatic",
            AntigravityModelKind::Gemini => b"gemini",
            AntigravityModelKind::Gemini36FlashMedium => b"gemini_3_6_flash_medium",
            AntigravityModelKind::Gemini36FlashHigh => b"gemini_3_6_flash_high",
            AntigravityModelKind::Gemini35Flash => b"gemini_3_5_flash",
            AntigravityModelKind::Gemini31ProHigh => b"gemini_3_1_pro_high",
            AntigravityModelKind::Gemini31ProLow => b"gemini_3_1_pro_low",
            AntigravityModelKind::Gemini3Flash => b"gemini_3_flash",
            AntigravityModelKind::Claude => b"claude",
            AntigravityModelKind::ClaudeSonnet46Thinking => b"claude_sonnet_4_6_thinking",
            AntigravityModelKind::ClaudeOpus46Thinking => b"claude_opus_4_6_thinking",
            AntigravityModelKind::GptOss => b"gpt_oss",
            AntigravityModelKind::GptOss120b => b"gpt_oss_120b",
            AntigravityModelKind::GptOss120bMedium => b"gpt_oss_120b_medium",
            AntigravityModelKind::Unknown => b"unknown",
        },
        IdeSelectedModelPreference::Unrecognized => b"unrecognized",
    }
}

fn decode_base64(encoded: &[u8]) -> Option<Vec<u8>> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(4) {
        return None;
    }
    let mut decoded = Vec::with_capacity(encoded.len() / 4 * 3);
    for (chunk_index, chunk) in encoded.chunks_exact(4).enumerate() {
        let last = chunk_index + 1 == encoded.len() / 4;
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c_padding = chunk[2] == b'=';
        let d_padding = chunk[3] == b'=';
        if c_padding && !d_padding || (!last && (c_padding || d_padding)) {
            return None;
        }
        let c = if c_padding {
            0
        } else {
            base64_value(chunk[2])?
        };
        let d = if d_padding {
            0
        } else {
            base64_value(chunk[3])?
        };
        decoded.push((a << 2) | (b >> 4));
        if !c_padding {
            decoded.push((b << 4) | (c >> 2));
        }
        if !d_padding {
            decoded.push((c << 6) | d);
        }
        if (c_padding && b & 0x0f != 0) || (d_padding && !c_padding && c & 0x03 != 0) {
            return None;
        }
    }
    Some(decoded)
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn protobuf_map_value<'a>(bytes: &'a [u8], map_field: u64, wanted_key: &[u8]) -> Option<&'a [u8]> {
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let key = protobuf_varint(bytes, &mut cursor)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        if field == 0 {
            return None;
        }
        match wire_type {
            0 => {
                protobuf_varint(bytes, &mut cursor)?;
            }
            1 => cursor = cursor.checked_add(8).filter(|end| *end <= bytes.len())?,
            2 => {
                let length = usize::try_from(protobuf_varint(bytes, &mut cursor)?).ok()?;
                let end = cursor
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())?;
                if field == map_field {
                    let entry = &bytes[cursor..end];
                    if protobuf_length_delimited_field(entry, 1) == Some(wanted_key) {
                        return protobuf_length_delimited_field(entry, 2);
                    }
                }
                cursor = end;
            }
            5 => cursor = cursor.checked_add(4).filter(|end| *end <= bytes.len())?,
            _ => return None,
        }
    }
    None
}

fn protobuf_length_delimited_field(bytes: &[u8], wanted: u64) -> Option<&[u8]> {
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let key = protobuf_varint(bytes, &mut cursor)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        if field == 0 {
            return None;
        }
        match wire_type {
            0 => {
                protobuf_varint(bytes, &mut cursor)?;
            }
            1 => cursor = cursor.checked_add(8).filter(|end| *end <= bytes.len())?,
            2 => {
                let length = usize::try_from(protobuf_varint(bytes, &mut cursor)?).ok()?;
                let end = cursor
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())?;
                if field == wanted {
                    return Some(&bytes[cursor..end]);
                }
                cursor = end;
            }
            5 => cursor = cursor.checked_add(4).filter(|end| *end <= bytes.len())?,
            _ => return None,
        }
    }
    None
}

fn protobuf_varint_field(bytes: &[u8], wanted: u64) -> Option<u64> {
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let key = protobuf_varint(bytes, &mut cursor)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        if field == 0 {
            return None;
        }
        match wire_type {
            0 => {
                let value = protobuf_varint(bytes, &mut cursor)?;
                if field == wanted {
                    return Some(value);
                }
            }
            1 => cursor = cursor.checked_add(8).filter(|end| *end <= bytes.len())?,
            2 => {
                let length = usize::try_from(protobuf_varint(bytes, &mut cursor)?).ok()?;
                cursor = cursor
                    .checked_add(length)
                    .filter(|end| *end <= bytes.len())?;
            }
            5 => cursor = cursor.checked_add(4).filter(|end| *end <= bytes.len())?,
            _ => return None,
        }
    }
    None
}

fn protobuf_varint(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*cursor)?;
        *cursor += 1;
        if shift == 63 && byte & 0xfe != 0 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn classify_antigravity_model_name(value: &str) -> Option<AntigravityModelKind> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_MODEL_NAME_BYTES || !value.is_ascii() {
        return None;
    }

    // Normalize only a small identifier alphabet, then immediately reduce the
    // value to a closed token. This accepts documented slugs and familiar
    // display spellings without persisting custom deployment/model names.
    let mut normalized = String::with_capacity(value.len());
    let mut separator = false;
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push('-');
            }
            normalized.push((byte as char).to_ascii_lowercase());
            separator = false;
        } else if matches!(byte, b'-' | b'_' | b'.' | b' ' | b'(' | b')') {
            separator = true;
        } else {
            return None;
        }
    }

    match normalized.as_str() {
        "auto" | "automatic" => Some(AntigravityModelKind::Automatic),
        "gemini-3-6-flash-medium" => Some(AntigravityModelKind::Gemini36FlashMedium),
        "gemini-3-6-flash-high" => Some(AntigravityModelKind::Gemini36FlashHigh),
        "gemini-3-5-flash" => Some(AntigravityModelKind::Gemini35Flash),
        "gemini-3-1-pro-high" => Some(AntigravityModelKind::Gemini31ProHigh),
        "gemini-3-1-pro-low" => Some(AntigravityModelKind::Gemini31ProLow),
        "gemini-3-flash" => Some(AntigravityModelKind::Gemini3Flash),
        "gemini-3-flash-agent" => Some(AntigravityModelKind::Gemini35Flash),
        "claude-sonnet-4-6" | "claude-sonnet-4-6-thinking" => {
            Some(AntigravityModelKind::ClaudeSonnet46Thinking)
        }
        "claude-opus-4-6-thinking" => Some(AntigravityModelKind::ClaudeOpus46Thinking),
        "gpt-oss-120b" => Some(AntigravityModelKind::GptOss120b),
        "gpt-oss-120b-medium" => Some(AntigravityModelKind::GptOss120bMedium),
        "gemini" => Some(AntigravityModelKind::Gemini),
        "claude" => Some(AntigravityModelKind::Claude),
        "gpt-oss" => Some(AntigravityModelKind::GptOss),
        name if name.starts_with("gemini-") => Some(AntigravityModelKind::Gemini),
        name if name.starts_with("claude-") => Some(AntigravityModelKind::Claude),
        name if name.starts_with("gpt-oss-") => Some(AntigravityModelKind::GptOss),
        _ => None,
    }
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
                "the Antigravity global configuration directory must be an absolute path"
                    .to_string(),
            );
        }
        Ok(Self {
            config: config_dir.join(HOOKS_FILENAME),
            backup: config_dir.join(BACKUP_FILENAME),
        })
    }
}

fn managed_handlers(executable: &Path) -> Result<BTreeMap<AntigravityHookEvent, Value>, String> {
    EVENTS
        .into_iter()
        .map(|event| managed_handler(executable, event).map(|handler| (event, handler)))
        .collect()
}

fn managed_handler(executable: &Path, event: AntigravityHookEvent) -> Result<Value, String> {
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

fn canonical_managed_entry(handlers: &BTreeMap<AntigravityHookEvent, Value>) -> Value {
    serde_json::json!({
        "enabled": true,
        "PreInvocation": [handlers[&AntigravityHookEvent::PreInvocation].clone()],
        "PostToolUse": [{
            "matcher": "*",
            "hooks": [handlers[&AntigravityHookEvent::PostToolUse].clone()],
        }],
        "Stop": [handlers[&AntigravityHookEvent::Stop].clone()],
    })
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
    Ok((object, raw))
}

fn event_states(
    config: &Map<String, Value>,
    handlers: &BTreeMap<AntigravityHookEvent, Value>,
) -> BTreeMap<AntigravityHookEvent, EventState> {
    let entry = config.get(MANAGED_HOOK_NAME).and_then(Value::as_object);
    EVENTS
        .into_iter()
        .map(|event| {
            let state = match entry.and_then(|entry| entry.get(event.config_name())) {
                None => EventState::Missing,
                Some(value) if event_value_is_current(event, value, &handlers[&event]) => {
                    EventState::Current
                }
                Some(_) => EventState::Stale,
            };
            (event, state)
        })
        .collect()
}

fn event_value_is_current(event: AntigravityHookEvent, value: &Value, handler: &Value) -> bool {
    match event {
        AntigravityHookEvent::PreInvocation | AntigravityHookEvent::Stop => {
            value == &Value::Array(vec![handler.clone()])
        }
        AntigravityHookEvent::PostToolUse => {
            value
                == &serde_json::json!([{
                    "matcher": "*",
                    "hooks": [handler.clone()],
                }])
        }
    }
}

fn hooks_disabled(config: &Map<String, Value>) -> bool {
    config
        .get(MANAGED_HOOK_NAME)
        .and_then(Value::as_object)
        .and_then(|entry| entry.get("enabled"))
        .and_then(Value::as_bool)
        == Some(false)
}

fn managed_entry_is_owned(entry: &Value, handlers: &BTreeMap<AntigravityHookEvent, Value>) -> bool {
    let Some(entry) = entry.as_object() else {
        return false;
    };

    // Installation repairs and uninstallation replace/remove the whole named
    // entry, so ownership must cover the whole entry. Merely finding one
    // command with a familiar suffix is insufficient: a user may have added
    // another handler to this entry, and that handler must never be discarded.
    if entry.is_empty()
        || entry
            .keys()
            .any(|key| key != "enabled" && !EVENTS.iter().any(|event| event.config_name() == key))
        || entry
            .get("enabled")
            .is_some_and(|value| !value.is_boolean())
    {
        return false;
    }

    let mut managed_event_count = 0;
    for event in EVENTS {
        let Some(value) = entry.get(event.config_name()) else {
            continue;
        };
        if !managed_event_value_is_owned(event, value, &handlers[&event]) {
            return false;
        }
        managed_event_count += 1;
    }
    managed_event_count > 0
}

fn managed_event_value_is_owned(
    event: AntigravityHookEvent,
    value: &Value,
    expected_handler: &Value,
) -> bool {
    let Some(groups) = value.as_array() else {
        return false;
    };
    if groups.len() != 1 {
        return false;
    }

    match event {
        AntigravityHookEvent::PreInvocation | AntigravityHookEvent::Stop => {
            managed_handler_is_owned(&groups[0], event, expected_handler)
        }
        AntigravityHookEvent::PostToolUse => {
            let Some(group) = groups[0].as_object() else {
                return false;
            };
            if group.keys().any(|key| key != "matcher" && key != "hooks")
                || group.get("matcher").and_then(Value::as_str) != Some("*")
            {
                return false;
            }
            let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
                return false;
            };
            handlers.len() == 1 && managed_handler_is_owned(&handlers[0], event, expected_handler)
        }
    }
}

fn managed_handler_is_owned(
    handler: &Value,
    event: AntigravityHookEvent,
    expected_handler: &Value,
) -> bool {
    let Some(handler) = handler.as_object() else {
        return false;
    };
    if handler
        .keys()
        .any(|key| key != "type" && key != "command" && key != "timeout")
        || handler
            .get("type")
            .is_some_and(|value| value.as_str() != Some("command"))
        || handler.get("timeout").is_some_and(|value| !value.is_u64())
    {
        return false;
    }
    let handler = Value::Object(handler.clone());
    &handler == expected_handler || historical_vsparallel_handler(&handler, event)
}

fn historical_vsparallel_handler(handler: &Value, event: AntigravityHookEvent) -> bool {
    let Some(command) = handler
        .as_object()
        .and_then(|handler| handler.get("command"))
        .and_then(Value::as_str)
    else {
        return false;
    };
    let suffix = format!(" {HOOK_ARGUMENT} {}", event.cli_argument());
    let Some(prefix) = command.trim().strip_suffix(&suffix) else {
        return false;
    };
    let prefix = prefix.trim();
    let executable = if prefix.len() >= 2 && prefix.starts_with('\'') && prefix.ends_with('\'') {
        let inner = &prefix[1..prefix.len() - 1];
        (!inner.contains('\'')).then_some(inner)
    } else if prefix.len() >= 2 && prefix.starts_with('"') && prefix.ends_with('"') {
        let inner = &prefix[1..prefix.len() - 1];
        (!inner.contains('"')).then_some(inner)
    } else if !prefix.chars().any(char::is_whitespace)
        && !prefix.contains([';', '&', '|', '`', '$', '<', '>', '(', ')'])
    {
        Some(prefix)
    } else {
        None
    };
    executable.is_some_and(historical_vsparallel_executable)
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

fn status_from_config(
    paths: &IntegrationPaths,
    config: &Map<String, Value>,
    handlers: &BTreeMap<AntigravityHookEvent, Value>,
) -> Result<AntigravityIntegrationStatus, String> {
    let conflict = config
        .get(MANAGED_HOOK_NAME)
        .is_some_and(|entry| !managed_entry_is_owned(entry, handlers));
    Ok(status_from_states(
        paths,
        event_states(config, handlers),
        hooks_disabled(config),
        conflict,
    ))
}

fn status_from_states(
    paths: &IntegrationPaths,
    states: BTreeMap<AntigravityHookEvent, EventState>,
    disabled: bool,
    conflict: bool,
) -> AntigravityIntegrationStatus {
    let current = states
        .values()
        .filter(|state| **state == EventState::Current)
        .count();
    let stale = states
        .values()
        .filter(|state| **state == EventState::Stale)
        .count();
    let state = if conflict {
        "conflict"
    } else if disabled && current == EVENTS.len() {
        "disabled"
    } else if !disabled && current == EVENTS.len() {
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
            "Antigravity activity monitoring is configured; it records after an agent turn, not when a Project is merely opened."
        }
        "disabled" => "The VSParallel Antigravity hook entry is disabled.",
        "conflict" => {
            "The Antigravity hook name reserved for VSParallel is already used by another entry. Rename or remove that entry before installing."
        }
        "not_installed" => "Antigravity activity monitoring is not installed.",
        "stale" => "An older VSParallel Antigravity integration can be repaired.",
        _ => "Antigravity activity monitoring is only partially installed.",
    };

    AntigravityIntegrationStatus {
        state: state.to_string(),
        installed: state == "installed",
        config_path: paths.config.to_string_lossy().into_owned(),
        backup_path: paths.backup.to_string_lossy().into_owned(),
        event_states: states
            .into_iter()
            .map(|(event, state)| (event.config_name().to_string(), state.as_str().to_string()))
            .collect(),
        hooks_disabled: disabled,
        message: message.to_string(),
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

fn atomic_write_json(path: &Path, config: &Map<String, Value>) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("could not serialize Antigravity hooks: {error}"))?;
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
    if raw.trim().is_empty() || raw.len() > MAX_WORKSPACE_PATH_BYTES || raw.contains('\0') {
        return None;
    }
    let path = PathBuf::from(raw.trim());
    if !path.is_absolute() {
        return None;
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
                "Antigravity hook payload exceeds the safety limit",
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
                        "Antigravity hook payload exceeds the safety limit",
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

    fn hook_with_root(
        event: AntigravityHookEvent,
        input: &str,
        root: &Path,
        now: i64,
    ) -> (i32, String) {
        let mut output = Vec::new();
        let code = run_antigravity_hook_with(
            event,
            input.as_bytes(),
            &mut output,
            Ok(root),
            None,
            None,
            now,
        );
        (code, String::from_utf8(output).unwrap())
    }

    fn hook_with_root_and_ide_state(
        event: AntigravityHookEvent,
        input: &str,
        root: &Path,
        ide_state_database: &Path,
        now: i64,
    ) -> (i32, String) {
        let mut output = Vec::new();
        let code = run_antigravity_hook_with(
            event,
            input.as_bytes(),
            &mut output,
            Ok(root),
            Some(ide_state_database),
            None,
            now,
        );
        (code, String::from_utf8(output).unwrap())
    }

    fn push_test_varint(mut value: u64, output: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn test_length_delimited_field(field: u64, value: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        push_test_varint((field << 3) | 2, &mut output);
        push_test_varint(value.len() as u64, &mut output);
        output.extend_from_slice(value);
        output
    }

    fn test_varint_field(field: u64, value: u64) -> Vec<u8> {
        let mut output = Vec::new();
        push_test_varint(field << 3, &mut output);
        push_test_varint(value, &mut output);
        output
    }

    fn test_base64_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::new();
        for chunk in bytes.chunks(3) {
            let a = chunk[0];
            let b = chunk.get(1).copied().unwrap_or(0);
            let c = chunk.get(2).copied().unwrap_or(0);
            encoded.push(ALPHABET[(a >> 2) as usize] as char);
            encoded.push(ALPHABET[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
            encoded.push(if chunk.len() > 1 {
                ALPHABET[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char
            } else {
                '='
            });
            encoded.push(if chunk.len() > 2 {
                ALPHABET[(c & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        encoded
    }

    fn test_ide_model_preference_entry(key: &[u8], model_enum: u64) -> Vec<u8> {
        let model = test_base64_encode(&test_varint_field(2, model_enum));
        let wrapped = test_length_delimited_field(1, model.as_bytes());
        let mut entry = test_length_delimited_field(1, key);
        entry.extend(test_length_delimited_field(2, &wrapped));
        entry
    }

    fn test_ide_model_preferences(model_enum: u64) -> String {
        let entry =
            test_ide_model_preference_entry(b"last_selected_agent_model_sentinel_key", model_enum);
        test_base64_encode(&test_length_delimited_field(1, &entry))
    }

    fn write_ide_state_database(database: &Path, model_enum: u64) {
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS ItemTable \
                 (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
                rusqlite::params![
                    "antigravityUnifiedStateSync.modelPreferences",
                    test_ide_model_preferences(model_enum)
                ],
            )
            .unwrap();
    }

    fn test_ide_executor_metadata(model_name: &str) -> Vec<u8> {
        let planner = test_length_delimited_field(28, model_name.as_bytes());
        let cascade = test_length_delimited_field(1, &planner);
        test_length_delimited_field(10, &cascade)
    }

    fn write_ide_conversation_model(database: &Path, index: i64, model_name: &str) {
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS executor_metadata \
                 (idx INTEGER PRIMARY KEY, data BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO executor_metadata (idx, data) VALUES (?1, ?2)",
                rusqlite::params![index, test_ide_executor_metadata(model_name)],
            )
            .unwrap();
    }

    fn test_ide_user_input_step(model_enum: u64, queued: bool, private_padding: &[u8]) -> Vec<u8> {
        let requested_model = test_varint_field(1, model_enum);
        let planner_config = test_length_delimited_field(15, &requested_model);
        let user_config = test_length_delimited_field(1, &planner_config);

        let mut user_input = test_length_delimited_field(1, private_padding);
        if queued {
            user_input.extend(test_varint_field(6, 1));
        }
        user_input.extend(test_length_delimited_field(12, &user_config));
        // Field 13 is the previous user config and must never override the
        // current config in field 12.
        let previous_model = test_varint_field(1, 1072);
        let previous_planner = test_length_delimited_field(15, &previous_model);
        let previous_config = test_length_delimited_field(1, &previous_planner);
        user_input.extend(test_length_delimited_field(13, &previous_config));

        let mut step = test_length_delimited_field(2, private_padding);
        step.extend(test_length_delimited_field(19, &user_input));
        step
    }

    fn write_ide_conversation_turn_model(database: &Path, index: i64, model_enum: u64) {
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        let connection = Connection::open(database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS steps \
                 (idx INTEGER PRIMARY KEY, step_type INTEGER, step_payload BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO steps (idx, step_type, step_payload) \
                 VALUES (?1, 14, ?2)",
                rusqlite::params![
                    index,
                    test_ide_user_input_step(model_enum, false, b"private user input")
                ],
            )
            .unwrap();
    }

    struct CountingCursor {
        inner: io::Cursor<Vec<u8>>,
        bytes_read: usize,
    }

    impl Read for CountingCursor {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = self.inner.read(buffer)?;
            self.bytes_read += count;
            Ok(count)
        }
    }

    impl Seek for CountingCursor {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            self.inner.seek(position)
        }
    }

    fn state_record_paths(root: &Path, directory: &str) -> Vec<PathBuf> {
        let mut paths: Vec<_> = fs::read_dir(root.join(directory))
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                (path.extension().and_then(|extension| extension.to_str()) == Some("json"))
                    .then_some(path)
            })
            .collect();
        paths.sort();
        paths
    }

    fn state_record(root: &Path, directory: &str) -> Value {
        let paths = state_record_paths(root, directory);
        assert_eq!(paths.len(), 1);
        serde_json::from_slice(&fs::read(&paths[0]).unwrap()).unwrap()
    }

    fn state_records(root: &Path) -> Vec<Value> {
        state_record_paths(root, "antigravity")
            .into_iter()
            .map(|path| serde_json::from_slice(&fs::read(path).unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn cli_event_arguments_are_stable() {
        for event in EVENTS {
            assert_eq!(
                AntigravityHookEvent::from_cli_argument(event.cli_argument()),
                Some(event)
            );
        }
        assert_eq!(AntigravityHookEvent::from_cli_argument("unknown"), None);
    }

    #[test]
    fn record_lock_serializes_competing_hook_writes_and_preserves_newest_state() {
        use std::sync::mpsc;

        let temp = TempDir::new().unwrap();
        let directory = temp.path().join("antigravity");
        ensure_private_directory(&directory).unwrap();
        let record_key = sha256_hex(b"record");
        let session_key = sha256_hex(b"session");
        let held = acquire_record_lock(&directory, &record_key).unwrap();
        let (finished_tx, finished_rx) = mpsc::channel();

        let newer_root = temp.path().to_path_buf();
        let newer_key = record_key.clone();
        let newer_session = session_key.clone();
        let newer_tx = finished_tx.clone();
        let newer = std::thread::spawn(move || {
            let record = HookRecord {
                schema_version: SCHEMA_VERSION,
                session_key: newer_session,
                cwd: "/workspace".to_string(),
                state: "turn_finished".to_string(),
                changed_at_ms: 20,
                model_kind: None,
                ide_model_revision: None,
            };
            let result = persist_record(
                &newer_root,
                "antigravity",
                &newer_key,
                &record,
                AntigravityHookEvent::Stop,
                HookModelUpdate::Preserve,
            );
            newer_tx.send(result).unwrap();
        });

        let older_root = temp.path().to_path_buf();
        let older_key = record_key.clone();
        let older_tx = finished_tx.clone();
        let older = std::thread::spawn(move || {
            let record = HookRecord {
                schema_version: SCHEMA_VERSION,
                session_key,
                cwd: "/workspace".to_string(),
                state: "activity_detected".to_string(),
                changed_at_ms: 10,
                model_kind: None,
                ide_model_revision: None,
            };
            let result = persist_record(
                &older_root,
                "antigravity",
                &older_key,
                &record,
                AntigravityHookEvent::PreInvocation,
                HookModelUpdate::Preserve,
            );
            older_tx.send(result).unwrap();
        });
        drop(finished_tx);

        assert!(matches!(
            finished_rx.recv_timeout(Duration::from_millis(25)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(held);
        for _ in 0..2 {
            assert!(finished_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_ok());
        }
        newer.join().unwrap();
        older.join().unwrap();

        let saved: Value = serde_json::from_slice(
            &fs::read(directory.join(format!("{record_key}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["state"], "turn_finished");
        assert_eq!(saved["changedAtMs"], 20);
        assert!(directory.join(format!(".{record_key}.lock")).is_file());
    }

    #[test]
    fn record_lock_file_persists_and_can_be_reacquired() {
        let temp = TempDir::new().unwrap();
        let directory = temp.path().join("antigravity");
        ensure_private_directory(&directory).unwrap();
        let record_key = sha256_hex(b"persistent-record-lock");
        let lock_path = directory.join(format!(".{record_key}.lock"));

        let first = acquire_record_lock(&directory, &record_key).unwrap();
        drop(first);
        assert!(lock_path.is_file());

        let second = acquire_record_lock(&directory, &record_key).unwrap();
        drop(second);
        assert!(lock_path.is_file());

        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&lock_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn stale_and_equal_time_activity_cannot_regress_terminal_state() {
        let temp = TempDir::new().unwrap();
        let record_key = sha256_hex(b"record");
        let session_key = sha256_hex(b"session");
        let terminal_time = MAX_FUTURE_SKEW_MS + 20;
        let persist = |state: &str, changed_at_ms: i64, event: AntigravityHookEvent| {
            let record = HookRecord {
                schema_version: SCHEMA_VERSION,
                session_key: session_key.clone(),
                cwd: "/workspace".to_string(),
                state: state.to_string(),
                changed_at_ms,
                model_kind: None,
                ide_model_revision: None,
            };
            persist_record(
                temp.path(),
                "antigravity",
                &record_key,
                &record,
                event,
                HookModelUpdate::Preserve,
            )
            .unwrap();
        };

        persist("turn_finished", terminal_time, AntigravityHookEvent::Stop);
        // This delayed writer is older by more than the record future-skew
        // allowance. Validation uses wall time, not the stale event's time,
        // so the valid newer record still participates in monotonic merging.
        persist("activity_detected", 10, AntigravityHookEvent::PreInvocation);
        persist(
            "activity_detected",
            terminal_time,
            AntigravityHookEvent::PreInvocation,
        );
        let path = temp
            .path()
            .join("antigravity")
            .join(format!("{record_key}.json"));
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["state"], "turn_finished");
        assert_eq!(saved["changedAtMs"], terminal_time);

        // Equal-time terminal outcomes also converge deterministically: a
        // failure may replace a clean finish, but the reverse is rejected.
        persist("failed", terminal_time, AntigravityHookEvent::Stop);
        persist("turn_finished", terminal_time, AntigravityHookEvent::Stop);
        let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["state"], "failed");
        assert_eq!(saved["changedAtMs"], terminal_time);
    }

    #[test]
    fn hook_health_lock_and_event_order_prevent_receipt_regression() {
        use std::sync::mpsc;

        let temp = TempDir::new().unwrap();
        let surface = AntigravitySurface::Two;
        let terminal_time = MAX_FUTURE_SKEW_MS + 20;
        let newer = AntigravityHookObservation {
            schema_version: SCHEMA_VERSION,
            event: "stop".to_string(),
            surface: surface.observation_name().to_string(),
            outcome: AntigravityHookOutcome::Recorded,
            observed_at_ms: terminal_time,
            workspace_count: 1,
        };
        persist_hook_observation(temp.path(), surface, &newer).unwrap();

        let directory = temp.path().join(HOOK_HEALTH_DIRECTORY);
        let lock_key = sha256_hex(surface.observation_file().as_bytes());
        let held = acquire_record_lock(&directory, &lock_key).unwrap();
        let root = temp.path().to_path_buf();
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let older = AntigravityHookObservation {
                schema_version: SCHEMA_VERSION,
                event: "pre-invocation".to_string(),
                surface: surface.observation_name().to_string(),
                outcome: AntigravityHookOutcome::Recorded,
                observed_at_ms: 10,
                workspace_count: 1,
            };
            finished_tx
                .send(persist_hook_observation(&root, surface, &older))
                .unwrap();
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

        let equal_time_pre_invocation = AntigravityHookObservation {
            schema_version: SCHEMA_VERSION,
            event: "pre-invocation".to_string(),
            surface: surface.observation_name().to_string(),
            outcome: AntigravityHookOutcome::Recorded,
            observed_at_ms: terminal_time,
            workspace_count: 1,
        };
        persist_hook_observation(temp.path(), surface, &equal_time_pre_invocation).unwrap();

        let saved = antigravity_two_hook_observation(temp.path(), terminal_time)
            .unwrap()
            .unwrap();
        assert_eq!(saved.event, "stop");
        assert_eq!(saved.observed_at_ms, terminal_time);
        assert!(directory.join(format!(".{lock_key}.lock")).is_file());
    }

    #[test]
    fn model_names_reduce_to_stable_closed_tokens() {
        let cases = [
            (
                "gemini-3.6-flash-medium",
                AntigravityModelKind::Gemini36FlashMedium,
                "gemini_3_6_flash_medium",
            ),
            (
                "Gemini 3.1 Pro (High)",
                AntigravityModelKind::Gemini31ProHigh,
                "gemini_3_1_pro_high",
            ),
            (
                "gemini-3.6-flash-high",
                AntigravityModelKind::Gemini36FlashHigh,
                "gemini_3_6_flash_high",
            ),
            (
                "gemini-3.5-flash",
                AntigravityModelKind::Gemini35Flash,
                "gemini_3_5_flash",
            ),
            (
                "gemini-3.1-pro-low",
                AntigravityModelKind::Gemini31ProLow,
                "gemini_3_1_pro_low",
            ),
            (
                "gemini-3-flash",
                AntigravityModelKind::Gemini3Flash,
                "gemini_3_flash",
            ),
            (
                "gemini-3-flash-agent",
                AntigravityModelKind::Gemini35Flash,
                "gemini_3_5_flash",
            ),
            (
                "claude-sonnet-4.6",
                AntigravityModelKind::ClaudeSonnet46Thinking,
                "claude_sonnet_4_6_thinking",
            ),
            (
                "claude-sonnet-4.6-thinking",
                AntigravityModelKind::ClaudeSonnet46Thinking,
                "claude_sonnet_4_6_thinking",
            ),
            (
                "Claude Opus 4.6 (Thinking)",
                AntigravityModelKind::ClaudeOpus46Thinking,
                "claude_opus_4_6_thinking",
            ),
            (
                "gpt-oss-120b",
                AntigravityModelKind::GptOss120b,
                "gpt_oss_120b",
            ),
            (
                "gpt-oss-120b-medium",
                AntigravityModelKind::GptOss120bMedium,
                "gpt_oss_120b_medium",
            ),
            ("auto", AntigravityModelKind::Automatic, "automatic"),
            (
                "gemini-future-preview",
                AntigravityModelKind::Gemini,
                "gemini",
            ),
            (
                "claude-future-preview",
                AntigravityModelKind::Claude,
                "claude",
            ),
            (
                "gpt-oss-future-preview",
                AntigravityModelKind::GptOss,
                "gpt_oss",
            ),
        ];

        for (raw, expected_kind, expected_token) in cases {
            let kind = classify_antigravity_model_name(raw).unwrap();
            assert_eq!(kind, expected_kind);
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(expected_token));
        }

        assert_eq!(classify_antigravity_model_name("private-model"), None);
        assert_eq!(classify_antigravity_model_name("gemini/private"), None);
        assert_eq!(classify_antigravity_model_name("ジェミニ"), None);
        assert_eq!(
            classify_antigravity_model_name(&"x".repeat(MAX_MODEL_NAME_BYTES + 1)),
            None
        );
    }

    #[test]
    fn pre_invocation_persists_one_privacy_safe_record_per_workspace() {
        let temp = TempDir::new().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let input = json!({
            "conversationId": "private-conversation-id",
            "workspacePaths": [&first, &second],
            "transcriptPath": "/home/test/.gemini/antigravity/brain/private-conversation-id/.system_generated/logs/transcript.jsonl",
            "artifactDirectoryPath": "/secret/artifacts",
            "modelName": "gemini-3.6-flash-medium",
            "toolCall": {"args": {"CommandLine": "SECRET COMMAND"}},
        })
        .to_string();

        let (code, output) = hook_with_root(
            AntigravityHookEvent::PreInvocation,
            &input,
            temp.path(),
            1_700_000_000_123,
        );

        assert_eq!(code, 0);
        assert_eq!(output, "{}\n");
        let records = state_records(temp.path());
        assert_eq!(records.len(), 2);
        for record in records {
            assert_eq!(record.as_object().unwrap().len(), 6);
            assert_eq!(record["schemaVersion"], 1);
            assert_eq!(record["state"], "activity_detected");
            assert_eq!(record["changedAtMs"], 1_700_000_000_123i64);
            assert_eq!(record["modelKind"], "gemini_3_6_flash_medium");
            assert_eq!(record["sessionKey"].as_str().unwrap().len(), 64);
            let saved = record.to_string();
            assert!(!saved.contains("private-conversation-id"));
            assert!(!saved.contains("SECRET"));
            assert!(!saved.contains("transcript"));
            assert!(!saved.contains("artifact"));
            assert!(!saved.contains("gemini-3.6-flash-medium"));
        }
        let observation = antigravity_two_hook_observation(temp.path(), 1_700_000_000_123)
            .unwrap()
            .unwrap();
        assert_eq!(observation.event, "pre-invocation");
        assert_eq!(observation.surface, "antigravity_2");
        assert_eq!(observation.outcome, AntigravityHookOutcome::Recorded);
        assert_eq!(observation.workspace_count, 2);
        let saved_health = fs::read_to_string(
            temp.path()
                .join(HOOK_HEALTH_DIRECTORY)
                .join("antigravity-2.json"),
        )
        .unwrap();
        assert!(!saved_health.contains("private-conversation-id"));
        assert!(!saved_health.contains("SECRET"));
        assert!(!saved_health.contains("modelKind"));
        assert!(!saved_health.contains("gemini"));
        assert!(!saved_health.contains(first.to_string_lossy().as_ref()));
        assert!(!saved_health.contains(second.to_string_lossy().as_ref()));
    }

    #[test]
    fn unknown_model_identifier_is_never_persisted() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let private_model = "private-deployment-model";
        let input = json!({
            "conversationId": "conversation",
            "workspacePaths": [workspace],
            "transcriptPath": "/home/test/.gemini/antigravity/brain/conversation/transcript.jsonl",
            "modelName": private_model,
        })
        .to_string();

        let (code, _) = hook_with_root(
            AntigravityHookEvent::PreInvocation,
            &input,
            temp.path(),
            1_700_000_000_123,
        );

        assert_eq!(code, 0);
        let records = state_records(temp.path());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].as_object().unwrap().len(), 5);
        let saved = records[0].to_string();
        assert!(!saved.contains("modelKind"));
        assert!(!saved.contains(private_model));
    }

    #[test]
    fn documented_transcript_roots_select_only_supported_surfaces() {
        let cases = [
            ("antigravity", Some("antigravity")),
            ("antigravity-ide", Some("antigravity-ide")),
            ("antigravity-cli", None),
            ("future-surface", None),
        ];

        for (surface, expected_directory) in cases {
            let temp = TempDir::new().unwrap();
            let workspace = temp.path().join("workspace");
            fs::create_dir(&workspace).unwrap();
            let payload = json!({
                "conversationId": format!("conversation-{surface}"),
                "workspacePaths": [workspace],
                "transcriptPath": format!(
                    "/home/test/.gemini/{surface}/brain/conversation/.system_generated/logs/transcript.jsonl"
                )
            });

            let (code, output) = hook_with_root(
                AntigravityHookEvent::PreInvocation,
                &payload.to_string(),
                temp.path(),
                10,
            );
            assert_eq!(code, 0);
            assert_eq!(output, "{}\n");
            match expected_directory {
                Some(directory) => assert_eq!(state_record_paths(temp.path(), directory).len(), 1),
                None => assert!(
                    !temp.path().join("antigravity").exists()
                        && !temp.path().join("antigravity-ide").exists()
                ),
            }
        }
    }

    #[test]
    fn documented_artifact_path_is_a_surface_fallback() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let payload = json!({
            "conversationId": "conversation",
            "workspacePaths": [workspace],
            "artifactDirectoryPath": "/home/test/.gemini/antigravity/brain/conversation/artifacts"
        });

        let (code, output) = hook_with_root(
            AntigravityHookEvent::PreInvocation,
            &payload.to_string(),
            temp.path(),
            10,
        );

        assert_eq!(code, 0);
        assert_eq!(output, "{}\n");
        assert_eq!(state_records(temp.path()).len(), 1);
        assert_eq!(
            antigravity_two_hook_observation(temp.path(), 10)
                .unwrap()
                .unwrap()
                .outcome,
            AntigravityHookOutcome::Recorded
        );
    }

    #[test]
    fn ide_hook_execution_has_a_separate_readable_health_receipt() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let payload = json!({
            "conversationId": "ide-conversation",
            "workspacePaths": [workspace],
            "transcriptPath": "/home/test/.gemini/antigravity-ide/brain/ide-conversation/.system_generated/logs/transcript.jsonl",
            "artifactDirectoryPath": "/home/test/.gemini/antigravity-ide/brain/ide-conversation"
        });

        let (code, output) = hook_with_root(
            AntigravityHookEvent::PreInvocation,
            &payload.to_string(),
            temp.path(),
            10,
        );

        assert_eq!(code, 0);
        assert_eq!(output, "{}\n");
        assert!(antigravity_two_hook_observation(temp.path(), 10)
            .unwrap()
            .is_none());
        let observation = antigravity_ide_hook_observation(temp.path(), 10)
            .unwrap()
            .unwrap();
        assert_eq!(observation.surface, "antigravity_ide");
        assert_eq!(observation.outcome, AntigravityHookOutcome::Recorded);
        assert_eq!(observation.workspace_count, 1);
        let record = state_record(temp.path(), "antigravity-ide");
        assert!(record.get("modelKind").is_none());
    }

    #[test]
    fn ide_hook_tracks_the_latest_model_and_late_tool_events_cannot_reopen_a_turn() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let ide_state_database = temp.path().join("globalStorage/state.vscdb");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let conversation_id = "9710f92a-3aac-40d8-9b34-1a19de34735b";
        write_ide_state_database(&ide_state_database, 1072);
        let payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/.system_generated/logs/transcript.jsonl"
            )
        });

        let (code, output) = hook_with_root_and_ide_state(
            AntigravityHookEvent::PreInvocation,
            &payload.to_string(),
            &state_root,
            &ide_state_database,
            10,
        );

        assert_eq!(code, 0);
        assert_eq!(output, "{}\n");
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["modelKind"], "gemini_3_6_flash_medium");
        let saved = record.to_string();
        assert!(!saved.contains(conversation_id));
        assert!(!saved.contains("gemini-3-6"));

        // invocationNum does not define a reliable user-turn boundary. Every
        // PreInvocation must pick up a later model selection.
        write_ide_state_database(&ide_state_database, 1035);
        let later_invocation = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 7,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/.system_generated/logs/transcript.jsonl"
            )
        });
        hook_with_root_and_ide_state(
            AntigravityHookEvent::PreInvocation,
            &later_invocation.to_string(),
            &state_root,
            &ide_state_database,
            15,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["modelKind"], "claude_sonnet_4_6_thinking");
        assert_eq!(record["changedAtMs"], 15);

        // Successful tool completion is state-neutral. It must not manufacture
        // a later activity marker or replace the latest trusted model.
        let (code, output) = hook_with_root(
            AntigravityHookEvent::PostToolUse,
            &later_invocation.to_string(),
            &state_root,
            20,
        );
        assert_eq!(code, 0);
        assert_eq!(output, "{}\n");
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["modelKind"], "claude_sonnet_4_6_thinking");
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["changedAtMs"], 15);

        // The IDE preference write is asynchronous. A Stop refresh corrects a
        // single-invocation turn whose PreInvocation observed the older value.
        write_ide_state_database(&ide_state_database, 342);
        let stop = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "executionNum": 1,
            "fullyIdle": true,
            "terminationReason": "model_stop",
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/.system_generated/logs/transcript.jsonl"
            )
        });
        hook_with_root_and_ide_state(
            AntigravityHookEvent::Stop,
            &stop.to_string(),
            &state_root,
            &ide_state_database,
            30,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["modelKind"], "gpt_oss_120b_medium");
        assert_eq!(record["state"], "turn_finished");
        assert_eq!(record["changedAtMs"], 30);

        // Antigravity regularly emits PostToolUse after Stop. The terminal
        // status and its timestamp must remain intact.
        hook_with_root(
            AntigravityHookEvent::PostToolUse,
            &later_invocation.to_string(),
            &state_root,
            40,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["modelKind"], "gpt_oss_120b_medium");
        assert_eq!(record["state"], "turn_finished");
        assert_eq!(record["changedAtMs"], 30);

        // A transient lookup failure on the next model call preserves the
        // latest trusted model but legitimately reopens lifecycle activity.
        hook_with_root(
            AntigravityHookEvent::PreInvocation,
            &later_invocation.to_string(),
            &state_root,
            50,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["modelKind"], "gpt_oss_120b_medium");
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["changedAtMs"], 50);

        // A delayed event with an older timestamp cannot replace newer state.
        hook_with_root_and_ide_state(
            AntigravityHookEvent::Stop,
            &stop.to_string(),
            &state_root,
            &ide_state_database,
            45,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["changedAtMs"], 50);

        let unknown_payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            ),
            "modelName": "private-deployment-model"
        });
        hook_with_root_and_ide_state(
            AntigravityHookEvent::PreInvocation,
            &unknown_payload.to_string(),
            &state_root,
            &ide_state_database,
            60,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert!(record.get("modelKind").is_none());
        assert!(!record.to_string().contains("private-deployment-model"));

        // A successfully decoded but unknown IDE enum also clears a stale
        // qualifier rather than misreporting the previously selected model.
        write_ide_state_database(&ide_state_database, 342);
        hook_with_root_and_ide_state(
            AntigravityHookEvent::PreInvocation,
            &later_invocation.to_string(),
            &state_root,
            &ide_state_database,
            65,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["modelKind"], "gpt_oss_120b_medium");
        write_ide_state_database(&ide_state_database, 999_999);
        let compatibility_payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            )
        });
        hook_with_root_and_ide_state(
            AntigravityHookEvent::PreInvocation,
            &compatibility_payload.to_string(),
            &state_root,
            &ide_state_database,
            70,
        );
        let record = state_record(&state_root, "antigravity-ide");
        assert!(record.get("modelKind").is_none());
    }

    #[test]
    fn ide_hook_prefers_the_latest_per_conversation_execution_model() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let ide_state_database = temp.path().join("globalStorage/state.vscdb");
        let conversations = temp.path().join("conversations");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let conversation_id = "403467f7-041a-420e-84cb-87da2ba51959";
        let conversation_database = conversations.join(format!("{conversation_id}.db"));
        let payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            )
        });

        // The turn starts from the current selection before any executor row
        // exists.
        write_ide_state_database(&ide_state_database, 1072);
        let mut output = Vec::new();
        assert_eq!(
            run_antigravity_hook_with(
                AntigravityHookEvent::PreInvocation,
                payload.to_string().as_bytes(),
                &mut output,
                Ok(&state_root),
                Some(&ide_state_database),
                Some(&conversations),
                99,
            ),
            0
        );
        let initial = state_record(&state_root, "antigravity-ide");
        assert_eq!(initial["modelKind"], "gemini_3_6_flash_medium");

        // Once each new execution row appears, it is authoritative even while
        // the global preference remains pinned to the initial Gemini value.
        for (index, model_name, expected) in [
            (0, "gemini-3-flash-agent", "gemini_3_5_flash"),
            (1, "claude-sonnet-4-6", "claude_sonnet_4_6_thinking"),
            (2, "gpt-oss-120b-medium", "gpt_oss_120b_medium"),
        ] {
            write_ide_conversation_model(&conversation_database, index, model_name);
            let mut output = Vec::new();
            assert_eq!(
                run_antigravity_hook_with(
                    AntigravityHookEvent::PreInvocation,
                    payload.to_string().as_bytes(),
                    &mut output,
                    Ok(&state_root),
                    Some(&ide_state_database),
                    Some(&conversations),
                    100 + index,
                ),
                0
            );
            assert_eq!(output, b"{}\n");
            let record = state_record(&state_root, "antigravity-ide");
            assert_eq!(record["modelKind"], expected);
        }

        let wanted = BTreeSet::from([sha256_hex(conversation_id.as_bytes())]);
        assert_eq!(
            antigravity_ide_execution_models(&conversations, &wanted)
                .get(&sha256_hex(conversation_id.as_bytes()))
                .map(|model| model.preference),
            Some(IdeSelectedModelPreference::Recognized(
                AntigravityModelKind::GptOss120bMedium
            ))
        );
    }

    #[test]
    fn ide_execution_model_reader_never_falls_back_to_an_older_row() {
        let temp = TempDir::new().unwrap();
        let database = temp
            .path()
            .join("conversations/403467f7-041a-420e-84cb-87da2ba51959.db");
        write_ide_conversation_model(&database, 0, "gemini-3.6-flash-high");
        write_ide_conversation_model(&database, 1, "private-deployment-model");
        assert_eq!(
            read_antigravity_ide_conversation_model_path(&database)
                .unwrap()
                .map(|model| model.preference),
            Some(IdeSelectedModelPreference::Unrecognized)
        );

        let connection = Connection::open(&database).unwrap();
        connection
            .execute(
                "INSERT INTO executor_metadata (idx, data) VALUES (?1, ?2)",
                rusqlite::params![2, vec![0_u8; MAX_IDE_EXECUTOR_METADATA_BYTES + 1]],
            )
            .unwrap();
        assert_eq!(
            read_antigravity_ide_conversation_model_path(&database).unwrap(),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn ide_database_reader_resolves_parent_alias_but_rejects_a_database_symlink() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let real_root = temp.path().join("real");
        let conversation_id = "403467f7-041a-420e-84cb-87da2ba51959";
        let database = real_root
            .join("conversations")
            .join(format!("{conversation_id}.db"));
        write_ide_conversation_model(&database, 0, "claude-sonnet-4-6");

        let alias = temp.path().join("alias");
        symlink(&real_root, &alias).unwrap();
        let aliased_database = alias
            .join("conversations")
            .join(format!("{conversation_id}.db"));
        assert_eq!(
            read_antigravity_ide_conversation_model_path(&aliased_database)
                .unwrap()
                .map(|model| model.preference),
            Some(IdeSelectedModelPreference::Recognized(
                AntigravityModelKind::ClaudeSonnet46Thinking
            ))
        );

        let linked_database = real_root.join("conversations/linked.db");
        symlink(&database, &linked_database).unwrap();
        assert_eq!(
            read_antigravity_ide_conversation_model_path(&linked_database).unwrap(),
            None
        );
    }

    #[test]
    fn ide_first_observed_turn_prefers_the_current_selection_over_an_older_execution() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let ide_state_database = temp.path().join("globalStorage/state.vscdb");
        let conversations = temp.path().join("conversations");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let conversation_id = "403467f7-041a-420e-84cb-87da2ba51959";
        let conversation_database = conversations.join(format!("{conversation_id}.db"));
        write_ide_state_database(&ide_state_database, 1035);
        write_ide_conversation_model(&conversation_database, 0, "gemini-3.6-flash-medium");
        let payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            )
        });

        let mut output = Vec::new();
        run_antigravity_hook_with(
            AntigravityHookEvent::PreInvocation,
            payload.to_string().as_bytes(),
            &mut output,
            Ok(&state_root),
            Some(&ide_state_database),
            Some(&conversations),
            10,
        );

        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["modelKind"], "claude_sonnet_4_6_thinking");
        assert!(record["ideModelRevision"].as_str().is_some());
    }

    #[test]
    fn ide_new_turn_uses_selection_immediately_then_confirms_each_execution_revision() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let ide_state_database = temp.path().join("globalStorage/state.vscdb");
        let conversations = temp.path().join("conversations");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let conversation_id = "403467f7-041a-420e-84cb-87da2ba51959";
        let conversation_database = conversations.join(format!("{conversation_id}.db"));
        let payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            )
        });
        let stop = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "fullyIdle": true,
            "terminationReason": "model_stop",
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            )
        });
        let read_record = || state_record(&state_root, "antigravity-ide");
        let run = |event, body: &Value, now| {
            let mut output = Vec::new();
            run_antigravity_hook_with(
                event,
                body.to_string().as_bytes(),
                &mut output,
                Ok(&state_root),
                Some(&ide_state_database),
                Some(&conversations),
                now,
            )
        };

        write_ide_state_database(&ide_state_database, 1072);
        write_ide_conversation_model(&conversation_database, 0, "gemini-3.6-flash-medium");
        run(AntigravityHookEvent::PreInvocation, &payload, 10);
        run(AntigravityHookEvent::Stop, &stop, 20);
        assert_eq!(read_record()["modelKind"], "gemini_3_6_flash_medium");

        // The prior Gemini executor row still exists, but the new turn must use
        // Claude's freshly selected model as soon as Activity detected begins.
        write_ide_state_database(&ide_state_database, 1035);
        run(AntigravityHookEvent::PreInvocation, &payload, 30);
        assert_eq!(read_record()["state"], "activity_detected");
        assert_eq!(read_record()["modelKind"], "claude_sonnet_4_6_thinking");
        let gemini_revision = read_antigravity_ide_conversation_model_path(&conversation_database)
            .unwrap()
            .unwrap()
            .revision;
        assert_eq!(read_record()["ideModelRevision"], gemini_revision);

        // The desktop refresh will reconcile this row while the turn is
        // active; a subsequent lifecycle hook observes the same revision too.
        write_ide_conversation_model(&conversation_database, 1, "claude-sonnet-4-6");
        run(AntigravityHookEvent::Stop, &stop, 40);
        assert_eq!(read_record()["state"], "turn_finished");
        assert_eq!(read_record()["modelKind"], "claude_sonnet_4_6_thinking");

        // A third turn starts before any desktop snapshot. It must display the
        // GPT-OSS selection immediately while anchoring Claude's finished row
        // so a refresh cannot revert the active label.
        write_ide_state_database(&ide_state_database, 342);
        run(AntigravityHookEvent::PreInvocation, &payload, 50);
        assert_eq!(read_record()["state"], "activity_detected");
        assert_eq!(read_record()["modelKind"], "gpt_oss_120b_medium");
        let claude_revision = read_antigravity_ide_conversation_model_path(&conversation_database)
            .unwrap()
            .unwrap()
            .revision;
        assert_eq!(read_record()["ideModelRevision"], claude_revision);

        write_ide_conversation_model(&conversation_database, 2, "gpt-oss-120b-medium");
        run(AntigravityHookEvent::Stop, &stop, 60);
        assert_eq!(read_record()["state"], "turn_finished");
        assert_eq!(read_record()["modelKind"], "gpt_oss_120b_medium");
    }

    #[test]
    fn ide_activity_uses_the_current_user_input_model_before_execution_finishes() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let ide_state_database = temp.path().join("globalStorage/state.vscdb");
        let conversations = temp.path().join("conversations");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let conversation_id = "403467f7-041a-420e-84cb-87da2ba51959";
        let conversation_database = conversations.join(format!("{conversation_id}.db"));
        let payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            )
        });
        let stop = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "fullyIdle": true,
            "terminationReason": "model_stop",
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            )
        });
        let run = |event, body: &Value, now| {
            let mut output = Vec::new();
            assert_eq!(
                run_antigravity_hook_with(
                    event,
                    body.to_string().as_bytes(),
                    &mut output,
                    Ok(&state_root),
                    Some(&ide_state_database),
                    Some(&conversations),
                    now,
                ),
                0
            );
            assert_eq!(
                output,
                event.fail_open_output(),
                "hook output must remain fail-open"
            );
        };
        let read_record = || state_record(&state_root, "antigravity-ide");

        // Keep both fallback signals pinned to the first model. Only the
        // newest user-input step reflects switches made for later turns.
        write_ide_state_database(&ide_state_database, 1072);
        write_ide_conversation_model(&conversation_database, 0, "gemini-3.6-flash-medium");
        write_ide_conversation_turn_model(&conversation_database, 10, 1072);
        run(AntigravityHookEvent::PreInvocation, &payload, 10);
        assert_eq!(read_record()["state"], "activity_detected");
        assert_eq!(read_record()["modelKind"], "gemini_3_6_flash_medium");
        run(AntigravityHookEvent::Stop, &stop, 20);
        assert_eq!(read_record()["state"], "turn_finished");

        write_ide_conversation_turn_model(&conversation_database, 20, 1035);
        run(AntigravityHookEvent::PreInvocation, &payload, 30);
        let claude_activity = read_record();
        assert_eq!(claude_activity["state"], "activity_detected");
        assert_eq!(claude_activity["modelKind"], "claude_sonnet_4_6_thinking");
        let current_model = read_antigravity_ide_conversation_model_path(&conversation_database)
            .unwrap()
            .unwrap();
        assert_eq!(
            current_model.preference,
            IdeSelectedModelPreference::Recognized(AntigravityModelKind::ClaudeSonnet46Thinking)
        );
        assert_eq!(claude_activity["ideModelRevision"], current_model.revision);
        run(AntigravityHookEvent::Stop, &stop, 40);
        assert_eq!(read_record()["state"], "turn_finished");
        assert_eq!(read_record()["modelKind"], "claude_sonnet_4_6_thinking");

        write_ide_conversation_turn_model(&conversation_database, 30, 342);
        run(AntigravityHookEvent::PreInvocation, &payload, 50);
        assert_eq!(read_record()["state"], "activity_detected");
        assert_eq!(read_record()["modelKind"], "gpt_oss_120b_medium");

        // A queued future input must not make this active turn fall back to
        // either of the deliberately stale Gemini signals.
        let connection = Connection::open(&conversation_database).unwrap();
        connection
            .execute(
                "INSERT INTO steps (idx, step_type, step_payload) VALUES (?1, 14, ?2)",
                rusqlite::params![
                    40,
                    test_ide_user_input_step(1035, true, b"queued private input")
                ],
            )
            .unwrap();
        drop(connection);
        run(AntigravityHookEvent::PreInvocation, &payload, 60);
        assert_eq!(read_record()["state"], "activity_detected");
        assert_eq!(read_record()["modelKind"], "gpt_oss_120b_medium");
        assert_eq!(
            read_antigravity_ide_conversation_model_path(&conversation_database).unwrap(),
            None
        );
    }

    #[test]
    fn ide_user_input_model_scanner_seeks_over_private_bodies() {
        let private_padding = vec![b'x'; 256 * 1024];
        let encoded = test_ide_user_input_step(1035, false, &private_padding);
        let mut reader = CountingCursor {
            inner: io::Cursor::new(encoded.clone()),
            bytes_read: 0,
        };

        assert_eq!(
            decode_ide_user_input_model(&mut reader, encoded.len() as u64),
            Some((
                IdeSelectedModelPreference::Recognized(
                    AntigravityModelKind::ClaudeSonnet46Thinking
                ),
                1035
            ))
        );
        assert!(
            reader.bytes_read < 128,
            "streaming model decode unexpectedly read {} bytes",
            reader.bytes_read
        );

        let queued = test_ide_user_input_step(342, true, b"queued private input");
        assert_eq!(
            decode_ide_user_input_model(&mut io::Cursor::new(&queued), queued.len() as u64),
            None
        );

        let mut structurally_expensive = Vec::new();
        for _ in 0..MAX_IDE_PROTOBUF_STRUCTURE_BYTES {
            structurally_expensive.extend(test_varint_field(1, 0));
        }
        structurally_expensive.extend(test_ide_user_input_step(342, false, b"private"));
        let mut reader = CountingCursor {
            inner: io::Cursor::new(structurally_expensive.clone()),
            bytes_read: 0,
        };
        assert_eq!(
            decode_ide_user_input_model(&mut reader, structurally_expensive.len() as u64),
            None
        );
        assert!(reader.bytes_read <= MAX_IDE_PROTOBUF_STRUCTURE_BYTES);

        let mut duplicate = test_ide_user_input_step(1035, false, b"first");
        duplicate.extend(test_ide_user_input_step(342, false, b"second"));
        assert_eq!(
            decode_ide_user_input_model(&mut io::Cursor::new(&duplicate), duplicate.len() as u64),
            None
        );
    }

    #[test]
    fn ide_turn_model_reader_never_falls_back_to_an_older_user_input() {
        let temp = TempDir::new().unwrap();
        let database = temp
            .path()
            .join("conversations/403467f7-041a-420e-84cb-87da2ba51959.db");
        write_ide_conversation_turn_model(&database, 1, 1072);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute(
                "INSERT INTO steps (idx, step_type, step_payload) VALUES (?1, 14, ?2)",
                rusqlite::params![
                    2,
                    test_ide_user_input_step(1035, true, b"queued private input")
                ],
            )
            .unwrap();
        drop(connection);

        assert_eq!(
            read_antigravity_ide_current_turn_model_path(&database).unwrap(),
            IdeCurrentTurnModel::Unusable
        );
    }

    #[test]
    fn ide_payload_model_takes_precedence_over_the_selected_model_preference() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let ide_state_database = temp.path().join("globalStorage/state.vscdb");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let conversation_id = "9710f92a-3aac-40d8-9b34-1a19de34735b";
        write_ide_state_database(&ide_state_database, 1035);
        let payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            ),
            "modelName": "gpt-oss-120b-medium"
        });

        hook_with_root_and_ide_state(
            AntigravityHookEvent::PreInvocation,
            &payload.to_string(),
            &state_root,
            &ide_state_database,
            10,
        );

        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["modelKind"], "gpt_oss_120b_medium");
    }

    #[test]
    fn empty_ide_payload_model_uses_the_current_turn_signal() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let conversations = temp.path().join("conversations");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let conversation_id = "9710f92a-3aac-40d8-9b34-1a19de34735b";
        write_ide_conversation_turn_model(
            &conversations.join(format!("{conversation_id}.db")),
            1,
            1035,
        );
        let payload = json!({
            "conversationId": conversation_id,
            "workspacePaths": [workspace],
            "invocationNum": 0,
            "transcriptPath": format!(
                "/home/test/.gemini/antigravity-ide/brain/{conversation_id}/transcript.jsonl"
            ),
            "modelName": null
        });

        let mut output = Vec::new();
        run_antigravity_hook_with(
            AntigravityHookEvent::PreInvocation,
            payload.to_string().as_bytes(),
            &mut output,
            Ok(&state_root),
            None,
            Some(&conversations),
            10,
        );

        let record = state_record(&state_root, "antigravity-ide");
        assert_eq!(record["state"], "activity_detected");
        assert_eq!(record["modelKind"], "claude_sonnet_4_6_thinking");
    }

    #[test]
    fn ide_selected_model_preference_accepts_only_known_closed_enum_values() {
        // Real Antigravity IDE 2.1.1 preference shape for Claude Sonnet 4.6.
        assert_eq!(
            model_kind_from_ide_preferences(
                b"CjAKJmxhc3Rfc2VsZWN0ZWRfYWdlbnRfbW9kZWxfc2VudGluZWxfa2V5EgYKBEVJc0k="
            ),
            Some(AntigravityModelKind::ClaudeSonnet46Thinking)
        );
        let cases = [
            (1071, AntigravityModelKind::Gemini36FlashHigh),
            (1072, AntigravityModelKind::Gemini36FlashMedium),
            (1073, AntigravityModelKind::Gemini),
            (1084, AntigravityModelKind::Gemini35Flash),
            (1020, AntigravityModelKind::Gemini35Flash),
            (1187, AntigravityModelKind::Gemini35Flash),
            (1016, AntigravityModelKind::Gemini31ProHigh),
            (1036, AntigravityModelKind::Gemini31ProLow),
            (1035, AntigravityModelKind::ClaudeSonnet46Thinking),
            (1026, AntigravityModelKind::ClaudeOpus46Thinking),
            (342, AntigravityModelKind::GptOss120bMedium),
        ];
        for (model_enum, expected) in cases {
            assert_eq!(
                model_kind_from_ide_preferences(test_ide_model_preferences(model_enum).as_bytes()),
                Some(expected)
            );
        }
        assert_eq!(
            model_kind_from_ide_preferences(test_ide_model_preferences(999_999).as_bytes()),
            None
        );
        assert_eq!(
            decode_ide_model_preferences(test_ide_model_preferences(999_999).as_bytes()),
            Some(IdeSelectedModelPreference::Unrecognized)
        );
        let mut multiple_entries = test_length_delimited_field(
            1,
            &test_ide_model_preference_entry(b"unrelated_preference", 342),
        );
        multiple_entries.extend(test_length_delimited_field(
            1,
            &test_ide_model_preference_entry(b"last_selected_agent_model_sentinel_key", 1026),
        ));
        assert_eq!(
            model_kind_from_ide_preferences(test_base64_encode(&multiple_entries).as_bytes()),
            Some(AntigravityModelKind::ClaudeOpus46Thinking)
        );
        assert_eq!(model_kind_from_ide_preferences(b"not base64"), None);
        assert_eq!(decode_ide_model_preferences(b"not base64"), None);
    }

    #[test]
    fn conflicting_product_paths_are_not_mislabeled() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let payload = json!({
            "conversationId": "conversation",
            "workspacePaths": [workspace],
            "transcriptPath": "/home/test/.gemini/antigravity/brain/conversation/transcript.jsonl",
            "artifactDirectoryPath": "/home/test/.gemini/antigravity-cli/conversation/artifacts"
        });

        hook_with_root(
            AntigravityHookEvent::PreInvocation,
            &payload.to_string(),
            temp.path(),
            10,
        );

        assert!(!temp.path().join("antigravity").exists());
        assert!(antigravity_two_hook_observation(temp.path(), 10)
            .unwrap()
            .is_none());
        let unknown: AntigravityHookObservation = serde_json::from_slice(
            &fs::read(temp.path().join(HOOK_HEALTH_DIRECTORY).join("unknown.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(unknown.outcome, AntigravityHookOutcome::UnsupportedSurface);
    }

    #[test]
    fn lifecycle_fields_map_to_coarse_states_without_being_persisted() {
        let cases = [
            (
                AntigravityHookEvent::Stop,
                json!({"fullyIdle":false,"terminationReason":"model_stop"}),
                "activity_detected",
                "{\"decision\":\"allow\"}\n",
            ),
            (
                AntigravityHookEvent::Stop,
                json!({"fullyIdle":true,"terminationReason":"user_cancelled"}),
                "interrupted",
                "{\"decision\":\"allow\"}\n",
            ),
            (
                AntigravityHookEvent::Stop,
                json!({"fullyIdle":true,"terminationReason":"error"}),
                "failed",
                "{\"decision\":\"allow\"}\n",
            ),
            (
                AntigravityHookEvent::Stop,
                json!({"fullyIdle":true,"terminationReason":"model_stop"}),
                "turn_finished",
                "{\"decision\":\"allow\"}\n",
            ),
        ];

        for (index, (event, event_fields, expected_state, expected_output)) in
            cases.into_iter().enumerate()
        {
            let temp = TempDir::new().unwrap();
            let workspace = temp.path().join("workspace");
            fs::create_dir(&workspace).unwrap();
            let mut payload = event_fields.as_object().unwrap().clone();
            payload.insert("conversationId".into(), json!(format!("c-{index}")));
            payload.insert("workspacePaths".into(), json!([workspace]));
            payload.insert(
                "transcriptPath".into(),
                json!(format!(
                    "/home/test/.gemini/antigravity/brain/c-{index}/.system_generated/logs/transcript.jsonl"
                )),
            );
            let (_, output) = hook_with_root(
                event,
                &Value::Object(payload).to_string(),
                temp.path(),
                index as i64,
            );
            assert_eq!(output, expected_output);
            let records = state_records(temp.path());
            assert_eq!(records[0]["state"], expected_state);
            let saved = records[0].to_string();
            assert!(!saved.contains("tool failed"));
            assert!(!saved.contains("model_stop"));
            assert!(!saved.contains("user_cancelled"));
        }
    }

    #[test]
    fn post_tool_use_never_writes_or_reopens_activity_state() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let payload = json!({
            "conversationId": "conversation",
            "workspacePaths": [workspace],
            "transcriptPath": "/home/test/.gemini/antigravity-ide/brain/conversation/transcript.jsonl"
        });

        hook_with_root(
            AntigravityHookEvent::PostToolUse,
            &payload.to_string(),
            temp.path(),
            10,
        );
        assert!(!temp.path().join("antigravity-ide").exists());
        let mut failed_tool = payload.clone();
        failed_tool["error"] = json!("tool failed");
        hook_with_root(
            AntigravityHookEvent::PostToolUse,
            &failed_tool.to_string(),
            temp.path(),
            15,
        );
        assert!(!temp.path().join("antigravity-ide").exists());

        hook_with_root(
            AntigravityHookEvent::PreInvocation,
            &payload.to_string(),
            temp.path(),
            20,
        );
        let before = state_record(temp.path(), "antigravity-ide");
        hook_with_root(
            AntigravityHookEvent::PostToolUse,
            &payload.to_string(),
            temp.path(),
            30,
        );
        let after = state_record(temp.path(), "antigravity-ide");
        assert_eq!(after, before);
    }

    #[test]
    fn malformed_and_oversized_payloads_are_bounded_and_fail_open() {
        let temp = TempDir::new().unwrap();
        for event in EVENTS {
            let (code, output) = hook_with_root(event, "not json", temp.path(), 10);
            assert_eq!(code, 0);
            assert_eq!(output.as_bytes(), event.fail_open_output());
        }

        let workspace = serde_json::to_string(&temp.path().to_string_lossy()).unwrap();
        let input = format!(
            "{{\"conversationId\":\"c\",\"workspacePaths\":[{workspace}],\"transcriptPath\":\"{}\"}}",
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
        let code = run_antigravity_hook_with(
            AntigravityHookEvent::Stop,
            reader,
            &mut output,
            Ok(temp.path()),
            None,
            None,
            20,
        );
        assert_eq!(code, 0);
        assert_eq!(output, AntigravityHookEvent::Stop.fail_open_output());
        assert!(consumed.get() <= MAX_HOOK_INPUT_BYTES + 1);
        assert!(!temp.path().join("antigravity").exists());
    }

    #[test]
    fn invalid_identity_or_workspace_paths_do_not_create_state() {
        let temp = TempDir::new().unwrap();
        let inputs = [
            json!({"workspacePaths":[temp.path()]}),
            json!({"conversationId":"", "workspacePaths":[temp.path()]}),
            json!({"conversationId":"c", "workspacePaths":["relative/path"]}),
            json!({"conversationId":"c", "workspacePaths":[]}),
        ];
        for input in inputs {
            let (code, output) = hook_with_root(
                AntigravityHookEvent::PreInvocation,
                &input.to_string(),
                temp.path(),
                10,
            );
            assert_eq!(code, 0);
            assert_eq!(output, "{}\n");
        }
        assert!(!temp.path().join("antigravity").exists());
    }

    #[test]
    fn install_merges_named_entry_and_preserves_a_one_time_backup() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join(".gemini").join("config");
        let config = config_dir.join(HOOKS_FILENAME);
        write_json(
            &config,
            &json!({
                "team-linter": {
                    "PostToolUse": [{"matcher":"run_command","hooks":[{"command":"lint"}]}]
                }
            }),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());

        let result = install_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(result.changed);
        assert!(!result.migrated);
        assert!(result.status.installed);
        assert_eq!(
            fs::read(config_dir.join(BACKUP_FILENAME)).unwrap(),
            original
        );

        let installed = parse_config(&config_dir);
        assert_eq!(
            installed["team-linter"]["PostToolUse"][0]["hooks"][0]["command"],
            "lint"
        );
        let managed = &installed[MANAGED_HOOK_NAME];
        assert_eq!(managed["enabled"], true);
        assert!(managed["PreInvocation"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with(" antigravity-hook pre-invocation"));
        assert_eq!(managed["PostToolUse"][0]["matcher"], "*");
        assert!(managed["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with(" antigravity-hook post-tool-use"));
        assert!(managed["Stop"][0]["command"]
            .as_str()
            .unwrap()
            .ends_with(" antigravity-hook stop"));

        let backup = fs::read(config_dir.join(BACKUP_FILENAME)).unwrap();
        let second = install_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(!second.changed);
        assert_eq!(fs::read(config_dir.join(BACKUP_FILENAME)).unwrap(), backup);
    }

    #[test]
    fn stale_entry_is_repaired_and_uninstall_preserves_other_named_hooks() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join("config");
        let executable = executable(temp.path());
        let stale_command = if cfg!(windows) {
            r#""C:\old\vsparallel.exe" antigravity-hook pre-invocation"#
        } else {
            "/old/vsparallel antigravity-hook pre-invocation"
        };
        write_json(
            &config_dir.join(HOOKS_FILENAME),
            &json!({
                MANAGED_HOOK_NAME: {
                    "PreInvocation": [{"command":stale_command}]
                },
                "keep": {"Stop":[{"command":"keep"}]}
            }),
        );

        let before = antigravity_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(before.state, "stale");
        let installed = install_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(installed.changed);
        assert!(installed.migrated);
        assert!(installed.status.installed);

        let removed = uninstall_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(removed.changed);
        assert!(!removed.status.installed);
        let remaining = parse_config(&config_dir);
        assert!(remaining.get(MANAGED_HOOK_NAME).is_none());
        assert_eq!(remaining["keep"]["Stop"][0]["command"], "keep");
    }

    #[test]
    fn disabled_entry_is_reported_and_reenabled_on_install() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join("config");
        let executable = executable(temp.path());
        let handlers = managed_handlers(&executable).unwrap();
        let mut entry = canonical_managed_entry(&handlers);
        entry["enabled"] = json!(false);
        write_json(
            &config_dir.join(HOOKS_FILENAME),
            &json!({MANAGED_HOOK_NAME: entry}),
        );

        let status = antigravity_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(status.state, "disabled");
        assert!(status.hooks_disabled);
        let repaired = install_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(repaired.changed);
        assert!(repaired.migrated);
        assert!(repaired.status.installed);
    }

    #[test]
    fn malformed_config_is_never_modified() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join(HOOKS_FILENAME);
        fs::write(&config, b"{not json").unwrap();
        let before = fs::read(&config).unwrap();

        assert!(install_antigravity_integration(&config_dir, &executable(temp.path())).is_err());
        assert_eq!(fs::read(&config).unwrap(), before);
        assert!(!config_dir.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn same_name_collision_is_reported_and_never_modified_or_removed() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join("config");
        let config = config_dir.join(HOOKS_FILENAME);
        write_json(
            &config,
            &json!({
                MANAGED_HOOK_NAME: {
                    "PreInvocation": [{"type":"command", "command":"run-my-own-hook"}]
                },
                "keep": {"Stop":[{"command":"keep"}]}
            }),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());

        let status = antigravity_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(status.state, "conflict");
        assert!(!status.installed);
        assert!(status.message.contains("Rename or remove"));

        let error = install_antigravity_integration(&config_dir, &executable).unwrap_err();
        assert!(error.contains("does not own"));
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!config_dir.join(BACKUP_FILENAME).exists());

        let removed = uninstall_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(!removed.changed);
        assert_eq!(removed.status.state, "conflict");
        assert_eq!(fs::read(&config).unwrap(), original);
    }

    #[test]
    fn user_handlers_inside_the_named_entry_are_never_claimed_or_removed() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join("config");
        let config = config_dir.join(HOOKS_FILENAME);
        write_json(
            &config,
            &json!({
                MANAGED_HOOK_NAME: {
                    "PreInvocation": [
                        {"command":"/old/vsparallel antigravity-hook pre-invocation"},
                        {"command":"keep-my-handler"}
                    ],
                    "Stop": [{"command":"/old/vsparallel antigravity-hook stop"}]
                }
            }),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());

        let status = antigravity_integration_status(&config_dir, &executable).unwrap();
        assert_eq!(status.state, "conflict");
        assert!(install_antigravity_integration(&config_dir, &executable).is_err());

        let removed = uninstall_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(!removed.changed);
        assert_eq!(removed.status.state, "conflict");
        assert_eq!(fs::read(&config).unwrap(), original);
        assert!(!config_dir.join(BACKUP_FILENAME).exists());
    }

    #[test]
    fn foreign_command_with_managed_argument_suffix_is_not_owned() {
        let temp = TempDir::new().unwrap();
        let config_dir = temp.path().join("config");
        let config = config_dir.join(HOOKS_FILENAME);
        write_json(
            &config,
            &json!({
                MANAGED_HOOK_NAME: {
                    "PreInvocation": [{
                        "type": "command",
                        "command": "echo antigravity-hook pre-invocation",
                        "timeout": HOOK_TIMEOUT_SECONDS
                    }]
                }
            }),
        );
        let original = fs::read(&config).unwrap();
        let executable = executable(temp.path());

        assert_eq!(
            antigravity_integration_status(&config_dir, &executable)
                .unwrap()
                .state,
            "conflict"
        );
        assert!(install_antigravity_integration(&config_dir, &executable).is_err());
        let removal = uninstall_antigravity_integration(&config_dir, &executable).unwrap();
        assert!(!removal.changed);
        assert_eq!(removal.status.state, "conflict");
        assert_eq!(fs::read(&config).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn state_writer_refuses_a_symbolic_link_subdirectory() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let victim = temp.path().join("victim");
        fs::create_dir(&state_root).unwrap();
        fs::create_dir(&victim).unwrap();
        symlink(&victim, state_root.join("antigravity")).unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        let input = json!({
            "conversationId":"c",
            "workspacePaths":[workspace],
            "transcriptPath":"/home/test/.gemini/antigravity/brain/c/transcript.jsonl",
        })
        .to_string();

        let (code, output) =
            hook_with_root(AntigravityHookEvent::PreInvocation, &input, &state_root, 10);
        assert_eq!(code, 0);
        assert_eq!(output, "{}\n");
        assert!(fs::read_dir(&victim).unwrap().next().is_none());
        let observation = antigravity_two_hook_observation(&state_root, 10)
            .unwrap()
            .unwrap();
        assert_eq!(observation.outcome, AntigravityHookOutcome::PersistFailed);
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
}
