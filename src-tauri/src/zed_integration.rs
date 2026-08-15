//! Bounded, read-only observation of Zed's native workspace metadata.
//!
//! Zed does not host VS Code extensions, so its workspace presence cannot use
//! VSParallel's companion heartbeat protocol. Zed does, however, persist a
//! small amount of suitable metadata in each release channel's `db.sqlite`.
//! This module reads only the workspace/session/window-stack fields needed to
//! distinguish recent rows from conservatively live windows, plus coarse
//! thread-sidebar metadata. It never returns thread titles, thread IDs, session
//! IDs, prompts, responses, or tool content.
//!
//! Integration contract for `state.rs`:
//!
//! * Database state alone is deliberately never considered proof that a window
//!   is open because Zed retains session bindings at exit. The production
//!   loader combines it with a separately established Zed process signal.
//! * `ZedWorkspaceObservation::paths` preserves Zed's display/open order and
//!   may contain multiple roots. `open_target` is present only when that order
//!   was decoded exactly and all paths were safe local absolute paths. Polling
//!   never probes whether those paths still exist; callers validate immediately
//!   before opening and must not silently reduce a multi-root project.
//! * `window_stack_index` is useful only for deterministic ordering. Zed's
//!   persisted window IDs are application-internal, so this module does not
//!   expose them or infer native-window focus from them.
//! * Native Zed Agent lifecycle is a coarse persisted turn-boundary signal,
//!   not exact process telemetry. The thread database does not serialize its
//!   in-memory running state or stop reason. A trailing user boundary indicates
//!   a submitted turn; a trailing assistant boundary indicates that Zed flushed
//!   the response. Unknown structures remain recency-only.
//! * Lifecycle and model fields are populated only after a bounded
//!   sidebar/session join and selective JSON or zstd thread-data parse. The
//!   parser retains only message variant names, the presence of a tool-use
//!   boundary, the bounded provider/model pair, and Zed's four cumulative
//!   token counters. It discards all decoded message and tool content.

use rusqlite::{Connection, OpenFlags};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::{Command, Stdio};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::sync::mpsc::{self, TryRecvError};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::thread;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::time::Instant;

#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

pub(crate) const ZED_DATA_DIR_ENV: &str = "VSPARALLEL_ZED_DATA_DIR";

const MAX_DATA_ROOTS: usize = 4;
const MAX_CHANNELS: usize = 8;
const MAX_WORKSPACES_PER_CHANNEL: usize = 256;
const MAX_THREADS_PER_CHANNEL: usize = 512;
const MAX_PATHS: usize = 64;
const MAX_PATH_LIST_BYTES: usize = 32 * 1024;
const MAX_PATH_ORDER_BYTES: usize = 2 * 1024;
const MAX_SINGLE_PATH_BYTES: usize = 16 * 1024;
const MAX_SESSION_ID_BYTES: usize = 128;
const MAX_WINDOW_STACK_BYTES: usize = 8 * 1024;
const MAX_WINDOW_STACK_ENTRIES: usize = 128;
const MAX_AGENT_ID_BYTES: usize = 128;
const MAX_THREAD_JOIN_ID_BYTES: usize = 256;
const MAX_THREAD_DATA_TYPE_BYTES: usize = 16;
const MAX_MODEL_VALUE_BYTES: usize = 128;
const MAX_TIMESTAMP_BYTES: usize = 64;
const MAX_DATABASE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DATABASE_WAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DATABASE_SHM_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_millis(100);
const MAX_THREAD_DATABASE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_THREAD_BLOB_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODEL_ROWS_PER_REFRESH: usize = 4;
const MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH: usize = 16 * 1024 * 1024;
#[cfg(target_os = "linux")]
const MAX_PROCESS_ENTRIES: usize = 65_536;
#[cfg(target_os = "linux")]
const MAX_PROCESS_NAME_BYTES: u64 = 128;
#[cfg(any(target_os = "macos", target_os = "windows"))]
const MAX_PROCESS_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;
#[cfg(any(target_os = "macos", target_os = "windows"))]
const PROCESS_COMMAND_TIMEOUT: Duration = Duration::from_secs(1);
const KNOWN_CHANNELS: [&str; 4] = ["0-stable", "0-preview", "0-nightly", "0-dev"];

/// Native Zed observations ready to be merged into the application's state
/// model. This type contains no opaque Zed identifiers or conversation text.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZedSnapshot {
    pub(crate) workspaces: Vec<ZedWorkspaceObservation>,
    pub(crate) diagnostics: ZedDiagnostics,
}

/// One local Zed workspace, either conservatively open or retained as recent.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZedWorkspaceObservation {
    /// Stable SHA-256 pseudonym of the channel and path set. This is suitable
    /// for state/open-target lookup without exposing Zed's session/window IDs.
    pub(crate) instance_id: String,
    /// A bounded release-channel token such as `0-stable`.
    pub(crate) channel: String,
    /// Workspace roots in Zed's original display/open order.
    pub(crate) paths: Vec<PathBuf>,
    /// Exact paths suitable for validation immediately before a Zed CLI
    /// invocation, when reconstruction was lossless and paths were local.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) open_target: Option<Vec<PathBuf>>,
    /// True only with process liveness, a current-session match, and a window
    /// present in the current persisted stack.
    pub(crate) open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_active_at_ms: Option<i64>,
    /// Persisted stack index for ordering only; it does not assert focus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) window_stack_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<ZedAgentActivity>,
}

/// A coarse native Zed Agent turn boundary derived from persisted structure.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ZedAgentLifecycle {
    ActivityDetected,
    TurnFinished,
}

/// Latest non-archived local thread metadata associated with a workspace.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZedAgentActivity {
    /// `zed` for the native Zed agent, a bounded ACP identifier when safe, or
    /// `external` when an external identifier could not be represented safely.
    pub(crate) agent_kind: String,
    pub(crate) changed_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interacted_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) lifecycle: Option<ZedAgentLifecycle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) lifecycle_changed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model_name: Option<String>,
}

/// Cumulative token usage for the newest valid local native Zed thread.
///
/// Zed's upstream `TokenUsage::total_tokens()` adds regular input, output,
/// cache-creation input, and cache-read input tokens. We mirror that definition
/// exactly, while using checked addition so corrupt persisted values fail
/// closed instead of wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ZedUsageObservation {
    pub(crate) total_tokens: u64,
    pub(crate) updated_at_ms: i64,
}

/// Aggregate, non-sensitive adapter health counters.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ZedDiagnostics {
    pub(crate) data_roots_considered: usize,
    pub(crate) omitted_data_roots: usize,
    pub(crate) channel_candidates: usize,
    pub(crate) channels_loaded: usize,
    pub(crate) malformed_channels: usize,
    pub(crate) omitted_channels: usize,
    pub(crate) agent_metadata_channels: usize,
    pub(crate) malformed_records: usize,
    pub(crate) omitted_workspaces: usize,
    pub(crate) omitted_threads: usize,
    pub(crate) model_rows_considered: usize,
    pub(crate) models_loaded: usize,
    pub(crate) malformed_model_rows: usize,
    pub(crate) compressed_model_rows: usize,
    /// More than one channel looked internally live while the supplied process
    /// signal could not identify which channel owns that process. All such rows
    /// are classified recent rather than risking false open state.
    pub(crate) ambiguous_live_channels: usize,
}

#[derive(Debug, Clone)]
struct ChannelDatabase {
    channel: String,
    database: PathBuf,
    data_root: PathBuf,
}

#[derive(Debug)]
struct ChannelSnapshot {
    channel: String,
    current_session: Option<String>,
    window_stack: Vec<u64>,
    workspaces: Vec<InternalWorkspace>,
    activities: BTreeMap<Vec<PathBuf>, InternalAgentActivity>,
    malformed_records: usize,
    omitted_workspaces: usize,
    omitted_threads: usize,
    agent_metadata_available: bool,
    model_rows_considered: usize,
    models_loaded: usize,
    malformed_model_rows: usize,
    compressed_model_rows: usize,
}

#[derive(Debug, Clone)]
struct InternalAgentActivity {
    view: ZedAgentActivity,
    /// Used only for the bounded join against `threads.db`; never returned.
    thread_join_id: Option<String>,
}

#[derive(Debug)]
struct InternalWorkspace {
    path_key: Vec<PathBuf>,
    paths: Vec<PathBuf>,
    open_target: Option<Vec<PathBuf>>,
    last_active_at_ms: Option<i64>,
    session_id: Option<String>,
    window_id: Option<u64>,
}

#[derive(Debug)]
struct DecodedPathList {
    sorted_key: Vec<PathBuf>,
    ordered_paths: Vec<PathBuf>,
    exact_order: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotDetail {
    WorkspacesOnly,
    WithAgentMetadata,
}

#[derive(Debug)]
struct ModelReadBudget {
    rows_remaining: usize,
    decompressed_bytes_remaining: usize,
}

impl Default for ModelReadBudget {
    fn default() -> Self {
        Self {
            rows_remaining: MAX_MODEL_ROWS_PER_REFRESH,
            decompressed_bytes_remaining: MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH,
        }
    }
}

/// Production entry point: resolve the home/data-root environment and perform
/// the platform process probe in this module so state integration cannot
/// accidentally treat retained database state as liveness.
pub(crate) fn load_zed_snapshot_from_environment(now_ms: i64) -> ZedSnapshot {
    load_zed_snapshot_from_environment_with_detail(now_ms, SnapshotDetail::WithAgentMetadata)
}

/// Load the newest valid cumulative token count from a local native Zed Agent
/// thread. This performs the same bounded, read-only sidebar/session join and
/// selective thread parse as the workspace adapter, without requiring Zed to
/// be running and without returning any thread identifier or content.
pub(crate) fn load_zed_usage_from_environment(now_ms: i64) -> Option<ZedUsageObservation> {
    let roots = environment_data_roots();
    load_zed_usage_from_data_roots(&roots, now_ms)
}

/// Lightweight production loader for resolving an open target. It reads only
/// workspace/session metadata and deliberately skips sidebar and thread blobs.
pub(crate) fn load_zed_workspace_snapshot_from_environment(now_ms: i64) -> ZedSnapshot {
    load_zed_snapshot_from_environment_with_detail(now_ms, SnapshotDetail::WorkspacesOnly)
}

fn load_zed_snapshot_from_environment_with_detail(
    now_ms: i64,
    detail: SnapshotDetail,
) -> ZedSnapshot {
    let roots = environment_data_roots();
    load_zed_snapshot_from_data_roots_with_detail(&roots, now_ms, zed_process_is_live(), detail)
}

fn environment_data_roots() -> Vec<PathBuf> {
    if let Some(value) = env::var_os(ZED_DATA_DIR_ENV) {
        let path = PathBuf::from(value);
        return (!path.as_os_str().is_empty() && path.is_absolute())
            .then_some(path)
            .into_iter()
            .collect();
    }
    environment_home_directory()
        .map(|home| zed_data_roots(&home))
        .unwrap_or_default()
}

/// Deterministic loader for tests and callers that have already resolved an
/// application-specific data directory. `process_is_live` must come from an
/// independent probe; `false` still returns recent workspaces and agents.
#[cfg(test)]
pub(crate) fn load_zed_snapshot_from_data_roots(
    data_roots: &[PathBuf],
    now_ms: i64,
    process_is_live: bool,
) -> ZedSnapshot {
    load_zed_snapshot_from_data_roots_with_detail(
        data_roots,
        now_ms,
        process_is_live,
        SnapshotDetail::WithAgentMetadata,
    )
}

fn load_zed_usage_from_data_roots(
    data_roots: &[PathBuf],
    now_ms: i64,
) -> Option<ZedUsageObservation> {
    let bounded_roots = &data_roots[..data_roots.len().min(MAX_DATA_ROOTS)];
    let mut diagnostics = ZedDiagnostics::default();
    let databases = discover_channel_databases(bounded_roots, &mut diagnostics);
    let mut newest_by_thread = BTreeMap::new();
    for candidate in databases {
        let Ok(connection) = open_bounded_read_only_database(&candidate.database) else {
            continue;
        };
        let Ok(join_ids) = read_native_thread_join_ids(&connection, now_ms) else {
            continue;
        };
        let thread_database = candidate.data_root.join("threads").join("threads.db");
        for (join_id, updated_at_ms) in join_ids {
            newest_by_thread
                .entry((thread_database.clone(), join_id))
                .and_modify(|existing: &mut i64| *existing = (*existing).max(updated_at_ms))
                .or_insert(updated_at_ms);
        }
    }
    let mut candidates: Vec<_> = newest_by_thread.into_iter().collect();
    candidates.sort_by(
        |((left_database, left_id), left_timestamp),
         ((right_database, right_id), right_timestamp)| {
            right_timestamp
                .cmp(left_timestamp)
                .then_with(|| left_database.cmp(right_database))
                .then_with(|| left_id.cmp(right_id))
        },
    );

    // The blob-read limit is global, so choose candidates globally before any
    // channel can consume the four-row budget merely because it sorts first.
    let mut budget = ModelReadBudget::default();
    let mut newest = None;
    for ((thread_database, join_id), sidebar_updated_at_ms) in candidates {
        if budget.rows_remaining == 0 || budget.decompressed_bytes_remaining == 0 {
            break;
        }
        let (signals, _) = read_thread_signals(
            &thread_database,
            vec![(join_id.clone(), sidebar_updated_at_ms)],
            &mut budget,
            now_ms,
        );
        if let Some(observation) = signals
            .get(&join_id)
            .and_then(|(signal, updated_at_ms)| usage_observation(signal, *updated_at_ms))
        {
            if newest.is_none_or(|current: ZedUsageObservation| {
                observation.updated_at_ms > current.updated_at_ms
                    || (observation.updated_at_ms == current.updated_at_ms
                        && observation.total_tokens > current.total_tokens)
            }) {
                newest = Some(observation);
            }
        }
    }
    newest
}

fn load_zed_snapshot_from_data_roots_with_detail(
    data_roots: &[PathBuf],
    now_ms: i64,
    process_is_live: bool,
    detail: SnapshotDetail,
) -> ZedSnapshot {
    let mut diagnostics = ZedDiagnostics {
        data_roots_considered: data_roots.len().min(MAX_DATA_ROOTS),
        omitted_data_roots: data_roots.len().saturating_sub(MAX_DATA_ROOTS),
        ..ZedDiagnostics::default()
    };
    let databases = discover_channel_databases(
        &data_roots[..data_roots.len().min(MAX_DATA_ROOTS)],
        &mut diagnostics,
    );
    diagnostics.channel_candidates = databases.len();

    let mut channels = Vec::new();
    let mut model_budget = ModelReadBudget::default();
    for candidate in databases {
        match read_channel_snapshot(&candidate, now_ms, detail, &mut model_budget) {
            Ok(channel) => {
                diagnostics.channels_loaded += 1;
                diagnostics.malformed_records = diagnostics
                    .malformed_records
                    .saturating_add(channel.malformed_records);
                diagnostics.omitted_workspaces = diagnostics
                    .omitted_workspaces
                    .saturating_add(channel.omitted_workspaces);
                diagnostics.omitted_threads = diagnostics
                    .omitted_threads
                    .saturating_add(channel.omitted_threads);
                if channel.agent_metadata_available {
                    diagnostics.agent_metadata_channels += 1;
                }
                diagnostics.model_rows_considered = diagnostics
                    .model_rows_considered
                    .saturating_add(channel.model_rows_considered);
                diagnostics.models_loaded = diagnostics
                    .models_loaded
                    .saturating_add(channel.models_loaded);
                diagnostics.malformed_model_rows = diagnostics
                    .malformed_model_rows
                    .saturating_add(channel.malformed_model_rows);
                diagnostics.compressed_model_rows = diagnostics
                    .compressed_model_rows
                    .saturating_add(channel.compressed_model_rows);
                channels.push(channel);
            }
            Err(_) => diagnostics.malformed_channels += 1,
        }
    }

    // A single boolean cannot distinguish a stable process from a preview/dev
    // process. If multiple channel databases independently look live, failing
    // closed avoids reclassifying persisted rows from a stopped channel.
    let internally_live_channels: Vec<usize> = channels
        .iter()
        .enumerate()
        .filter_map(|(index, channel)| channel_has_current_window(channel).then_some(index))
        .collect();
    let live_channel = if process_is_live && internally_live_channels.len() == 1 {
        internally_live_channels
            .first()
            .copied()
            .filter(|index| channels[*index].channel == "0-stable")
    } else {
        None
    };
    if process_is_live && internally_live_channels.len() > 1 {
        diagnostics.ambiguous_live_channels = internally_live_channels.len();
    }

    let mut deduplicated: BTreeMap<(String, Vec<PathBuf>), ZedWorkspaceObservation> =
        BTreeMap::new();
    for (channel_index, channel) in channels.into_iter().enumerate() {
        for workspace in &channel.workspaces {
            let window_stack_index = if live_channel == Some(channel_index) {
                current_window_stack_index(&channel, workspace)
            } else {
                None
            };
            let open = process_is_live && window_stack_index.is_some();
            let observation = ZedWorkspaceObservation {
                instance_id: zed_instance_id(&channel.channel, &workspace.path_key),
                channel: channel.channel.clone(),
                paths: workspace.paths.clone(),
                open_target: workspace.open_target.clone(),
                open,
                last_active_at_ms: workspace.last_active_at_ms,
                window_stack_index,
                agent: channel
                    .activities
                    .get(&workspace.path_key)
                    .map(|activity| activity.view.clone()),
            };
            match deduplicated.entry((channel.channel.clone(), workspace.path_key.clone())) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(observation);
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    if prefer_workspace(&observation, entry.get()) {
                        entry.insert(observation);
                    }
                }
            }
        }
    }

    let mut workspaces: Vec<_> = deduplicated.into_values().collect();
    workspaces.sort_by(|left, right| {
        right
            .open
            .cmp(&left.open)
            .then_with(|| right.last_active_at_ms.cmp(&left.last_active_at_ms))
            .then_with(|| left.paths.cmp(&right.paths))
            .then_with(|| left.channel.cmp(&right.channel))
    });
    ZedSnapshot {
        workspaces,
        diagnostics,
    }
}

