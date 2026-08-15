//! Experimental, read-only monitoring for Cursor's local Desktop Bridge.
//!
//! Cursor does not currently document this protocol for third-party clients.
//! The integration is therefore explicitly opt-in and deliberately limited to
//! `listThreads`. Discovery credentials and raw thread/window identifiers stay
//! inside this module; callers receive only pseudonymized thread keys and
//! coarse status values.

use serde::{de::IgnoredAny, Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
#[cfg(any(unix, windows))]
use std::thread;
use std::time::{Duration, Instant};
use tempfile::Builder as TempFileBuilder;

#[cfg(unix)]
use std::io::{self, Read};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as WindowsMetadataExt;

const PREFERENCE_FILENAME: &str = "cursor-agents-monitoring.json";
const PREFERENCE_SCHEMA_VERSION: u32 = 1;
const BRIDGE_PROTOCOL_VERSION: u32 = 1;
const MAX_PREFERENCE_BYTES: u64 = 1_024;
const MAX_DISCOVERY_BYTES: u64 = 16 * 1_024;
const MAX_DISCOVERY_FILES: usize = 32;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_THREADS_ACROSS_INSTANCES: usize = 200 * MAX_DISCOVERY_FILES;
const MAX_IDENTIFIER_BYTES: usize = 16 * 1024;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_APP_FIELD_BYTES: usize = 128;
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_COOLDOWN: Duration = Duration::from_secs(2);
const FAILED_POLL_GRACE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorBridgeThreadStatus {
    Idle,
    Running,
    Completed,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorBridgeThreadSource {
    Local,
    Cloud,
    Draft,
    ClaudeCode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorBridgeThread {
    /// SHA-256 of Cursor's raw thread ID, matching Cursor hook `sessionKey`.
    pub(crate) session_key: String,
    pub(crate) source: CursorBridgeThreadSource,
    pub(crate) status: CursorBridgeThreadStatus,
    pub(crate) last_updated_at_ms: i64,
    /// Bridge-instance-scoped hash of Cursor's window ID. This is never
    /// persisted or exposed and exists only to avoid merging distinct live
    /// windows, including windows whose numeric IDs collide across processes.
    pub(crate) window_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorAgentsBridgeSnapshot {
    /// Time at which a bridge instance successfully answered `listThreads`.
    pub(crate) observed_at_ms: i64,
    pub(crate) threads: Vec<CursorBridgeThread>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CursorAgentsBridgeStatusView {
    pub(crate) schema_version: u32,
    pub(crate) enabled: bool,
    pub(crate) availability: String,
    pub(crate) connected: bool,
    pub(crate) instance_count: usize,
    pub(crate) thread_count: usize,
    pub(crate) last_checked_at_ms: Option<i64>,
    pub(crate) error_code: Option<String>,
    pub(crate) detail: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CursorAgentsBridgePoll {
    pub(crate) status: CursorAgentsBridgeStatusView,
    pub(crate) snapshot: Option<CursorAgentsBridgeSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preference {
    schema_version: u32,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryRecord {
    protocol_version: u32,
    pid: i64,
    socket_path: String,
    token: String,
    app_name: String,
    app_version: String,
    user_data_dir: String,
    created_at: f64,
}

#[derive(Debug, Deserialize)]
struct ListThreadsResponse {
    #[serde(deserialize_with = "deserialize_threads")]
    threads: Vec<RawThread>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThread {
    id: String,
    #[serde(deserialize_with = "deserialize_ignored_string")]
    title: IgnoredAny,
    source: RawThreadSource,
    status: RawThreadStatus,
    last_updated_at: f64,
    window_id: i64,
}

fn deserialize_ignored_string<'de, D>(deserializer: D) -> Result<IgnoredAny, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let _ = String::deserialize(deserializer)?;
    Ok(IgnoredAny)
}

fn deserialize_threads<'de, D>(deserializer: D) -> Result<Vec<RawThread>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{SeqAccess, Visitor};
    use std::fmt;

    struct ThreadsVisitor;

    impl<'de> Visitor<'de> for ThreadsVisitor {
        type Value = Vec<RawThread>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded array of Cursor agent threads")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut threads = Vec::new();
            while threads.len() < MAX_THREADS_ACROSS_INSTANCES {
                let Some(thread) = sequence.next_element::<RawThread>()? else {
                    return Ok(threads);
                };
                threads.push(thread);
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom(
                    "Cursor bridge returned too many threads",
                ));
            }
            Ok(threads)
        }
    }

    deserializer.deserialize_seq(ThreadsVisitor)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum RawThreadSource {
    Local,
    Cloud,
    Draft,
    ClaudeCode,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawThreadStatus {
    Idle,
    Running,
    Completed,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
struct BridgeFailure {
    code: &'static str,
}

impl BridgeFailure {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

#[derive(Debug, Clone)]
struct SuccessfulProbe {
    checked_at: Instant,
    snapshot: CursorAgentsBridgeSnapshot,
}

#[derive(Debug, Clone)]
struct CachedPoll {
    state_root: PathBuf,
    bridge_dir: PathBuf,
    attempted_at: Instant,
    poll: CursorAgentsBridgePoll,
    last_success: Option<SuccessfulProbe>,
}

static POLL_CACHE: OnceLock<Mutex<Option<CachedPoll>>> = OnceLock::new();

pub(crate) fn poll(now_ms: i64, force: bool) -> CursorAgentsBridgePoll {
    let state_root = match crate::state::state_dir_from_environment() {
        Ok(path) => path,
        Err(_) => return preference_error("state_directory_unavailable"),
    };
    let enabled = match read_preference(&state_root) {
        Ok(value) => value,
        Err(error) => return preference_error(error.code),
    };
    if !enabled {
        return disabled_poll();
    }

    let bridge_dir = match bridge_directory_from_environment() {
        Ok(path) => path,
        Err(error) => return failed_poll(now_ms, "error", error.code, None),
    };
    poll_with_cache(&state_root, &bridge_dir, now_ms, force)
}

pub(crate) fn set_enabled(enabled: bool, now_ms: i64) -> Result<CursorAgentsBridgePoll, String> {
    let state_root = crate::state::state_dir_from_environment()?;
    write_preference(&state_root, enabled).map_err(|error| {
        format!(
            "could not save Cursor Agents Window monitoring preference ({})",
            error.code
        )
    })?;
    clear_cache();
    Ok(if enabled {
        poll(now_ms, true)
    } else {
        disabled_poll()
    })
}

fn poll_with_cache(
    state_root: &Path,
    bridge_dir: &Path,
    now_ms: i64,
    force: bool,
) -> CursorAgentsBridgePoll {
    let cache = POLL_CACHE.get_or_init(|| Mutex::new(None));
    let now = Instant::now();
    let prior_success = {
        let guard = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !force {
            if let Some(cached) = guard.as_ref().filter(|cached| {
                cached.state_root == state_root
                    && cached.bridge_dir == bridge_dir
                    && now.saturating_duration_since(cached.attempted_at) < POLL_COOLDOWN
            }) {
                return cached.poll.clone();
            }
        }
        guard
            .as_ref()
            .filter(|cached| cached.state_root == state_root && cached.bridge_dir == bridge_dir)
            .and_then(|cached| cached.last_success.clone())
    };
    let mut result = probe(bridge_dir, now_ms);
    let last_success = if let Some(snapshot) = result.snapshot.clone() {
        Some(SuccessfulProbe {
            checked_at: now,
            snapshot,
        })
    } else {
        prior_success
    };

    if result.snapshot.is_none() {
        if let Some(success) = last_success.as_ref().filter(|success| {
            now.saturating_duration_since(success.checked_at) <= FAILED_POLL_GRACE
        }) {
            result.snapshot = Some(success.snapshot.clone());
            result.status.thread_count = success.snapshot.threads.len();
            result.status.detail.push_str(
                " The last successful observation is being retained briefly while Cursor reconnects.",
            );
        }
    }

    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !force {
        if let Some(newer) = guard.as_ref().filter(|cached| {
            cached.state_root == state_root
                && cached.bridge_dir == bridge_dir
                && cached.attempted_at > now
        }) {
            return newer.poll.clone();
        }
    }
    *guard = Some(CachedPoll {
        state_root: state_root.to_path_buf(),
        bridge_dir: bridge_dir.to_path_buf(),
        attempted_at: now,
        poll: result.clone(),
        last_success,
    });
    result
}

fn clear_cache() {
    if let Some(cache) = POLL_CACHE.get() {
        *cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

fn disabled_poll() -> CursorAgentsBridgePoll {
    CursorAgentsBridgePoll {
        status: CursorAgentsBridgeStatusView {
            schema_version: PREFERENCE_SCHEMA_VERSION,
            enabled: false,
            availability: "disabled".to_string(),
            connected: false,
            instance_count: 0,
            thread_count: 0,
            last_checked_at_ms: None,
            error_code: None,
            detail: "Experimental Cursor Agents Window monitoring is off.".to_string(),
        },
        snapshot: None,
    }
}

fn preference_error(code: &'static str) -> CursorAgentsBridgePoll {
    CursorAgentsBridgePoll {
        status: CursorAgentsBridgeStatusView {
            schema_version: PREFERENCE_SCHEMA_VERSION,
            enabled: false,
            availability: "error".to_string(),
            connected: false,
            instance_count: 0,
            thread_count: 0,
            last_checked_at_ms: None,
            error_code: Some(code.to_string()),
            detail: "The private monitoring preference could not be read safely; monitoring remains off."
                .to_string(),
        },
        snapshot: None,
    }
}

fn failed_poll(
    now_ms: i64,
    availability: &str,
    code: &'static str,
    instance_count: Option<usize>,
) -> CursorAgentsBridgePoll {
    let detail = match availability {
        "unsupported" => {
            "This build cannot connect to Cursor's local Desktop Bridge on this platform."
        }
        "waiting" => {
            "Cursor's local Desktop Bridge is not available or has not started. Cursor exposes this private feature only to a limited server-controlled rollout. If Desktop Bridge is absent from Cursor Settings > Beta, live agent-thread monitoring is unavailable in this Cursor installation; Cursor hooks remain the recent, hook-only fallback."
        }
        _ => {
            "Cursor's local Desktop Bridge could not be read safely. Cursor may be restarting or its private protocol may have changed."
        }
    };
    CursorAgentsBridgePoll {
        status: CursorAgentsBridgeStatusView {
            schema_version: PREFERENCE_SCHEMA_VERSION,
            enabled: true,
            availability: availability.to_string(),
            connected: false,
            instance_count: instance_count.unwrap_or(0),
            thread_count: 0,
            last_checked_at_ms: Some(now_ms),
            error_code: Some(code.to_string()),
            detail: detail.to_string(),
        },
        snapshot: None,
    }
}

fn probe(bridge_dir: &Path, now_ms: i64) -> CursorAgentsBridgePoll {
    #[cfg(not(any(unix, windows)))]
    {
        let _ = bridge_dir;
        return failed_poll(now_ms, "unsupported", "unsupported_platform", None);
    }

    #[cfg(any(unix, windows))]
    {
        let discoveries = match read_discoveries(bridge_dir, now_ms) {
            Ok(records) => records,
            Err(error) if matches!(error.code, "bridge_not_found" | "no_live_bridge") => {
                return failed_poll(now_ms, "waiting", error.code, None)
            }
            Err(error) => return failed_poll(now_ms, "error", error.code, None),
        };

        let mut connected = 0usize;
        let mut newest_by_key: HashMap<(String, String), CursorBridgeThread> = HashMap::new();
        thread::scope(|scope| {
            let mut discoveries = discoveries.iter();
            loop {
                let handles: Vec<_> = discoveries
                    .by_ref()
                    .take(4)
                    .map(|discovery| scope.spawn(move || request_threads(discovery, now_ms)))
                    .collect();
                if handles.is_empty() {
                    break;
                }
                for threads in handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .flatten()
                {
                    connected += 1;
                    for thread in threads {
                        let key = (thread.session_key.clone(), thread.window_key.clone());
                        match newest_by_key.get(&key) {
                            Some(existing)
                                if existing.last_updated_at_ms >= thread.last_updated_at_ms => {}
                            _ => {
                                newest_by_key.insert(key, thread);
                            }
                        }
                    }
                }
            }
        });
        if connected == 0 {
            return failed_poll(now_ms, "error", "bridge_connection_failed", None);
        }

        let mut threads: Vec<_> = newest_by_key.into_values().collect();
        threads.sort_by(|left, right| {
            right
                .last_updated_at_ms
                .cmp(&left.last_updated_at_ms)
                .then_with(|| left.session_key.cmp(&right.session_key))
                .then_with(|| left.window_key.cmp(&right.window_key))
        });
        threads.truncate(200);
        let snapshot = CursorAgentsBridgeSnapshot {
            observed_at_ms: now_ms,
            threads,
        };
        CursorAgentsBridgePoll {
            status: CursorAgentsBridgeStatusView {
                schema_version: PREFERENCE_SCHEMA_VERSION,
                enabled: true,
                availability: "connected".to_string(),
                connected: true,
                instance_count: connected,
                thread_count: snapshot.threads.len(),
                last_checked_at_ms: Some(now_ms),
                error_code: None,
                detail: "Connected read-only to Cursor's experimental local Desktop Bridge."
                    .to_string(),
            },
            snapshot: Some(snapshot),
        }
    }
}

fn bridge_directory_from_environment() -> Result<PathBuf, BridgeFailure> {
    if let Some(path) = env::var_os("CURSOR_DESKTOP_BRIDGE_DIR").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        return path
            .is_absolute()
            .then_some(path)
            .ok_or_else(|| BridgeFailure::new("invalid_bridge_directory"));
    }

    #[cfg(target_os = "windows")]
    let home = env::var_os("USERPROFILE").or_else(|| {
        let drive = env::var_os("HOMEDRIVE")?;
        let path = env::var_os("HOMEPATH")?;
        let mut home = PathBuf::from(drive);
        home.push(path);
        Some(home.into_os_string())
    });
    #[cfg(not(target_os = "windows"))]
    let home = env::var_os("HOME");
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| path.join(".cursor").join("desktop-bridge"))
        .ok_or_else(|| BridgeFailure::new("bridge_directory_unavailable"))
}

fn read_preference(state_root: &Path) -> Result<bool, BridgeFailure> {
    let path = state_root.join(PREFERENCE_FILENAME);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(BridgeFailure::new("preference_read_failed")),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_PREFERENCE_BYTES
    {
        return Err(BridgeFailure::new("invalid_preference"));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(BridgeFailure::new("unsafe_preference_permissions"));
    }
    let bytes = fs::read(&path).map_err(|_| BridgeFailure::new("preference_read_failed"))?;
    let preference: Preference =
        serde_json::from_slice(&bytes).map_err(|_| BridgeFailure::new("invalid_preference"))?;
    if preference.schema_version != PREFERENCE_SCHEMA_VERSION {
        return Err(BridgeFailure::new("invalid_preference"));
    }
    Ok(preference.enabled)
}

fn write_preference(state_root: &Path, enabled: bool) -> Result<(), BridgeFailure> {
    match fs::symlink_metadata(state_root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(BridgeFailure::new("unsafe_state_directory"))
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(state_root)
                .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
            #[cfg(unix)]
            fs::set_permissions(state_root, fs::Permissions::from_mode(0o700))
                .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
        }
        Err(_) => return Err(BridgeFailure::new("preference_write_failed")),
    }

    #[cfg(unix)]
    fs::set_permissions(state_root, fs::Permissions::from_mode(0o700))
        .map_err(|_| BridgeFailure::new("preference_write_failed"))?;

    let path = state_root.join(PREFERENCE_FILENAME);
    if fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(BridgeFailure::new("unsafe_preference"));
    }
    let bytes = serde_json::to_vec_pretty(&Preference {
        schema_version: PREFERENCE_SCHEMA_VERSION,
        enabled,
    })
    .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
    let mut temporary = TempFileBuilder::new()
        .prefix(".cursor-agents-monitoring-")
        .tempfile_in(state_root)
        .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
    #[cfg(unix)]
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.write_all(b"\n"))
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
    temporary
        .persist(&path)
        .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|_| BridgeFailure::new("preference_write_failed"))?;
    Ok(())
}

#[cfg(unix)]
fn read_discoveries(bridge_dir: &Path, now_ms: i64) -> Result<Vec<DiscoveryRecord>, BridgeFailure> {
    let directory_metadata = match fs::symlink_metadata(bridge_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(BridgeFailure::new("bridge_not_found"))
        }
        Err(_) => return Err(BridgeFailure::new("bridge_directory_read_failed")),
    };
    if directory_metadata.file_type().is_symlink()
        || !directory_metadata.is_dir()
        || directory_metadata.permissions().mode() & 0o077 != 0
        || directory_metadata.uid() != effective_user_id()
    {
        return Err(BridgeFailure::new("unsafe_bridge_directory"));
    }

    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(bridge_dir).map_err(|_| BridgeFailure::new("bridge_directory_read_failed"))?
    {
        let entry = entry.map_err(|_| BridgeFailure::new("bridge_directory_read_failed"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !valid_discovery_filename(name) {
            continue;
        }
        if candidates.len() == MAX_DISCOVERY_FILES {
            return Err(BridgeFailure::new("too_many_discovery_files"));
        }
        candidates.push(entry.path());
    }
    if candidates.is_empty() {
        return Err(BridgeFailure::new("bridge_not_found"));
    }

    let mut discoveries = Vec::with_capacity(candidates.len());
    let mut saw_invalid = false;
    for path in candidates {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            saw_invalid = true;
            continue;
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > MAX_DISCOVERY_BYTES
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != directory_metadata.uid()
        {
            saw_invalid = true;
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            saw_invalid = true;
            continue;
        };
        let Ok(record) = serde_json::from_slice::<DiscoveryRecord>(&bytes) else {
            saw_invalid = true;
            continue;
        };
        if validate_discovery(&path, &record, now_ms, directory_metadata.uid()).is_err() {
            saw_invalid = true;
            continue;
        }
        discoveries.push(record);
    }
    if discoveries.is_empty() {
        Err(BridgeFailure::new(if saw_invalid {
            "no_live_bridge"
        } else {
            "bridge_not_found"
        }))
    } else {
        Ok(discoveries)
    }
}

#[cfg(windows)]
fn read_discoveries(bridge_dir: &Path, now_ms: i64) -> Result<Vec<DiscoveryRecord>, BridgeFailure> {
    let directory_metadata = match fs::symlink_metadata(bridge_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(BridgeFailure::new("bridge_not_found"))
        }
        Err(_) => return Err(BridgeFailure::new("bridge_directory_read_failed")),
    };
    if windows_link_or_reparse_point(&directory_metadata) || !directory_metadata.is_dir() {
        return Err(BridgeFailure::new("unsafe_bridge_directory"));
    }

    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(bridge_dir).map_err(|_| BridgeFailure::new("bridge_directory_read_failed"))?
    {
        let entry = entry.map_err(|_| BridgeFailure::new("bridge_directory_read_failed"))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !valid_discovery_filename(name) {
            continue;
        }
        if candidates.len() == MAX_DISCOVERY_FILES {
            return Err(BridgeFailure::new("too_many_discovery_files"));
        }
        candidates.push(entry.path());
    }
    if candidates.is_empty() {
        return Err(BridgeFailure::new("bridge_not_found"));
    }

    let mut discoveries = Vec::with_capacity(candidates.len());
    let mut saw_invalid = false;
    for path in candidates {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            saw_invalid = true;
            continue;
        };
        if windows_link_or_reparse_point(&metadata)
            || !metadata.is_file()
            || metadata.len() == 0
            || metadata.len() > MAX_DISCOVERY_BYTES
        {
            saw_invalid = true;
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            saw_invalid = true;
            continue;
        };
        let Ok(record) = serde_json::from_slice::<DiscoveryRecord>(&bytes) else {
            saw_invalid = true;
            continue;
        };
        if validate_windows_discovery(&path, &record, now_ms).is_err() {
            saw_invalid = true;
            continue;
        }
        discoveries.push(record);
    }
    if discoveries.is_empty() {
        Err(BridgeFailure::new(if saw_invalid {
            "no_live_bridge"
        } else {
            "bridge_not_found"
        }))
    } else {
        Ok(discoveries)
    }
}

#[cfg(windows)]
fn windows_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn valid_discovery_filename(name: &str) -> bool {
    name.len() == 21
        && name.ends_with(".json")
        && name[..16]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_common_discovery(
    path: &Path,
    record: &DiscoveryRecord,
    now_ms: i64,
) -> Result<(), BridgeFailure> {
    if record.protocol_version != BRIDGE_PROTOCOL_VERSION
        || record.pid <= 0
        || !valid_lower_hex(&record.token, 64)
        || record.socket_path.is_empty()
        || record.socket_path.len() > MAX_PATH_BYTES
        || record.user_data_dir.is_empty()
        || record.user_data_dir.len() > MAX_PATH_BYTES
        || !Path::new(&record.user_data_dir).is_absolute()
        || record.app_name.is_empty()
        || record.app_name.len() > MAX_APP_FIELD_BYTES
        || !record.app_name.to_ascii_lowercase().contains("cursor")
        || record.app_version.is_empty()
        || record.app_version.len() > MAX_APP_FIELD_BYTES
        || !record.created_at.is_finite()
        || record.created_at < 0.0
        || record.created_at > now_ms.saturating_add(MAX_FUTURE_SKEW_MS) as f64
    {
        return Err(BridgeFailure::new("invalid_discovery"));
    }
    let expected = crate::cursor_integration::cursor_identity_hash(&record.user_data_dir);
    let file_stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if file_stem != &expected[..16] {
        return Err(BridgeFailure::new("invalid_discovery"));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_discovery(
    path: &Path,
    record: &DiscoveryRecord,
    now_ms: i64,
    owner: u32,
) -> Result<(), BridgeFailure> {
    validate_common_discovery(path, record, now_ms)?;
    if !process_is_alive(record.pid) {
        return Err(BridgeFailure::new("stale_discovery"));
    }

    let socket = Path::new(&record.socket_path);
    let socket_name = socket
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !socket.is_absolute()
        || !socket_name.starts_with("vscode-ipc-")
        || !socket_name.ends_with(".sock")
        || !allowed_socket_parent(socket.parent())
    {
        return Err(BridgeFailure::new("unsafe_socket_path"));
    }
    let socket_metadata =
        fs::symlink_metadata(socket).map_err(|_| BridgeFailure::new("stale_discovery"))?;
    if socket_metadata.file_type().is_symlink()
        || !socket_metadata.file_type().is_socket()
        || socket_metadata.uid() != owner
    {
        return Err(BridgeFailure::new("unsafe_socket_path"));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_windows_discovery(
    path: &Path,
    record: &DiscoveryRecord,
    now_ms: i64,
) -> Result<(), BridgeFailure> {
    validate_common_discovery(path, record, now_ms)?;
    if !windows_process_is_alive(record.pid) {
        return Err(BridgeFailure::new("stale_discovery"));
    }
    let pipe_name = record
        .socket_path
        .strip_prefix(r"\\.\pipe\vscode-ipc-")
        .and_then(|value| value.strip_suffix("-sock"))
        .ok_or_else(|| BridgeFailure::new("unsafe_socket_path"))?;
    if pipe_name.is_empty()
        || pipe_name.len() > 128
        || !pipe_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(BridgeFailure::new("unsafe_socket_path"));
    }
    Ok(())
}

#[cfg(windows)]
fn windows_process_is_alive(pid: i64) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    // SAFETY: this opens the supplied PID only for read-only process identity
    // access and closes the returned handle immediately.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    // SAFETY: process was returned by OpenProcess above and is not reused.
    let _ = unsafe { CloseHandle(process) };
    true
}

#[cfg(unix)]
fn process_is_alive(pid: i64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal zero does not alter the target process. The PID is a
    // validated positive value from a private, owner-only discovery file.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn effective_user_id() -> u32 {
    // SAFETY: geteuid has no parameters, side effects, or failure mode.
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn allowed_socket_parent(parent: Option<&Path>) -> bool {
    let Some(parent) = parent else { return false };
    let mut allowed = vec![env::temp_dir()];
    for key in ["XDG_RUNTIME_DIR", "TMPDIR"] {
        if let Some(path) = env::var_os(key).filter(|value| !value.is_empty()) {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                allowed.push(path);
            }
        }
    }
    allowed.into_iter().any(|candidate| candidate == parent)
}

fn valid_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(unix)]
fn request_threads(
    discovery: &DiscoveryRecord,
    now_ms: i64,
) -> Result<Vec<CursorBridgeThread>, BridgeFailure> {
    request_threads_with_timeout(discovery, now_ms, IO_TIMEOUT)
}

#[cfg(unix)]
fn request_threads_with_timeout(
    discovery: &DiscoveryRecord,
    now_ms: i64,
    timeout: Duration,
) -> Result<Vec<CursorBridgeThread>, BridgeFailure> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| BridgeFailure::new("bridge_connection_failed"))?;
    let mut stream = unix_stream_connect_with_deadline(Path::new(&discovery.socket_path), deadline)
        .map_err(|_| BridgeFailure::new("bridge_connection_failed"))?;
    let body = br#"{"type":"listThreads"}"#;
    let request = format!(
        "POST / HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        discovery.token,
        body.len()
    );
    let mut outbound = request.into_bytes();
    outbound.extend_from_slice(body);
    write_all_with_deadline(&mut stream, &outbound, deadline)
        .map_err(|_| BridgeFailure::new("bridge_request_failed"))?;

    let response = read_http_response_with_deadline(&mut stream, deadline)?;
    parse_threads_response(&response, now_ms, &bridge_instance_key(discovery))
}

#[cfg(unix)]
fn unix_stream_connect_with_deadline(path: &Path, deadline: Instant) -> io::Result<UnixStream> {
    let path_bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path_bytes.is_empty()
        || path_bytes.contains(&0)
        || path_bytes.len() >= address.sun_path.len()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Unix socket path",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (destination, source) in address.sun_path.iter_mut().zip(path_bytes) {
        *destination = *source as libc::c_char;
    }
    let address_start = std::ptr::addr_of!(address) as usize;
    let path_start = std::ptr::addr_of!(address.sun_path) as usize;
    let address_length = path_start
        .saturating_sub(address_start)
        .saturating_add(path_bytes.len())
        .saturating_add(1);
    // Match std's pathname layout: the sun_path offset, the path bytes, and
    // their terminating NUL. Darwin derives sockaddr.sa_len from this explicit
    // syscall length, so its zeroed BSD-only sun_len field is intentionally
    // left untouched, as it is by std::os::unix::net and socket2.
    let address_length = libc::socklen_t::try_from(address_length)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Unix socket path is too long"))?;

    // SAFETY: socket returns a new descriptor, which is immediately owned by
    // OwnedFd and closed automatically along every error path.
    let raw_fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw_fd was just returned by socket and has no other owner.
    let socket = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let descriptor_flags = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_GETFD) };
    if descriptor_flags < 0
        || unsafe {
            libc::fcntl(
                socket.as_raw_fd(),
                libc::F_SETFD,
                descriptor_flags | libc::FD_CLOEXEC,
            )
        } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let status_flags = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_GETFL) };
    if status_flags < 0
        || unsafe {
            libc::fcntl(
                socket.as_raw_fd(),
                libc::F_SETFL,
                status_flags | libc::O_NONBLOCK,
            )
        } < 0
    {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: address is initialized as sockaddr_un, address_length covers its
    // family and NUL-terminated path, and socket remains valid for the call.
    let connected = unsafe {
        libc::connect(
            socket.as_raw_fd(),
            std::ptr::addr_of!(address).cast::<libc::sockaddr>(),
            address_length,
        )
    };
    if connected < 0 {
        let error = io::Error::last_os_error();
        let raw_error = error.raw_os_error();
        if ![
            Some(libc::EINPROGRESS),
            Some(libc::EALREADY),
            Some(libc::EINTR),
            Some(libc::EAGAIN),
            Some(libc::EWOULDBLOCK),
        ]
        .contains(&raw_error)
        {
            return Err(error);
        }
        wait_for_unix_connect(&socket, deadline)?;
    }

    if unsafe {
        libc::fcntl(
            socket.as_raw_fd(),
            libc::F_SETFL,
            status_flags & !libc::O_NONBLOCK,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(UnixStream::from(socket))
}

#[cfg(unix)]
fn wait_for_unix_connect(socket: &OwnedFd, deadline: Instant) -> io::Result<()> {
    loop {
        let remaining = remaining_io_time(deadline)?;
        let timeout_ms = remaining
            .as_millis()
            .saturating_add(u128::from(remaining.subsec_nanos() % 1_000_000 != 0))
            .clamp(1, libc::c_int::MAX as u128) as libc::c_int;
        let mut poll_descriptor = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLIN | libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: poll_descriptor points to one initialized pollfd for the
        // duration of the call.
        let result = unsafe { libc::poll(&mut poll_descriptor, 1, timeout_ms) };
        if result == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Unix socket connection timed out",
            ));
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }

        let mut socket_error = 0 as libc::c_int;
        let mut socket_error_length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: both output pointers reference initialized storage of the
        // advertised length, and socket is a valid socket descriptor.
        if unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                std::ptr::addr_of_mut!(socket_error).cast(),
                &mut socket_error_length,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        return if socket_error == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(socket_error))
        };
    }
}

