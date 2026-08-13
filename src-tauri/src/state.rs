use crate::antigravity_integration::AntigravityModelKind;
use crate::opener::EditorKind;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{hash_map::DefaultHasher, BTreeSet, BinaryHeap, HashMap};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: u32 = 1;
pub const ACTIVE_TTL_MS: i64 = 15_000;
pub const STALE_RETENTION_MS: i64 = 60_000;
pub const ACTIVITY_STALE_MS: i64 = 24 * 60 * 60 * 1_000;
const MAX_RECORD_BYTES: u64 = 64 * 1024;
const MAX_RECORD_CANDIDATES_PER_DIRECTORY: usize = 4_096;
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
const MAX_CURSOR_METADATA_BYTES: usize = 128;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceFolderRecord {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    index: u32,
    path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceFileRecord {
    path: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ExtensionPresenceRecord {
    #[serde(default)]
    available: bool,
    #[serde(default)]
    installed: bool,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    remote: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AgentExtensionsRecord {
    codex: Option<ExtensionPresenceRecord>,
    claude: Option<ExtensionPresenceRecord>,
}

/// Editor values accepted from the companion file protocol. Antigravity 2.0
/// does not host the companion and must be synthesized from a separate trusted
/// local source instead of being accepted from a heartbeat.
#[derive(Debug, Clone, Copy, Deserialize)]
enum CompanionEditorKind {
    #[serde(rename = "vscode")]
    VsCode,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "antigravity_ide")]
    AntigravityIde,
}

impl From<CompanionEditorKind> for EditorKind {
    fn from(value: CompanionEditorKind) -> Self {
        match value {
            CompanionEditorKind::VsCode => Self::VsCode,
            CompanionEditorKind::Cursor => Self::Cursor,
            CompanionEditorKind::AntigravityIde => Self::AntigravityIde,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstanceRecord {
    schema_version: u32,
    instance_id: String,
    #[serde(default)]
    editor: Option<CompanionEditorKind>,
    workspace_name: Option<String>,
    #[serde(default)]
    workspace_folders: Vec<WorkspaceFolderRecord>,
    workspace_file: Option<WorkspaceFileRecord>,
    primary_path: Option<String>,
    open_target: Option<String>,
    focused: bool,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    remote_window: bool,
    agent_extensions: Option<AgentExtensionsRecord>,
    last_seen_at_ms: i64,
    started_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivityRecord {
    schema_version: u32,
    session_key: String,
    cwd: String,
    #[serde(skip)]
    normalized_cwd: PathBuf,
    #[serde(skip)]
    cursor_instance_id: Option<String>,
    state: ActivityRecordState,
    changed_at_ms: i64,
    #[serde(default)]
    model_kind: Option<AntigravityModelKind>,
    #[serde(default)]
    ide_model_revision: Option<String>,
    #[serde(default)]
    model_name: Option<String>,
    #[serde(default)]
    agent_kind: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ActivityRecordState {
    // Cursor emits this editor/workspace observation before any agent turn. It
    // may establish a recent path, but never lifecycle or window liveness.
    WorkspaceOpened,
    SessionStarted,
    ActivityDetected,
    TurnFinished,
    SessionEnded,
    FailedOrInterrupted,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityView {
    pub state: String,
    pub label: String,
    pub changed_at_ms: Option<i64>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_kind: Option<AntigravityModelKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    pub extension_detection_available: Option<bool>,
    pub extension_installed: Option<bool>,
    pub extension_active: Option<bool>,
    pub extension_remote: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    pub instance_id: String,
    pub editor: EditorKind,
    pub editor_name: String,
    pub name: String,
    pub path: Option<String>,
    pub openable: bool,
    pub active: bool,
    pub focused: bool,
    pub recently_active: bool,
    pub remote_window: bool,
    pub last_seen_at_ms: i64,
    pub started_at_ms: i64,
    pub antigravity: Option<ActivityView>,
    pub cursor: Option<ActivityView>,
    pub codex: ActivityView,
    pub claude: ActivityView,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: u32,
    pub generated_at_ms: i64,
    pub workspaces: Vec<WorkspaceView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceOpenTarget {
    pub path: PathBuf,
    /// `None` is a legacy heartbeat and deliberately uses the configured
    /// default VS Code command rather than trusting an on-disk executable.
    pub editor: Option<EditorKind>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub schema_version: u32,
    pub state_directory: String,
    pub active_ttl_ms: i64,
    pub stale_retention_ms: i64,
    pub activity_stale_ms: i64,
    pub code_command: String,
    pub antigravity_ide_command: String,
    pub cursor_command: String,
    pub valid_instance_records: usize,
    pub malformed_instance_records: usize,
    pub omitted_instance_records: usize,
    pub valid_codex_records: usize,
    pub malformed_codex_records: usize,
    pub omitted_codex_records: usize,
    pub valid_claude_records: usize,
    pub malformed_claude_records: usize,
    pub omitted_claude_records: usize,
    pub valid_antigravity_records: usize,
    pub malformed_antigravity_records: usize,
    pub omitted_antigravity_records: usize,
    pub valid_cursor_records: usize,
    pub malformed_cursor_records: usize,
    pub omitted_cursor_records: usize,
    pub active_cursor_instance_records: usize,
    pub retained_cursor_instance_records: usize,
    pub latest_cursor_instance_at_ms: Option<i64>,
    pub recent_cursor_workspace_open_records: usize,
    pub latest_cursor_workspace_opened_at_ms: Option<i64>,
    pub antigravity_two_hook_observed_at_ms: Option<i64>,
    pub antigravity_two_hook_event: Option<String>,
    pub antigravity_two_hook_outcome: String,
    pub antigravity_two_hook_workspace_count: Option<u32>,
    pub antigravity_ide_hook_observed_at_ms: Option<i64>,
    pub antigravity_ide_hook_event: Option<String>,
    pub antigravity_ide_hook_outcome: String,
    pub antigravity_ide_hook_workspace_count: Option<u32>,
}

#[derive(Debug)]
struct LoadResult<T> {
    records: Vec<T>,
    malformed: usize,
    omitted: usize,
}

#[derive(Debug, Clone)]
pub struct StateStore {
    root: PathBuf,
    antigravity_ide_conversations: Option<PathBuf>,
}

impl StateStore {
    pub fn from_environment() -> Result<Self, String> {
        Ok(Self {
            root: state_dir_from_environment()?,
            antigravity_ide_conversations: crate::antigravity_integration::antigravity_ide_conversations_directory_from_environment().ok(),
        })
    }

    #[cfg(test)]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            antigravity_ide_conversations: None,
        }
    }

    #[cfg(test)]
    fn new_with_antigravity_ide_conversations(root: PathBuf, conversations: PathBuf) -> Self {
        Self {
            root,
            antigravity_ide_conversations: Some(conversations),
        }
    }

    pub fn snapshot(&self, now_ms: i64) -> Snapshot {
        let instances = self.load_instances(now_ms);
        let codex = self.load_codex(now_ms);
        let claude = self.load_claude(now_ms);
        let antigravity = self.load_antigravity(now_ms);
        let mut antigravity_ide = self.load_antigravity_ide(now_ms);
        let mut cursor = self.load_cursor(now_ms);
        self.reconcile_antigravity_ide_models(&mut antigravity_ide.records, now_ms);
        let mut latest_by_id: HashMap<String, InstanceRecord> = HashMap::new();

        for record in instances.records {
            let id = record.instance_id.clone();
            match latest_by_id.get(&id) {
                Some(existing) if existing.last_seen_at_ms >= record.last_seen_at_ms => {}
                _ => {
                    latest_by_id.insert(id, record);
                }
            }
        }

        let antigravity_ide_paths: Vec<PathBuf> = latest_by_id
            .values()
            .filter(|record| {
                matches!(record.editor, Some(CompanionEditorKind::AntigravityIde))
                    && now_ms.saturating_sub(record.last_seen_at_ms) <= STALE_RETENTION_MS
            })
            .flat_map(workspace_paths)
            .collect();
        let cursor_workspaces: Vec<(String, Vec<PathBuf>)> = latest_by_id
            .values()
            .filter(|record| {
                matches!(record.editor, Some(CompanionEditorKind::Cursor))
                    && now_ms.saturating_sub(record.last_seen_at_ms) <= STALE_RETENTION_MS
            })
            .map(|record| (record.instance_id.clone(), workspace_paths(record)))
            .collect();
        assign_cursor_heartbeat_owners(&mut cursor.records, &cursor_workspaces);

        let mut workspaces: Vec<_> = latest_by_id
            .into_values()
            .filter(|record| now_ms.saturating_sub(record.last_seen_at_ms) <= STALE_RETENTION_MS)
            .map(|record| {
                self.workspace_view(
                    record,
                    &antigravity_ide.records,
                    &cursor.records,
                    &codex.records,
                    &claude.records,
                    now_ms,
                )
            })
            .collect();

        workspaces.extend(self.antigravity_workspace_views(
            &antigravity.records,
            &codex.records,
            &claude.records,
            EditorKind::Antigravity2,
            &[],
            now_ms,
        ));
        workspaces.extend(self.cursor_workspace_views(
            &cursor.records,
            &codex.records,
            &claude.records,
            now_ms,
        ));
        workspaces.extend(self.antigravity_workspace_views(
            &antigravity_ide.records,
            &codex.records,
            &claude.records,
            EditorKind::AntigravityIde,
            &antigravity_ide_paths,
            now_ms,
        ));

        workspaces.sort_by(|left, right| {
            right
                .focused
                .cmp(&left.focused)
                .then_with(|| right.active.cmp(&left.active))
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.instance_id.cmp(&right.instance_id))
        });

        Snapshot {
            schema_version: SCHEMA_VERSION,
            generated_at_ms: now_ms,
            workspaces,
        }
    }

    pub fn diagnostics(
        &self,
        now_ms: i64,
        code_command: String,
        antigravity_ide_command: String,
        cursor_command: String,
    ) -> Diagnostics {
        let instances = self.load_instances(now_ms);
        let codex = self.load_codex(now_ms);
        let claude = self.load_claude(now_ms);
        let antigravity = self.load_antigravity(now_ms);
        let antigravity_ide = self.load_antigravity_ide(now_ms);
        let cursor = self.load_cursor(now_ms);
        let cursor_instances = instances
            .records
            .iter()
            .filter(|record| matches!(record.editor, Some(CompanionEditorKind::Cursor)));
        let active_cursor_instance_records = cursor_instances
            .clone()
            .filter(|record| now_ms.saturating_sub(record.last_seen_at_ms) <= ACTIVE_TTL_MS)
            .count();
        let retained_cursor_instance_records = cursor_instances
            .clone()
            .filter(|record| now_ms.saturating_sub(record.last_seen_at_ms) <= STALE_RETENTION_MS)
            .count();
        let latest_cursor_instance_at_ms =
            cursor_instances.map(|record| record.last_seen_at_ms).max();
        let cursor_workspace_opens = cursor.records.iter().filter(|record| {
            record.state == ActivityRecordState::WorkspaceOpened
                && now_ms.saturating_sub(record.changed_at_ms) <= ACTIVITY_STALE_MS
        });
        let recent_cursor_workspace_open_records = cursor_workspace_opens.clone().count();
        let latest_cursor_workspace_opened_at_ms = cursor_workspace_opens
            .map(|record| record.changed_at_ms)
            .max();
        let antigravity_hook =
            crate::antigravity_integration::antigravity_two_hook_observation(&self.root, now_ms);
        let (
            antigravity_two_hook_observed_at_ms,
            antigravity_two_hook_event,
            antigravity_two_hook_outcome,
            antigravity_two_hook_workspace_count,
        ) = match antigravity_hook {
            Ok(Some(observation)) => (
                Some(observation.observed_at_ms),
                Some(observation.event),
                observation.outcome.as_str().to_string(),
                Some(observation.workspace_count),
            ),
            Ok(None) => (None, None, "not_observed".to_string(), None),
            Err(_) => (None, None, "health_unreadable".to_string(), None),
        };
        let antigravity_ide_hook =
            crate::antigravity_integration::antigravity_ide_hook_observation(&self.root, now_ms);
        let (
            antigravity_ide_hook_observed_at_ms,
            antigravity_ide_hook_event,
            antigravity_ide_hook_outcome,
            antigravity_ide_hook_workspace_count,
        ) = match antigravity_ide_hook {
            Ok(Some(observation)) => (
                Some(observation.observed_at_ms),
                Some(observation.event),
                observation.outcome.as_str().to_string(),
                Some(observation.workspace_count),
            ),
            Ok(None) => (None, None, "not_observed".to_string(), None),
            Err(_) => (None, None, "health_unreadable".to_string(), None),
        };
        Diagnostics {
            schema_version: SCHEMA_VERSION,
            state_directory: self.root.to_string_lossy().into_owned(),
            active_ttl_ms: ACTIVE_TTL_MS,
            stale_retention_ms: STALE_RETENTION_MS,
            activity_stale_ms: ACTIVITY_STALE_MS,
            code_command,
            antigravity_ide_command,
            cursor_command,
            valid_instance_records: instances.records.len(),
            malformed_instance_records: instances.malformed,
            omitted_instance_records: instances.omitted,
            valid_codex_records: codex.records.len(),
            malformed_codex_records: codex.malformed,
            omitted_codex_records: codex.omitted,
            valid_claude_records: claude.records.len(),
            malformed_claude_records: claude.malformed,
            omitted_claude_records: claude.omitted,
            valid_antigravity_records: antigravity
                .records
                .len()
                .saturating_add(antigravity_ide.records.len()),
            malformed_antigravity_records: antigravity
                .malformed
                .saturating_add(antigravity_ide.malformed),
            omitted_antigravity_records: antigravity
                .omitted
                .saturating_add(antigravity_ide.omitted),
            valid_cursor_records: cursor.records.len(),
            malformed_cursor_records: cursor.malformed,
            omitted_cursor_records: cursor.omitted,
            active_cursor_instance_records,
            retained_cursor_instance_records,
            latest_cursor_instance_at_ms,
            recent_cursor_workspace_open_records,
            latest_cursor_workspace_opened_at_ms,
            antigravity_two_hook_observed_at_ms,
            antigravity_two_hook_event,
            antigravity_two_hook_outcome,
            antigravity_two_hook_workspace_count,
            antigravity_ide_hook_observed_at_ms,
            antigravity_ide_hook_event,
            antigravity_ide_hook_outcome,
            antigravity_ide_hook_workspace_count,
        }
    }

    pub(crate) fn find_workspace_open_target(
        &self,
        instance_id: &str,
        now_ms: i64,
    ) -> Option<WorkspaceOpenTarget> {
        self.find_workspace_open_target_with_max_age(instance_id, now_ms, STALE_RETENTION_MS)
    }

    pub(crate) fn find_active_workspace_open_target(
        &self,
        instance_id: &str,
        now_ms: i64,
    ) -> Option<WorkspaceOpenTarget> {
        self.find_workspace_open_target_with_max_age(instance_id, now_ms, ACTIVE_TTL_MS)
    }

    fn find_workspace_open_target_with_max_age(
        &self,
        instance_id: &str,
        now_ms: i64,
        max_age_ms: i64,
    ) -> Option<WorkspaceOpenTarget> {
        if instance_id.is_empty() || instance_id.len() > 256 {
            return None;
        }

        self.load_instances(now_ms)
            .records
            .into_iter()
            .filter(|record| record.instance_id == instance_id)
            .filter(|record| now_ms.saturating_sub(record.last_seen_at_ms) <= max_age_ms)
            .max_by_key(|record| record.last_seen_at_ms)
            .and_then(|record| {
                let path = open_target_path(&record)?;
                path.exists().then(|| WorkspaceOpenTarget {
                    path,
                    editor: record.editor.map(EditorKind::from),
                })
            })
    }

    fn workspace_view(
        &self,
        record: InstanceRecord,
        antigravity_records: &[ActivityRecord],
        cursor_records: &[ActivityRecord],
        codex_records: &[ActivityRecord],
        claude_records: &[ActivityRecord],
        now_ms: i64,
    ) -> WorkspaceView {
        let age = now_ms.saturating_sub(record.last_seen_at_ms);
        let active = age <= ACTIVE_TTL_MS;
        let target = open_target_path(&record);
        let display_path = record
            .primary_path
            .as_deref()
            .and_then(normalized_absolute_path)
            .or_else(|| target.clone())
            .map(|path| path.to_string_lossy().into_owned());
        let name = record
            .workspace_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                display_path.as_deref().and_then(|path| {
                    Path::new(path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                })
            })
            .unwrap_or_else(|| "Untitled workspace".to_string());
        let workspace_paths = workspace_paths(&record);
        let codex_extension = record
            .agent_extensions
            .as_ref()
            .and_then(|extensions| extensions.codex);
        let claude_extension = record
            .agent_extensions
            .as_ref()
            .and_then(|extensions| extensions.claude);
        let editor = record
            .editor
            .map(EditorKind::from)
            .unwrap_or(EditorKind::VsCode);
        let instance_id = record.instance_id;
        let antigravity = (editor == EditorKind::AntigravityIde)
            .then(|| {
                aggregate_activity_if_observed(
                    "Antigravity",
                    &workspace_paths,
                    antigravity_records,
                    now_ms,
                )
            })
            .flatten();
        let cursor = (editor == EditorKind::Cursor)
            .then(|| {
                aggregate_cursor_activity_if_observed(
                    &workspace_paths,
                    cursor_records,
                    Some(&instance_id),
                    now_ms,
                )
            })
            .flatten();

        WorkspaceView {
            instance_id,
            editor,
            editor_name: editor.display_name().to_string(),
            name,
            path: display_path,
            openable: target.as_ref().is_some_and(|path| path.exists()),
            active,
            focused: active && record.focused,
            recently_active: active && record.active,
            remote_window: record.remote_window,
            last_seen_at_ms: record.last_seen_at_ms,
            started_at_ms: record.started_at_ms,
            antigravity,
            cursor,
            codex: aggregate_activity(
                "Codex",
                &workspace_paths,
                codex_records,
                codex_extension,
                Some(editor.display_name()),
                record.remote_window,
                now_ms,
            ),
            claude: aggregate_activity(
                "Claude Code",
                &workspace_paths,
                claude_records,
                claude_extension,
                Some(editor.display_name()),
                record.remote_window,
                now_ms,
            ),
        }
    }

    fn antigravity_workspace_views(
        &self,
        records: &[ActivityRecord],
        codex_records: &[ActivityRecord],
        claude_records: &[ActivityRecord],
        editor: EditorKind,
        companion_paths: &[PathBuf],
        now_ms: i64,
    ) -> Vec<WorkspaceView> {
        let mut newest_by_path: HashMap<PathBuf, &ActivityRecord> = HashMap::new();
        for record in records
            .iter()
            .filter(|record| now_ms.saturating_sub(record.changed_at_ms) <= ACTIVITY_STALE_MS)
        {
            match newest_by_path.get(&record.normalized_cwd) {
                Some(existing) if existing.changed_at_ms >= record.changed_at_ms => {}
                _ => {
                    newest_by_path.insert(record.normalized_cwd.clone(), record);
                }
            }
        }

        newest_by_path
            .into_iter()
            .filter(|(path, _)| !companion_paths.iter().any(|known| known == path))
            .map(|(path, newest)| {
                let workspace_paths = vec![path.clone()];
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "Antigravity project".to_string());
                let instance_id = format!(
                    "{}:{}",
                    match editor {
                        EditorKind::AntigravityIde => "antigravity-ide",
                        _ => "antigravity-2",
                    },
                    stable_path_key(&path)
                );
                WorkspaceView {
                    instance_id,
                    editor,
                    editor_name: editor.display_name().to_string(),
                    name,
                    path: Some(path.to_string_lossy().into_owned()),
                    openable: false,
                    active: false,
                    focused: false,
                    recently_active: true,
                    remote_window: false,
                    last_seen_at_ms: newest.changed_at_ms,
                    started_at_ms: newest.changed_at_ms,
                    antigravity: aggregate_activity_if_observed(
                        "Antigravity",
                        &workspace_paths,
                        records,
                        now_ms,
                    ),
                    cursor: None,
                    codex: aggregate_activity(
                        "Codex",
                        &workspace_paths,
                        codex_records,
                        None,
                        None,
                        false,
                        now_ms,
                    ),
                    claude: aggregate_activity(
                        "Claude Code",
                        &workspace_paths,
                        claude_records,
                        None,
                        None,
                        false,
                        now_ms,
                    ),
                }
            })
            .collect()
    }

    fn cursor_workspace_views(
        &self,
        records: &[ActivityRecord],
        codex_records: &[ActivityRecord],
        claude_records: &[ActivityRecord],
        now_ms: i64,
    ) -> Vec<WorkspaceView> {
        let mut newest_by_path: HashMap<PathBuf, &ActivityRecord> = HashMap::new();
        for record in records.iter().filter(|record| {
            record.state != ActivityRecordState::SessionStarted
                && record.cursor_instance_id.is_none()
                && now_ms.saturating_sub(record.changed_at_ms) <= ACTIVITY_STALE_MS
        }) {
            match newest_by_path.get(&record.normalized_cwd) {
                Some(existing) if existing.changed_at_ms >= record.changed_at_ms => {}
                _ => {
                    newest_by_path.insert(record.normalized_cwd.clone(), record);
                }
            }
        }

        newest_by_path
            .into_iter()
            .map(|(path, newest)| {
                let workspace_paths = vec![path.clone()];
                let cursor =
                    aggregate_cursor_activity_if_observed(&workspace_paths, records, None, now_ms);
                let lifecycle_observed = cursor.is_some();
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "Cursor project".to_string());
                WorkspaceView {
                    instance_id: format!(
                        "{}:{}",
                        if lifecycle_observed {
                            "cursor-agent"
                        } else {
                            "cursor-workspace"
                        },
                        stable_path_key(&path)
                    ),
                    editor: EditorKind::Cursor,
                    editor_name: if lifecycle_observed {
                        "Cursor Agent".to_string()
                    } else {
                        "Cursor".to_string()
                    },
                    name,
                    path: Some(path.to_string_lossy().into_owned()),
                    openable: false,
                    active: false,
                    focused: false,
                    // A workspaceOpen observation is presence metadata, not
                    // agent activity. Real lifecycle records retain the
                    // historical hook-only recent-activity affordance.
                    recently_active: lifecycle_observed,
                    remote_window: false,
                    last_seen_at_ms: newest.changed_at_ms,
                    started_at_ms: newest.changed_at_ms,
                    antigravity: None,
                    cursor,
                    codex: aggregate_activity(
                        "Codex",
                        &workspace_paths,
                        codex_records,
                        None,
                        None,
                        false,
                        now_ms,
                    ),
                    claude: aggregate_activity(
                        "Claude Code",
                        &workspace_paths,
                        claude_records,
                        None,
                        None,
                        false,
                        now_ms,
                    ),
                }
            })
            .collect()
    }

    fn load_instances(&self, now_ms: i64) -> LoadResult<InstanceRecord> {
        load_records(
            &self.root.join("instances"),
            |record: &mut InstanceRecord| {
                record.schema_version == SCHEMA_VERSION
                    && !record.instance_id.trim().is_empty()
                    && record.instance_id.len() <= 256
                    && valid_timestamp(record.last_seen_at_ms, now_ms)
                    && valid_timestamp(record.started_at_ms, now_ms)
                    && record.started_at_ms
                        <= record.last_seen_at_ms.saturating_add(MAX_FUTURE_SKEW_MS)
            },
        )
    }

    fn load_codex(&self, now_ms: i64) -> LoadResult<ActivityRecord> {
        self.load_activity("codex", now_ms)
    }

    fn load_claude(&self, now_ms: i64) -> LoadResult<ActivityRecord> {
        self.load_activity("claude", now_ms)
    }

    fn load_antigravity(&self, now_ms: i64) -> LoadResult<ActivityRecord> {
        self.load_activity("antigravity", now_ms)
    }

    fn load_antigravity_ide(&self, now_ms: i64) -> LoadResult<ActivityRecord> {
        self.load_activity("antigravity-ide", now_ms)
    }

    fn load_cursor(&self, now_ms: i64) -> LoadResult<ActivityRecord> {
        self.load_activity("cursor", now_ms)
    }

    fn reconcile_antigravity_ide_models(&self, records: &mut [ActivityRecord], now_ms: i64) {
        let Some(directory) = self.antigravity_ide_conversations.as_deref() else {
            return;
        };
        let session_keys: BTreeSet<_> = records
            .iter()
            .filter(|record| now_ms.saturating_sub(record.changed_at_ms) <= ACTIVITY_STALE_MS)
            .map(|record| record.session_key.clone())
            .collect();
        let models = crate::antigravity_integration::antigravity_ide_execution_models(
            directory,
            &session_keys,
        );
        for record in records {
            if let Some(model) = models.get(&record.session_key) {
                if record.ide_model_revision.as_deref() != Some(model.revision.as_str()) {
                    // A USER_INPUT row appears just before its PreInvocation.
                    // Only that hook can correlate the row to an activity
                    // boundary; adopting it here could relabel the preceding
                    // turn during that narrow window. Execution metadata is
                    // safe to reconcile after the fact, while a matching
                    // current-turn revision simply preserves the hook result.
                    if model.source == crate::antigravity_integration::IdeModelSource::CurrentTurn {
                        continue;
                    }
                    record.model_kind = match model.preference {
                        crate::antigravity_integration::IdeSelectedModelPreference::Recognized(
                            kind,
                        ) => Some(kind),
                        crate::antigravity_integration::IdeSelectedModelPreference::Unrecognized => {
                            None
                        }
                    };
                    record.ide_model_revision = Some(model.revision.clone());
                }
            }
        }
    }

    fn load_activity(&self, provider: &str, now_ms: i64) -> LoadResult<ActivityRecord> {
        let supports_model = matches!(provider, "antigravity" | "antigravity-ide");
        let supports_cursor_metadata = provider == "cursor";
        load_records(&self.root.join(provider), |record: &mut ActivityRecord| {
            if record.schema_version != SCHEMA_VERSION
                || record.session_key.trim().is_empty()
                || record.session_key.len() > 128
                || !valid_timestamp(record.changed_at_ms, now_ms)
                || (matches!(
                    record.state,
                    ActivityRecordState::WorkspaceOpened | ActivityRecordState::SessionStarted
                ) && !supports_cursor_metadata)
            {
                return false;
            }

            let Some(normalized_cwd) = normalized_absolute_path(&record.cwd) else {
                return false;
            };
            record.normalized_cwd = normalized_cwd;
            record.cwd.clear();
            if !supports_model || record.model_kind == Some(AntigravityModelKind::Unknown) {
                record.model_kind = None;
            }
            if !supports_model
                || record
                    .ide_model_revision
                    .as_deref()
                    .is_some_and(|revision| !valid_sha256_token(revision))
            {
                record.ide_model_revision = None;
            }
            if supports_cursor_metadata {
                record.model_name = record
                    .model_name
                    .take()
                    .and_then(normalized_cursor_model_name);
                record.agent_kind = record
                    .agent_kind
                    .take()
                    .and_then(normalized_cursor_agent_kind);
            } else {
                record.model_name = None;
                record.agent_kind = None;
            }
            true
        })
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub fn state_dir_from_environment() -> Result<PathBuf, String> {
    if let Some(path) = nonempty_env_path("VSPARALLEL_STATE_DIR") {
        if path.is_absolute() {
            return Ok(path);
        }
        return Err("VSPARALLEL_STATE_DIR must be an absolute path".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(path) =
            nonempty_env_path("LOCALAPPDATA").or_else(|| nonempty_env_path("APPDATA"))
        {
            return Ok(path.join("VSParallel"));
        }
        if let Some(home) = nonempty_env_path("USERPROFILE") {
            return Ok(home.join("AppData").join("Local").join("VSParallel"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = nonempty_env_path("HOME") {
            return Ok(home
                .join("Library")
                .join("Application Support")
                .join("VSParallel"));
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(path) = nonempty_env_path("XDG_STATE_HOME") {
            return Ok(path.join("vsparallel"));
        }
        if let Some(home) = nonempty_env_path("HOME") {
            return Ok(home.join(".local").join("state").join("vsparallel"));
        }
    }

    Err("could not determine the VSParallel state directory; set VSPARALLEL_STATE_DIR".to_string())
}

fn nonempty_env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn valid_timestamp(timestamp: i64, now_ms: i64) -> bool {
    timestamp >= 0 && timestamp <= now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
}

fn valid_sha256_token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn normalized_cursor_model_name(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= MAX_CURSOR_METADATA_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'_' | b'.' | b'/' | b':' | b'+' | b'(' | b')' | b' ' | b','
                )
        }))
    .then(|| value.to_string())
}

fn normalized_cursor_agent_kind(value: String) -> Option<String> {
    matches!(
        value.as_str(),
        "Background agent" | "Agent" | "Ask" | "Edit"
    )
    .then_some(value)
}

fn load_records<T, F>(directory: &Path, validate: F) -> LoadResult<T>
where
    T: for<'de> Deserialize<'de>,
    F: FnMut(&mut T) -> bool,
{
    load_records_with_limit(directory, MAX_RECORD_CANDIDATES_PER_DIRECTORY, validate)
}

fn load_records_with_limit<T, F>(
    directory: &Path,
    candidate_limit: usize,
    mut validate: F,
) -> LoadResult<T>
where
    T: for<'de> Deserialize<'de>,
    F: FnMut(&mut T) -> bool,
{
    let mut result = LoadResult {
        records: Vec::new(),
        malformed: 0,
        omitted: 0,
    };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return result,
        Err(_) => {
            result.malformed = 1;
            return result;
        }
    };

    let mut candidate_count = 0usize;
    let mut candidates = BinaryHeap::with_capacity(candidate_limit);
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                result.malformed += 1;
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_RECORD_BYTES => metadata,
            _ => {
                result.malformed += 1;
                continue;
            }
        };
        if metadata.len() == 0 {
            result.malformed += 1;
            continue;
        }

        candidate_count = candidate_count.saturating_add(1);
        retain_newest_candidate(
            &mut candidates,
            candidate_limit,
            metadata.modified().unwrap_or(UNIX_EPOCH),
            path,
        );
    }

    result.omitted = candidate_count.saturating_sub(candidates.len());
    let mut candidates: Vec<_> = candidates
        .into_iter()
        .map(|Reverse((modified, path))| (modified, path))
        .collect();
    candidates
        .sort_unstable_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

    for (_, path) in candidates {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                result.malformed += 1;
                continue;
            }
        };
        match serde_json::from_slice::<T>(&bytes) {
            Ok(mut record) => {
                if validate(&mut record) {
                    result.records.push(record);
                } else {
                    result.malformed += 1;
                }
            }
            Err(_) => result.malformed += 1,
        }
    }

    result
}