/// Resolve data roots without inspecting their contents. An explicit override
/// is authoritative and intentionally does not fall back to implicit roots if
/// it is malformed.
pub(crate) fn zed_data_roots(home: &Path) -> Vec<PathBuf> {
    zed_data_roots_with_environment(
        home,
        env::var_os(ZED_DATA_DIR_ENV),
        env::var_os("XDG_DATA_HOME"),
        env::var_os("LOCALAPPDATA"),
    )
}

fn environment_home_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let value = env::var_os("USERPROFILE").or_else(|| {
        let drive = env::var_os("HOMEDRIVE")?;
        let path = env::var_os("HOMEPATH")?;
        let mut joined = PathBuf::from(drive);
        joined.push(path);
        Some(joined.into_os_string())
    });
    #[cfg(not(target_os = "windows"))]
    let value = env::var_os("HOME");
    value
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
}

/// Independent, bounded process probe used by the production loader. It only
/// establishes that some local Zed process exists; the current database
/// session/window-stack checks still have to agree before any row is open.
pub(crate) fn zed_process_is_live() -> bool {
    platform_zed_process_is_live()
}

#[cfg(target_os = "linux")]
fn platform_zed_process_is_live() -> bool {
    let Ok(current_process) = fs::metadata("/proc/self") else {
        return false;
    };
    let current_uid = current_process.uid();
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten().take(MAX_PROCESS_ENTRIES) {
        if !entry
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        if !fs::metadata(entry.path()).is_ok_and(|metadata| metadata.uid() == current_uid) {
            continue;
        }
        let Ok(file) = fs::File::open(entry.path().join("comm")) else {
            continue;
        };
        let mut name = String::new();
        if file
            .take(MAX_PROCESS_NAME_BYTES)
            .read_to_string(&mut name)
            .is_ok()
            && matches!(
                name.trim(),
                "zed" | "zed-editor" | "zeditor" | "zedit" | "Zed"
            )
        {
            return true;
        }
    }
    false
}

#[cfg(target_os = "macos")]
fn platform_zed_process_is_live() -> bool {
    // Without `-a`, BSD `ps` restricts the listing to the current user; `-x`
    // includes GUI processes that have no controlling terminal.
    bounded_process_output("ps", &["-x", "-o", "comm="]).is_some_and(|output| {
        output.lines().any(|line| {
            Path::new(line.trim())
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| matches!(name, "Zed" | "zed" | "zed-editor"))
        })
    })
}

#[cfg(target_os = "windows")]
fn platform_zed_process_is_live() -> bool {
    bounded_process_output(
        "tasklist",
        &["/FI", "IMAGENAME eq zed.exe", "/FO", "CSV", "/NH"],
    )
    .is_some_and(|output| output.to_ascii_lowercase().contains("\"zed.exe\""))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_zed_process_is_live() -> bool {
    false
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn bounded_process_output(command: &str, arguments: &[&str]) -> Option<String> {
    let mut child = Command::new(command)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .take((MAX_PROCESS_COMMAND_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut output);
        let _ = sender.send((result, output));
    });
    let deadline = Instant::now() + PROCESS_COMMAND_TIMEOUT;
    let mut read_result = None;
    loop {
        if read_result.is_none() {
            match receiver.try_recv() {
                Ok(result) => read_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return None;
                }
            }
        }
        if read_result
            .as_ref()
            .is_some_and(|(_, output)| output.len() > MAX_PROCESS_COMMAND_OUTPUT_BYTES)
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let (read_status, output) = read_result
                    .or_else(|| receiver.recv_timeout(Duration::from_millis(100)).ok())?;
                let _ = reader.join();
                if !status.success()
                    || read_status.is_err()
                    || output.len() > MAX_PROCESS_COMMAND_OUTPUT_BYTES
                {
                    return None;
                }
                return String::from_utf8(output).ok();
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn zed_data_roots_with_environment(
    home: &Path,
    override_root: Option<OsString>,
    xdg_data_home: Option<OsString>,
    local_app_data: Option<OsString>,
) -> Vec<PathBuf> {
    if let Some(override_root) = override_root {
        let path = PathBuf::from(override_root);
        return (!path.as_os_str().is_empty() && path.is_absolute())
            .then_some(path)
            .into_iter()
            .collect();
    }

    let mut roots = BTreeSet::new();
    #[cfg(target_os = "macos")]
    {
        roots.insert(home.join("Library").join("Application Support").join("Zed"));
    }
    #[cfg(target_os = "windows")]
    {
        let local = local_app_data
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        roots.insert(local.join("Zed"));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let data_home = xdg_data_home
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".local").join("share"));
        roots.insert(data_home.join("zed"));
        // Community Flatpak packaging uses an isolated XDG data home.
        roots.insert(
            home.join(".var")
                .join("app")
                .join("dev.zed.Zed")
                .join("data")
                .join("zed"),
        );
    }

    // Silence configuration-specific unused-variable warnings while keeping a
    // single deterministic helper for cross-platform unit tests.
    #[cfg(target_os = "macos")]
    let _ = (&xdg_data_home, &local_app_data);
    #[cfg(target_os = "windows")]
    let _ = &xdg_data_home;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = &local_app_data;
    roots
        .into_iter()
        .filter(|path| path.is_absolute())
        .collect()
}

fn discover_channel_databases(
    data_roots: &[PathBuf],
    diagnostics: &mut ZedDiagnostics,
) -> Vec<ChannelDatabase> {
    let mut candidates: BTreeMap<(String, PathBuf), ChannelDatabase> = BTreeMap::new();
    for root in data_roots {
        if !bounded_directory(root) {
            continue;
        }
        let db_root = root.join("db");
        if !bounded_directory(&db_root) {
            continue;
        }

        // Restrict discovery to the documented channel set. In particular, do
        // not enumerate attacker-controlled sibling directories.
        for channel in KNOWN_CHANNELS {
            add_channel_candidate(&db_root, channel, &mut candidates);
        }
    }

    let candidate_count = candidates.len();
    let mut databases: Vec<_> = candidates.into_values().collect();
    databases.sort_by(|left, right| {
        channel_rank(&left.channel)
            .cmp(&channel_rank(&right.channel))
            .then_with(|| left.database.cmp(&right.database))
    });
    if databases.len() > MAX_CHANNELS {
        databases.truncate(MAX_CHANNELS);
        diagnostics.omitted_channels = diagnostics
            .omitted_channels
            .saturating_add(candidate_count - MAX_CHANNELS);
    }
    databases
}

fn add_channel_candidate(
    db_root: &Path,
    channel: &str,
    candidates: &mut BTreeMap<(String, PathBuf), ChannelDatabase>,
) {
    if !KNOWN_CHANNELS.contains(&channel) {
        return;
    }
    let directory = db_root.join(channel);
    if !bounded_directory(&directory) {
        return;
    }
    let database = directory.join("db.sqlite");
    candidates
        .entry((channel.to_owned(), database.clone()))
        .or_insert_with(|| ChannelDatabase {
            channel: channel.to_owned(),
            database,
            data_root: db_root
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| db_root.to_path_buf()),
        });
}

fn channel_rank(channel: &str) -> usize {
    KNOWN_CHANNELS
        .iter()
        .position(|candidate| *candidate == channel)
        .unwrap_or(KNOWN_CHANNELS.len())
}

fn read_channel_snapshot(
    candidate: &ChannelDatabase,
    now_ms: i64,
    detail: SnapshotDetail,
    model_budget: &mut ModelReadBudget,
) -> Result<ChannelSnapshot, String> {
    let connection = open_bounded_read_only_database(&candidate.database)?;
    if !table_has_columns(
        &connection,
        "workspaces",
        &[
            "paths",
            "paths_order",
            "remote_connection_id",
            "timestamp",
            "session_id",
            "window_id",
        ],
    )? {
        return Err("the Zed workspace schema is unavailable".to_string());
    }

    let current_session = read_current_session(&connection).unwrap_or(None);
    let window_stack = read_window_stack(&connection).unwrap_or_default();
    let (workspaces, workspace_malformed, omitted_workspaces) =
        read_workspaces(&connection, now_ms)?;
    let (mut activities, thread_malformed, omitted_threads, agent_metadata_available) =
        if detail == SnapshotDetail::WithAgentMetadata {
            match read_agent_activities(&connection, now_ms) {
                Ok(result) => result,
                Err(_) => (BTreeMap::new(), 1, 0, false),
            }
        } else {
            (BTreeMap::new(), 0, 0, false)
        };
    let workspace_keys: BTreeSet<_> = workspaces
        .iter()
        .map(|workspace| workspace.path_key.clone())
        .collect();
    activities.retain(|path_key, _| workspace_keys.contains(path_key));
    let model_diagnostics = if detail == SnapshotDetail::WithAgentMetadata {
        enrich_agent_signals(
            &candidate.data_root.join("threads").join("threads.db"),
            &mut activities,
            model_budget,
            now_ms,
        )
    } else {
        ModelDiagnostics::default()
    };

    Ok(ChannelSnapshot {
        channel: candidate.channel.clone(),
        current_session,
        window_stack,
        workspaces,
        activities,
        malformed_records: workspace_malformed.saturating_add(thread_malformed),
        omitted_workspaces,
        omitted_threads,
        agent_metadata_available,
        model_rows_considered: model_diagnostics.rows_considered,
        models_loaded: model_diagnostics.models_loaded,
        malformed_model_rows: model_diagnostics.malformed_rows,
        compressed_model_rows: model_diagnostics.compressed_rows,
    })
}

