//! Privacy-conscious usage-limit collection for Codex and Claude Code.
//!
//! Codex exposes live limits through its documented app-server protocol. Claude
//! Code supplies equivalent values to status-line commands, so this module keeps
//! a compact global cache containing only those limits. In particular, Claude's
//! session identifier, working directory, transcript path, and prompt are never
//! represented by the deserialization or persistence types below.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::Builder as TempFileBuilder;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

pub const CLAUDE_STATUSLINE_ARGUMENT: &str = "claude-usage";

const USAGE_SCHEMA_VERSION: u32 = 1;
const CLAUDE_RECORD_SCHEMA_VERSION: u32 = 1;
const CLAUDE_RECORD_DIRECTORY: &str = "usage";
const CLAUDE_RECORD_FILENAME: &str = "claude.json";
const CODEX_PROTOCOL_TIMEOUT: Duration = Duration::from_secs(6);
const CODEX_OUTPUT_LIMIT: usize = 256 * 1024;
const CODEX_LINE_LIMIT: usize = 128 * 1024;
const CLAUDE_INPUT_LIMIT: usize = 256 * 1024;
const CLAUDE_RECORD_LIMIT: u64 = 16 * 1024;
const CLAUDE_STALE_AFTER_MS: i64 = 15 * 60 * 1_000;
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn kill_process_group(process_group: i32, signal: i32) -> i32;
}

#[cfg(windows)]
const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
#[cfg(windows)]
const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn MoveFileExW(existing_file_name: *const u16, new_file_name: *const u16, flags: u32) -> i32;
}

/// One independently resetting usage-limit window.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindowView {
    pub label: String,
    pub duration_minutes: Option<i64>,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub resets_at_ms: Option<i64>,
}

/// UI-safe usage state for one provider.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageView {
    /// `available`, `stale`, or `unavailable`.
    pub state: String,
    /// The most constrained window, so a single gauge never overstates capacity.
    pub remaining_percent: Option<f64>,
    pub windows: Vec<UsageWindowView>,
    pub updated_at_ms: Option<i64>,
    pub detail: String,
}

/// Usage data is intentionally separate from the frequently-polled workspace
/// snapshot because collecting Codex limits starts a network-capable subprocess.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub schema_version: u32,
    pub generated_at_ms: i64,
    pub codex: ProviderUsageView,
    pub claude: ProviderUsageView,
}

/// Injectable boundary around the Codex app-server conversation.
pub trait CodexUsageRunner {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessCodexUsageRunner;

/// Resolve the Codex executable without splitting or interpreting shell syntax.
pub fn codex_command() -> OsString {
    codex_command_from(env::var_os("VSPARALLEL_CODEX_COMMAND"))
}

/// Collect a fresh snapshot using the current clock and environment.
///
/// Provider failures are represented in the returned data; a missing executable,
/// signed-out account, malformed response, or unavailable state directory never
/// prevents the rest of the snapshot from rendering.
pub fn get_usage_snapshot() -> UsageSnapshot {
    build_usage_snapshot(crate::state::now_ms())
}

/// Collect a fresh snapshot at an injected timestamp.
pub fn build_usage_snapshot(now_ms: i64) -> UsageSnapshot {
    let command = codex_command();
    let state_root = crate::state::state_dir_from_environment().ok();
    build_usage_snapshot_with(
        &ProcessCodexUsageRunner,
        command.as_os_str(),
        state_root.as_deref(),
        now_ms,
    )
}

/// Testable snapshot builder with injected process and state boundaries.
pub fn build_usage_snapshot_with<R: CodexUsageRunner + ?Sized>(
    runner: &R,
    executable: &OsStr,
    state_root: Option<&Path>,
    now_ms: i64,
) -> UsageSnapshot {
    let codex = runner
        .read_rate_limits(executable)
        .ok()
        .and_then(|result| codex_provider_view(&result, now_ms))
        .unwrap_or_else(|| {
            unavailable_provider("Install or sign in to Codex to view usage limits.")
        });
    let claude = state_root
        .map(|root| load_claude_usage(root, now_ms))
        .unwrap_or_else(|| unavailable_provider("No capture yet. Check Setup & diagnostics."));

    UsageSnapshot {
        schema_version: USAGE_SCHEMA_VERSION,
        generated_at_ms: now_ms,
        codex,
        claude,
    }
}

impl CodexUsageRunner for ProcessCodexUsageRunner {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String> {
        let mut command = Command::new(executable);
        command
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let mut child = command
            .spawn()
            .map_err(|_| "could not start the Codex usage service".to_string())?;
        let mut stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let _ = terminate_and_reap(&mut child);
                return Err("Codex usage service input was unavailable".to_string());
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                drop(stdin);
                let _ = terminate_and_reap(&mut child);
                return Err("Codex usage service output was unavailable".to_string());
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                drop(stdin);
                drop(stdout);
                let _ = terminate_and_reap(&mut child);
                return Err("Codex usage service diagnostics were unavailable".to_string());
            }
        };