fn retain_newest_candidate(
    candidates: &mut BinaryHeap<Reverse<(SystemTime, PathBuf)>>,
    candidate_limit: usize,
    modified: SystemTime,
    path: PathBuf,
) {
    if candidate_limit == 0 {
        return;
    }

    let candidate = (modified, path);
    if candidates.len() < candidate_limit {
        candidates.push(Reverse(candidate));
    } else if candidates.peek().is_some_and(|oldest| candidate > oldest.0) {
        candidates.pop();
        candidates.push(Reverse(candidate));
    }
}

fn open_target_path(record: &InstanceRecord) -> Option<PathBuf> {
    record
        .open_target
        .as_deref()
        .and_then(normalized_absolute_path)
        .or_else(|| {
            record
                .workspace_file
                .as_ref()
                .and_then(|workspace| workspace.path.as_deref())
                .and_then(normalized_absolute_path)
        })
        .or_else(|| {
            record
                .primary_path
                .as_deref()
                .and_then(normalized_absolute_path)
        })
}

fn workspace_paths(record: &InstanceRecord) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = record
        .primary_path
        .as_deref()
        .and_then(normalized_absolute_path)
    {
        paths.push(path);
    }
    for folder in &record.workspace_folders {
        if let Some(path) = folder.path.as_deref().and_then(normalized_absolute_path) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn normalized_absolute_path(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw.trim());
    if raw.trim().is_empty() || !path.is_absolute() {
        return None;
    }
    if let Ok(canonical) = fs::canonicalize(path) {
        return Some(canonical);
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

fn path_is_within(path: &Path, root: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let path = path.to_string_lossy().to_lowercase();
        let root = root.to_string_lossy().to_lowercase();
        return path == root
            || path
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with(['/', '\\']));
    }

    #[cfg(not(target_os = "windows"))]
    {
        path.starts_with(root)
    }
}