fn read_current_session(connection: &Connection) -> Result<Option<String>, String> {
    if !table_has_columns(connection, "kv_store", &["key", "value"])? {
        return Ok(None);
    }
    let mut statement = connection
        .prepare(
            "SELECT value FROM kv_store \
             WHERE key = 'session_id' AND typeof(value) = 'text' \
             AND length(CAST(value AS BLOB)) BETWEEN 1 AND ?1 LIMIT 1",
        )
        .map_err(|error| format!("could not prepare the Zed session query: {error}"))?;
    let value = statement
        .query_row([MAX_SESSION_ID_BYTES as i64], |row| row.get::<_, String>(0))
        .optional_without_import()
        .map_err(|error| format!("could not read the Zed session: {error}"))?;
    Ok(value.filter(|value| valid_opaque_value(value, MAX_SESSION_ID_BYTES)))
}

fn read_window_stack(connection: &Connection) -> Result<Vec<u64>, String> {
    if !table_has_columns(connection, "kv_store", &["key", "value"])? {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT value FROM kv_store \
             WHERE key = 'session_window_stack' AND typeof(value) = 'text' \
             AND length(CAST(value AS BLOB)) BETWEEN 2 AND ?1 LIMIT 1",
        )
        .map_err(|error| format!("could not prepare the Zed window-stack query: {error}"))?;
    let encoded = statement
        .query_row([MAX_WINDOW_STACK_BYTES as i64], |row| {
            row.get::<_, String>(0)
        })
        .optional_without_import()
        .map_err(|error| format!("could not read the Zed window stack: {error}"))?;
    let Some(encoded) = encoded else {
        return Ok(Vec::new());
    };
    let stack: Vec<u64> = serde_json::from_str(&encoded)
        .map_err(|_| "the Zed window stack has invalid bounded JSON".to_string())?;
    if stack.len() > MAX_WINDOW_STACK_ENTRIES
        || stack.iter().copied().collect::<BTreeSet<_>>().len() != stack.len()
    {
        return Err("the Zed window stack is invalid".to_string());
    }
    Ok(stack)
}

fn read_workspaces(
    connection: &Connection,
    now_ms: i64,
) -> Result<(Vec<InternalWorkspace>, usize, usize), String> {
    let mut statement = connection
        .prepare(
            "SELECT paths, \
                    CASE WHEN typeof(paths_order) = 'text' THEN paths_order ELSE '' END, \
                    CASE WHEN typeof(session_id) = 'text' \
                              AND length(CAST(session_id AS BLOB)) BETWEEN 1 AND ?1 \
                         THEN session_id END, \
                    CASE WHEN typeof(window_id) = 'integer' THEN window_id END, \
                    CASE WHEN typeof(timestamp) = 'text' \
                              AND length(CAST(timestamp AS BLOB)) BETWEEN 1 AND ?4 \
                         THEN CAST(strftime('%s', timestamp) AS INTEGER) * 1000 \
                              + CAST(substr(strftime('%f', timestamp), 4, 3) AS INTEGER) END \
             FROM workspaces \
             WHERE remote_connection_id IS NULL \
               AND typeof(paths) = 'text' \
               AND length(CAST(paths AS BLOB)) BETWEEN 1 AND ?2 \
               AND length(CAST(CASE WHEN typeof(paths_order) = 'text' \
                                    THEN paths_order ELSE '' END AS BLOB)) <= ?3 \
               AND typeof(timestamp) = 'text' \
               AND length(CAST(timestamp AS BLOB)) BETWEEN 1 AND ?4 \
             ORDER BY timestamp DESC \
             LIMIT ?5",
        )
        .map_err(|error| format!("could not prepare the Zed workspace query: {error}"))?;
    let mut rows = statement
        .query(rusqlite::params![
            MAX_SESSION_ID_BYTES as i64,
            MAX_PATH_LIST_BYTES as i64,
            MAX_PATH_ORDER_BYTES as i64,
            MAX_TIMESTAMP_BYTES as i64,
            (MAX_WORKSPACES_PER_CHANNEL + 1) as i64,
        ])
        .map_err(|error| format!("could not query Zed workspaces: {error}"))?;

    let mut workspaces = Vec::new();
    let mut malformed = 0usize;
    let mut saw_extra = false;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("could not read a Zed workspace row: {error}"))?
    {
        if workspaces.len() >= MAX_WORKSPACES_PER_CHANNEL {
            saw_extra = true;
            break;
        }
        let fields = (
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, Option<String>>(2),
            row.get::<_, Option<i64>>(3),
            row.get::<_, Option<i64>>(4),
        );
        let (Ok(paths), Ok(order), Ok(session_id), Ok(window_id), Ok(timestamp_ms)) = fields else {
            malformed += 1;
            continue;
        };
        let Some(decoded) = decode_path_list(&paths, &order) else {
            malformed += 1;
            continue;
        };
        let open_target = decoded.exact_order.then(|| decoded.ordered_paths.clone());
        workspaces.push(InternalWorkspace {
            path_key: decoded.sorted_key,
            paths: decoded.ordered_paths,
            open_target,
            last_active_at_ms: timestamp_ms.and_then(|value| bounded_timestamp(value, now_ms)),
            session_id: session_id.filter(|value| valid_opaque_value(value, MAX_SESSION_ID_BYTES)),
            window_id: window_id.and_then(|value| u64::try_from(value).ok()),
        });
    }
    Ok((workspaces, malformed, usize::from(saw_extra)))
}

type AgentActivityLoad = (
    BTreeMap<Vec<PathBuf>, InternalAgentActivity>,
    usize,
    usize,
    bool,
);

/// Read only the bounded native-session join keys needed to select candidate
/// thread blobs. Titles, paths, request IDs, and content never enter memory.
fn read_native_thread_join_ids(
    connection: &Connection,
    now_ms: i64,
) -> Result<Vec<(String, i64)>, String> {
    if !table_has_columns(
        connection,
        "sidebar_threads",
        &[
            "session_id",
            "agent_id",
            "updated_at",
            "archived",
            "remote_connection",
        ],
    )? {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT session_id, \
                    CAST(strftime('%s', updated_at) AS INTEGER) * 1000 \
                         + CAST(substr(strftime('%f', updated_at), 4, 3) AS INTEGER) \
             FROM sidebar_threads \
             WHERE agent_id IS NULL \
               AND COALESCE(archived, 0) = 0 \
               AND remote_connection IS NULL \
               AND typeof(session_id) = 'text' \
               AND length(CAST(session_id AS BLOB)) BETWEEN 1 AND ?1 \
               AND typeof(updated_at) = 'text' \
               AND length(CAST(updated_at AS BLOB)) BETWEEN 1 AND ?2 \
             ORDER BY updated_at DESC \
             LIMIT ?3",
        )
        .map_err(|error| format!("could not prepare the Zed native usage query: {error}"))?;
    let mut rows = statement
        .query(rusqlite::params![
            MAX_THREAD_JOIN_ID_BYTES as i64,
            MAX_TIMESTAMP_BYTES as i64,
            MAX_MODEL_ROWS_PER_REFRESH as i64,
        ])
        .map_err(|error| format!("could not query Zed native usage metadata: {error}"))?;
    let mut newest_by_join_id = BTreeMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("could not read Zed native usage metadata: {error}"))?
    {
        let (Ok(join_id), Ok(updated_at_ms)) =
            (row.get::<_, String>(0), row.get::<_, Option<i64>>(1))
        else {
            continue;
        };
        let Some(updated_at_ms) = updated_at_ms.and_then(|value| bounded_timestamp(value, now_ms))
        else {
            continue;
        };
        if !valid_opaque_value(&join_id, MAX_THREAD_JOIN_ID_BYTES) {
            continue;
        }
        newest_by_join_id
            .entry(join_id)
            .and_modify(|existing: &mut i64| *existing = (*existing).max(updated_at_ms))
            .or_insert(updated_at_ms);
    }
    let mut join_ids: Vec<_> = newest_by_join_id.into_iter().collect();
    join_ids.sort_by(|(left_id, left_timestamp), (right_id, right_timestamp)| {
        right_timestamp
            .cmp(left_timestamp)
            .then_with(|| left_id.cmp(right_id))
    });
    Ok(join_ids)
}

fn read_agent_activities(
    connection: &Connection,
    now_ms: i64,
) -> Result<AgentActivityLoad, String> {
    let required = [
        "session_id",
        "agent_id",
        "updated_at",
        "folder_paths",
        "folder_paths_order",
        "archived",
        "remote_connection",
    ];
    if !table_has_columns(connection, "sidebar_threads", &required)? {
        return Ok((BTreeMap::new(), 0, 0, false));
    }
    let has_main_paths = table_has_columns(
        connection,
        "sidebar_threads",
        &["main_worktree_paths", "main_worktree_paths_order"],
    )?;
    let has_interacted_at = table_has_columns(connection, "sidebar_threads", &["interacted_at"])?;
    let main_columns = if has_main_paths {
        "CASE WHEN typeof(main_worktree_paths) = 'text' \
                   AND length(CAST(main_worktree_paths AS BLOB)) BETWEEN 1 AND ?3 \
              THEN main_worktree_paths END, \
         CASE WHEN typeof(main_worktree_paths_order) = 'text' \
                   AND length(CAST(main_worktree_paths_order AS BLOB)) <= ?4 \
              THEN main_worktree_paths_order ELSE '' END"
    } else {
        "NULL, ''"
    };
    let interacted_column = if has_interacted_at {
        "CASE WHEN typeof(interacted_at) = 'text' \
                   AND length(CAST(interacted_at AS BLOB)) BETWEEN 1 AND ?5 \
              THEN CAST(strftime('%s', interacted_at) AS INTEGER) * 1000 \
                   + CAST(substr(strftime('%f', interacted_at), 4, 3) AS INTEGER) END"
    } else {
        "NULL"
    };
    // Only fixed, schema-derived fragments are interpolated; no database or
    // user value can become SQL syntax.
    let sql = format!(
        "SELECT agent_id IS NULL, \
                CASE WHEN typeof(agent_id) = 'text' \
                          AND length(CAST(agent_id AS BLOB)) BETWEEN 1 AND ?1 \
                     THEN agent_id END, \
                CASE WHEN typeof(session_id) = 'text' \
                          AND length(CAST(session_id AS BLOB)) BETWEEN 1 AND ?2 \
                     THEN session_id END, \
                CASE WHEN typeof(updated_at) = 'text' \
                          AND length(CAST(updated_at AS BLOB)) BETWEEN 1 AND ?5 \
                     THEN CAST(strftime('%s', updated_at) AS INTEGER) * 1000 \
                          + CAST(substr(strftime('%f', updated_at), 4, 3) AS INTEGER) END, \
                {interacted_column}, \
                folder_paths, \
                CASE WHEN typeof(folder_paths_order) = 'text' \
                     THEN folder_paths_order ELSE '' END, \
                {main_columns} \
         FROM sidebar_threads \
         WHERE COALESCE(archived, 0) = 0 \
           AND remote_connection IS NULL \
           AND typeof(updated_at) = 'text' \
           AND length(CAST(updated_at AS BLOB)) BETWEEN 1 AND ?5 \
           AND typeof(folder_paths) = 'text' \
           AND length(CAST(folder_paths AS BLOB)) BETWEEN 1 AND ?3 \
           AND length(CAST(CASE WHEN typeof(folder_paths_order) = 'text' \
                                THEN folder_paths_order ELSE '' END AS BLOB)) <= ?4 \
         ORDER BY updated_at DESC \
         LIMIT ?6"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("could not prepare the Zed agent query: {error}"))?;
    let mut rows = statement
        .query(rusqlite::params![
            MAX_AGENT_ID_BYTES as i64,
            MAX_THREAD_JOIN_ID_BYTES as i64,
            MAX_PATH_LIST_BYTES as i64,
            MAX_PATH_ORDER_BYTES as i64,
            MAX_TIMESTAMP_BYTES as i64,
            (MAX_THREADS_PER_CHANNEL + 1) as i64,
        ])
        .map_err(|error| format!("could not query Zed agent metadata: {error}"))?;

    let mut activities = BTreeMap::new();
    let mut accepted_rows = 0usize;
    let mut malformed = 0usize;
    let mut saw_extra = false;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("could not read a Zed agent row: {error}"))?
    {
        if accepted_rows >= MAX_THREADS_PER_CHANNEL {
            saw_extra = true;
            break;
        }
        let fields = (
            row.get::<_, bool>(0),
            row.get::<_, Option<String>>(1),
            row.get::<_, Option<String>>(2),
            row.get::<_, Option<i64>>(3),
            row.get::<_, Option<i64>>(4),
            row.get::<_, String>(5),
            row.get::<_, String>(6),
            row.get::<_, Option<String>>(7),
            row.get::<_, String>(8),
        );
        let (
            Ok(native_agent),
            Ok(agent_id),
            Ok(thread_join_id),
            Ok(changed_at_ms),
            Ok(interacted_at_ms),
            Ok(paths),
            Ok(order),
            Ok(main_paths),
            Ok(main_order),
        ) = fields
        else {
            malformed += 1;
            continue;
        };
        let Some(changed_at_ms) = changed_at_ms.and_then(|value| bounded_timestamp(value, now_ms))
        else {
            malformed += 1;
            continue;
        };
        let Some(decoded) = decode_path_list(&paths, &order) else {
            malformed += 1;
            continue;
        };
        accepted_rows += 1;
        let activity = InternalAgentActivity {
            view: ZedAgentActivity {
                agent_kind: if native_agent {
                    "zed".to_string()
                } else {
                    agent_kind(agent_id.as_deref())
                },
                changed_at_ms,
                interacted_at_ms: interacted_at_ms
                    .and_then(|value| bounded_timestamp(value, now_ms)),
                lifecycle: None,
                lifecycle_changed_at_ms: None,
                model_provider: None,
                model_name: None,
            },
            thread_join_id: native_agent
                .then_some(thread_join_id)
                .flatten()
                .filter(|value| valid_opaque_value(value, MAX_THREAD_JOIN_ID_BYTES)),
        };
        retain_newest_activity(&mut activities, decoded.sorted_key, activity.clone());
        if let Some(main_paths) = main_paths {
            if main_paths.len() <= MAX_PATH_LIST_BYTES {
                if let Some(main) = decode_path_list(&main_paths, &main_order) {
                    retain_newest_activity(&mut activities, main.sorted_key, activity);
                }
            }
        }
    }
    Ok((activities, malformed, usize::from(saw_extra), true))
}