        let (line_sender, line_receiver) = mpsc::channel();
        let stdout_reader = thread::spawn(move || pump_protocol_lines(stdout, line_sender));
        let stderr_reader =
            thread::spawn(move || drain_capped(stderr, CODEX_OUTPUT_LIMIT).map(|_| ()));
        let deadline = Instant::now() + CODEX_PROTOCOL_TIMEOUT;

        let protocol_result = (|| {
            send_rpc(
                &mut stdin,
                &json!({
                    "method": "initialize",
                    "id": 1,
                    "params": {
                        "clientInfo": {
                            "name": "vsparallel",
                            "title": "VSParallel",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                }),
            )?;
            let _ = wait_for_rpc_response(&line_receiver, 1, deadline)?;
            send_rpc(&mut stdin, &json!({"method": "initialized", "params": {}}))?;
            send_rpc(
                &mut stdin,
                &json!({
                    "method": "account/rateLimits/read",
                    "id": 2,
                    "params": {}
                }),
            )?;
            wait_for_rpc_response(&line_receiver, 2, deadline)
        })();

        drop(stdin);
        let _ = terminate_and_reap(&mut child);
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        protocol_result
    }
}

fn send_rpc(writer: &mut impl Write, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, message)
        .map_err(|_| "could not encode a Codex usage request".to_string())?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|_| "could not send a Codex usage request".to_string())
}

fn wait_for_rpc_response(
    receiver: &mpsc::Receiver<Result<Vec<u8>, String>>,
    expected_id: i64,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "Codex usage request timed out".to_string())?;
        let line = receiver
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "Codex usage request timed out".to_string(),
                mpsc::RecvTimeoutError::Disconnected => {
                    "Codex usage service stopped before responding".to_string()
                }
            })??;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let response: Value = serde_json::from_slice(&line)
            .map_err(|_| "Codex usage service returned malformed output".to_string())?;
        if response.get("id").and_then(Value::as_i64) != Some(expected_id) {
            continue;
        }
        if response.get("error").is_some() {
            return Err("Codex did not provide usage limits for this account".to_string());
        }
        return response
            .get("result")
            .cloned()
            .ok_or_else(|| "Codex usage response had no result".to_string());
    }
}

fn pump_protocol_lines(
    stdout: impl Read,
    sender: mpsc::Sender<Result<Vec<u8>, String>>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stdout);
    let mut total = 0;
    loop {
        match read_protocol_line(
            &mut reader,
            &mut total,
            CODEX_LINE_LIMIT,
            CODEX_OUTPUT_LIMIT,
        ) {
            Ok(Some(line)) => {
                if sender.send(Ok(line)).is_err() {
                    return Ok(());
                }
            }
            Ok(None) => return Ok(()),
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                return Err(error);
            }
        }
    }
}