fn stable_path_key(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn assign_cursor_heartbeat_owners(
    records: &mut [ActivityRecord],
    cursor_workspaces: &[(String, Vec<PathBuf>)],
) {
    for record in records {
        let mut matching = cursor_workspaces.iter().filter(|(_, workspace_paths)| {
            workspace_paths
                .iter()
                .any(|workspace| path_is_within(&record.normalized_cwd, workspace))
        });
        record.cursor_instance_id = match (matching.next(), matching.next()) {
            (Some((instance_id, _)), None) => Some(instance_id.clone()),
            _ => None,
        };
    }
}

fn aggregate_activity_if_observed(
    provider: &str,
    workspace_paths: &[PathBuf],
    records: &[ActivityRecord],
    now_ms: i64,
) -> Option<ActivityView> {
    records
        .iter()
        .any(|record| {
            workspace_paths
                .iter()
                .any(|workspace| path_is_within(&record.normalized_cwd, workspace))
        })
        .then(|| {
            aggregate_activity(
                provider,
                workspace_paths,
                records,
                None,
                None,
                false,
                now_ms,
            )
        })
}

fn aggregate_cursor_activity_if_observed(
    workspace_paths: &[PathBuf],
    records: &[ActivityRecord],
    instance_id: Option<&str>,
    now_ms: i64,
) -> Option<ActivityView> {
    let matching: Vec<_> = records
        .iter()
        .filter(|record| {
            // Neither workspaceOpen nor sessionStart says that an agent turn
            // has begun. They must not become an ActivityView.
            !matches!(
                record.state,
                ActivityRecordState::WorkspaceOpened | ActivityRecordState::SessionStarted
            ) && record.cursor_instance_id.as_deref() == instance_id
                && workspace_paths
                    .iter()
                    .any(|workspace| path_is_within(&record.normalized_cwd, workspace))
        })
        .collect();
    (!matching.is_empty())
        .then(|| aggregate_matching_activity("Cursor Agent", matching, None, None, now_ms))
}

fn aggregate_activity(
    provider: &str,
    workspace_paths: &[PathBuf],
    records: &[ActivityRecord],
    extension: Option<ExtensionPresenceRecord>,
    editor_name: Option<&str>,
    remote_window: bool,
    now_ms: i64,
) -> ActivityView {
    if workspace_paths.is_empty() {
        return pathless_activity(provider, extension, remote_window);
    }

    let matching: Vec<&ActivityRecord> = records
        .iter()
        .filter(|record| {
            workspace_paths
                .iter()
                .any(|workspace| path_is_within(&record.normalized_cwd, workspace))
        })
        .collect();

    if matching.is_empty() {
        return unknown_activity(provider, extension, editor_name);
    }

    aggregate_matching_activity(provider, matching, extension, editor_name, now_ms)
}

fn aggregate_matching_activity(
    provider: &str,
    mut matching: Vec<&ActivityRecord>,
    extension: Option<ExtensionPresenceRecord>,
    _editor_name: Option<&str>,
    now_ms: i64,
) -> ActivityView {
    if provider == "Cursor Agent" {
        let mut newest_by_session: HashMap<&str, &ActivityRecord> = HashMap::new();
        for record in matching {
            match newest_by_session.get(record.session_key.as_str()) {
                Some(existing) if !cursor_record_is_newer(record, existing) => {}
                _ => {
                    newest_by_session.insert(record.session_key.as_str(), record);
                }
            }
        }
        matching = newest_by_session.into_values().collect();
    }

    matching.sort_by_key(|record| record.changed_at_ms);
    let newest = *matching.last().expect("matching is not empty");
    let fresh_activity = if provider == "Antigravity" {
        // Antigravity qualifiers and lifecycle status must describe the same
        // most-recent record.
        (newest.state == ActivityRecordState::ActivityDetected
            && now_ms.saturating_sub(newest.changed_at_ms) <= ACTIVITY_STALE_MS)
            .then_some(newest)
    } else if provider == "Cursor Agent" {
        // Each Cursor session was reduced to its newest record above. A
        // terminal marker therefore closes only its own session, while another
        // concurrent session can remain active with coherent metadata.
        matching.iter().rev().copied().find(|record| {
            record.state == ActivityRecordState::ActivityDetected
                && now_ms.saturating_sub(record.changed_at_ms) <= ACTIVITY_STALE_MS
        })
    } else {
        matching.iter().rev().copied().find(|record| {
            record.state == ActivityRecordState::ActivityDetected
                && now_ms.saturating_sub(record.changed_at_ms) <= ACTIVITY_STALE_MS
        })
    };
    if let Some(record) = fresh_activity {
        return activity_view_with_metadata(
            "activity_detected",
            "Activity detected",
            Some(record.changed_at_ms),
            format!(
                "A {provider} turn-start hook was observed. This is a lifecycle marker, not live progress."
            ),
            (
                record.model_kind,
                record.model_name.clone(),
                record.agent_kind.clone(),
            ),
            extension,
        );
    }

    if now_ms.saturating_sub(newest.changed_at_ms) > ACTIVITY_STALE_MS {
        return activity_view_with_metadata(
            "unknown",
            "Unknown",
            Some(newest.changed_at_ms),
            format!("The last {provider} lifecycle signal is stale."),
            (
                newest.model_kind,
                newest.model_name.clone(),
                newest.agent_kind.clone(),
            ),
            extension,
        );
    }

    match newest.state {
        ActivityRecordState::TurnFinished | ActivityRecordState::SessionEnded => {
            activity_view_with_metadata(
                "turn_finished",
                "Turn finished",
                Some(newest.changed_at_ms),
                if newest.state == ActivityRecordState::SessionEnded {
                    format!("A {provider} session-end hook was observed.")
                } else {
                    format!("A {provider} Stop hook was observed.")
                },
                (
                    newest.model_kind,
                    newest.model_name.clone(),
                    newest.agent_kind.clone(),
                ),
                extension,
            )
        }
        ActivityRecordState::FailedOrInterrupted
        | ActivityRecordState::Failed
        | ActivityRecordState::Interrupted => activity_view_with_metadata(
            "failed_or_interrupted",
            "Failed/interrupted",
            Some(newest.changed_at_ms),
            if provider == "Claude Code" {
                "A Claude Code StopFailure hook reported an API failure. User interrupts do not emit a documented completion hook."
                    .to_string()
            } else {
                format!("A {provider} failure or interruption lifecycle signal was observed.")
            },
            (
                newest.model_kind,
                newest.model_name.clone(),
                newest.agent_kind.clone(),
            ),
            extension,
        ),
        ActivityRecordState::WorkspaceOpened | ActivityRecordState::SessionStarted => {
            unreachable!("metadata-only Cursor records are filtered before aggregation")
        }
        ActivityRecordState::ActivityDetected => unreachable!("fresh activity returned above"),
    }
}

fn cursor_record_is_newer(candidate: &ActivityRecord, existing: &ActivityRecord) -> bool {
    candidate.changed_at_ms > existing.changed_at_ms
        || (candidate.changed_at_ms == existing.changed_at_ms
            && cursor_state_precedence(candidate.state) > cursor_state_precedence(existing.state))
}

fn cursor_state_precedence(state: ActivityRecordState) -> u8 {
    match state {
        ActivityRecordState::WorkspaceOpened | ActivityRecordState::SessionStarted => 0,
        ActivityRecordState::ActivityDetected => 1,
        ActivityRecordState::TurnFinished
        | ActivityRecordState::SessionEnded
        | ActivityRecordState::FailedOrInterrupted
        | ActivityRecordState::Failed
        | ActivityRecordState::Interrupted => 2,
    }
}

fn pathless_activity(
    provider: &str,
    extension: Option<ExtensionPresenceRecord>,
    remote_window: bool,
) -> ActivityView {
    if remote_window {
        return activity_view(
            "unknown",
            "Remote workspace",
            None,
            format!(
                "Remote workspace paths are omitted for privacy. This release has no remote bridge for associating {provider} lifecycle signals with this window."
            ),
            None,
            extension,
        );
    }

    activity_view(
        "unknown",
        "Workspace path needed",
        None,
        format!(
            "Open a local folder or saved workspace so VSParallel can associate {provider} lifecycle signals with this window."
        ),
        None,
        extension,
    )
}

fn unknown_activity(
    provider: &str,
    extension: Option<ExtensionPresenceRecord>,
    editor_name: Option<&str>,
) -> ActivityView {
    let mut detail = match extension {
        Some(extension) if !extension.available => format!(
            "{provider} extension presence could not be checked; no lifecycle signal has been observed."
        ),
        Some(extension) if extension.active => format!(
            "The {provider} extension is active in this window, but no lifecycle signal has been observed."
        ),
        Some(extension) if extension.installed => format!(
            "The {provider} extension is installed but not active in this window; no lifecycle signal has been observed."
        ),
        Some(_) => format!(
            "The {provider} extension was not detected in this {} window.",
            editor_name.unwrap_or("VS Code")
        ),
        None => format!("No matching {provider} lifecycle signal has been observed."),
    };
    detail.push_str(&format!(
        " Start {provider} in this workspace and submit a prompt to create the first lifecycle marker."
    ));
    activity_view("unknown", "No activity yet", None, detail, None, extension)
}

fn activity_view(
    state: &str,
    label: &str,
    changed_at_ms: Option<i64>,
    detail: String,
    model_kind: Option<AntigravityModelKind>,
    extension: Option<ExtensionPresenceRecord>,
) -> ActivityView {
    activity_view_with_metadata(
        state,
        label,
        changed_at_ms,
        detail,
        (model_kind, None, None),
        extension,
    )
}

fn activity_view_with_metadata(
    state: &str,
    label: &str,
    changed_at_ms: Option<i64>,
    detail: String,
    metadata: (Option<AntigravityModelKind>, Option<String>, Option<String>),
    extension: Option<ExtensionPresenceRecord>,
) -> ActivityView {
    let (model_kind, model_name, agent_kind) = metadata;
    ActivityView {
        state: state.to_string(),
        label: label.to_string(),
        changed_at_ms,
        detail,
        model_kind,
        model_name,
        agent_kind,
        extension_detection_available: extension.map(|value| value.available),
        extension_installed: extension.map(|value| value.installed),
        extension_active: extension.map(|value| value.active),
        extension_remote: extension.and_then(|value| value.remote),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    fn write_json(path: &Path, value: serde_json::Value) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
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

    fn write_test_ide_executor_model(database: &Path, index: i64, model_name: &str) {
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        let planner = test_length_delimited_field(28, model_name.as_bytes());
        let cascade = test_length_delimited_field(1, &planner);
        let executor = test_length_delimited_field(10, &cascade);
        let connection = rusqlite::Connection::open(database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS executor_metadata \
                 (idx INTEGER PRIMARY KEY, data BLOB);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO executor_metadata (idx, data) VALUES (?1, ?2)",
                rusqlite::params![index, executor],
            )
            .unwrap();
    }

    fn write_test_ide_turn_model(database: &Path, index: i64, model_enum: u64) {
        fs::create_dir_all(database.parent().unwrap()).unwrap();
        let requested_model = test_varint_field(1, model_enum);
        let planner_config = test_length_delimited_field(15, &requested_model);
        let user_config = test_length_delimited_field(1, &planner_config);
        let user_input = test_length_delimited_field(12, &user_config);
        let step = test_length_delimited_field(19, &user_input);
        let connection = rusqlite::Connection::open(database).unwrap();
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
                rusqlite::params![index, step],
            )
            .unwrap();
    }

    fn instance(id: &str, path: &Path, seen: i64, focused: bool) -> serde_json::Value {
        json!({
            "schemaVersion": 1,
            "instanceId": id,
            "workspaceName": id,
            "workspaceFolders": [{
                "name": id,
                "index": 0,
                "path": path
            }],
            "workspaceFile": null,
            "primaryPath": path,
            "openTarget": path,
            "focused": focused,
            "active": focused,
            "lastSeenAtMs": seen,
            "startedAtMs": seen - 1000
        })
    }

    #[test]
    fn candidate_heap_retains_only_the_newest_records() {
        let mut candidates = BinaryHeap::new();
        for (seconds, name) in [(1, "old"), (4, "newest"), (2, "middle"), (3, "newer")] {
            retain_newest_candidate(
                &mut candidates,
                2,
                UNIX_EPOCH + Duration::from_secs(seconds),
                PathBuf::from(name),
            );
        }

        let mut retained: Vec<_> = candidates
            .into_iter()
            .map(|Reverse((_, path))| path)
            .collect();
        retained.sort();
        assert_eq!(
            retained,
            vec![PathBuf::from("newer"), PathBuf::from("newest")]
        );
    }

    #[test]
    fn bounded_loader_reports_omitted_candidates_and_limits_validation() {
        let temp = TempDir::new().unwrap();
        let records = temp.path().join("records");
        for index in 0..5 {
            write_json(
                &records.join(format!("{index}.json")),
                json!({"record": index}),
            );
        }
        fs::write(records.join("empty.json"), b"").unwrap();

        let validated = Cell::new(0usize);
        let loaded: LoadResult<serde_json::Value> =
            load_records_with_limit(&records, 2, |_: &mut serde_json::Value| {
                validated.set(validated.get() + 1);
                true
            });

        assert_eq!(loaded.records.len(), 2);
        assert_eq!(validated.get(), 2);
        assert_eq!(loaded.omitted, 3);
        assert_eq!(loaded.malformed, 1);
    }

    #[test]
    fn diagnostics_enforce_and_report_the_directory_candidate_limit() {
        let temp = TempDir::new().unwrap();
        let instances = temp.path().join("instances");
        fs::create_dir_all(&instances).unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let source = temp.path().join("instance-record-source");
        fs::write(
            &source,
            serde_json::to_vec(&instance("shared", &repo, 20_000, false)).unwrap(),
        )
        .unwrap();
        for index in 0..=MAX_RECORD_CANDIDATES_PER_DIRECTORY {
            fs::hard_link(&source, instances.join(format!("{index:04}.json"))).unwrap();
        }

        let diagnostics = StateStore::new(temp.path().to_path_buf()).diagnostics(
            20_000,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(
            diagnostics.valid_instance_records,
            MAX_RECORD_CANDIDATES_PER_DIRECTORY
        );
        assert_eq!(diagnostics.omitted_instance_records, 1);
        assert_eq!(diagnostics.omitted_codex_records, 0);
        assert_eq!(diagnostics.omitted_claude_records, 0);
        assert_eq!(diagnostics.omitted_cursor_records, 0);

        let serialized = serde_json::to_value(diagnostics).unwrap();
        assert_eq!(serialized["codeCommand"], "code");
        assert_eq!(serialized["antigravityIdeCommand"], "antigravity-ide");
        assert_eq!(serialized["cursorCommand"], "cursor");
        assert_eq!(serialized["omittedInstanceRecords"], 1);
        assert_eq!(serialized["omittedCodexRecords"], 0);
        assert_eq!(serialized["omittedClaudeRecords"], 0);
        assert_eq!(serialized["omittedCursorRecords"], 0);
        assert_eq!(serialized["activeCursorInstanceRecords"], 0);
        assert_eq!(serialized["retainedCursorInstanceRecords"], 0);
        assert!(serialized["latestCursorInstanceAtMs"].is_null());
        assert_eq!(serialized["recentCursorWorkspaceOpenRecords"], 0);
        assert!(serialized["latestCursorWorkspaceOpenedAtMs"].is_null());
        assert_eq!(serialized["antigravityTwoHookOutcome"], "not_observed");
        assert!(serialized["antigravityTwoHookObservedAtMs"].is_null());
        assert_eq!(serialized["antigravityIdeHookOutcome"], "not_observed");
        assert!(serialized["antigravityIdeHookObservedAtMs"].is_null());
    }

    #[test]
    fn diagnostics_distinguish_hook_execution_from_configuration() {
        let temp = TempDir::new().unwrap();
        write_json(
            &temp
                .path()
                .join("antigravity-hook-health/antigravity-2.json"),
            json!({
                "schemaVersion": 1,
                "event": "pre-invocation",
                "surface": "antigravity_2",
                "outcome": "no_workspace",
                "observedAtMs": 20_000,
                "workspaceCount": 0
            }),
        );
        write_json(
            &temp
                .path()
                .join("antigravity-hook-health/antigravity-ide.json"),
            json!({
                "schemaVersion": 1,
                "event": "stop",
                "surface": "antigravity_ide",
                "outcome": "recorded",
                "observedAtMs": 19_000,
                "workspaceCount": 1
            }),
        );
        let store = StateStore::new(temp.path().to_path_buf());

        let observed = store.diagnostics(
            20_000,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(
            observed.antigravity_two_hook_event.as_deref(),
            Some("pre-invocation")
        );
        assert_eq!(observed.antigravity_two_hook_outcome, "no_workspace");
        assert_eq!(observed.antigravity_two_hook_observed_at_ms, Some(20_000));
        assert_eq!(observed.antigravity_two_hook_workspace_count, Some(0));
        assert_eq!(observed.antigravity_ide_hook_event.as_deref(), Some("stop"));
        assert_eq!(observed.antigravity_ide_hook_outcome, "recorded");
        assert_eq!(observed.antigravity_ide_hook_observed_at_ms, Some(19_000));
        assert_eq!(observed.antigravity_ide_hook_workspace_count, Some(1));

        fs::write(
            temp.path()
                .join("antigravity-hook-health/antigravity-2.json"),
            b"{not json",
        )
        .unwrap();
        let unreadable = store.diagnostics(
            20_000,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(unreadable.antigravity_two_hook_outcome, "health_unreadable");
        assert_eq!(unreadable.antigravity_ide_hook_outcome, "recorded");
    }

    #[test]
    fn activity_loader_caches_normalized_cwd() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let nested = repo.join("nested");
        fs::create_dir_all(&nested).unwrap();
        write_json(
            &temp.path().join("codex/session.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "session",
                "cwd": nested.join("..").join("nested"),
                "state": "turn_finished",
                "changedAtMs": 20_000
            }),
        );

        let loaded = StateStore::new(temp.path().to_path_buf()).load_codex(20_000);
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(
            loaded.records[0].normalized_cwd,
            fs::canonicalize(nested).unwrap()
        );
        assert!(loaded.records[0].cwd.is_empty());
    }

    #[test]
    fn handles_multiple_active_stale_and_malformed_instances() {
        let temp = TempDir::new().unwrap();
        let repo_a = temp.path().join("a");
        let repo_b = temp.path().join("b");
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();
        let now = 2_000_000;
        write_json(
            &temp.path().join("instances/a.json"),
            instance("a", &repo_a, now - 100, false),
        );
        write_json(
            &temp.path().join("instances/b.json"),
            instance("b", &repo_b, now - ACTIVE_TTL_MS - 1, false),
        );
        fs::write(temp.path().join("instances/broken.json"), b"{not json").unwrap();

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 2);
        assert!(snapshot
            .workspaces
            .iter()
            .any(|item| item.instance_id == "a" && item.active));
        assert!(snapshot
            .workspaces
            .iter()
            .any(|item| item.instance_id == "b" && !item.active));
        let diagnostics = store.diagnostics(
            now,
            "code".to_string(),
            "antigravity-ide".to_string(),
            "cursor".to_string(),
        );
        assert_eq!(diagnostics.valid_instance_records, 2);
        assert_eq!(diagnostics.malformed_instance_records, 1);
    }

    #[test]
    fn accepts_the_privacy_minimal_companion_v1_shape() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("minimal");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("instances/minimal.json"),
            json!({
                "schemaVersion": 1,
                "instanceId": "51adf7cb-d0ee-42a2-8d5d-dc8ef93d74f8",
                "workspaceName": "minimal",
                "workspaceFolders": [{
                    "name": "minimal",
                    "index": 0,
                    "path": repo
                }],
                "workspaceFile": null,
                "primaryPath": repo,
                "openTarget": repo,
                "focused": true,
                "active": true,
                "lastSeenAtMs": now,
                "startedAtMs": now - 1_000
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].editor, EditorKind::VsCode);
        assert_eq!(snapshot.workspaces[0].editor_name, "VS Code");
        assert!(snapshot.workspaces[0].openable);
        assert!(snapshot.workspaces[0].focused);
        let target = store
            .find_workspace_open_target("51adf7cb-d0ee-42a2-8d5d-dc8ef93d74f8", now)
            .unwrap();
        assert_eq!(target.path, repo);
        assert_eq!(
            target.editor, None,
            "legacy records use the configured default"
        );
        assert_eq!(
            store
                .diagnostics(
                    now,
                    "code".to_string(),
                    "antigravity-ide".to_string(),
                    "cursor".to_string(),
                )
                .malformed_instance_records,
            0
        );
    }

    #[test]
    fn trusted_companion_editor_is_exposed_and_selects_its_launcher_kind() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("antigravity-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("antigravity-window", &repo, now, true);
        heartbeat["editor"] = json!("antigravity_ide");
        write_json(
            &temp.path().join("instances/antigravity-window.json"),
            heartbeat,
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].editor, EditorKind::AntigravityIde);
        assert_eq!(snapshot.workspaces[0].editor_name, "Antigravity IDE");
        let serialized = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(serialized["workspaces"][0]["editor"], "antigravity_ide");
        assert_eq!(serialized["workspaces"][0]["editorName"], "Antigravity IDE");

        assert_eq!(
            store.find_workspace_open_target("antigravity-window", now),
            Some(WorkspaceOpenTarget {
                path: repo,
                editor: Some(EditorKind::AntigravityIde),
            })
        );
    }

    #[test]
    fn trusted_cursor_heartbeat_is_exposed_and_selects_its_launcher_kind() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("cursor-window", &repo, now, true);
        heartbeat["editor"] = json!("cursor");
        write_json(&temp.path().join("instances/cursor-window.json"), heartbeat);

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(workspace.editor, EditorKind::Cursor);
        assert_eq!(workspace.editor_name, "Cursor");
        assert!(workspace.openable);
        assert!(workspace.cursor.is_none());
        assert_eq!(
            store.find_workspace_open_target("cursor-window", now),
            Some(WorkspaceOpenTarget {
                path: repo,
                editor: Some(EditorKind::Cursor),
            })
        );
    }

    #[test]
    fn diagnostics_distinguish_live_cursor_heartbeats_from_workspace_hooks() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = STALE_RETENTION_MS + 10_000;

        for (id, age) in [
            ("active", ACTIVE_TTL_MS),
            ("retained", ACTIVE_TTL_MS + 1),
            ("expired", STALE_RETENTION_MS + 1),
        ] {
            let mut heartbeat = instance(id, &repo, now - age, false);
            heartbeat["editor"] = json!("cursor");
            write_json(&temp.path().join(format!("instances/{id}.json")), heartbeat);
        }
        let legacy = instance("legacy-vscode", &repo, now, false);
        write_json(&temp.path().join("instances/legacy-vscode.json"), legacy);
        write_json(
            &temp.path().join("cursor/workspace.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-workspace",
                "cwd": repo,
                "state": "workspace_opened",
                "changedAtMs": now - 123
            }),
        );

        let diagnostics = StateStore::new(temp.path().to_path_buf()).diagnostics(
            now,
            "code".to_string(),
            "antigravity-ide".to_string(),
            "cursor".to_string(),
        );
        assert_eq!(diagnostics.active_cursor_instance_records, 1);
        assert_eq!(diagnostics.retained_cursor_instance_records, 2);
        assert_eq!(
            diagnostics.latest_cursor_instance_at_ms,
            Some(now - ACTIVE_TTL_MS)
        );
        assert_eq!(diagnostics.recent_cursor_workspace_open_records, 1);
        assert_eq!(
            diagnostics.latest_cursor_workspace_opened_at_ms,
            Some(now - 123)
        );
    }

    #[test]
    fn cursor_workspace_opened_synthesizes_a_non_live_workspace_without_activity() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("cursor/workspace.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-workspace",
                "cwd": repo,
                "state": "workspace_opened",
                "changedAtMs": now
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        let workspace = &snapshot.workspaces[0];
        assert!(workspace.instance_id.starts_with("cursor-workspace:"));
        assert_eq!(workspace.editor, EditorKind::Cursor);
        assert_eq!(workspace.editor_name, "Cursor");
        assert_eq!(workspace.path.as_deref(), repo.to_str());
        assert!(!workspace.openable);
        assert!(!workspace.active);
        assert!(!workspace.focused);
        assert!(!workspace.recently_active);
        assert!(workspace.cursor.is_none());
        assert!(store
            .find_workspace_open_target(&workspace.instance_id, now)
            .is_none());
    }

    #[test]
    fn cursor_workspace_opened_reconciles_into_one_matching_heartbeat() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        let nested = repo.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("cursor-window", &repo, now, true);
        heartbeat["editor"] = json!("cursor");
        write_json(&temp.path().join("instances/cursor-window.json"), heartbeat);
        write_json(
            &temp.path().join("cursor/workspace.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-workspace",
                "cwd": nested,
                "state": "workspace_opened",
                "changedAtMs": now
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(workspace.instance_id, "cursor-window");
        assert_eq!(workspace.editor_name, "Cursor");
        assert!(workspace.openable);
        assert!(workspace.active);
        assert!(workspace.cursor.is_none());
    }

    #[test]
    fn ambiguous_cursor_workspace_opened_stays_generic_and_non_live() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        let nested = repo.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let now = 30_000;
        for instance_id in ["cursor-window-a", "cursor-window-b"] {
            let mut heartbeat = instance(instance_id, &repo, now, true);
            heartbeat["editor"] = json!("cursor");
            write_json(
                &temp.path().join(format!("instances/{instance_id}.json")),
                heartbeat,
            );
        }
        write_json(
            &temp.path().join("cursor/workspace.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-workspace",
                "cwd": nested,
                "state": "workspace_opened",
                "changedAtMs": now
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 3);
        let generic = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.instance_id.starts_with("cursor-workspace:"))
            .unwrap();
        assert_eq!(generic.editor_name, "Cursor");
        assert!(!generic.openable);
        assert!(!generic.active);
        assert!(!generic.focused);
        assert!(!generic.recently_active);
        assert!(generic.cursor.is_none());
    }

    #[test]
    fn cursor_same_session_completion_overrides_prior_activity_and_metadata() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("cursor-window", &repo, now, true);
        heartbeat["editor"] = json!("cursor");
        write_json(&temp.path().join("instances/cursor-window.json"), heartbeat);
        write_json(
            &temp.path().join("cursor/older-activity.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "same-agent",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now - 100,
                "modelName": "claude-4-sonnet",
                "agentKind": "Background agent"
            }),
        );
        write_json(
            &temp.path().join("cursor/newer-finished.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "same-agent",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now - 10,
                "modelName": "gpt-5.2",
                "agentKind": "Agent"
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        let cursor = snapshot.workspaces[0].cursor.as_ref().unwrap();
        assert_eq!(cursor.state, "turn_finished");
        assert_eq!(cursor.changed_at_ms, Some(now - 10));
        assert_eq!(cursor.model_name.as_deref(), Some("gpt-5.2"));
        assert_eq!(cursor.agent_kind.as_deref(), Some("Agent"));
        assert_eq!(cursor.model_kind, None);
        let serialized = serde_json::to_value(cursor).unwrap();
        assert_eq!(serialized["modelName"], "gpt-5.2");
        assert_eq!(serialized["agentKind"], "Agent");
    }

    #[test]
    fn cursor_concurrent_active_session_survives_another_sessions_completion() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("cursor-window", &repo, now, true);
        heartbeat["editor"] = json!("cursor");
        write_json(&temp.path().join("instances/cursor-window.json"), heartbeat);
        write_json(
            &temp.path().join("cursor/active.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "active-agent",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now - 100,
                "modelName": "claude-4-sonnet",
                "agentKind": "Background agent"
            }),
        );
        write_json(
            &temp.path().join("cursor/finished.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "finished-agent",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now - 10,
                "modelName": "gpt-5.2",
                "agentKind": "Agent"
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        let cursor = snapshot.workspaces[0].cursor.as_ref().unwrap();
        assert_eq!(cursor.state, "activity_detected");
        assert_eq!(cursor.changed_at_ms, Some(now - 100));
        assert_eq!(cursor.model_name.as_deref(), Some("claude-4-sonnet"));
        assert_eq!(cursor.agent_kind.as_deref(), Some("Background agent"));
    }

    #[test]
    fn cursor_hook_activity_synthesizes_a_recent_generic_agent_workspace() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-agent-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("cursor/agent.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-agent",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now,
                "modelName": "claude-4-sonnet",
                "agentKind": "Background agent"
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        let workspace = &snapshot.workspaces[0];
        assert!(workspace.instance_id.starts_with("cursor-agent:"));
        assert_eq!(workspace.editor, EditorKind::Cursor);
        assert_eq!(workspace.editor_name, "Cursor Agent");
        assert!(!workspace.openable);
        assert!(!workspace.active);
        assert!(!workspace.focused);
        assert!(workspace.recently_active);
        let cursor = workspace.cursor.as_ref().unwrap();
        assert_eq!(cursor.state, "activity_detected");
        assert_eq!(cursor.model_name.as_deref(), Some("claude-4-sonnet"));
        assert_eq!(cursor.agent_kind.as_deref(), Some("Background agent"));
        assert!(cursor.detail.contains("Cursor Agent turn-start hook"));
        assert!(store
            .find_workspace_open_target(&workspace.instance_id, now)
            .is_none());

        let diagnostics = store.diagnostics(
            now,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(diagnostics.valid_cursor_records, 1);
        assert_eq!(diagnostics.malformed_cursor_records, 0);
        assert_eq!(diagnostics.omitted_cursor_records, 0);
    }

    #[test]
    fn cursor_heartbeat_owns_nested_hook_activity_without_a_duplicate_row() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        let nested = repo.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("cursor-window", &repo, now, true);
        heartbeat["editor"] = json!("cursor");
        write_json(&temp.path().join("instances/cursor-window.json"), heartbeat);
        write_json(
            &temp.path().join("cursor/agent.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-agent",
                "cwd": nested,
                "state": "activity_detected",
                "changedAtMs": now
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].instance_id, "cursor-window");
        assert_eq!(
            snapshot.workspaces[0]
                .cursor
                .as_ref()
                .map(|activity| activity.state.as_str()),
            Some("activity_detected")
        );
    }

    #[test]
    fn cursor_activity_covered_by_multiple_windows_stays_generic_and_unopenable() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        let nested = repo.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let now = 30_000;
        for instance_id in ["cursor-window-a", "cursor-window-b"] {
            let mut heartbeat = instance(instance_id, &repo, now, true);
            heartbeat["editor"] = json!("cursor");
            write_json(
                &temp.path().join(format!("instances/{instance_id}.json")),
                heartbeat,
            );
        }
        write_json(
            &temp.path().join("cursor/agent.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-agent",
                "cwd": nested,
                "state": "activity_detected",
                "changedAtMs": now
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 3);
        for workspace in snapshot
            .workspaces
            .iter()
            .filter(|workspace| workspace.editor_name == "Cursor")
        {
            assert!(workspace.openable);
            assert!(workspace.cursor.is_none());
        }
        let generic = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.editor_name == "Cursor Agent")
            .unwrap();
        assert!(!generic.openable);
        assert!(!generic.active);
        assert!(!generic.focused);
        assert_eq!(
            generic
                .cursor
                .as_ref()
                .map(|activity| activity.state.as_str()),
            Some("activity_detected")
        );
    }

    #[test]
    fn cursor_session_started_metadata_does_not_create_workspace_activity() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("cursor-window", &repo, now, true);
        heartbeat["editor"] = json!("cursor");
        write_json(&temp.path().join("instances/cursor-window.json"), heartbeat);
        write_json(
            &temp.path().join("cursor/session.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-session",
                "cwd": repo,
                "state": "session_started",
                "changedAtMs": now,
                "agentKind": "Background agent"
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].instance_id, "cursor-window");
        assert!(snapshot.workspaces[0].cursor.is_none());
        assert_eq!(
            store
                .diagnostics(
                    now,
                    "code".into(),
                    "antigravity-ide".into(),
                    "cursor".into(),
                )
                .valid_cursor_records,
            1
        );
    }

    #[test]
    fn cursor_session_started_without_a_heartbeat_stays_hidden() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("cursor/session.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "cursor-session",
                "cwd": repo,
                "state": "session_started",
                "changedAtMs": now,
                "agentKind": "Background agent"
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        assert!(store.snapshot(now).workspaces.is_empty());
        assert_eq!(
            store
                .diagnostics(
                    now,
                    "code".into(),
                    "antigravity-ide".into(),
                    "cursor".into(),
                )
                .valid_cursor_records,
            1
        );
    }

    #[test]
    fn malformed_cursor_metadata_is_omitted_without_exposing_extra_fields() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("cursor/sanitized.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "sanitized",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now,
                "modelName": " \n\t ",
                "agentKind": "x".repeat(MAX_CURSOR_METADATA_BYTES + 1),
                "prompt": "SECRET PROMPT",
                "transcript": "SECRET TRANSCRIPT"
            }),
        );
        write_json(
            &temp.path().join("cursor/wrong-type.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "wrong-type",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now,
                "modelName": {"unexpected": true}
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        let cursor = snapshot.workspaces[0].cursor.as_ref().unwrap();
        assert_eq!(cursor.state, "turn_finished");
        assert_eq!(cursor.model_name, None);
        assert_eq!(cursor.agent_kind, None);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("modelName"));
        assert!(!serialized.contains("agentKind"));
        assert!(!serialized.contains("SECRET"));

        let diagnostics = store.diagnostics(
            now,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(diagnostics.valid_cursor_records, 1);
        assert_eq!(diagnostics.malformed_cursor_records, 1);
    }

    #[test]
    fn cursor_metadata_is_limited_to_public_display_values() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("cursor-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("cursor/private.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "private",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now,
                "modelName": "private<script>",
                "agentKind": "Private orchestrator"
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        let cursor = snapshot.workspaces[0].cursor.as_ref().unwrap();
        assert_eq!(cursor.model_name, None);
        assert_eq!(cursor.agent_kind, None);
        let diagnostics = StateStore::new(temp.path().to_path_buf()).diagnostics(
            now,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(diagnostics.valid_cursor_records, 1);
        assert_eq!(diagnostics.malformed_cursor_records, 0);
    }

    #[test]
    fn missing_provider_copy_names_antigravity_ide_instead_of_vs_code() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("antigravity-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("antigravity-window", &repo, now, true);
        heartbeat["editor"] = json!("antigravity_ide");
        heartbeat["agentExtensions"] = json!({
            "codex": {"available": true, "installed": false, "active": false, "remote": null},
            "claude": {"available": true, "installed": false, "active": false, "remote": null}
        });
        write_json(
            &temp.path().join("instances/antigravity-window.json"),
            heartbeat,
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert!(snapshot.workspaces[0]
            .codex
            .detail
            .contains("Antigravity IDE window"));
        assert!(!snapshot.workspaces[0]
            .codex
            .detail
            .contains("VS Code window"));
    }

    #[test]
    fn antigravity_hook_activity_synthesizes_a_recent_non_openable_workspace() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("antigravity-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("antigravity/conversation-project.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "hashed-conversation",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now - 10,
                "modelKind": "gemini_3_6_flash_medium"
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(workspace.editor, EditorKind::Antigravity2);
        assert_eq!(workspace.editor_name, "Antigravity 2.0");
        assert_eq!(
            workspace.path.as_deref(),
            Some(repo.to_string_lossy().as_ref())
        );
        assert!(!workspace.openable);
        assert!(!workspace.active);
        assert!(!workspace.focused);
        assert!(workspace.recently_active);
        let antigravity = workspace.antigravity.as_ref().unwrap();
        assert_eq!(antigravity.state, "turn_finished");
        assert_eq!(
            antigravity.model_kind,
            Some(AntigravityModelKind::Gemini36FlashMedium)
        );
        let serialized = serde_json::to_value(workspace).unwrap();
        assert_eq!(
            serialized["antigravity"]["modelKind"],
            "gemini_3_6_flash_medium"
        );
        assert!(store
            .find_workspace_open_target(&workspace.instance_id, now)
            .is_none());

        let diagnostics = store.diagnostics(
            now,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(diagnostics.valid_antigravity_records, 1);
        assert_eq!(diagnostics.malformed_antigravity_records, 0);
        assert_eq!(diagnostics.omitted_antigravity_records, 0);
    }

    #[test]
    fn antigravity_ide_heartbeat_owns_matching_hook_activity_without_a_duplicate_row() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("shared-antigravity-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let mut heartbeat = instance("antigravity-ide-window", &repo, now, true);
        heartbeat["editor"] = json!("antigravity_ide");
        write_json(
            &temp.path().join("instances/antigravity-ide-window.json"),
            heartbeat,
        );
        write_json(
            &temp
                .path()
                .join("antigravity-ide/conversation-project.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "hashed-conversation",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now,
                "modelKind": "claude_sonnet_4_6_thinking"
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].editor, EditorKind::AntigravityIde);
        assert_eq!(
            snapshot.workspaces[0]
                .antigravity
                .as_ref()
                .map(|activity| activity.state.as_str()),
            Some("activity_detected")
        );
        assert_eq!(
            snapshot.workspaces[0]
                .antigravity
                .as_ref()
                .and_then(|activity| activity.model_kind),
            Some(AntigravityModelKind::ClaudeSonnet46Thinking)
        );
    }

    #[test]
    fn antigravity_ide_snapshot_reconciles_each_turn_from_execution_metadata() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let conversations = temp.path().join("conversations");
        let repo = temp.path().join("model-switch-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let conversation_id = "403467f7-041a-420e-84cb-87da2ba51959";
        let session_key = "d6c372f338b5498e87ad5de82285727934ecc5db005e1aea4e5ae308f6f8555e";
        write_json(
            &state_root.join("antigravity-ide/conversation.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": session_key,
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now,
                "modelKind": "gemini_3_6_flash_medium"
            }),
        );
        let conversation_database = conversations.join(format!("{conversation_id}.db"));
        write_test_ide_executor_model(&conversation_database, 0, "claude-sonnet-4-6");
        let store =
            StateStore::new_with_antigravity_ide_conversations(state_root, conversations.clone());

        let first = store.snapshot(now);
        assert_eq!(
            first.workspaces[0]
                .antigravity
                .as_ref()
                .map(|activity| activity.state.as_str()),
            Some("activity_detected")
        );
        assert_eq!(
            first.workspaces[0]
                .antigravity
                .as_ref()
                .and_then(|activity| activity.model_kind),
            Some(AntigravityModelKind::ClaudeSonnet46Thinking)
        );
        let claude_revision = crate::antigravity_integration::antigravity_ide_execution_models(
            &conversations,
            &BTreeSet::from([session_key.to_string()]),
        )
        .remove(session_key)
        .unwrap()
        .revision;
        assert_eq!(claude_revision.len(), 64);

        // A record already associated with this executor revision must retain
        // its model while the next row is still pending.
        write_json(
            &store.root.join("antigravity-ide/conversation.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": session_key,
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now,
                "modelKind": "gpt_oss_120b_medium",
                "ideModelRevision": &claude_revision
            }),
        );
        let pending = store.snapshot(now);
        assert_eq!(
            pending.workspaces[0]
                .antigravity
                .as_ref()
                .and_then(|activity| activity.model_kind),
            Some(AntigravityModelKind::GptOss120bMedium)
        );

        write_json(
            &store.root.join("antigravity-ide/conversation.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": session_key,
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now,
                "modelKind": "gpt_oss_120b_medium",
                "ideModelRevision": &claude_revision
            }),
        );
        write_test_ide_executor_model(&conversation_database, 1, "gpt-oss-120b-medium");
        let switched = store.snapshot(now);
        assert_eq!(
            switched.workspaces[0]
                .antigravity
                .as_ref()
                .map(|activity| activity.state.as_str()),
            Some("turn_finished")
        );
        assert_eq!(
            switched.workspaces[0]
                .antigravity
                .as_ref()
                .and_then(|activity| activity.model_kind),
            Some(AntigravityModelKind::GptOss120bMedium)
        );
    }

    #[test]
    fn antigravity_ide_snapshot_waits_for_the_hook_before_adopting_a_turn_model() {
        let temp = TempDir::new().unwrap();
        let state_root = temp.path().join("state");
        let conversations = temp.path().join("conversations");
        let repo = temp.path().join("model-switch-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        let conversation_id = "403467f7-041a-420e-84cb-87da2ba51959";
        let session_key = "d6c372f338b5498e87ad5de82285727934ecc5db005e1aea4e5ae308f6f8555e";
        write_json(
            &state_root.join("antigravity-ide/conversation.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": session_key,
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now,
                "modelKind": "gemini_3_6_flash_medium",
                "ideModelRevision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }),
        );
        let conversation_database = conversations.join(format!("{conversation_id}.db"));
        write_test_ide_turn_model(&conversation_database, 1, 1035);
        let store =
            StateStore::new_with_antigravity_ide_conversations(state_root, conversations.clone());

        // The USER_INPUT row precedes PreInvocation by a few milliseconds.
        // A background refresh in that interval must keep the prior terminal
        // label rather than assigning Claude to a turn that has not started.
        let before_hook = store.snapshot(now);
        let before_hook_activity = before_hook.workspaces[0].antigravity.as_ref().unwrap();
        assert_eq!(before_hook_activity.state, "turn_finished");
        assert_eq!(
            before_hook_activity.model_kind,
            Some(AntigravityModelKind::Gemini36FlashMedium)
        );

        let current = crate::antigravity_integration::antigravity_ide_execution_models(
            &conversations,
            &BTreeSet::from([session_key.to_string()]),
        )
        .remove(session_key)
        .unwrap();
        assert_eq!(
            current.source,
            crate::antigravity_integration::IdeModelSource::CurrentTurn
        );
        write_json(
            &store.root.join("antigravity-ide/conversation.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": session_key,
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now + 1,
                "modelKind": "claude_sonnet_4_6_thinking",
                "ideModelRevision": current.revision
            }),
        );
        let after_hook = store.snapshot(now + 1);
        let after_hook_activity = after_hook.workspaces[0].antigravity.as_ref().unwrap();
        assert_eq!(after_hook_activity.state, "activity_detected");
        assert_eq!(
            after_hook_activity.model_kind,
            Some(AntigravityModelKind::ClaudeSonnet46Thinking)
        );
    }

    #[test]
    fn antigravity_uses_the_newest_lifecycle_state_and_its_model() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("model-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("antigravity/finished.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "finished",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now - 10,
                "modelKind": "claude_sonnet_4_6_thinking"
            }),
        );
        write_json(
            &temp.path().join("antigravity/activity.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "activity",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now - 100,
                "modelKind": "gemini_3_1_pro_high"
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        let activity = snapshot.workspaces[0].antigravity.as_ref().unwrap();
        assert_eq!(activity.state, "turn_finished");
        assert_eq!(activity.changed_at_ms, Some(now - 10));
        assert_eq!(
            activity.model_kind,
            Some(AntigravityModelKind::ClaudeSonnet46Thinking)
        );
    }

    #[test]
    fn unknown_future_antigravity_model_token_is_omitted() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("future-model-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("antigravity/future.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "future",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now,
                "modelKind": "private_future_model"
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        let activity = snapshot.workspaces[0].antigravity.as_ref().unwrap();
        assert_eq!(activity.model_kind, None);
        let serialized = serde_json::to_value(activity).unwrap();
        assert!(serialized.get("modelKind").is_none());
    }

    #[test]
    fn ide_hook_without_a_heartbeat_stays_identified_as_antigravity_ide() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("ide-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        write_json(
            &temp.path().join("antigravity-ide/conversation.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "ide-conversation",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(workspace.editor, EditorKind::AntigravityIde);
        assert_eq!(workspace.editor_name, "Antigravity IDE");
        assert!(!workspace.openable);
        assert!(workspace.recently_active);
        assert_eq!(
            workspace
                .antigravity
                .as_ref()
                .and_then(|activity| activity.model_kind),
            None,
            "IDE events without modelName remain generic Antigravity activity"
        );
    }

    #[test]
    fn stale_antigravity_ide_heartbeat_does_not_hide_recent_shared_hook_activity() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("reopened-in-antigravity");
        fs::create_dir_all(&repo).unwrap();
        let now = 2_000_000;
        let mut heartbeat = instance(
            "old-antigravity-ide-window",
            &repo,
            now - STALE_RETENTION_MS - 1,
            false,
        );
        heartbeat["editor"] = json!("antigravity_ide");
        write_json(
            &temp
                .path()
                .join("instances/old-antigravity-ide-window.json"),
            heartbeat,
        );
        write_json(
            &temp.path().join("antigravity/current.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "current-conversation",
                "cwd": repo,
                "state": "activity_detected",
                "changedAtMs": now
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].editor, EditorKind::Antigravity2);
    }

    #[test]
    fn stale_or_malformed_antigravity_activity_does_not_create_a_workspace() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("old-project");
        fs::create_dir_all(&repo).unwrap();
        let now = ACTIVITY_STALE_MS + 10_000;
        write_json(
            &temp.path().join("antigravity/old.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "old-conversation",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": 1
            }),
        );
        fs::write(temp.path().join("antigravity/broken.json"), b"{not json").unwrap();

        let store = StateStore::new(temp.path().to_path_buf());
        assert!(store.snapshot(now).workspaces.is_empty());
        let diagnostics = store.diagnostics(
            now,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(diagnostics.valid_antigravity_records, 1);
        assert_eq!(diagnostics.malformed_antigravity_records, 1);
    }

    #[test]
    fn heartbeat_editor_is_a_closed_set_and_cannot_supply_a_launcher() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        for (id, editor) in [
            ("standalone-antigravity", "antigravity_2"),
            ("untrusted", "/tmp/untrusted-editor"),
        ] {
            let mut heartbeat = instance(id, &repo, now, false);
            heartbeat["editor"] = json!(editor);
            write_json(&temp.path().join(format!("instances/{id}.json")), heartbeat);
        }

        let store = StateStore::new(temp.path().to_path_buf());
        assert!(store.snapshot(now).workspaces.is_empty());
        assert!(store
            .find_workspace_open_target("standalone-antigravity", now)
            .is_none());
        assert!(store.find_workspace_open_target("untrusted", now).is_none());
        assert_eq!(
            store
                .diagnostics(
                    now,
                    "code".into(),
                    "antigravity-ide".into(),
                    "cursor".into(),
                )
                .malformed_instance_records,
            2
        );
    }

    #[test]
    fn same_path_from_different_companion_editors_remains_two_workspaces() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("shared-project");
        fs::create_dir_all(&repo).unwrap();
        let now = 30_000;
        for (id, editor) in [
            ("vscode-window", "vscode"),
            ("antigravity-window", "antigravity_ide"),
        ] {
            let mut heartbeat = instance(id, &repo, now, false);
            heartbeat["editor"] = json!(editor);
            write_json(&temp.path().join(format!("instances/{id}.json")), heartbeat);
        }

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        assert_eq!(snapshot.workspaces.len(), 2);
        assert!(snapshot
            .workspaces
            .iter()
            .any(|workspace| workspace.editor == EditorKind::VsCode));
        assert!(snapshot
            .workspaces
            .iter()
            .any(|workspace| workspace.editor == EditorKind::AntigravityIde));
        assert_eq!(
            store
                .find_workspace_open_target("vscode-window", now)
                .unwrap()
                .editor,
            Some(EditorKind::VsCode)
        );
        assert_eq!(
            store
                .find_workspace_open_target("antigravity-window", now)
                .unwrap()
                .editor,
            Some(EditorKind::AntigravityIde)
        );
    }

    #[test]
    fn drops_instances_after_retention_window() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 2_000_000;
        write_json(
            &temp.path().join("instances/old.json"),
            instance("old", &repo, now - STALE_RETENTION_MS - 1, false),
        );
        assert!(StateStore::new(temp.path().to_path_buf())
            .snapshot(now)
            .workspaces
            .is_empty());
    }

    #[test]
    fn associates_nested_codex_cwd_and_prefers_unfinished_activity() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let nested = repo.join("src");
        fs::create_dir_all(&nested).unwrap();
        let now = 2_000_000;
        write_json(
            &temp.path().join("instances/repo.json"),
            instance("repo", &repo, now, false),
        );
        write_json(
            &temp.path().join("codex/finished.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "finished",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now - 10
            }),
        );
        write_json(
            &temp.path().join("codex/running.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "running",
                "cwd": nested,
                "state": "activity_detected",
                "changedAtMs": now - 100
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces[0].codex.state, "activity_detected");
        assert_eq!(snapshot.workspaces[0].codex.changed_at_ms, Some(now - 100));
    }

    #[test]
    fn codex_parent_cwd_is_not_workspace_activity_and_explains_first_signal() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 2_000_000;
        write_json(
            &temp.path().join("instances/repo.json"),
            instance("repo", &repo, now, false),
        );
        write_json(
            &temp.path().join("codex/parent.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "parent",
                "cwd": temp.path(),
                "state": "activity_detected",
                "changedAtMs": now - 10
            }),
        );

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        let codex = &snapshot.workspaces[0].codex;
        assert_eq!(codex.state, "unknown");
        assert_eq!(codex.label, "No activity yet");
        assert!(codex
            .detail
            .contains("Start Codex in this workspace and submit a prompt"));
    }

    #[test]
    fn pathless_workspace_explains_that_activity_cannot_be_associated() {
        let view = aggregate_activity("Codex", &[], &[], None, None, false, 20_000);
        assert_eq!(view.state, "unknown");
        assert_eq!(view.label, "Workspace path needed");
        assert!(view
            .detail
            .contains("Open a local folder or saved workspace"));
        assert!(!view.detail.contains("submit a prompt"));
    }

    #[test]
    fn pathless_remote_workspace_explains_the_host_boundary() {
        let view = aggregate_activity("Claude Code", &[], &[], None, None, true, 20_000);
        assert_eq!(view.state, "unknown");
        assert_eq!(view.label, "Remote workspace");
        assert!(view.detail.contains("Remote workspace paths are omitted"));
        assert!(view.detail.contains("no remote bridge"));
        assert!(!view.detail.contains("Open a local folder"));
    }

    #[test]
    fn marks_old_codex_signal_unknown() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = ACTIVITY_STALE_MS + 10_000;
        write_json(
            &temp.path().join("instances/repo.json"),
            instance("repo", &repo, now, false),
        );
        write_json(
            &temp.path().join("codex/old.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "old",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": 1
            }),
        );
        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces[0].codex.state, "unknown");
        assert_eq!(snapshot.workspaces[0].codex.label, "Unknown");
    }

    #[test]
    fn associates_claude_failure_and_exposes_public_extension_presence() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 2_000_000;
        let mut heartbeat = instance("repo", &repo, now, true);
        heartbeat["remoteWindow"] = json!(true);
        heartbeat["agentExtensions"] = json!({
            "codex": {"available": true, "installed": true, "active": false, "remote": false},
            "claude": {"available": true, "installed": true, "active": true, "remote": true}
        });
        write_json(&temp.path().join("instances/repo.json"), heartbeat);
        write_json(
            &temp.path().join("claude/failure.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "failure",
                "cwd": repo,
                "state": "failed_or_interrupted",
                "changedAtMs": now - 10
            }),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        let snapshot = store.snapshot(now);
        let workspace = &snapshot.workspaces[0];
        assert_eq!(workspace.claude.state, "failed_or_interrupted");
        assert_eq!(workspace.claude.extension_detection_available, Some(true));
        assert_eq!(workspace.claude.extension_installed, Some(true));
        assert_eq!(workspace.claude.extension_active, Some(true));
        assert_eq!(workspace.claude.extension_remote, Some(true));
        assert!(workspace.remote_window);
        assert_eq!(workspace.codex.state, "unknown");
        assert_eq!(workspace.codex.extension_installed, Some(true));
        assert_eq!(workspace.codex.extension_active, Some(false));
        assert_eq!(workspace.codex.extension_remote, Some(false));
        assert_eq!(
            store
                .diagnostics(
                    now,
                    "code".into(),
                    "antigravity-ide".into(),
                    "cursor".into(),
                )
                .valid_claude_records,
            1
        );
    }

    #[test]
    fn missing_or_failed_extension_detection_remains_unknown() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 20_000;
        let mut heartbeat = instance("repo", &repo, now, false);
        heartbeat["agentExtensions"] = json!({
            "codex": {"available": false, "installed": false, "active": false},
            "claude": {"available": true, "installed": false, "active": false}
        });
        write_json(&temp.path().join("instances/repo.json"), heartbeat);

        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert_eq!(snapshot.workspaces[0].codex.state, "unknown");
        assert_eq!(
            snapshot.workspaces[0].codex.extension_detection_available,
            Some(false)
        );
        assert_eq!(snapshot.workspaces[0].claude.state, "unknown");
        assert_eq!(
            snapshot.workspaces[0].claude.extension_installed,
            Some(false)
        );
    }

    #[test]
    fn malformed_claude_record_isolated_from_valid_workspaces() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 20_000;
        write_json(
            &temp.path().join("instances/repo.json"),
            instance("repo", &repo, now, false),
        );
        fs::create_dir_all(temp.path().join("claude")).unwrap();
        fs::write(temp.path().join("claude/broken.json"), b"{not json").unwrap();

        let store = StateStore::new(temp.path().to_path_buf());
        assert_eq!(store.snapshot(now).workspaces.len(), 1);
        let diagnostics = store.diagnostics(
            now,
            "code".into(),
            "antigravity-ide".into(),
            "cursor".into(),
        );
        assert_eq!(diagnostics.malformed_claude_records, 1);
        assert_eq!(diagnostics.valid_claude_records, 0);
    }

    #[test]
    fn malformed_or_missing_state_never_breaks_snapshot() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("instances")).unwrap();
        fs::write(temp.path().join("instances/empty.json"), b"").unwrap();
        write_json(
            &temp.path().join("instances/wrong-schema.json"),
            json!({"schemaVersion": 99}),
        );
        let store = StateStore::new(temp.path().to_path_buf());
        assert!(store.snapshot(123).workspaces.is_empty());
        assert_eq!(
            store
                .diagnostics(
                    123,
                    "code".into(),
                    "antigravity-ide".into(),
                    "cursor".into(),
                )
                .malformed_instance_records,
            2
        );
    }

    #[test]
    fn output_preserves_privacy_boundary_even_with_extra_sensitive_input() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 20_000;
        write_json(
            &temp.path().join("instances/repo.json"),
            instance("repo", &repo, now, false),
        );
        write_json(
            &temp.path().join("codex/privacy.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "privacy",
                "cwd": repo,
                "state": "turn_finished",
                "changedAtMs": now,
                "prompt": "SECRET PROMPT",
                "lastAssistantMessage": "SECRET ANSWER",
                "transcriptPath": "/secret/transcript",
                "sourceCode": "private source"
            }),
        );
        write_json(
            &temp.path().join("claude/privacy.json"),
            json!({
                "schemaVersion": 1,
                "sessionKey": "claude-privacy",
                "cwd": repo,
                "state": "failed_or_interrupted",
                "changedAtMs": now,
                "prompt": "CLAUDE SECRET PROMPT",
                "lastAssistantMessage": "CLAUDE SECRET ANSWER",
                "transcriptPath": "/secret/claude-transcript",
                "errorDetails": "private failure details"
            }),
        );
        let serialized =
            serde_json::to_string(&StateStore::new(temp.path().to_path_buf()).snapshot(now))
                .unwrap();
        for forbidden in ["SECRET", "transcript", "sourceCode", "lastAssistantMessage"] {
            assert!(!serialized.contains(forbidden));
        }
        assert!(serialized.contains("turn_finished"));
        assert!(serialized.contains("failed_or_interrupted"));
    }

    #[test]
    fn open_target_and_editor_are_resolved_from_trusted_record_not_ui_path() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let missing_repo = temp.path().join("missing-repo");
        fs::create_dir_all(&repo).unwrap();
        let now = 20_000;
        write_json(
            &temp.path().join("instances/repo.json"),
            instance("repo", &repo, now, false),
        );
        write_json(
            &temp.path().join("instances/missing.json"),
            instance("missing", &missing_repo, now, false),
        );
        assert_eq!(
            StateStore::new(temp.path().to_path_buf()).find_workspace_open_target("repo", now),
            Some(WorkspaceOpenTarget {
                path: repo,
                editor: None,
            })
        );
        assert!(StateStore::new(temp.path().to_path_buf())
            .find_workspace_open_target("unknown", now)
            .is_none());
        let snapshot = StateStore::new(temp.path().to_path_buf()).snapshot(now);
        assert!(
            !snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.instance_id == "missing")
                .unwrap()
                .openable
        );
        assert!(StateStore::new(temp.path().to_path_buf())
            .find_workspace_open_target("missing", now)
            .is_none());
    }

    #[test]
    fn quick_switch_target_requires_a_current_workspace_heartbeat() {
        let temp = TempDir::new().unwrap();
        let active_repo = temp.path().join("active");
        let retained_repo = temp.path().join("retained");
        fs::create_dir_all(&active_repo).unwrap();
        fs::create_dir_all(&retained_repo).unwrap();
        let now = 2_000_000;
        write_json(
            &temp.path().join("instances/active.json"),
            instance("active", &active_repo, now - ACTIVE_TTL_MS, false),
        );
        write_json(
            &temp.path().join("instances/retained.json"),
            instance("retained", &retained_repo, now - ACTIVE_TTL_MS - 1, false),
        );

        let store = StateStore::new(temp.path().to_path_buf());
        assert_eq!(
            store.find_active_workspace_open_target("active", now),
            Some(WorkspaceOpenTarget {
                path: active_repo,
                editor: None,
            })
        );
        assert!(store
            .find_active_workspace_open_target("retained", now)
            .is_none());
        assert_eq!(
            store.find_workspace_open_target("retained", now),
            Some(WorkspaceOpenTarget {
                path: retained_repo,
                editor: None,
            })
        );
    }
}