#[derive(Debug, Default)]
struct ModelDiagnostics {
    rows_considered: usize,
    models_loaded: usize,
    malformed_rows: usize,
    compressed_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThreadModel {
    provider: String,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThreadTail {
    Unknown,
    User,
    Agent { has_tool_use: bool },
    Resume,
    Compaction,
}

#[derive(Debug, PartialEq, Eq)]
struct ThreadSignal {
    model: Option<ThreadModel>,
    tail: ThreadTail,
    cumulative_token_usage: Option<ThreadTokenUsage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ThreadTokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

impl ThreadTokenUsage {
    /// Match Zed's upstream `TokenUsage::total_tokens()` definition, but reject
    /// corrupt values that cannot be represented instead of wrapping a `u64`.
    fn checked_total_tokens(self) -> Option<u64> {
        self.input_tokens
            .checked_add(self.output_tokens)?
            .checked_add(self.cache_read_input_tokens)?
            .checked_add(self.cache_creation_input_tokens)
    }
}

fn enrich_agent_signals(
    database: &Path,
    activities: &mut BTreeMap<Vec<PathBuf>, InternalAgentActivity>,
    budget: &mut ModelReadBudget,
    now_ms: i64,
) -> ModelDiagnostics {
    let mut newest_by_join_id = BTreeMap::new();
    for activity in activities.values() {
        if activity.view.agent_kind != "zed" {
            continue;
        }
        let Some(join_id) = activity.thread_join_id.clone() else {
            continue;
        };
        newest_by_join_id
            .entry(join_id)
            .and_modify(|timestamp: &mut i64| {
                *timestamp = (*timestamp).max(activity.view.changed_at_ms);
            })
            .or_insert(activity.view.changed_at_ms);
    }
    let mut join_ids: Vec<_> = newest_by_join_id.into_iter().collect();
    join_ids.sort_by(|(left_id, left_timestamp), (right_id, right_timestamp)| {
        right_timestamp
            .cmp(left_timestamp)
            .then_with(|| left_id.cmp(right_id))
    });
    let (signals, diagnostics) = read_thread_signals(database, join_ids, budget, now_ms);

    for activity in activities.values_mut() {
        let Some((signal, thread_updated_at_ms)) = activity
            .thread_join_id
            .as_ref()
            .and_then(|join_id| signals.get(join_id))
        else {
            continue;
        };
        if let Some(model) = &signal.model {
            activity.view.model_provider = Some(model.provider.clone());
            activity.view.model_name = Some(model.name.clone());
        }
        if let Some((lifecycle, changed_at_ms)) = derive_native_lifecycle(
            activity.view.interacted_at_ms,
            *thread_updated_at_ms,
            signal.tail,
        ) {
            activity.view.lifecycle = Some(lifecycle);
            activity.view.lifecycle_changed_at_ms = Some(changed_at_ms);
        }
    }
    diagnostics
}

type JoinedThreadSignals = BTreeMap<String, (ThreadSignal, Option<i64>)>;

fn read_thread_signals(
    database: &Path,
    join_ids: Vec<(String, i64)>,
    budget: &mut ModelReadBudget,
    now_ms: i64,
) -> (JoinedThreadSignals, ModelDiagnostics) {
    if join_ids.is_empty() {
        return (BTreeMap::new(), ModelDiagnostics::default());
    }
    let Ok(connection) =
        open_bounded_read_only_database_with_limit(database, MAX_THREAD_DATABASE_BYTES)
    else {
        return (BTreeMap::new(), ModelDiagnostics::default());
    };
    if !table_has_columns(
        &connection,
        "threads",
        &["id", "updated_at", "data_type", "data"],
    )
    .unwrap_or(false)
    {
        return (BTreeMap::new(), ModelDiagnostics::default());
    }
    let Ok(mut statement) = connection.prepare(
        "SELECT CASE WHEN typeof(data_type) = 'text' \
                          AND length(CAST(data_type AS BLOB)) BETWEEN 1 AND ?2 \
                     THEN data_type END, \
                CASE WHEN typeof(data_type) = 'text' \
                          AND length(CAST(data_type AS BLOB)) BETWEEN 1 AND ?2 \
                          AND typeof(data) = 'blob' AND length(data) BETWEEN 1 AND ?3 \
                     THEN data END, \
                CASE WHEN typeof(data) = 'blob' THEN length(data) END, \
                CASE WHEN typeof(updated_at) = 'text' \
                              AND length(CAST(updated_at AS BLOB)) BETWEEN 1 AND ?5 \
                         THEN CAST(strftime('%s', updated_at) AS INTEGER) * 1000 \
                              + CAST(substr(strftime('%f', updated_at), 4, 3) AS INTEGER) END \
         FROM threads \
         WHERE id = ?1 AND typeof(id) = 'text' \
           AND length(CAST(id AS BLOB)) BETWEEN 1 AND ?4 \
         LIMIT 1",
    ) else {
        return (BTreeMap::new(), ModelDiagnostics::default());
    };

    let mut diagnostics = ModelDiagnostics::default();
    let mut signals = BTreeMap::new();
    for (join_id, _) in join_ids {
        if budget.rows_remaining == 0 || budget.decompressed_bytes_remaining == 0 {
            break;
        }
        budget.rows_remaining -= 1;
        let row = statement
            .query_row(
                rusqlite::params![
                    join_id,
                    MAX_THREAD_DATA_TYPE_BYTES as i64,
                    MAX_THREAD_BLOB_BYTES as i64,
                    MAX_THREAD_JOIN_ID_BYTES as i64,
                    MAX_TIMESTAMP_BYTES as i64,
                ],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<Vec<u8>>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional_without_import();
        let Ok(Some((data_type, data, length, thread_updated_at_ms))) = row else {
            if row.is_err() {
                diagnostics.malformed_rows += 1;
            }
            continue;
        };
        diagnostics.rows_considered += 1;
        if data_type.as_deref() == Some("zstd") {
            diagnostics.compressed_rows += 1;
        }
        let Some(data_type) = data_type else {
            diagnostics.malformed_rows += 1;
            continue;
        };
        let Some(data) = data.filter(|_| {
            length.is_some_and(|length| length > 0 && length <= MAX_THREAD_BLOB_BYTES as i64)
        }) else {
            diagnostics.malformed_rows += 1;
            continue;
        };
        match extract_thread_signal_with_budget(
            &data_type,
            &data,
            &mut budget.decompressed_bytes_remaining,
        ) {
            Ok(signal) => {
                if signal.model.is_some() {
                    diagnostics.models_loaded += 1;
                }
                signals.insert(
                    join_id,
                    (
                        signal,
                        thread_updated_at_ms
                            .and_then(|timestamp| bounded_timestamp(timestamp, now_ms)),
                    ),
                );
            }
            Err(()) => diagnostics.malformed_rows += 1,
        }
    }

    (signals, diagnostics)
}

fn usage_observation(
    signal: &ThreadSignal,
    thread_updated_at_ms: Option<i64>,
) -> Option<ZedUsageObservation> {
    signal
        .cumulative_token_usage
        .and_then(ThreadTokenUsage::checked_total_tokens)
        .filter(|total_tokens| *total_tokens > 0)
        .zip(thread_updated_at_ms)
        .map(|(total_tokens, updated_at_ms)| ZedUsageObservation {
            total_tokens,
            updated_at_ms,
        })
}

fn derive_native_lifecycle(
    interacted_at_ms: Option<i64>,
    thread_updated_at_ms: Option<i64>,
    tail: ThreadTail,
) -> Option<(ZedAgentLifecycle, i64)> {
    let interacted_at_ms = interacted_at_ms?;
    match tail {
        ThreadTail::User | ThreadTail::Resume => {
            Some((ZedAgentLifecycle::ActivityDetected, interacted_at_ms))
        }
        ThreadTail::Agent { has_tool_use } => {
            if thread_updated_at_ms.is_some_and(|updated| interacted_at_ms > updated) {
                // The metadata write for a newly submitted prompt can race the
                // native thread-blob write, temporarily leaving the previous
                // completed assistant boundary on disk.
                Some((ZedAgentLifecycle::ActivityDetected, interacted_at_ms))
            } else if has_tool_use {
                Some((
                    ZedAgentLifecycle::ActivityDetected,
                    thread_updated_at_ms
                        .map(|updated| updated.max(interacted_at_ms))
                        .unwrap_or(interacted_at_ms),
                ))
            } else {
                thread_updated_at_ms
                    .filter(|updated| *updated >= interacted_at_ms)
                    .map(|updated| (ZedAgentLifecycle::TurnFinished, updated))
            }
        }
        // A manual `/compact` writes a terminal compaction record without
        // starting another model turn. Unknown structures are equally
        // unsuitable as lifecycle evidence.
        ThreadTail::Compaction | ThreadTail::Unknown => None,
    }
}

fn extract_thread_signal_with_budget(
    data_type: &str,
    data: &[u8],
    decompressed_bytes_remaining: &mut usize,
) -> Result<ThreadSignal, ()> {
    match data_type {
        "json" => {
            extract_thread_signal_from_reader(io::Cursor::new(data), decompressed_bytes_remaining)
        }
        "zstd" => {
            let decoder = zstd::stream::read::Decoder::new(data).map_err(|_| ())?;
            extract_thread_signal_from_reader(decoder, decompressed_bytes_remaining)
        }
        _ => Err(()),
    }
}

#[cfg(test)]
fn extract_thread_model_with_budget(
    data_type: &str,
    data: &[u8],
    decompressed_bytes_remaining: &mut usize,
) -> Result<Option<ThreadModel>, ()> {
    extract_thread_signal_with_budget(data_type, data, decompressed_bytes_remaining)
        .map(|signal| signal.model)
}

#[cfg(test)]
fn extract_thread_model(data_type: &str, data: &[u8]) -> Result<Option<ThreadModel>, ()> {
    let mut budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
    extract_thread_model_with_budget(data_type, data, &mut budget)
}

fn extract_thread_signal_from_reader<R: Read>(
    reader: R,
    decompressed_bytes_remaining: &mut usize,
) -> Result<ThreadSignal, ()> {
    let capped = CappedReader::new(reader, decompressed_bytes_remaining);
    let mut deserializer = serde_json::Deserializer::from_reader(capped);
    let signal = ThreadSignalSeed
        .deserialize(&mut deserializer)
        .map_err(|_| ())?;
    deserializer.end().map_err(|_| ())?;
    Ok(signal)
}

struct ThreadSignalSeed;

impl<'de> DeserializeSeed<'de> for ThreadSignalSeed {
    type Value = ThreadSignal;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ThreadSignalVisitor)
    }
}

struct ThreadSignalVisitor;

impl<'de> Visitor<'de> for ThreadSignalVisitor {
    type Value = ThreadSignal;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Zed thread object with bounded structural metadata")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut model = None;
        let mut tail = ThreadTail::Unknown;
        let mut cumulative_token_usage = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "model" => model = map.next_value_seed(OptionalModelSeed)?,
                "messages" => tail = map.next_value_seed(MessagesTailSeed)?,
                "cumulative_token_usage" => {
                    cumulative_token_usage = map.next_value_seed(ThreadTokenUsageSeed)?;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(ThreadSignal {
            model,
            tail,
            cumulative_token_usage,
        })
    }
}

struct ThreadTokenUsageSeed;

impl<'de> DeserializeSeed<'de> for ThreadTokenUsageSeed {
    type Value = Option<ThreadTokenUsage>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ThreadTokenUsageVisitor)
    }
}

struct ThreadTokenUsageVisitor;

impl<'de> Visitor<'de> for ThreadTokenUsageVisitor {
    type Value = Option<ThreadTokenUsage>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Zed cumulative token counters")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut input_tokens = None;
        let mut output_tokens = None;
        let mut cache_creation_input_tokens = None;
        let mut cache_read_input_tokens = None;
        let mut saw_counter = false;
        while let Some(key) = map.next_key::<String>()? {
            let slot = match key.as_str() {
                "input_tokens" => Some(&mut input_tokens),
                "output_tokens" => Some(&mut output_tokens),
                "cache_creation_input_tokens" => Some(&mut cache_creation_input_tokens),
                "cache_read_input_tokens" => Some(&mut cache_read_input_tokens),
                _ => None,
            };
            if let Some(slot) = slot {
                if slot.is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate Zed cumulative token counter",
                    ));
                }
                *slot = Some(map.next_value::<u64>()?);
                saw_counter = true;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        if !saw_counter {
            return Ok(None);
        }
        let usage = ThreadTokenUsage {
            input_tokens: input_tokens.unwrap_or(0),
            output_tokens: output_tokens.unwrap_or(0),
            cache_creation_input_tokens: cache_creation_input_tokens.unwrap_or(0),
            cache_read_input_tokens: cache_read_input_tokens.unwrap_or(0),
        };
        if usage.checked_total_tokens().is_none() {
            return Err(serde::de::Error::custom(
                "Zed cumulative token counters overflow",
            ));
        }
        Ok(Some(usage))
    }
}

struct MessagesTailSeed;

impl<'de> DeserializeSeed<'de> for MessagesTailSeed {
    type Value = ThreadTail;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(MessagesTailVisitor)
    }
}

struct MessagesTailVisitor;

impl<'de> Visitor<'de> for MessagesTailVisitor {
    type Value = ThreadTail;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Zed message sequence")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut tail = ThreadTail::Unknown;
        while let Some(message) = sequence.next_element_seed(MessageTailSeed)? {
            tail = message;
        }
        Ok(tail)
    }
}

struct MessageTailSeed;

impl<'de> DeserializeSeed<'de> for MessageTailSeed {
    type Value = ThreadTail;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(MessageTailVisitor)
    }
}

struct MessageTailVisitor;