#[cfg(unix)]
fn remaining_io_time(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "bridge request timed out"))
}

#[cfg(unix)]
fn write_all_with_deadline(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> io::Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining_io_time(deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    stream.set_write_timeout(Some(remaining_io_time(deadline)?))?;
    stream.flush()
}

#[cfg(unix)]
fn read_http_response_with_deadline(
    stream: &mut UnixStream,
    deadline: Instant,
) -> Result<Vec<u8>, BridgeFailure> {
    let mut response = Vec::new();
    let mut buffer = [0u8; 8 * 1024];
    loop {
        let remaining = remaining_io_time(deadline)
            .map_err(|_| BridgeFailure::new("bridge_response_failed"))?;
        if http_response_is_complete(&response)? {
            return Ok(response);
        }
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| BridgeFailure::new("bridge_response_failed"))?;
        let available = MAX_RESPONSE_BYTES
            .saturating_add(1)
            .saturating_sub(response.len());
        let read_limit = available.min(buffer.len());
        if read_limit == 0 {
            return Err(BridgeFailure::new("bridge_response_too_large"));
        }
        match stream.read(&mut buffer[..read_limit]) {
            Ok(0) => {
                remaining_io_time(deadline)
                    .map_err(|_| BridgeFailure::new("bridge_response_failed"))?;
                return Ok(response);
            }
            Ok(read) => {
                response.extend_from_slice(&buffer[..read]);
                if response.len() > MAX_RESPONSE_BYTES {
                    return Err(BridgeFailure::new("bridge_response_too_large"));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(BridgeFailure::new("bridge_response_failed")),
        }
    }
}

#[cfg(windows)]
fn request_threads(
    discovery: &DiscoveryRecord,
    now_ms: i64,
) -> Result<Vec<CursorBridgeThread>, BridgeFailure> {
    let body = br#"{"type":"listThreads"}"#;
    let request = format!(
        "POST / HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        discovery.token,
        body.len()
    );
    let mut outbound = request.into_bytes();
    outbound.extend_from_slice(body);
    let pipe_path = discovery.socket_path.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|_| BridgeFailure::new("bridge_connection_failed"))?;
    let response = runtime
        .block_on(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            use tokio::net::windows::named_pipe::ClientOptions;

            tokio::time::timeout(IO_TIMEOUT, async move {
                let mut pipe = loop {
                    match ClientOptions::new().open(&pipe_path) {
                        Ok(pipe) => break pipe,
                        Err(error) if error.raw_os_error() == Some(231) => {
                            // ERROR_PIPE_BUSY means a healthy server has no free
                            // instance yet. Cursor's own CLI waits and retries.
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        Err(error) => return Err(error),
                    }
                };
                pipe.write_all(&outbound).await?;
                pipe.flush().await?;
                let mut response = Vec::new();
                pipe.take((MAX_RESPONSE_BYTES + 1) as u64)
                    .read_to_end(&mut response)
                    .await?;
                Ok::<_, std::io::Error>(response)
            })
            .await
        })
        .map_err(|_| BridgeFailure::new("bridge_response_failed"))?
        .map_err(|_| BridgeFailure::new("bridge_connection_failed"))?;
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(BridgeFailure::new("bridge_response_too_large"));
    }
    parse_threads_response(&response, now_ms, &bridge_instance_key(discovery))
}