fn read_protocol_line<R: BufRead>(
    reader: &mut R,
    total: &mut usize,
    line_limit: usize,
    total_limit: usize,
) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(consumed) > line_limit
            || total.saturating_add(consumed) > total_limit
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Codex usage output exceeded its safety limit",
            ));
        }
        let has_newline = available[consumed - 1] == b'\n';
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        *total += consumed;
        if has_newline {
            return Ok(Some(line));
        }
    }
}

fn drain_capped(mut reader: impl Read, limit: usize) -> io::Result<usize> {
    let mut total = 0;
    let mut buffer = [0_u8; 8 * 1024];
    while total <= limit {
        let remaining = limit.saturating_add(1).saturating_sub(total);
        let chunk_length = remaining.min(buffer.len());
        let read = reader.read(&mut buffer[..chunk_length])?;
        if read == 0 {
            return Ok(total);
        }
        total += read;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Codex diagnostic output exceeded its safety limit",
    ))
}

fn codex_provider_view(result: &Value, now_ms: i64) -> Option<ProviderUsageView> {
    let selected = result
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object)
        .and_then(|limits| limits.get("codex"))
        .or_else(|| result.get("rateLimits"))?;

    let mut windows = Vec::with_capacity(2);
    if let Some(window) = selected
        .get("primary")
        .and_then(|window| codex_window_view(window, "Primary limit"))
    {
        windows.push(window);
    }
    if let Some(window) = selected
        .get("secondary")
        .and_then(|window| codex_window_view(window, "Secondary limit"))
    {
        windows.push(window);
    }
    provider_from_windows(
        windows,
        now_ms,
        "Live usage limits from Codex.",
        "available",
    )
}

fn codex_window_view(value: &Value, fallback_label: &str) -> Option<UsageWindowView> {
    let used_percent = finite_number(value.get("usedPercent")?)?;
    let duration_minutes = value
        .get("windowDurationMins")
        .and_then(Value::as_i64)
        .filter(|duration| *duration > 0);
    let resets_at_ms = value
        .get("resetsAt")
        .and_then(Value::as_i64)
        .and_then(seconds_to_millis);
    Some(usage_window(
        duration_label(duration_minutes, fallback_label),
        duration_minutes,
        used_percent,
        resets_at_ms,
    ))
}

fn duration_label(duration_minutes: Option<i64>, fallback: &str) -> String {
    match duration_minutes {
        Some(300) => "5-hour limit".to_string(),
        Some(10_080) => "7-day limit".to_string(),
        Some(minutes) if minutes % (24 * 60) == 0 => {
            format!("{}-day limit", minutes / (24 * 60))
        }
        Some(minutes) if minutes % 60 == 0 => format!("{}-hour limit", minutes / 60),
        Some(minutes) => format!("{minutes}-minute limit"),
        None => fallback.to_string(),
    }
}

fn usage_window(
    label: String,
    duration_minutes: Option<i64>,
    used_percent: f64,
    resets_at_ms: Option<i64>,
) -> UsageWindowView {
    let used_percent = used_percent.clamp(0.0, 100.0);
    UsageWindowView {
        label,
        duration_minutes,
        used_percent,
        remaining_percent: 100.0 - used_percent,
        resets_at_ms,
    }
}

fn finite_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

fn seconds_to_millis(seconds: i64) -> Option<i64> {
    (seconds >= 0).then(|| seconds.checked_mul(1_000)).flatten()
}

fn provider_from_windows(
    windows: Vec<UsageWindowView>,
    updated_at_ms: i64,
    detail: &str,
    state: &str,
) -> Option<ProviderUsageView> {
    let remaining_percent = windows
        .iter()
        .map(|window| window.remaining_percent)
        .reduce(f64::min)?;
    Some(ProviderUsageView {
        state: state.to_string(),
        remaining_percent: Some(remaining_percent),
        windows,
        updated_at_ms: Some(updated_at_ms),
        detail: detail.to_string(),
    })
}