impl<'de> Visitor<'de> for MessageTailVisitor {
    type Value = ThreadTail;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a structurally recognizable Zed message")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(match value {
            "Resume" => ThreadTail::Resume,
            _ => ThreadTail::Unknown,
        })
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&value)
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut count = 0usize;
        let mut tail = ThreadTail::Unknown;
        while let Some(key) = map.next_key::<String>()? {
            count = count.saturating_add(1);
            tail = match key.as_str() {
                "User" => {
                    map.next_value::<IgnoredAny>()?;
                    ThreadTail::User
                }
                "Agent" => map.next_value_seed(AgentMessageSignalSeed)?,
                "Compaction" => {
                    map.next_value::<IgnoredAny>()?;
                    ThreadTail::Compaction
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                    ThreadTail::Unknown
                }
            };
        }
        Ok(if count == 1 {
            tail
        } else {
            ThreadTail::Unknown
        })
    }
}

struct AgentMessageSignalSeed;

impl<'de> DeserializeSeed<'de> for AgentMessageSignalSeed {
    type Value = ThreadTail;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(AgentMessageSignalVisitor)
    }
}

struct AgentMessageSignalVisitor;

impl<'de> Visitor<'de> for AgentMessageSignalVisitor {
    type Value = ThreadTail;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Zed assistant message object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut has_tool_use = false;
        let mut content_fields = 0usize;
        let mut content_known = true;
        while let Some(key) = map.next_key::<String>()? {
            if key == "content" {
                content_fields = content_fields.saturating_add(1);
                match map.next_value_seed(AgentContentSignalSeed)? {
                    Some(value) => has_tool_use |= value,
                    None => content_known = false,
                }
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(if content_fields == 1 && content_known {
            ThreadTail::Agent { has_tool_use }
        } else {
            ThreadTail::Unknown
        })
    }
}

struct AgentContentSignalSeed;

impl<'de> DeserializeSeed<'de> for AgentContentSignalSeed {
    type Value = Option<bool>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(AgentContentSignalVisitor)
    }
}

struct AgentContentSignalVisitor;

impl<'de> Visitor<'de> for AgentContentSignalVisitor {
    type Value = Option<bool>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Zed assistant content sequence")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut has_tool_use = false;
        let mut all_known = true;
        while let Some(item) = sequence.next_element_seed(AgentContentItemSeed)? {
            match item {
                Some(is_tool_use) => has_tool_use |= is_tool_use,
                None => all_known = false,
            }
        }
        Ok(all_known.then_some(has_tool_use))
    }
}

struct AgentContentItemSeed;

impl<'de> DeserializeSeed<'de> for AgentContentItemSeed {
    type Value = Option<bool>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(AgentContentItemVisitor)
    }
}

struct AgentContentItemVisitor;

impl<'de> Visitor<'de> for AgentContentItemVisitor {
    type Value = Option<bool>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an externally tagged Zed assistant content item")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut count = 0usize;
        let mut item = None;
        while let Some(key) = map.next_key::<String>()? {
            count = count.saturating_add(1);
            item = match key.as_str() {
                "Text" | "Thinking" | "RedactedThinking" => Some(false),
                "ToolUse" => Some(true),
                _ => None,
            };
            map.next_value::<IgnoredAny>()?;
        }
        Ok((count == 1).then_some(item).flatten())
    }
}

struct OptionalModelSeed;

impl<'de> DeserializeSeed<'de> for OptionalModelSeed {
    type Value = Option<ThreadModel>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_option(OptionalModelVisitor)
    }
}

struct OptionalModelVisitor;

impl<'de> Visitor<'de> for OptionalModelVisitor {
    type Value = Option<ThreadModel>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null or a Zed model object")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(ModelVisitor).map(Some)
    }
}

struct ModelVisitor;

impl<'de> Visitor<'de> for ModelVisitor {
    type Value = ThreadModel;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded Zed model object")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut provider = None;
        let mut name = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "provider" => provider = Some(map.next_value::<String>()?),
                "model" => name = Some(map.next_value::<String>()?),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        let provider = provider
            .filter(|value| valid_model_value(value))
            .ok_or_else(|| serde::de::Error::custom("invalid model provider"))?;
        let name = name
            .filter(|value| valid_model_value(value))
            .ok_or_else(|| serde::de::Error::custom("invalid model name"))?;
        Ok(ThreadModel { provider, name })
    }
}

fn valid_model_value(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_MODEL_VALUE_BYTES
        && !value.chars().any(char::is_control)
}

struct CappedReader<'a, R> {
    inner: R,
    remaining: &'a mut usize,
    exceeded: bool,
}

impl<'a, R> CappedReader<'a, R> {
    fn new(inner: R, remaining: &'a mut usize) -> Self {
        Self {
            inner,
            remaining,
            exceeded: false,
        }
    }
}

impl<R: Read> Read for CappedReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.exceeded {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "decompressed Zed thread exceeds its safety bound",
            ));
        }
        if *self.remaining == 0 {
            let mut probe = [0_u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => {
                    self.exceeded = true;
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "decompressed Zed thread exceeds its safety bound",
                    ))
                }
            };
        }
        let allowed = buffer.len().min(*self.remaining);
        let count = self.inner.read(&mut buffer[..allowed])?;
        *self.remaining -= count;
        Ok(count)
    }
}

fn retain_newest_activity(
    activities: &mut BTreeMap<Vec<PathBuf>, InternalAgentActivity>,
    path_key: Vec<PathBuf>,
    candidate: InternalAgentActivity,
) {
    match activities.entry(path_key) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(candidate);
        }
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            if candidate.view.changed_at_ms > entry.get().view.changed_at_ms {
                entry.insert(candidate);
            }
        }
    }
}

fn agent_kind(agent_id: Option<&str>) -> String {
    match agent_id {
        None => "external".to_string(),
        Some(value) if valid_agent_id(value) => value.to_string(),
        Some(_) => "external".to_string(),
    }
}

fn valid_agent_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_AGENT_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@')
        })
}

fn decode_path_list(paths: &str, order: &str) -> Option<DecodedPathList> {
    if paths.is_empty() || paths.len() > MAX_PATH_LIST_BYTES || order.len() > MAX_PATH_ORDER_BYTES {
        return None;
    }
    let mut sorted_key = Vec::new();
    for encoded_path in paths.split('\n') {
        if encoded_path.is_empty()
            || encoded_path.len() > MAX_SINGLE_PATH_BYTES
            || encoded_path.chars().any(char::is_control)
        {
            return None;
        }
        let path = PathBuf::from(encoded_path);
        if !valid_local_absolute_path(&path) {
            return None;
        }
        sorted_key.push(path);
        if sorted_key.len() > MAX_PATHS {
            return None;
        }
    }
    if sorted_key.is_empty() {
        return None;
    }
    let mut exact_order = sorted_key.windows(2).all(|pair| pair[0] < pair[1]);
    if !exact_order {
        sorted_key.sort();
        sorted_key.dedup();
        if sorted_key.len() != paths.split('\n').count() {
            return None;
        }
    }

    let parsed_order: Option<Vec<usize>> = if order.is_empty() {
        None
    } else {
        order
            .split(',')
            .map(|value| {
                (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
                    .then(|| value.parse::<usize>().ok())
                    .flatten()
            })
            .collect()
    };
    let mut ordered_paths = sorted_key.clone();
    if let Some(parsed_order) = parsed_order {
        let is_permutation = parsed_order.len() == sorted_key.len()
            && parsed_order.iter().all(|index| *index < sorted_key.len())
            && parsed_order.iter().copied().collect::<BTreeSet<_>>().len() == sorted_key.len();
        if is_permutation && exact_order {
            let mut indexed: Vec<_> = parsed_order
                .into_iter()
                .zip(sorted_key.iter().cloned())
                .collect();
            indexed.sort_by_key(|(original_index, _)| *original_index);
            ordered_paths = indexed.into_iter().map(|(_, path)| path).collect();
        } else {
            exact_order = false;
        }
    } else {
        exact_order = false;
    }

    Some(DecodedPathList {
        sorted_key,
        ordered_paths,
        exact_order,
    })
}

fn valid_local_absolute_path(path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return false;
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;

        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return false;
        };
        matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    }
    #[cfg(not(windows))]
    true
}

fn valid_opaque_value(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= maximum_bytes && !value.chars().any(char::is_control)
}

fn bounded_timestamp(value: i64, now_ms: i64) -> Option<i64> {
    (value > 0 && value <= now_ms.saturating_add(MAX_FUTURE_SKEW_MS)).then_some(value)
}

fn channel_has_current_window(channel: &ChannelSnapshot) -> bool {
    channel
        .workspaces
        .iter()
        .any(|workspace| current_window_stack_index(channel, workspace).is_some())
}

fn current_window_stack_index(
    channel: &ChannelSnapshot,
    workspace: &InternalWorkspace,
) -> Option<usize> {
    let current_session = channel.current_session.as_deref()?;
    if workspace.session_id.as_deref()? != current_session {
        return None;
    }
    let window_id = workspace.window_id?;
    channel
        .window_stack
        .iter()
        .position(|value| *value == window_id)
}

fn prefer_workspace(
    candidate: &ZedWorkspaceObservation,
    existing: &ZedWorkspaceObservation,
) -> bool {
    (candidate.open && !existing.open)
        || (candidate.open == existing.open
            && (candidate.last_active_at_ms > existing.last_active_at_ms
                || (candidate.last_active_at_ms == existing.last_active_at_ms
                    && candidate.channel < existing.channel)))
}

fn zed_instance_id(channel: &str, path_key: &[PathBuf]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"vsparallel-zed-workspace-v1\0");
    hasher.update(channel.as_bytes());
    for path in path_key {
        hasher.update([0]);
        hasher.update(path.to_string_lossy().as_bytes());
    }
    format!("zed:{}", format_args!("{:x}", hasher.finalize()))
}

fn table_has_columns(
    connection: &Connection,
    table: &str,
    required_columns: &[&str],
) -> Result<bool, String> {
    let is_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema \
             WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(|error| format!("could not inspect the Zed schema: {error}"))?;
    if !is_table {
        return Ok(false);
    }
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .map_err(|error| format!("could not inspect the Zed table schema: {error}"))?;
    let columns = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(|error| format!("could not query the Zed table schema: {error}"))?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| format!("could not read the Zed table schema: {error}"))?;
    Ok(required_columns
        .iter()
        .all(|column| columns.contains(*column)))
}

fn open_bounded_read_only_database(database: &Path) -> Result<Connection, String> {
    open_bounded_read_only_database_with_limit(database, MAX_DATABASE_BYTES)
}

fn open_bounded_read_only_database_with_limit(
    database: &Path,
    maximum_database_bytes: u64,
) -> Result<Connection, String> {
    if !bounded_regular_file(database, maximum_database_bytes, false, false)? {
        return Err("the Zed database is missing or outside its size/type bound".to_string());
    }
    let Some(filename) = database.file_name().and_then(OsStr::to_str) else {
        return Err("the Zed database filename is invalid".to_string());
    };
    for (suffix, maximum_bytes) in [
        ("-wal", MAX_DATABASE_WAL_BYTES),
        ("-shm", MAX_DATABASE_SHM_BYTES),
    ] {
        let auxiliary = database.with_file_name(format!("{filename}{suffix}"));
        if !bounded_regular_file(&auxiliary, maximum_bytes, true, true)? {
            return Err("a Zed database auxiliary file is outside its bound".to_string());
        }
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let connection = Connection::open_with_flags(database, flags)
        .map_err(|error| format!("could not open the Zed database read-only: {error}"))?;
    connection
        .busy_timeout(DATABASE_BUSY_TIMEOUT)
        .map_err(|error| format!("could not bound the Zed database query: {error}"))?;
    connection
        .pragma_update(None, "query_only", true)
        .map_err(|error| format!("could not enforce a read-only Zed query: {error}"))?;
    connection
        .pragma_update(None, "trusted_schema", false)
        .map_err(|error| format!("could not disable trusted Zed schema objects: {error}"))?;
    Ok(connection)
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
        Err(error) => return Err(format!("could not inspect a Zed database file: {error}")),
    };
    Ok(!is_link_or_reparse_point(&metadata)
        && metadata.is_file()
        && (empty_is_valid || metadata.len() > 0)
        && metadata.len() <= maximum_bytes)
}

fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Small compatibility helper that avoids importing `rusqlite::OptionalExtension`
/// into the module's public namespace.
trait OptionalRow<T> {
    fn optional_without_import(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional_without_import(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::fs::File;

    const NOW_MS: i64 = 2_000_000_000_000;

    fn create_channel_database(root: &Path, channel: &str) -> PathBuf {
        let directory = root.join("db").join(channel);
        fs::create_dir_all(&directory).unwrap();
        let database = directory.join("db.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE kv_store(key TEXT PRIMARY KEY, value TEXT); \
                 CREATE TABLE workspaces( \
                     workspace_id INTEGER PRIMARY KEY, \
                     paths TEXT, \
                     paths_order TEXT, \
                     remote_connection_id INTEGER, \
                     timestamp TEXT, \
                     session_id TEXT, \
                     window_id INTEGER \
                 ); \
                 CREATE TABLE sidebar_threads( \
                     thread_id BLOB PRIMARY KEY, \
                     session_id TEXT, \
                     agent_id TEXT, \
                     title TEXT NOT NULL, \
                     updated_at TEXT NOT NULL, \
                     created_at TEXT, \
                     folder_paths TEXT, \
                     folder_paths_order TEXT, \
                     archived INTEGER DEFAULT 0, \
                     main_worktree_paths TEXT, \
                     main_worktree_paths_order TEXT, \
                     remote_connection TEXT, \
                     interacted_at TEXT, \
                     title_override TEXT \
                 );",
            )
            .unwrap();
        database
    }

    fn set_session(database: &Path, session: &str, window_stack: &str) {
        let connection = Connection::open(database).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO kv_store(key, value) VALUES ('session_id', ?1)",
                [session],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO kv_store(key, value) \
                 VALUES ('session_window_stack', ?1)",
                [window_stack],
            )
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_workspace(
        database: &Path,
        id: i64,
        paths: &str,
        order: &str,
        session: Option<&str>,
        window_id: Option<i64>,
        remote: Option<i64>,
        timestamp: &str,
    ) {
        let connection = Connection::open(database).unwrap();
        connection
            .execute(
                "INSERT INTO workspaces( \
                     workspace_id, paths, paths_order, remote_connection_id, \
                     timestamp, session_id, window_id \
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id, paths, order, remote, timestamp, session, window_id],
            )
            .unwrap();
    }

    struct ThreadFixture<'a> {
        id: u8,
        paths: &'a str,
        order: &'a str,
        agent_id: Option<&'a str>,
        updated_at: &'a str,
        archived: bool,
        remote: Option<&'a str>,
        title: &'a str,
        session: &'a str,
        main_paths: Option<&'a str>,
        main_order: Option<&'a str>,
    }

    fn insert_thread(database: &Path, fixture: ThreadFixture<'_>) {
        let connection = Connection::open(database).unwrap();
        connection
            .execute(
                "INSERT INTO sidebar_threads( \
                     thread_id, session_id, agent_id, title, updated_at, created_at, \
                     folder_paths, folder_paths_order, archived, \
                     main_worktree_paths, main_worktree_paths_order, \
                     remote_connection, interacted_at \
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?5)",
                params![
                    vec![fixture.id; 16],
                    fixture.session,
                    fixture.agent_id,
                    fixture.title,
                    fixture.updated_at,
                    fixture.paths,
                    fixture.order,
                    i64::from(fixture.archived),
                    fixture.main_paths,
                    fixture.main_order,
                    fixture.remote,
                ],
            )
            .unwrap();
    }

    fn encoded_paths(paths: &[&Path]) -> (String, String) {
        let mut indexed: Vec<_> = paths.iter().enumerate().collect();
        indexed.sort_by(|left, right| left.1.cmp(right.1));
        let encoded_paths = indexed
            .iter()
            .map(|(_, path)| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        let order = indexed
            .iter()
            .map(|(index, _)| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        (encoded_paths, order)
    }

    fn create_threads_database(root: &Path) -> PathBuf {
        let directory = root.join("threads");
        fs::create_dir_all(&directory).unwrap();
        let database = directory.join("threads.db");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads( \
                     id TEXT PRIMARY KEY, summary TEXT NOT NULL, updated_at TEXT NOT NULL, \
                     data_type TEXT NOT NULL, data BLOB NOT NULL, parent_id TEXT, \
                     folder_paths TEXT, folder_paths_order TEXT, created_at TEXT \
                 );",
            )
            .unwrap();
        database
    }

    fn insert_thread_model_blob(database: &Path, id: &str, data_type: &str, data: &[u8]) {
        insert_thread_blob_at(database, id, "2026-01-02T05:00:00Z", data_type, data);
    }

    fn insert_thread_blob_at(
        database: &Path,
        id: &str,
        updated_at: &str,
        data_type: &str,
        data: &[u8],
    ) {
        let connection = Connection::open(database).unwrap();
        connection
            .execute(
                "INSERT INTO threads(id, summary, updated_at, data_type, data) \
                 VALUES (?1, 'private', ?2, ?3, ?4)",
                params![id, updated_at, data_type, data],
            )
            .unwrap();
    }

    #[test]
    fn explicit_data_root_override_is_authoritative_and_absolute() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            zed_data_roots_with_environment(
                temp.path(),
                Some(temp.path().as_os_str().to_owned()),
                Some(OsString::from("/ignored")),
                Some(OsString::from("/ignored")),
            ),
            vec![temp.path().to_path_buf()]
        );
        assert!(zed_data_roots_with_environment(
            temp.path(),
            Some(OsString::from("relative")),
            None,
            None,
        )
        .is_empty());
    }