fn parse_threads_response(
    response: &[u8],
    now_ms: i64,
    bridge_instance_key: &str,
) -> Result<Vec<CursorBridgeThread>, BridgeFailure> {
    let body = parse_http_response(response)?;
    let raw: ListThreadsResponse =
        serde_json::from_slice(&body).map_err(|_| BridgeFailure::new("invalid_bridge_response"))?;
    raw.threads
        .into_iter()
        .map(|thread| convert_thread(thread, now_ms, bridge_instance_key))
        .collect()
}

fn bridge_instance_key(discovery: &DiscoveryRecord) -> String {
    let mut identity = b"vsparallel.cursor.desktop-bridge.instance.v1\0".to_vec();
    identity.extend_from_slice(&discovery.pid.to_le_bytes());
    identity.extend_from_slice(&discovery.created_at.to_bits().to_le_bytes());
    identity.extend_from_slice(discovery.user_data_dir.as_bytes());
    crate::cursor_integration::cursor_bytes_hash(&identity)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpBodyFraming {
    ContentLength(usize),
    Chunked,
    UntilEof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HttpResponseHead {
    header_len: usize,
    framing: HttpBodyFraming,
}

fn parse_http_head(response: &[u8]) -> Result<Option<HttpResponseHead>, BridgeFailure> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Response::new(&mut headers);
    let header_len = match parsed
        .parse(response)
        .map_err(|_| BridgeFailure::new("invalid_bridge_response"))?
    {
        httparse::Status::Complete(length) => length,
        httparse::Status::Partial => return Ok(None),
    };
    if parsed.code != Some(200) {
        return Err(BridgeFailure::new("bridge_rejected_request"));
    }
    let mut content_length = None;
    let mut chunked = false;
    for header in parsed.headers {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            if chunked {
                return Err(BridgeFailure::new("invalid_bridge_response"));
            }
            let value = std::str::from_utf8(header.value)
                .map_err(|_| BridgeFailure::new("invalid_bridge_response"))?;
            let encodings: Vec<_> = value.split(',').map(str::trim).collect();
            if encodings.len() != 1 || !encodings[0].eq_ignore_ascii_case("chunked") {
                return Err(BridgeFailure::new("unsupported_bridge_response"));
            }
            chunked = true;
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(BridgeFailure::new("invalid_bridge_response"));
            }
            let value = std::str::from_utf8(header.value)
                .ok()
                .filter(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                })
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
            if value > MAX_RESPONSE_BYTES {
                return Err(BridgeFailure::new("bridge_response_too_large"));
            }
            content_length = Some(value);
        }
    }
    if chunked && content_length.is_some() {
        return Err(BridgeFailure::new("invalid_bridge_response"));
    }
    let framing = if chunked {
        HttpBodyFraming::Chunked
    } else if let Some(content_length) = content_length {
        HttpBodyFraming::ContentLength(content_length)
    } else {
        HttpBodyFraming::UntilEof
    };
    Ok(Some(HttpResponseHead {
        header_len,
        framing,
    }))
}