fn unavailable_provider(detail: &str) -> ProviderUsageView {
    ProviderUsageView {
        state: "unavailable".to_string(),
        remaining_percent: None,
        windows: Vec::new(),
        updated_at_ms: None,
        detail: detail.to_string(),
    }
}

fn codex_command_from(value: Option<OsString>) -> OsString {
    value
        .filter(|command| !command.is_empty())
        .unwrap_or_else(|| OsString::from("codex"))
}

/// Fail-open entry point for the Claude Code status-line command.
pub fn run_claude_statusline_stdio() -> i32 {
    let state_root = crate::state::state_dir_from_environment().ok();
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_claude_statusline(
        stdin.lock(),
        stdout.lock(),
        state_root.as_deref(),
        crate::state::now_ms(),
    )
}

/// Testable Claude status-line handler. It deliberately emits no status text;
/// integrations may compose it with an existing status-line renderer.
/// Regardless of malformed input or storage errors, it returns success so a
/// telemetry failure cannot disrupt Claude Code.
pub fn run_claude_statusline<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    state_root: Option<&Path>,
    captured_at_ms: i64,
) -> i32 {
    if let Some(state_root) = state_root {
        let _ = capture_claude_usage(&mut input, state_root, captured_at_ms);
    }
    let _ = output.flush();
    0
}

#[derive(Debug, Deserialize)]
struct ClaudeStatusLineInput {
    #[serde(default)]
    rate_limits: Option<ClaudeRateLimitsInput>,
}

#[derive(Debug, Deserialize)]
struct ClaudeRateLimitsInput {
    #[serde(default)]
    five_hour: Option<ClaudeWindowInput>,
    #[serde(default)]
    seven_day: Option<ClaudeWindowInput>,
}