    #[test]
    fn open_requires_live_process_current_session_and_current_window_stack() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        fs::create_dir(&workspace).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        set_session(&database, "current-session", "[9,7]");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            Some("current-session"),
            Some(7),
            None,
            "2026-01-02 03:04:05",
        );

        let live = load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, true);
        assert_eq!(live.workspaces.len(), 1);
        assert!(live.workspaces[0].open);
        assert_eq!(live.workspaces[0].window_stack_index, Some(1));
        assert_eq!(
            live.workspaces[0].open_target,
            Some(vec![workspace.clone()])
        );

        let stopped =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert!(!stopped.workspaces[0].open);
        assert_eq!(stopped.workspaces[0].window_stack_index, None);

        set_session(&database, "another-session", "[7]");
        let mismatched =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, true);
        assert!(!mismatched.workspaces[0].open);
    }

    #[test]
    fn multi_root_order_is_preserved_only_when_exact() {
        let temp = tempfile::tempdir().unwrap();
        let alpha = temp.path().join("alpha");
        let beta = temp.path().join("beta");
        fs::create_dir(&alpha).unwrap();
        fs::create_dir(&beta).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[beta.as_path(), alpha.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_workspace(
            &database,
            2,
            &format!("{}\n{}", alpha.display(), beta.display()),
            "0,0",
            None,
            None,
            None,
            "2026-01-01 03:04:05",
        );

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        // Both rows describe the same path set; the newer exact row wins.
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(
            snapshot.workspaces[0].paths,
            vec![beta.clone(), alpha.clone()]
        );
        assert_eq!(snapshot.workspaces[0].open_target, Some(vec![beta, alpha]));

        let decoded = decode_path_list(&paths, "0,0").unwrap();
        assert!(!decoded.exact_order);
    }

    #[test]
    fn polling_reconstructs_missing_path_without_probing_the_filesystem() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("gone");
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[missing.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.workspaces[0].paths, vec![missing.clone()]);
        assert_eq!(snapshot.workspaces[0].open_target, Some(vec![missing]));
    }

    #[test]
    fn latest_local_unarchived_agent_metadata_is_attached_without_private_fields() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        fs::create_dir(&workspace).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &paths,
                order: &order,
                agent_id: None,
                updated_at: "2026-01-02 04:00:00",
                archived: false,
                remote: None,
                title: "PRIVATE PROMPT-LIKE TITLE",
                session: "PRIVATE-SESSION-ID",
                main_paths: None,
                main_order: None,
            },
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 2,
                paths: &paths,
                order: &order,
                agent_id: Some("registry:codex"),
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "ANOTHER PRIVATE TITLE",
                session: "ANOTHER-PRIVATE-SESSION",
                main_paths: None,
                main_order: None,
            },
        );

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        let activity = snapshot.workspaces[0].agent.as_ref().unwrap();
        assert_eq!(activity.agent_kind, "registry:codex");
        assert!(activity.changed_at_ms > 0);
        assert_eq!(activity.model_provider, None);
        assert_eq!(activity.model_name, None);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("PRIVATE"));
        assert!(!serialized.contains("prompt"));
        assert!(!serialized.contains("sessionId"));
        assert!(!serialized.contains("threadId"));
        assert!(!serialized.contains("title"));
    }

    #[test]
    fn bounded_json_model_is_joined_without_exposing_thread_content_or_ids() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        fs::create_dir(&workspace).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &paths,
                order: &order,
                agent_id: None,
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "PRIVATE TITLE",
                session: "join-only-thread-id",
                main_paths: None,
                main_order: None,
            },
        );
        let threads = create_threads_database(temp.path());
        let private_prompt = "PRIVATE PROMPT AND RESPONSE MUST NEVER ESCAPE";
        let body = serde_json::to_vec(&serde_json::json!({
            "title": "private title",
            "messages": [{"content": private_prompt}, {"tool": {"result": private_prompt}}],
            "model": {"provider": "openai", "model": "gpt-5.4"},
            "version": "0.3.0"
        }))
        .unwrap();
        insert_thread_model_blob(&threads, "join-only-thread-id", "json", &body);

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        let agent = snapshot.workspaces[0].agent.as_ref().unwrap();
        assert_eq!(agent.model_provider.as_deref(), Some("openai"));
        assert_eq!(agent.model_name.as_deref(), Some("gpt-5.4"));
        assert_eq!(snapshot.diagnostics.models_loaded, 1);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains(private_prompt));
        assert!(!serialized.contains("join-only-thread-id"));
    }

    #[test]
    fn native_cumulative_usage_is_summed_and_kept_out_of_workspace_json() {
        let temp = tempfile::tempdir().unwrap();
        let native_workspace = temp.path().join("native-project");
        let external_workspace = temp.path().join("external-project");
        let database = create_channel_database(temp.path(), "0-stable");
        let (native_paths, native_order) = encoded_paths(&[native_workspace.as_path()]);
        let (external_paths, external_order) = encoded_paths(&[external_workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &native_paths,
            &native_order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_workspace(
            &database,
            2,
            &external_paths,
            &external_order,
            None,
            None,
            None,
            "2026-01-02 03:04:06",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &native_paths,
                order: &native_order,
                agent_id: None,
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "private native title",
                session: "native-usage-join",
                main_paths: None,
                main_order: None,
            },
        );
        // A still newer native row with corrupt counters is ignored in favor
        // of the newest valid native observation.
        insert_thread(
            &database,
            ThreadFixture {
                id: 3,
                paths: &native_paths,
                order: &native_order,
                agent_id: None,
                updated_at: "2026-01-02 07:00:00",
                archived: false,
                remote: None,
                title: "private corrupt title",
                session: "corrupt-usage-join",
                main_paths: None,
                main_order: None,
            },
        );
        // A newer external ACP row must not displace native Zed usage.
        insert_thread(
            &database,
            ThreadFixture {
                id: 2,
                paths: &external_paths,
                order: &external_order,
                agent_id: Some("registry:cursor"),
                updated_at: "2026-01-02 06:00:00",
                archived: false,
                remote: None,
                title: "private external title",
                session: "external-usage-join",
                main_paths: None,
                main_order: None,
            },
        );
        let threads = create_threads_database(temp.path());
        let private_content = "PRIVATE TOKEN-USAGE THREAD CONTENT";
        let native_body = serde_json::to_vec(&serde_json::json!({
            "messages": [{"User": {
                "id": "private-request-id",
                "content": [{"Text": private_content}]
            }}],
            "cumulative_token_usage": {
                "input_tokens": 100,
                "output_tokens": 20,
                "cache_creation_input_tokens": 5,
                "cache_read_input_tokens": 7,
                "future_counter": 999_999
            }
        }))
        .unwrap();
        insert_thread_blob_at(
            &threads,
            "native-usage-join",
            "2026-01-02T05:00:03Z",
            "json",
            &native_body,
        );
        let external_body = serde_json::to_vec(&serde_json::json!({
            "cumulative_token_usage": {"input_tokens": 999_999}
        }))
        .unwrap();
        insert_thread_blob_at(
            &threads,
            "external-usage-join",
            "2026-01-02T06:00:00Z",
            "json",
            &external_body,
        );
        let overflow_body = format!(
            "{{\"cumulative_token_usage\":{{\"input_tokens\":{},\"output_tokens\":1}}}}",
            u64::MAX
        );
        insert_thread_blob_at(
            &threads,
            "corrupt-usage-join",
            "2026-01-02T07:00:00Z",
            "json",
            overflow_body.as_bytes(),
        );

        assert_eq!(
            load_zed_usage_from_data_roots(&[temp.path().to_path_buf()], NOW_MS),
            Some(ZedUsageObservation {
                total_tokens: 132,
                updated_at_ms: 1_767_330_003_000,
            })
        );
        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        let serialized = serde_json::to_string(&snapshot).unwrap();
        assert!(!serialized.contains("cumulativeTokenUsage"));
        assert!(!serialized.contains("totalTokens"));
        assert!(!serialized.contains(private_content));
        assert!(!serialized.contains("private-request-id"));
    }

    #[test]
    fn cumulative_usage_budget_selects_newest_candidates_across_channels() {
        let temp = tempfile::tempdir().unwrap();
        let stable = create_channel_database(temp.path(), "0-stable");
        let preview = create_channel_database(temp.path(), "0-preview");
        let nightly = create_channel_database(temp.path(), "0-nightly");
        let workspace = temp.path().join("project");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);

        for index in 0..MAX_MODEL_ROWS_PER_REFRESH {
            let session = format!("stable-usage-{index}");
            let updated_at = format!("2026-01-02 05:00:{index:02}");
            insert_thread(
                &stable,
                ThreadFixture {
                    id: index as u8,
                    paths: &paths,
                    order: &order,
                    agent_id: None,
                    updated_at: &updated_at,
                    archived: false,
                    remote: None,
                    title: "private stable title",
                    session: &session,
                    main_paths: None,
                    main_order: None,
                },
            );
        }
        for (database, id, updated_at, session) in [
            (&preview, 10, "2026-01-02 06:00:00", "preview-usage"),
            (&nightly, 11, "2026-01-02 07:00:00", "nightly-usage"),
        ] {
            insert_thread(
                database,
                ThreadFixture {
                    id,
                    paths: &paths,
                    order: &order,
                    agent_id: None,
                    updated_at,
                    archived: false,
                    remote: None,
                    title: "private non-stable title",
                    session,
                    main_paths: None,
                    main_order: None,
                },
            );
        }

        let threads = create_threads_database(temp.path());
        for index in 0..MAX_MODEL_ROWS_PER_REFRESH {
            let session = format!("stable-usage-{index}");
            let updated_at = format!("2026-01-02T05:00:{index:02}Z");
            let body = serde_json::to_vec(&serde_json::json!({
                "cumulative_token_usage": {"input_tokens": 100 + index}
            }))
            .unwrap();
            insert_thread_blob_at(&threads, &session, &updated_at, "json", &body);
        }
        for (session, updated_at, total_tokens) in [
            ("preview-usage", "2026-01-02T06:00:00Z", 600_u64),
            ("nightly-usage", "2026-01-02T07:00:00Z", 700_u64),
        ] {
            let body = serde_json::to_vec(&serde_json::json!({
                "cumulative_token_usage": {"input_tokens": total_tokens}
            }))
            .unwrap();
            insert_thread_blob_at(&threads, session, updated_at, "json", &body);
        }

        let usage = load_zed_usage_from_data_roots(&[temp.path().to_path_buf()], NOW_MS).unwrap();
        assert_eq!(usage.total_tokens, 700);
        assert_eq!(usage.updated_at_ms, 1_767_337_200_000);
    }

    #[test]
    fn cumulative_usage_parser_handles_json_zstd_empty_malformed_and_overflow() {
        let body = serde_json::to_vec(&serde_json::json!({
            "cumulative_token_usage": {
                "input_tokens": 11,
                "output_tokens": 13,
                "cache_creation_input_tokens": 17,
                "cache_read_input_tokens": 19
            },
            "request_token_usage": {"private-request-id": {"input_tokens": 999_999}},
            "messages": [{"User": {"content": "private"}}]
        }))
        .unwrap();
        let mut json_budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
        let json = extract_thread_signal_with_budget("json", &body, &mut json_budget).unwrap();
        assert_eq!(
            json.cumulative_token_usage
                .and_then(ThreadTokenUsage::checked_total_tokens),
            Some(60)
        );

        let compressed = zstd::stream::encode_all(body.as_slice(), 1).unwrap();
        let mut zstd_budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
        let zstd =
            extract_thread_signal_with_budget("zstd", &compressed, &mut zstd_budget).unwrap();
        assert_eq!(zstd.cumulative_token_usage, json.cumulative_token_usage);

        for empty in [
            br#"{}"#.as_slice(),
            br#"{"cumulative_token_usage":{}}"#.as_slice(),
        ] {
            let mut budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
            assert_eq!(
                extract_thread_signal_with_budget("json", empty, &mut budget)
                    .unwrap()
                    .cumulative_token_usage,
                None
            );
        }

        let malformed = br#"{"cumulative_token_usage":{"input_tokens":"private"}}"#;
        let mut malformed_budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
        assert!(
            extract_thread_signal_with_budget("json", malformed, &mut malformed_budget).is_err()
        );

        let overflow = format!(
            "{{\"cumulative_token_usage\":{{\"input_tokens\":{},\"output_tokens\":1}}}}",
            u64::MAX
        );
        let mut overflow_budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
        assert!(extract_thread_signal_with_budget(
            "json",
            overflow.as_bytes(),
            &mut overflow_budget
        )
        .is_err());
    }

    #[test]
    fn native_thread_boundaries_report_activity_then_finished_without_exposing_content() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &paths,
                order: &order,
                agent_id: None,
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "PRIVATE TITLE",
                session: "private-native-join-id",
                main_paths: None,
                main_order: None,
            },
        );
        let threads = create_threads_database(temp.path());
        let private_content = "PRIVATE PROMPT OR RESPONSE MUST NEVER ESCAPE";
        let active_body = serde_json::to_vec(&serde_json::json!({
            "messages": [{
                "User": {
                    "id": "private-user-message-id",
                    "content": [{"Text": private_content}]
                }
            }],
            "model": {"provider": "openai", "model": "gpt-5.4"},
            "version": "0.3.0"
        }))
        .unwrap();
        insert_thread_blob_at(
            &threads,
            "private-native-join-id",
            "2026-01-02T05:00:00Z",
            "json",
            &active_body,
        );

        let active = load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        let agent = active.workspaces[0].agent.as_ref().unwrap();
        assert_eq!(agent.lifecycle, Some(ZedAgentLifecycle::ActivityDetected));
        assert_eq!(agent.lifecycle_changed_at_ms, Some(1_767_330_000_000));
        assert_eq!(agent.model_provider.as_deref(), Some("openai"));
        assert_eq!(agent.model_name.as_deref(), Some("gpt-5.4"));
        let serialized = serde_json::to_string(&active).unwrap();
        assert!(!serialized.contains(private_content));
        assert!(!serialized.contains("private-user-message-id"));
        assert!(!serialized.contains("private-native-join-id"));

        let finished_body = serde_json::to_vec(&serde_json::json!({
            "messages": [
                {"User": {"content": [{"Text": private_content}]}},
                {"Agent": {
                    "content": [{"Text": private_content}],
                    "tool_results": {}
                }}
            ],
            "model": {"provider": "anthropic", "model": "claude-sonnet-4"},
            "version": "0.3.0"
        }))
        .unwrap();
        Connection::open(&threads)
            .unwrap()
            .execute(
                "UPDATE threads SET updated_at = ?1, data = ?2 WHERE id = ?3",
                params![
                    "2026-01-02T05:00:03Z",
                    finished_body,
                    "private-native-join-id"
                ],
            )
            .unwrap();

        let finished =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        let agent = finished.workspaces[0].agent.as_ref().unwrap();
        assert_eq!(agent.lifecycle, Some(ZedAgentLifecycle::TurnFinished));
        assert_eq!(agent.lifecycle_changed_at_ms, Some(1_767_330_003_000));
        assert_eq!(agent.model_provider.as_deref(), Some("anthropic"));
        assert_eq!(agent.model_name.as_deref(), Some("claude-sonnet-4"));
        assert!(!serde_json::to_string(&finished)
            .unwrap()
            .contains(private_content));
    }

    #[test]
    fn fractional_timestamps_preserve_the_native_save_race() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02T05:00:00.900Z",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &paths,
                order: &order,
                agent_id: None,
                updated_at: "2026-01-02T05:00:00.900Z",
                archived: false,
                remote: None,
                title: "private",
                session: "fractional-race",
                main_paths: None,
                main_order: None,
            },
        );
        let threads = create_threads_database(temp.path());
        let previous_assistant = serde_json::to_vec(&serde_json::json!({
            "messages": [{"Agent": {"content": [{"Text": "private"}]}}]
        }))
        .unwrap();
        insert_thread_blob_at(
            &threads,
            "fractional-race",
            "2026-01-02T05:00:00.100Z",
            "json",
            &previous_assistant,
        );

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, true);
        let agent = snapshot.workspaces[0].agent.as_ref().unwrap();
        assert_eq!(agent.interacted_at_ms, Some(1_767_330_000_900));
        assert_eq!(agent.lifecycle, Some(ZedAgentLifecycle::ActivityDetected));
        assert_eq!(agent.lifecycle_changed_at_ms, Some(1_767_330_000_900));
    }

    #[test]
    fn native_lifecycle_uses_structural_tail_and_fails_closed_on_unknown_messages() {
        let user = serde_json::to_vec(&serde_json::json!({
            "messages": [{"User": {"content": [{"Text": "private"}]}}]
        }))
        .unwrap();
        let resume = serde_json::to_vec(&serde_json::json!({
            "messages": ["Resume"]
        }))
        .unwrap();
        let compaction = serde_json::to_vec(&serde_json::json!({
            "messages": [{"Compaction": {"Summary": "private"}}]
        }))
        .unwrap();
        let tool = serde_json::to_vec(&serde_json::json!({
            "messages": [{"Agent": {
                "content": [{"ToolUse": {"id": "private", "input": "private"}}],
                "tool_results": {}
            }}]
        }))
        .unwrap();
        let finished = serde_json::to_vec(&serde_json::json!({
            "messages": [{"Agent": {"content": [{"Text": "private"}]}}]
        }))
        .unwrap();
        let unknown = serde_json::to_vec(&serde_json::json!({
            "messages": [{"FutureMessageVariant": {"private": "private"}}]
        }))
        .unwrap();
        let malformed_agent = serde_json::to_vec(&serde_json::json!({
            "messages": [{"Agent": {"tool_results": {}}}]
        }))
        .unwrap();
        let unknown_agent_content = serde_json::to_vec(&serde_json::json!({
            "messages": [{"Agent": {"content": [{"FutureContent": "private"}]}}]
        }))
        .unwrap();

        let mut budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
        assert_eq!(
            extract_thread_signal_with_budget("json", &user, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::User
        );
        assert_eq!(
            extract_thread_signal_with_budget("json", &resume, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::Resume
        );
        assert_eq!(
            extract_thread_signal_with_budget("json", &compaction, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::Compaction
        );
        assert_eq!(
            extract_thread_signal_with_budget("json", &tool, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::Agent { has_tool_use: true }
        );
        assert_eq!(
            extract_thread_signal_with_budget("json", &finished, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::Agent {
                has_tool_use: false
            }
        );
        assert_eq!(
            extract_thread_signal_with_budget("json", &unknown, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::Unknown
        );
        assert_eq!(
            extract_thread_signal_with_budget("json", &malformed_agent, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::Unknown
        );
        assert_eq!(
            extract_thread_signal_with_budget("json", &unknown_agent_content, &mut budget)
                .unwrap()
                .tail,
            ThreadTail::Unknown
        );

        assert_eq!(
            derive_native_lifecycle(Some(20), Some(10), ThreadTail::Resume),
            Some((ZedAgentLifecycle::ActivityDetected, 20))
        );
        assert_eq!(
            derive_native_lifecycle(Some(20), Some(25), ThreadTail::Compaction),
            None
        );

        assert_eq!(
            derive_native_lifecycle(
                Some(20),
                Some(10),
                ThreadTail::Agent {
                    has_tool_use: false
                }
            ),
            Some((ZedAgentLifecycle::ActivityDetected, 20))
        );
        assert_eq!(
            derive_native_lifecycle(Some(20), Some(25), ThreadTail::Agent { has_tool_use: true }),
            Some((ZedAgentLifecycle::ActivityDetected, 25))
        );
        assert_eq!(
            derive_native_lifecycle(
                Some(20),
                Some(25),
                ThreadTail::Agent {
                    has_tool_use: false
                }
            ),
            Some((ZedAgentLifecycle::TurnFinished, 25))
        );
        assert_eq!(
            derive_native_lifecycle(Some(20), Some(25), ThreadTail::Unknown),
            None
        );
        assert_eq!(
            derive_native_lifecycle(None, Some(25), ThreadTail::User),
            None
        );
    }

    #[test]
    fn model_join_is_limited_to_native_activity_for_loaded_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let loaded = temp.path().join("loaded");
        let unmatched = temp.path().join("unmatched");
        let database = create_channel_database(temp.path(), "0-stable");
        let (loaded_paths, loaded_order) = encoded_paths(&[loaded.as_path()]);
        let (unmatched_paths, unmatched_order) = encoded_paths(&[unmatched.as_path()]);
        insert_workspace(
            &database,
            1,
            &loaded_paths,
            &loaded_order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &loaded_paths,
                order: &loaded_order,
                agent_id: Some("registry:codex"),
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "private",
                session: "external-join",
                main_paths: None,
                main_order: None,
            },
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 2,
                paths: &unmatched_paths,
                order: &unmatched_order,
                agent_id: None,
                updated_at: "2026-01-02 06:00:00",
                archived: false,
                remote: None,
                title: "private",
                session: "unmatched-native-join",
                main_paths: None,
                main_order: None,
            },
        );
        let threads = create_threads_database(temp.path());
        let body = serde_json::to_vec(&serde_json::json!({
            "model": {"provider": "openai", "model": "gpt"}
        }))
        .unwrap();
        insert_thread_model_blob(&threads, "external-join", "json", &body);
        insert_thread_model_blob(&threads, "unmatched-native-join", "json", &body);

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.workspaces.len(), 1);
        let activity = snapshot.workspaces[0].agent.as_ref().unwrap();
        assert_eq!(activity.agent_kind, "registry:codex");
        assert_eq!(activity.lifecycle, None);
        assert_eq!(activity.model_name, None);
        assert_eq!(snapshot.diagnostics.model_rows_considered, 0);
    }

    #[test]
    fn workspace_only_loader_skips_sidebar_and_model_reads() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &paths,
                order: &order,
                agent_id: None,
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "private",
                session: "native-join",
                main_paths: None,
                main_order: None,
            },
        );
        let threads = create_threads_database(temp.path());
        insert_thread_model_blob(
            &threads,
            "native-join",
            "json",
            br#"{"model":{"provider":"openai","model":"gpt"}}"#,
        );

        let snapshot = load_zed_snapshot_from_data_roots_with_detail(
            &[temp.path().to_path_buf()],
            NOW_MS,
            false,
            SnapshotDetail::WorkspacesOnly,
        );
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].agent, None);
        assert_eq!(snapshot.diagnostics.agent_metadata_channels, 0);
        assert_eq!(snapshot.diagnostics.model_rows_considered, 0);
    }

    #[test]
    fn model_rows_use_newest_first_aggregate_refresh_budget() {
        let temp = tempfile::tempdir().unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let threads = create_threads_database(temp.path());
        for index in 0..(MAX_MODEL_ROWS_PER_REFRESH + 2) {
            let workspace = temp.path().join(format!("project-{index}"));
            let (paths, order) = encoded_paths(&[workspace.as_path()]);
            insert_workspace(
                &database,
                index as i64,
                &paths,
                &order,
                None,
                None,
                None,
                "2026-01-02 03:04:05",
            );
            let session = format!("native-{index}");
            let updated_at = format!("2026-01-02 05:00:{index:02}");
            insert_thread(
                &database,
                ThreadFixture {
                    id: index as u8,
                    paths: &paths,
                    order: &order,
                    agent_id: None,
                    updated_at: &updated_at,
                    archived: false,
                    remote: None,
                    title: "private",
                    session: &session,
                    main_paths: None,
                    main_order: None,
                },
            );
            let body = serde_json::to_vec(&serde_json::json!({
                "model": {"provider": "openai", "model": format!("model-{index}")}
            }))
            .unwrap();
            insert_thread_model_blob(&threads, &session, "json", &body);
        }

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(
            snapshot.diagnostics.model_rows_considered,
            MAX_MODEL_ROWS_PER_REFRESH
        );
        assert_eq!(
            snapshot.diagnostics.models_loaded,
            MAX_MODEL_ROWS_PER_REFRESH
        );
        let loaded_models: BTreeSet<_> = snapshot
            .workspaces
            .iter()
            .filter_map(|workspace| workspace.agent.as_ref()?.model_name.clone())
            .collect();
        assert_eq!(
            loaded_models,
            (2..6).map(|index| format!("model-{index}")).collect()
        );
    }

    #[test]
    fn oversized_thread_data_type_is_rejected_before_string_allocation() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &paths,
                order: &order,
                agent_id: None,
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "private",
                session: "native-join",
                main_paths: None,
                main_order: None,
            },
        );
        let threads = create_threads_database(temp.path());
        insert_thread_model_blob(
            &threads,
            "native-join",
            &"x".repeat(MAX_THREAD_DATA_TYPE_BYTES + 1),
            b"{}",
        );

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.diagnostics.model_rows_considered, 1);
        assert_eq!(snapshot.diagnostics.models_loaded, 0);
        assert_eq!(snapshot.diagnostics.malformed_model_rows, 1);
    }

    #[test]
    fn bounded_zstd_model_is_decoded_and_oversize_or_invalid_models_fail_closed() {
        let private = "private prompt".repeat(1_024);
        let body = serde_json::to_vec(&serde_json::json!({
            "messages": [{"User": {"content": [{"Text": private}]}}],
            "model": {"provider": "anthropic", "model": "claude-sonnet-4-6"}
        }))
        .unwrap();
        let compressed = zstd::stream::encode_all(body.as_slice(), 1).unwrap();
        assert_eq!(
            extract_thread_model("zstd", &compressed).unwrap(),
            Some(ThreadModel {
                provider: "anthropic".to_string(),
                name: "claude-sonnet-4-6".to_string(),
            })
        );
        let mut signal_budget = MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH;
        assert_eq!(
            extract_thread_signal_with_budget("zstd", &compressed, &mut signal_budget)
                .unwrap()
                .tail,
            ThreadTail::User
        );

        let invalid = serde_json::to_vec(&serde_json::json!({
            "model": {"provider": "openai", "model": "x".repeat(MAX_MODEL_VALUE_BYTES + 1)}
        }))
        .unwrap();
        assert!(extract_thread_model("json", &invalid).is_err());
        assert!(extract_thread_model("unknown", b"{}").is_err());

        // Highly compressible input still cannot expand beyond the independent
        // decompressed-size cap.
        let bomb = serde_json::to_vec(&serde_json::json!({
            "messages": ["x".repeat(MAX_MODEL_DECOMPRESSED_BYTES_PER_REFRESH + 1)],
            "model": {"provider": "openai", "model": "gpt"}
        }))
        .unwrap();
        let bomb = zstd::stream::encode_all(bomb.as_slice(), 1).unwrap();
        assert!(extract_thread_model("zstd", &bomb).is_err());
    }

    #[test]
    fn decompressed_model_work_uses_one_shared_budget() {
        let body = serde_json::to_vec(&serde_json::json!({
            "messages": [{"text": "private".repeat(128)}],
            "model": {"provider": "openai", "model": "gpt"}
        }))
        .unwrap();
        let mut budget = body.len() + body.len() / 2;
        assert!(extract_thread_model_with_budget("json", &body, &mut budget)
            .unwrap()
            .is_some());
        assert!(budget < body.len());
        assert!(extract_thread_model_with_budget("json", &body, &mut budget).is_err());
        assert_eq!(budget, 0);
    }

    #[test]
    fn opaque_instance_id_is_deterministic_and_contains_no_path_or_zed_ids() {
        let paths = vec![PathBuf::from("/private/project")];
        let first = zed_instance_id("0-stable", &paths);
        assert_eq!(first, zed_instance_id("0-stable", &paths));
        assert_ne!(first, zed_instance_id("0-preview", &paths));
        assert!(first.starts_with("zed:"));
        assert_eq!(first.len(), 68);
        assert!(!first.contains("private"));
    }

    #[test]
    fn archived_and_remote_threads_and_workspaces_are_omitted() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("local");
        let remote = temp.path().join("remote-shaped");
        fs::create_dir(&local).unwrap();
        fs::create_dir(&remote).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let (local_paths, local_order) = encoded_paths(&[local.as_path()]);
        let (remote_paths, remote_order) = encoded_paths(&[remote.as_path()]);
        insert_workspace(
            &database,
            1,
            &local_paths,
            &local_order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_workspace(
            &database,
            2,
            &remote_paths,
            &remote_order,
            None,
            None,
            Some(4),
            "2026-01-02 04:04:05",
        );
        for (id, archived, remote_connection) in [(1, true, None), (2, false, Some("{remote}"))] {
            insert_thread(
                &database,
                ThreadFixture {
                    id,
                    paths: &local_paths,
                    order: &local_order,
                    agent_id: None,
                    updated_at: "2026-01-02 05:00:00",
                    archived,
                    remote: remote_connection,
                    title: "private",
                    session: "private-session",
                    main_paths: None,
                    main_order: None,
                },
            );
        }

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].paths, vec![local]);
        assert_eq!(snapshot.workspaces[0].agent, None);
    }

    #[test]
    fn main_worktree_paths_can_associate_linked_worktree_activity() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let linked = temp.path().join("linked");
        fs::create_dir(&main).unwrap();
        fs::create_dir(&linked).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let (main_paths, main_order) = encoded_paths(&[main.as_path()]);
        let (linked_paths, linked_order) = encoded_paths(&[linked.as_path()]);
        insert_workspace(
            &database,
            1,
            &main_paths,
            &main_order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &linked_paths,
                order: &linked_order,
                agent_id: Some("registry:claude"),
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "private",
                session: "private",
                main_paths: Some(&main_paths),
                main_order: Some(&main_order),
            },
        );
        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(
            snapshot.workspaces[0].agent.as_ref().unwrap().agent_kind,
            "registry:claude"
        );
    }

    #[test]
    fn malformed_agent_id_is_reduced_to_generic_external_label() {
        assert_eq!(agent_kind(None), "external");
        assert_eq!(agent_kind(Some("registry:codex")), "registry:codex");
        assert_eq!(agent_kind(Some("private agent\nlabel")), "external");
        assert_eq!(
            agent_kind(Some(&"x".repeat(MAX_AGENT_ID_BYTES + 1))),
            "external"
        );
    }

    #[test]
    fn global_and_malformed_channels_are_not_read() {
        let temp = tempfile::tempdir().unwrap();
        create_channel_database(temp.path(), "0-global");
        create_channel_database(temp.path(), "0-canary");
        let stable = create_channel_database(temp.path(), "0-stable");
        let workspace = temp.path().join("project");
        fs::create_dir(&workspace).unwrap();
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &stable,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        fs::create_dir_all(temp.path().join("db").join("bad channel")).unwrap();

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.diagnostics.channel_candidates, 1);
        assert_eq!(snapshot.diagnostics.channels_loaded, 1);
        assert_eq!(snapshot.workspaces[0].channel, "0-stable");
    }

    #[test]
    fn schema_drift_fails_closed_without_panicking() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("db").join("0-stable");
        fs::create_dir_all(&directory).unwrap();
        let connection = Connection::open(directory.join("db.sqlite")).unwrap();
        connection
            .execute_batch("CREATE TABLE workspaces(workspace_id INTEGER PRIMARY KEY, paths TEXT);")
            .unwrap();
        drop(connection);

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, true);
        assert!(snapshot.workspaces.is_empty());
        assert_eq!(snapshot.diagnostics.malformed_channels, 1);
    }

    #[test]
    fn malformed_paths_are_rejected_and_malformed_order_is_not_openable() {
        assert!(decode_path_list("relative/path", "0").is_none());
        assert!(decode_path_list("/tmp/../private", "0").is_none());
        assert!(decode_path_list("/tmp/control\tpath", "0").is_none());

        let temp = tempfile::tempdir().unwrap();
        let alpha = temp.path().join("alpha");
        let beta = temp.path().join("beta");
        fs::create_dir(&alpha).unwrap();
        fs::create_dir(&beta).unwrap();
        let encoded = format!("{}\n{}", alpha.display(), beta.display());
        let decoded = decode_path_list(&encoded, "4,0").unwrap();
        assert!(!decoded.exact_order);
        assert_eq!(decoded.ordered_paths, vec![alpha, beta]);
    }

    #[test]
    fn future_timestamps_do_not_escape_the_adapter_bound() {
        assert_eq!(bounded_timestamp(NOW_MS, NOW_MS), Some(NOW_MS));
        assert_eq!(
            bounded_timestamp(NOW_MS + MAX_FUTURE_SKEW_MS + 1, NOW_MS),
            None
        );
        assert_eq!(bounded_timestamp(-1, NOW_MS), None);
    }

    #[test]
    fn ambiguous_live_channels_fail_closed_and_preserve_channel_identity() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        fs::create_dir(&workspace).unwrap();
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        for (channel, timestamp) in [
            ("0-stable", "2026-01-02 03:04:05"),
            ("0-preview", "2026-01-03 03:04:05"),
        ] {
            let database = create_channel_database(temp.path(), channel);
            set_session(&database, "current", "[7]");
            insert_workspace(
                &database,
                1,
                &paths,
                &order,
                Some("current"),
                Some(7),
                None,
                timestamp,
            );
        }
        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, true);
        assert_eq!(snapshot.workspaces.len(), 2);
        assert!(snapshot.workspaces.iter().all(|workspace| !workspace.open));
        assert_eq!(
            snapshot
                .workspaces
                .iter()
                .map(|workspace| workspace.channel.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["0-preview", "0-stable"])
        );
        assert_eq!(snapshot.diagnostics.ambiguous_live_channels, 2);
    }

    #[test]
    fn generic_process_signal_never_marks_non_stable_channel_open() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        let database = create_channel_database(temp.path(), "0-preview");
        set_session(&database, "current", "[7]");
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            Some("current"),
            Some(7),
            None,
            "2026-01-02 03:04:05",
        );

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, true);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].channel, "0-preview");
        assert!(!snapshot.workspaces[0].open);
        assert_eq!(snapshot.workspaces[0].window_stack_index, None);
    }

    #[test]
    fn workspace_query_is_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        for id in 0..(MAX_WORKSPACES_PER_CHANNEL + 8) {
            let workspace = temp.path().join(format!("project-{id:04}"));
            let (paths, order) = encoded_paths(&[workspace.as_path()]);
            insert_workspace(
                &database,
                id as i64,
                &paths,
                &order,
                None,
                None,
                None,
                "2026-01-02 03:04:05",
            );
        }
        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.workspaces.len(), MAX_WORKSPACES_PER_CHANNEL);
        assert_eq!(snapshot.diagnostics.omitted_workspaces, 1);
    }

    #[test]
    fn oversized_timestamp_fields_are_filtered_before_sql_date_work() {
        let temp = tempfile::tempdir().unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let valid_workspace = temp.path().join("valid");
        let oversized_workspace = temp.path().join("oversized");
        let (valid_paths, valid_order) = encoded_paths(&[valid_workspace.as_path()]);
        let (oversized_paths, oversized_order) = encoded_paths(&[oversized_workspace.as_path()]);
        let oversized_timestamp = "2".repeat(MAX_TIMESTAMP_BYTES + 1);

        insert_workspace(
            &database,
            1,
            &valid_paths,
            &valid_order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        insert_workspace(
            &database,
            2,
            &oversized_paths,
            &oversized_order,
            None,
            None,
            None,
            &oversized_timestamp,
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 1,
                paths: &valid_paths,
                order: &valid_order,
                agent_id: Some("registry:valid"),
                updated_at: "2026-01-02 05:00:00",
                archived: false,
                remote: None,
                title: "private",
                session: "valid-session",
                main_paths: None,
                main_order: None,
            },
        );
        insert_thread(
            &database,
            ThreadFixture {
                id: 2,
                paths: &valid_paths,
                order: &valid_order,
                agent_id: Some("registry:oversized"),
                updated_at: &oversized_timestamp,
                archived: false,
                remote: None,
                title: "private",
                session: "oversized-session",
                main_paths: None,
                main_order: None,
            },
        );
        Connection::open(&database)
            .unwrap()
            .execute(
                "UPDATE sidebar_threads SET interacted_at = ?1 WHERE agent_id = ?2",
                params![oversized_timestamp, "registry:valid"],
            )
            .unwrap();

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].paths, vec![valid_workspace]);
        let agent = snapshot.workspaces[0].agent.as_ref().unwrap();
        assert_eq!(agent.agent_kind, "registry:valid");
        assert_eq!(agent.changed_at_ms, 1_767_330_000_000);
    }

    #[test]
    fn oversized_database_and_auxiliary_files_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("db").join("0-stable");
        fs::create_dir_all(&directory).unwrap();
        File::create(directory.join("db.sqlite"))
            .unwrap()
            .set_len(MAX_DATABASE_BYTES + 1)
            .unwrap();
        let oversized =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(oversized.diagnostics.malformed_channels, 1);

        fs::remove_file(directory.join("db.sqlite")).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        File::create(database.with_file_name("db.sqlite-wal"))
            .unwrap()
            .set_len(MAX_DATABASE_WAL_BYTES + 1)
            .unwrap();
        let oversized_wal =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(oversized_wal.diagnostics.malformed_channels, 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_database_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let real_root = tempfile::tempdir().unwrap();
        let real_database = create_channel_database(real_root.path(), "0-stable");
        let directory = temp.path().join("db").join("0-stable");
        fs::create_dir_all(&directory).unwrap();
        symlink(&real_database, directory.join("db.sqlite")).unwrap();

        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert!(snapshot.workspaces.is_empty());
        assert_eq!(snapshot.diagnostics.malformed_channels, 1);
    }

    #[cfg(unix)]
    #[test]
    fn read_only_database_permissions_are_supported_without_mutation() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("project");
        fs::create_dir(&workspace).unwrap();
        let database = create_channel_database(temp.path(), "0-stable");
        let (paths, order) = encoded_paths(&[workspace.as_path()]);
        insert_workspace(
            &database,
            1,
            &paths,
            &order,
            None,
            None,
            None,
            "2026-01-02 03:04:05",
        );
        let before = fs::read(&database).unwrap();
        fs::set_permissions(&database, fs::Permissions::from_mode(0o444)).unwrap();
        let snapshot =
            load_zed_snapshot_from_data_roots(&[temp.path().to_path_buf()], NOW_MS, false);
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(fs::read(&database).unwrap(), before);
    }
}