fn http_response_is_complete(response: &[u8]) -> Result<bool, BridgeFailure> {
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(BridgeFailure::new("bridge_response_too_large"));
    }
    let Some(head) = parse_http_head(response)? else {
        return Ok(false);
    };
    match head.framing {
        HttpBodyFraming::ContentLength(content_length) => {
            let expected_length = head
                .header_len
                .checked_add(content_length)
                .filter(|length| *length <= MAX_RESPONSE_BYTES)
                .ok_or_else(|| BridgeFailure::new("bridge_response_too_large"))?;
            if response.len() > expected_length {
                return Err(BridgeFailure::new("invalid_bridge_response"));
            }
            Ok(response.len() == expected_length)
        }
        HttpBodyFraming::Chunked => {
            let body = &response[head.header_len..];
            let Some(encoded_length) = chunked_body_encoded_length(body)? else {
                return Ok(false);
            };
            let expected_length = head
                .header_len
                .checked_add(encoded_length)
                .filter(|length| *length <= MAX_RESPONSE_BYTES)
                .ok_or_else(|| BridgeFailure::new("bridge_response_too_large"))?;
            if response.len() != expected_length {
                return Err(BridgeFailure::new("invalid_bridge_response"));
            }
            Ok(true)
        }
        HttpBodyFraming::UntilEof => Ok(false),
    }
}