#[derive(Debug, Deserialize)]
struct ClaudeWindowInput {
    used_percentage: f64,
    #[serde(default)]
    resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaudeUsageRecord {
    schema_version: u32,
    captured_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    five_hour: Option<StoredUsageWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seven_day: Option<StoredUsageWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredUsageWindow {
    used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resets_at_ms: Option<i64>,
}

fn capture_claude_usage(
    input: &mut impl Read,
    state_root: &Path,
    captured_at_ms: i64,
) -> Result<(), String> {
    if captured_at_ms < 0 {
        return Err("Claude usage timestamp was invalid".to_string());
    }
    let mut bytes = Vec::with_capacity(CLAUDE_INPUT_LIMIT.min(8 * 1024));
    input
        .take((CLAUDE_INPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read Claude usage input".to_string())?;
    if bytes.len() > CLAUDE_INPUT_LIMIT {
        return Err("Claude usage input exceeded its safety limit".to_string());
    }
    let payload: ClaudeStatusLineInput = serde_json::from_slice(&bytes)
        .map_err(|_| "Claude usage input was malformed".to_string())?;
    let limits = payload
        .rate_limits
        .ok_or_else(|| "Claude usage input had no rate limits".to_string())?;
    let record = ClaudeUsageRecord {
        schema_version: CLAUDE_RECORD_SCHEMA_VERSION,
        captured_at_ms,
        five_hour: normalize_claude_window(limits.five_hour),
        seven_day: normalize_claude_window(limits.seven_day),
    };
    if record.five_hour.is_none() && record.seven_day.is_none() {
        return Err("Claude usage input had no valid windows".to_string());
    }
    write_claude_record(&claude_record_path(state_root), &record)
}

fn normalize_claude_window(window: Option<ClaudeWindowInput>) -> Option<StoredUsageWindow> {
    let window = window?;
    if !window.used_percentage.is_finite() {
        return None;
    }
    Some(StoredUsageWindow {
        used_percent: window.used_percentage.clamp(0.0, 100.0),
        resets_at_ms: window.resets_at.and_then(seconds_to_millis),
    })
}

fn claude_record_path(state_root: &Path) -> PathBuf {
    state_root
        .join(CLAUDE_RECORD_DIRECTORY)
        .join(CLAUDE_RECORD_FILENAME)
}

/// Load the most recent privacy-minimal Claude status-line capture.
pub fn load_claude_usage(state_root: &Path, now_ms: i64) -> ProviderUsageView {
    load_claude_record(state_root, now_ms)
        .unwrap_or_else(|| unavailable_provider("No capture yet. Check Setup & diagnostics."))
}

fn load_claude_record(state_root: &Path, now_ms: i64) -> Option<ProviderUsageView> {
    let path = claude_record_path(state_root);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if is_link_or_reparse_point(&metadata)
        || !metadata.is_file()
        || metadata.len() > CLAUDE_RECORD_LIMIT
    {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.len() as u64 > CLAUDE_RECORD_LIMIT {
        return None;
    }
    let record: ClaudeUsageRecord = serde_json::from_slice(&bytes).ok()?;
    if record.schema_version != CLAUDE_RECORD_SCHEMA_VERSION
        || record.captured_at_ms < 0
        || record.captured_at_ms > now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
    {
        return None;
    }

    let mut windows = Vec::with_capacity(2);
    if let Some(window) = stored_window_view(record.five_hour, 300, "5-hour limit", now_ms) {
        windows.push(window);
    }
    if let Some(window) = stored_window_view(record.seven_day, 10_080, "7-day limit", now_ms) {
        windows.push(window);
    }
    let stale = now_ms.saturating_sub(record.captured_at_ms) > CLAUDE_STALE_AFTER_MS;
    let state = if stale { "stale" } else { "available" };
    let detail = if stale {
        "Claude usage was captured earlier and may no longer be current."
    } else {
        "Usage limits captured by Claude Code."
    };
    provider_from_windows(windows, record.captured_at_ms, detail, state)
}

fn stored_window_view(
    window: Option<StoredUsageWindow>,
    duration_minutes: i64,
    label: &str,
    now_ms: i64,
) -> Option<UsageWindowView> {
    let window = window?;
    if !window.used_percent.is_finite() || !(0.0..=100.0).contains(&window.used_percent) {
        return None;
    }
    if window.resets_at_ms.is_some_and(|reset| reset < 0) {
        return None;
    }
    if window.resets_at_ms.is_some_and(|reset| reset <= now_ms) {
        return None;
    }
    Some(usage_window(
        label.to_string(),
        Some(duration_minutes),
        window.used_percent,
        window.resets_at_ms,
    ))
}

fn write_claude_record(path: &Path, record: &ClaudeUsageRecord) -> Result<(), String> {
    let bytes = serde_json::to_vec(record)
        .map_err(|_| "could not serialize the Claude usage record".to_string())?;
    atomic_write_bytes(path, &bytes)
}

fn atomic_write_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Claude usage record had no parent directory".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|_| "could not create the Claude usage directory".to_string())?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|_| "could not inspect the Claude usage directory".to_string())?;
    if is_link_or_reparse_point(&parent_metadata) || !parent_metadata.is_dir() {
        return Err("Claude usage directory was not a regular directory".to_string());
    }
    set_private_directory_permissions(parent);
    reject_unsafe_existing_target(path)?;

    let mut temporary = TempFileBuilder::new()
        .prefix(".claude-usage.")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|_| "could not create a temporary Claude usage record".to_string())?;
    set_private_file_permissions(temporary.path());
    temporary
        .write_all(content)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "could not write the temporary Claude usage record".to_string())?;
    replace_temporary_file(temporary, path)?;
    set_private_file_permissions(path);
    sync_parent(parent);
    Ok(())
}

fn reject_unsafe_existing_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) || !metadata.is_file() => {
            Err("Claude usage record target was not a regular file".to_string())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not inspect the Claude usage record".to_string()),
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
    temporary
        .persist(target)
        .map_err(|_| "could not atomically replace the Claude usage record".to_string())?;
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
        return Err("could not atomically replace the Claude usage record".to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn nul_terminated_wide_path(path: &Path) -> Result<Vec<u16>, String> {
    let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    if encoded.contains(&0) {
        return Err("Claude usage record path contained an embedded NUL".to_string());
    }
    encoded.push(0);
    Ok(encoded)
}

fn set_private_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
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

fn terminate_and_reap(child: &mut Child) -> io::Result<()> {
    if let Err(kill_error) = terminate_child(child) {
        match child.try_wait()? {
            Some(_) => return Ok(()),
            None => return Err(kill_error),
        }
    }
    child.wait().map(|_| ())
}

#[cfg(unix)]
fn terminate_child(child: &mut Child) -> io::Result<()> {
    const SIGKILL: i32 = 9;
    let process_group = i32::try_from(child.id())
        .map_err(|_| io::Error::other("Codex process identifier was out of range"))?;
    if unsafe { kill_process_group(-process_group, SIGKILL) } == 0 {
        Ok(())
    } else {
        child.kill()
    }
}

#[cfg(not(unix))]
fn terminate_child(child: &mut Child) -> io::Result<()> {
    child.kill()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::TempDir;

    #[derive(Clone)]
    struct FakeRunner(Result<Value, String>);

    impl CodexUsageRunner for FakeRunner {
        fn read_rate_limits(&self, _executable: &OsStr) -> Result<Value, String> {
            self.0.clone()
        }
    }

    fn claude_input(root: &Path, input: &[u8], now_ms: i64) -> (i32, Vec<u8>) {
        let mut output = Vec::new();
        let code = run_claude_statusline(input, &mut output, Some(root), now_ms);
        (code, output)
    }

    #[test]
    fn codex_prefers_named_bucket_and_reports_remaining_capacity() {
        let result = json!({
            "rateLimitsByLimitId": {
                "codex": {
                    "primary": {
                        "usedPercent": 23.5,
                        "windowDurationMins": 300,
                        "resetsAt": 1_800_000_000
                    },
                    "secondary": {
                        "usedPercent": 75,
                        "windowDurationMins": 10_080,
                        "resetsAt": 1_800_100_000
                    }
                }
            },
            "rateLimits": {
                "primary": {"usedPercent": 99, "windowDurationMins": 60}
            }
        });

        let view = codex_provider_view(&result, 10_000).unwrap();

        assert_eq!(view.state, "available");
        assert_eq!(view.remaining_percent, Some(25.0));
        assert_eq!(view.windows.len(), 2);
        assert_eq!(view.windows[0].label, "5-hour limit");
        assert_eq!(view.windows[0].remaining_percent, 76.5);
        assert_eq!(view.windows[1].label, "7-day limit");
        assert_eq!(view.windows[1].resets_at_ms, Some(1_800_100_000_000));
    }

    #[test]
    fn codex_legacy_bucket_supports_partial_and_clamped_windows() {
        let result = json!({
            "rateLimits": {
                "primary": null,
                "secondary": {
                    "usedPercent": 120,
                    "windowDurationMins": 120,
                    "resetsAt": -1
                }
            }
        });

        let view = codex_provider_view(&result, 20_000).unwrap();

        assert_eq!(view.remaining_percent, Some(0.0));
        assert_eq!(view.windows[0].label, "2-hour limit");
        assert_eq!(view.windows[0].used_percent, 100.0);
        assert_eq!(view.windows[0].resets_at_ms, None);
    }

    #[test]
    fn snapshot_runner_failures_are_unavailable_not_errors() {
        let temp = TempDir::new().unwrap();
        let snapshot = build_usage_snapshot_with(
            &FakeRunner(Err("signed out".to_string())),
            OsStr::new("fake-codex"),
            Some(temp.path()),
            42_000,
        );

        assert_eq!(snapshot.generated_at_ms, 42_000);
        assert_eq!(snapshot.codex.state, "unavailable");
        assert_eq!(snapshot.codex.remaining_percent, None);
        assert_eq!(snapshot.claude.state, "unavailable");
    }

    #[test]
    fn command_override_is_literal_and_empty_values_fall_back() {
        assert_eq!(
            codex_command_from(Some(OsString::from("/opt/Codex Preview/codex"))),
            OsString::from("/opt/Codex Preview/codex")
        );
        assert_eq!(codex_command_from(Some(OsString::new())), "codex");
        assert_eq!(codex_command_from(None), "codex");
    }

    #[test]
    fn duration_labels_are_ready_for_direct_display() {
        assert_eq!(duration_label(Some(300), "fallback"), "5-hour limit");
        assert_eq!(duration_label(Some(10_080), "fallback"), "7-day limit");
        assert_eq!(duration_label(Some(2_880), "fallback"), "2-day limit");
        assert_eq!(duration_label(Some(45), "fallback"), "45-minute limit");
        assert_eq!(duration_label(None, "Primary limit"), "Primary limit");
    }

    #[test]
    fn claude_capture_persists_only_rate_limit_fields() {
        let temp = TempDir::new().unwrap();
        let input = json!({
            "session_id": "private-session-id",
            "cwd": "/private/project",
            "transcript_path": "/private/transcript.jsonl",
            "prompt": "SECRET PROMPT",
            "rate_limits": {
                "five_hour": {"used_percentage": 20.5, "resets_at": 1_800_000_000},
                "seven_day": {"used_percentage": 45, "resets_at": 1_800_100_000}
            }
        });

        let (code, output) =
            claude_input(temp.path(), &serde_json::to_vec(&input).unwrap(), 50_000);

        assert_eq!(code, 0);
        assert!(output.is_empty());
        let record = fs::read(claude_record_path(temp.path())).unwrap();
        let text = String::from_utf8(record.clone()).unwrap();
        assert!(!text.contains("private"));
        assert!(!text.contains("SECRET"));
        assert!(!text.contains("session"));
        assert!(!text.contains("transcript"));
        let value: Value = serde_json::from_slice(&record).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            vec![
                "capturedAtMs".to_string(),
                "fiveHour".to_string(),
                "schemaVersion".to_string(),
                "sevenDay".to_string()
            ]
        );

        let view = load_claude_usage(temp.path(), 55_000);
        assert_eq!(view.state, "available");
        assert_eq!(view.remaining_percent, Some(55.0));
        assert_eq!(view.windows[0].remaining_percent, 79.5);
    }

    #[test]
    fn claude_partial_capture_and_stale_state_are_preserved() {
        let temp = TempDir::new().unwrap();
        let input = br#"{
            "rate_limits": {
                "five_hour": null,
                "seven_day": {"used_percentage": -5, "resets_at": 1800100000}
            }
        }"#;
        claude_input(temp.path(), input, 1_000);

        let fresh = load_claude_usage(temp.path(), 2_000);
        assert_eq!(fresh.state, "available");
        assert_eq!(fresh.windows.len(), 1);
        assert_eq!(fresh.windows[0].label, "7-day limit");
        assert_eq!(fresh.windows[0].used_percent, 0.0);
        assert_eq!(fresh.remaining_percent, Some(100.0));

        let stale = load_claude_usage(temp.path(), 1_000 + CLAUDE_STALE_AFTER_MS + 1);
        assert_eq!(stale.state, "stale");
        assert_eq!(stale.updated_at_ms, Some(1_000));
    }

    #[test]
    fn expired_claude_windows_are_omitted_and_never_shown_as_current() {
        let temp = TempDir::new().unwrap();
        let input = br#"{
            "rate_limits": {
                "five_hour": {"used_percentage": 90, "resets_at": 2},
                "seven_day": {"used_percentage": 40, "resets_at": 100}
            }
        }"#;
        claude_input(temp.path(), input, 1_000);

        let partially_expired = load_claude_usage(temp.path(), 3_000);
        assert_eq!(partially_expired.state, "available");
        assert_eq!(partially_expired.windows.len(), 1);
        assert_eq!(partially_expired.windows[0].label, "7-day limit");
        assert_eq!(partially_expired.remaining_percent, Some(60.0));

        let fully_expired = load_claude_usage(temp.path(), 100_000);
        assert_eq!(fully_expired.state, "unavailable");
        assert!(fully_expired.windows.is_empty());
        assert_eq!(fully_expired.remaining_percent, None);
    }

    #[test]
    fn missing_malformed_and_oversized_claude_inputs_fail_open() {
        let temp = TempDir::new().unwrap();
        for input in [b"{}".as_slice(), b"not json".as_slice()] {
            let (code, output) = claude_input(temp.path(), input, 1_000);
            assert_eq!(code, 0);
            assert!(output.is_empty());
        }
        let oversized = vec![b' '; CLAUDE_INPUT_LIMIT + 1];
        let (code, output) = claude_input(temp.path(), &oversized, 1_000);
        assert_eq!(code, 0);
        assert!(output.is_empty());
        assert!(!claude_record_path(temp.path()).exists());
    }

    #[cfg(unix)]
    #[test]
    fn claude_capture_refuses_a_symbolic_link_record() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let usage_directory = temp.path().join(CLAUDE_RECORD_DIRECTORY);
        fs::create_dir(&usage_directory).unwrap();
        let victim = temp.path().join("victim.json");
        fs::write(&victim, b"private\n").unwrap();
        symlink(&victim, claude_record_path(temp.path())).unwrap();
        let input = br#"{"rate_limits":{"five_hour":{"used_percentage":10}}}"#;

        let (code, output) = claude_input(temp.path(), input, 1_000);

        assert_eq!(code, 0);
        assert!(output.is_empty());
        assert_eq!(fs::read(victim).unwrap(), b"private\n");
    }

    #[test]
    fn claude_loader_rejects_unknown_or_oversized_records() {
        let temp = TempDir::new().unwrap();
        let path = claude_record_path(temp.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"{"schemaVersion":1,"capturedAtMs":1000,"fiveHour":{"usedPercent":20},"secret":"no"}"#,
        )
        .unwrap();
        assert_eq!(load_claude_usage(temp.path(), 2_000).state, "unavailable");

        fs::write(&path, vec![b'x'; CLAUDE_RECORD_LIMIT as usize + 1]).unwrap();
        assert_eq!(load_claude_usage(temp.path(), 2_000).state, "unavailable");
    }

    #[test]
    fn protocol_reader_enforces_line_and_total_caps() {
        let mut reader = Cursor::new(b"one\ntwo\nthree\n");
        let mut total = 0;
        assert_eq!(
            read_protocol_line(&mut reader, &mut total, 8, 8).unwrap(),
            Some(b"one\n".to_vec())
        );
        assert_eq!(
            read_protocol_line(&mut reader, &mut total, 8, 8).unwrap(),
            Some(b"two\n".to_vec())
        );
        assert!(read_protocol_line(&mut reader, &mut total, 8, 8).is_err());

        let mut reader = Cursor::new(b"too-long\n");
        let mut total = 0;
        assert!(read_protocol_line(&mut reader, &mut total, 4, 100).is_err());
    }

    #[test]
    fn overflowing_reset_timestamp_is_omitted() {
        let result = json!({
            "rateLimits": {
                "primary": {
                    "usedPercent": 10,
                    "windowDurationMins": 300,
                    "resetsAt": i64::MAX
                }
            }
        });
        let view = codex_provider_view(&result, 1).unwrap();
        assert_eq!(view.windows[0].resets_at_ms, None);
    }
}