fn chunked_body_encoded_length(encoded: &[u8]) -> Result<Option<usize>, BridgeFailure> {
    let mut offset = 0usize;
    let mut decoded_length = 0usize;
    loop {
        let remaining = &encoded[offset..];
        let Some(line_end) = remaining.windows(2).position(|window| window == b"\r\n") else {
            if remaining.len() > 33 {
                return Err(BridgeFailure::new("invalid_bridge_response"));
            }
            return Ok(None);
        };
        if line_end == 0 || line_end > 32 {
            return Err(BridgeFailure::new("invalid_bridge_response"));
        }
        let size_token = remaining[..line_end]
            .split(|byte| *byte == b';')
            .next()
            .filter(|value| !value.is_empty() && value.iter().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
        let size = std::str::from_utf8(size_token)
            .ok()
            .and_then(|value| usize::from_str_radix(value, 16).ok())
            .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
        offset = offset
            .checked_add(line_end)
            .and_then(|value| value.checked_add(2))
            .ok_or_else(|| BridgeFailure::new("bridge_response_too_large"))?;
        if size == 0 {
            let terminator = &encoded[offset..];
            if terminator.len() < 2 {
                return if b"\r\n".starts_with(terminator) {
                    Ok(None)
                } else {
                    Err(BridgeFailure::new("invalid_bridge_response"))
                };
            }
            if &terminator[..2] != b"\r\n" {
                return Err(BridgeFailure::new("invalid_bridge_response"));
            }
            return offset
                .checked_add(2)
                .map(Some)
                .ok_or_else(|| BridgeFailure::new("bridge_response_too_large"));
        }
        decoded_length = decoded_length
            .checked_add(size)
            .filter(|length| *length <= MAX_RESPONSE_BYTES)
            .ok_or_else(|| BridgeFailure::new("bridge_response_too_large"))?;
        let chunk_end = offset
            .checked_add(size)
            .and_then(|value| value.checked_add(2))
            .ok_or_else(|| BridgeFailure::new("bridge_response_too_large"))?;
        if encoded.len() < chunk_end {
            return Ok(None);
        }
        if &encoded[chunk_end - 2..chunk_end] != b"\r\n" {
            return Err(BridgeFailure::new("invalid_bridge_response"));
        }
        offset = chunk_end;
    }
}

fn parse_http_response(response: &[u8]) -> Result<Vec<u8>, BridgeFailure> {
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(BridgeFailure::new("bridge_response_too_large"));
    }
    let head =
        parse_http_head(response)?.ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
    let body = &response[head.header_len..];
    match head.framing {
        HttpBodyFraming::ContentLength(content_length) if content_length == body.len() => {
            Ok(body.to_vec())
        }
        HttpBodyFraming::ContentLength(_) => Err(BridgeFailure::new("invalid_bridge_response")),
        HttpBodyFraming::Chunked => {
            let encoded_length = chunked_body_encoded_length(body)?
                .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
            if encoded_length != body.len() {
                return Err(BridgeFailure::new("invalid_bridge_response"));
            }
            decode_chunked_body(body)
        }
        HttpBodyFraming::UntilEof => Ok(body.to_vec()),
    }
}

fn decode_chunked_body(mut encoded: &[u8]) -> Result<Vec<u8>, BridgeFailure> {
    let mut decoded = Vec::new();
    loop {
        let line_end = encoded
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
        if line_end == 0 || line_end > 32 {
            return Err(BridgeFailure::new("invalid_bridge_response"));
        }
        let size_token = encoded[..line_end]
            .split(|byte| *byte == b';')
            .next()
            .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
        let size = std::str::from_utf8(size_token)
            .ok()
            .and_then(|value| usize::from_str_radix(value, 16).ok())
            .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"))?;
        encoded = &encoded[line_end + 2..];
        if size == 0 {
            return (encoded == b"\r\n")
                .then_some(decoded)
                .ok_or_else(|| BridgeFailure::new("invalid_bridge_response"));
        }
        if size > MAX_RESPONSE_BYTES.saturating_sub(decoded.len())
            || encoded.len() < size.saturating_add(2)
            || &encoded[size..size + 2] != b"\r\n"
        {
            return Err(BridgeFailure::new("invalid_bridge_response"));
        }
        decoded.extend_from_slice(&encoded[..size]);
        encoded = &encoded[size + 2..];
    }
}

fn convert_thread(
    thread: RawThread,
    now_ms: i64,
    bridge_instance_key: &str,
) -> Result<CursorBridgeThread, BridgeFailure> {
    let _ = thread.title;
    if thread.id.is_empty()
        || thread.id.len() > MAX_IDENTIFIER_BYTES
        || thread.window_id < 0
        || !thread.last_updated_at.is_finite()
        || thread.last_updated_at < 0.0
        || thread.last_updated_at > now_ms.saturating_add(MAX_FUTURE_SKEW_MS) as f64
    {
        return Err(BridgeFailure::new("invalid_bridge_response"));
    }
    let source = match thread.source {
        RawThreadSource::Local => CursorBridgeThreadSource::Local,
        RawThreadSource::Cloud => CursorBridgeThreadSource::Cloud,
        RawThreadSource::Draft => CursorBridgeThreadSource::Draft,
        RawThreadSource::ClaudeCode => CursorBridgeThreadSource::ClaudeCode,
    };
    let status = match thread.status {
        RawThreadStatus::Idle => CursorBridgeThreadStatus::Idle,
        RawThreadStatus::Running => CursorBridgeThreadStatus::Running,
        RawThreadStatus::Completed => CursorBridgeThreadStatus::Completed,
        RawThreadStatus::Error => CursorBridgeThreadStatus::Error,
        RawThreadStatus::Unknown => CursorBridgeThreadStatus::Unknown,
    };
    let mut window_identity = b"vsparallel.cursor.desktop-bridge.window.v1\0".to_vec();
    window_identity.extend_from_slice(bridge_instance_key.as_bytes());
    window_identity.push(0);
    window_identity.extend_from_slice(thread.window_id.to_string().as_bytes());
    Ok(CursorBridgeThread {
        session_key: crate::cursor_integration::cursor_identity_hash(&thread.id),
        source,
        status,
        last_updated_at_ms: thread.last_updated_at as i64,
        window_key: crate::cursor_integration::cursor_bytes_hash(&window_identity),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::sync::mpsc;
    use std::thread;
    use tempfile::TempDir;

    #[cfg(unix)]
    fn bind_test_bridge() -> Option<(TempDir, std::os::unix::net::UnixListener, PathBuf)> {
        use std::os::unix::net::UnixListener;

        let temporary = TempDir::new().unwrap();
        let socket_path = temporary.path().join("bridge.sock");
        match UnixListener::bind(&socket_path) {
            Ok(listener) => Some((temporary, listener, socket_path)),
            // Some CI sandboxes deny creating even local Unix sockets. Parser
            // tests still exercise framing where local IPC is unavailable.
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => None,
            Err(error) => panic!("could not bind test bridge socket: {error}"),
        }
    }

    #[cfg(unix)]
    fn direct_test_discovery(socket_path: &Path) -> DiscoveryRecord {
        DiscoveryRecord {
            protocol_version: BRIDGE_PROTOCOL_VERSION,
            pid: i64::from(std::process::id()),
            socket_path: socket_path.to_string_lossy().into_owned(),
            token: "a".repeat(64),
            app_name: "Cursor".to_string(),
            app_version: "test".to_string(),
            user_data_dir: "/tmp/vsparallel-cursor-test".to_string(),
            created_at: 500.0,
        }
    }

    #[test]
    fn preference_is_off_by_default_and_round_trips_privately() {
        let temporary = TempDir::new().unwrap();
        assert!(!read_preference(temporary.path()).unwrap());
        write_preference(temporary.path(), true).unwrap();
        assert!(read_preference(temporary.path()).unwrap());
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(temporary.path().join(PREFERENCE_FILENAME))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn preference_is_off_when_the_state_directory_does_not_exist() {
        let temporary = TempDir::new().unwrap();
        assert!(!read_preference(&temporary.path().join("not-created")).unwrap());
    }

    #[test]
    fn malformed_or_public_preference_fails_closed() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join(PREFERENCE_FILENAME);
        fs::write(&path, b"not json").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_preference(temporary.path()).is_err());
        #[cfg(unix)]
        {
            fs::write(&path, br#"{"schemaVersion":1,"enabled":true}"#).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(read_preference(temporary.path()).is_err());
        }
    }

    #[test]
    fn thread_conversion_hashes_identity_and_keeps_only_coarse_fields() {
        let converted = convert_thread(
            RawThread {
                id: "raw-private-thread".to_string(),
                title: IgnoredAny,
                source: RawThreadSource::Local,
                status: RawThreadStatus::Running,
                last_updated_at: 1_000.0,
                window_id: 42,
            },
            2_000,
            "test-bridge-instance",
        )
        .unwrap();
        assert_eq!(
            converted.session_key,
            crate::cursor_integration::cursor_identity_hash("raw-private-thread")
        );
        assert_eq!(converted.status, CursorBridgeThreadStatus::Running);
        assert_eq!(converted.source, CursorBridgeThreadSource::Local);
        assert_eq!(converted.window_key.len(), 64);

        let fractional = convert_thread(
            RawThread {
                id: "fractional-thread".to_string(),
                title: IgnoredAny,
                source: RawThreadSource::Draft,
                status: RawThreadStatus::Idle,
                last_updated_at: 1_000.5,
                window_id: 7,
            },
            2_000,
            "test-bridge-instance",
        )
        .unwrap();
        assert_eq!(fractional.last_updated_at_ms, 1_000);
    }

    #[test]
    fn response_requires_a_string_title() {
        let body = br#"{"threads":[{"id":"x","title":null,"source":"local","status":"idle","lastUpdatedAt":1,"windowId":1}]}"#;
        let mut response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        response.extend_from_slice(body);
        assert!(parse_threads_response(&response, 2_000, "test-bridge-instance").is_err());
    }

    #[test]
    fn response_parser_rejects_non_success_and_length_mismatch() {
        assert!(
            parse_http_response(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n").is_err()
        );
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").is_ok());
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n{}").is_err());
        assert!(parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}"
        )
        .is_err());
        assert_eq!(
            parse_http_response(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n"
            )
            .unwrap(),
            b"{}"
        );
    }

    #[test]
    fn incremental_http_framing_detects_completion_and_rejects_excess() {
        assert!(
            !http_response_is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{").unwrap()
        );
        assert!(
            http_response_is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").unwrap()
        );
        assert!(http_response_is_complete(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n"
        )
        .unwrap());
        assert!(
            http_response_is_complete(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}extra")
                .is_err()
        );
        assert!(http_response_is_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n"
        )
        .is_err());

        let oversized = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_RESPONSE_BYTES + 1
        );
        assert_eq!(
            http_response_is_complete(oversized.as_bytes())
                .unwrap_err()
                .code,
            "bridge_response_too_large"
        );
        let oversized_chunk = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            MAX_RESPONSE_BYTES + 1
        );
        assert_eq!(
            http_response_is_complete(oversized_chunk.as_bytes())
                .unwrap_err()
                .code,
            "bridge_response_too_large"
        );
    }

    #[test]
    fn missing_bridge_does_not_claim_the_gated_cursor_setting_exists() {
        let poll = failed_poll(1_000, "waiting", "bridge_not_found", None);
        assert_eq!(poll.status.availability, "waiting");
        assert!(poll
            .status
            .detail
            .contains("limited server-controlled rollout"));
        assert!(poll
            .status
            .detail
            .contains("absent from Cursor Settings > Beta"));
        assert!(poll.status.detail.contains("hook-only fallback"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_request_stops_at_a_complete_body_while_the_peer_stays_open() {
        let Some((_temporary, listener, socket_path)) = bind_test_bridge() else {
            return;
        };
        let (release_sender, release_receiver) = mpsc::channel();
        let responder = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let read = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).contains("listThreads"));
            let body = br#"{"threads":[{"id":"thread-1","title":"secret","source":"local","status":"running","lastUpdatedAt":1000,"windowId":7}]}"#;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            release_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        });

        let discovery = direct_test_discovery(&socket_path);
        let started = Instant::now();
        let result = request_threads_with_timeout(&discovery, 2_000, Duration::from_millis(500));
        let elapsed = started.elapsed();
        let _ = release_sender.send(());
        responder.join().unwrap();

        let threads = result.unwrap();
        assert_eq!(threads.len(), 1);
        assert!(elapsed < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn unix_request_deadline_bounds_a_slow_trickle_response() {
        let Some((_temporary, listener, socket_path)) = bind_test_bridge() else {
            return;
        };
        let responder = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
            for byte in response {
                if stream.write_all(std::slice::from_ref(byte)).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });

        let discovery = direct_test_discovery(&socket_path);
        let started = Instant::now();
        let error = request_threads_with_timeout(&discovery, 2_000, Duration::from_millis(120))
            .unwrap_err();
        let elapsed = started.elapsed();
        responder.join().unwrap();

        assert_eq!(error.code, "bridge_response_failed");
        assert!(elapsed >= Duration::from_millis(80));
        assert!(elapsed < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn unix_request_rejects_an_oversized_frame_without_waiting_for_eof() {
        let Some((_temporary, listener, socket_path)) = bind_test_bridge() else {
            return;
        };
        let (release_sender, release_receiver) = mpsc::channel();
        let responder = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_RESPONSE_BYTES + 1
            );
            stream.write_all(response.as_bytes()).unwrap();
            release_receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        });

        let discovery = direct_test_discovery(&socket_path);
        let error = request_threads_with_timeout(&discovery, 2_000, Duration::from_millis(500))
            .unwrap_err();
        let _ = release_sender.send(());
        responder.join().unwrap();

        assert_eq!(error.code, "bridge_response_too_large");
    }

    #[cfg(unix)]
    #[test]
    fn reads_a_valid_local_bridge_response() {
        use std::os::unix::net::UnixListener;

        let temporary = TempDir::new().unwrap();
        let bridge_dir = temporary.path().join("desktop-bridge");
        fs::create_dir(&bridge_dir).unwrap();
        fs::set_permissions(&bridge_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let socket_path = env::temp_dir().join(format!(
            "vscode-ipc-vsparallel-test-{}-{}.sock",
            std::process::id(),
            crate::state::now_ms()
        ));
        let listener = match UnixListener::bind(&socket_path) {
            Ok(listener) => listener,
            // Some CI sandboxes deny creating even local Unix sockets. The
            // parser/conversion assertions above still exercise the protocol;
            // run this end-to-end portion where local IPC is permitted.
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("could not bind test bridge socket: {error}"),
        };
        let responder = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let read = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).contains("\"type\":\"listThreads\""));
            let body = br#"{"threads":[{"id":"thread-1","title":"secret title","source":"local","status":"running","lastUpdatedAt":1000,"windowId":7}]}"#;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
        });

        let user_data_dir = temporary.path().join("cursor-user-data");
        let user_data = user_data_dir.to_string_lossy();
        let filename = format!(
            "{}.json",
            &crate::cursor_integration::cursor_identity_hash(&user_data)[..16]
        );
        let discovery = serde_json::json!({
            "protocolVersion": 1,
            "pid": std::process::id(),
            "socketPath": socket_path,
            "token": "a".repeat(64),
            "appName": "Cursor",
            "appVersion": "test",
            "userDataDir": user_data,
            "createdAt": 500
        });
        let discovery_path = bridge_dir.join(filename);
        fs::write(&discovery_path, serde_json::to_vec(&discovery).unwrap()).unwrap();
        fs::set_permissions(&discovery_path, fs::Permissions::from_mode(0o600)).unwrap();

        let result = probe(&bridge_dir, 2_000);
        assert!(result.status.connected);
        assert_eq!(result.status.thread_count, 1);
        assert_eq!(
            result.snapshot.unwrap().threads[0].session_key,
            crate::cursor_integration::cursor_identity_hash("thread-1")
        );
        responder.join().unwrap();
        let _ = fs::remove_file(socket_path);
    }
}
