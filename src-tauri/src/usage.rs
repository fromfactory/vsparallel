//! Privacy-conscious usage collection for supported local coding agents.
//!
//! Codex exposes live limits through its documented app-server protocol. Claude
//! Code exposes equivalent values through the control channel used by its Agent
//! SDK; a compact status-line cache remains as a compatibility fallback. In
//! particular, Claude's session identifier, working directory, transcript path,
//! and prompt are never represented by the deserialization or persistence types
//! below. Antigravity quota is queried only through the provider-owned `agy`
//! read-only command. Gemini, Cursor Agent, and Zed expose local usage signals
//! rather than personal-plan quota, so their cards are explicitly labelled as
//! token or context metrics.

use crate::companion_integration::{CodeCliRunner, ProcessCodeCliRunner};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::Builder as TempFileBuilder;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

pub const CLAUDE_STATUSLINE_ARGUMENT: &str = "claude-usage";
pub const CURSOR_USAGE_ARGUMENT: &str = "cursor-usage";

const CODEX_EXTENSION_ID: &str = "openai.chatgpt";
const CLAUDE_EXTENSION_ID: &str = "anthropic.claude-code";
const USAGE_SCHEMA_VERSION: u32 = 1;
const CLAUDE_RECORD_SCHEMA_VERSION: u32 = 1;
const LOCAL_USAGE_RECORD_SCHEMA_VERSION: u32 = 1;
const CLAUDE_RECORD_DIRECTORY: &str = "usage";
const CLAUDE_RECORD_FILENAME: &str = "claude.json";
const GEMINI_RECORD_FILENAME: &str = "gemini.json";
const CURSOR_RECORD_FILENAME: &str = "cursor.json";
const CURSOR_TURN_RECORD_FILENAME: &str = "cursor-turn.json";
const CODEX_PROTOCOL_TIMEOUT: Duration = Duration::from_secs(6);
const CODEX_OUTPUT_LIMIT: usize = 256 * 1024;
const CODEX_LINE_LIMIT: usize = 128 * 1024;
const CLAUDE_PROTOCOL_TIMEOUT: Duration = Duration::from_secs(12);
const CLAUDE_OUTPUT_LIMIT: usize = 512 * 1024;
const CLAUDE_LINE_LIMIT: usize = 256 * 1024;
const VSCODE_EXTENSION_REGISTRY_LIMIT: u64 = 4 * 1024 * 1024;
const PROVIDER_GRACEFUL_EXIT_TIMEOUT: Duration = Duration::from_millis(250);
const PROVIDER_READER_JOIN_TIMEOUT: Duration = Duration::from_millis(250);
const PROVIDER_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLAUDE_INPUT_LIMIT: usize = 256 * 1024;
const GEMINI_USAGE_INPUT_LIMIT: usize = 32 * 1024 * 1024;
const CURSOR_USAGE_INPUT_LIMIT: usize = 2 * 1024 * 1024;
const CLAUDE_RECORD_LIMIT: u64 = 16 * 1024;
const LOCAL_USAGE_RECORD_LIMIT: u64 = 16 * 1024;
const CLAUDE_STALE_AFTER_MS: i64 = 15 * 60 * 1_000;
const LOCAL_USAGE_STALE_AFTER_MS: i64 = 15 * 60 * 1_000;
const LOCAL_USAGE_EXPIRES_AFTER_MS: i64 = 24 * 60 * 60 * 1_000;
const ANTIGRAVITY_COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
const ANTIGRAVITY_OUTPUT_LIMIT: usize = 512 * 1024;
const PROVIDER_EXTENSION_RETRY_AFTER: Duration = Duration::from_secs(5 * 60);
const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
const AUTOMATIC_SOURCE_PREFIX: &str = "vsparallel-source=automatic;";
const CONFIGURED_SOURCE_PREFIX: &str = "vsparallel-source=configured;";

static CODEX_EXTENSION_CACHE: OnceLock<Mutex<ProviderExtensionCache>> = OnceLock::new();
static CLAUDE_EXTENSION_CACHE: OnceLock<Mutex<ProviderExtensionCache>> = OnceLock::new();

#[derive(Debug, Default)]
struct ProviderExtensionCache {
    executable: Option<PathBuf>,
    checked_at: Option<Instant>,
}

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
    /// `quota`, `context`, `tokens`, or `none`.
    pub metric_kind: String,
    /// The most constrained window, so a single gauge never overstates capacity.
    pub remaining_percent: Option<f64>,
    /// Present only for an unbounded local token metric.
    pub token_count: Option<u64>,
    /// A concise description such as `Latest model call`.
    pub metric_label: String,
    pub windows: Vec<UsageWindowView>,
    pub updated_at_ms: Option<i64>,
    pub detail: String,
}

/// Usage data is intentionally separate from the frequently-polled workspace
/// snapshot because collecting live limits starts network-capable provider
/// subprocesses.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub schema_version: u32,
    pub generated_at_ms: i64,
    pub codex: ProviderUsageView,
    pub claude: ProviderUsageView,
    pub gemini: ProviderUsageView,
    pub antigravity: ProviderUsageView,
    pub zed: ProviderUsageView,
    pub cursor: ProviderUsageView,
}

/// Injectable boundary around the Codex app-server conversation.
pub trait CodexUsageRunner: Sync {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String>;
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessCodexUsageRunner {
    allow_extension_fallback: bool,
}

impl Default for ProcessCodexUsageRunner {
    fn default() -> Self {
        Self {
            allow_extension_fallback: true,
        }
    }
}

/// Injectable boundary around Claude Code's SDK control conversation.
pub trait ClaudeUsageRunner: Sync {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String>;
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessClaudeUsageRunner {
    allow_extension_fallback: bool,
}

impl Default for ProcessClaudeUsageRunner {
    fn default() -> Self {
        Self {
            allow_extension_fallback: true,
        }
    }
}

/// Injectable boundary around Antigravity CLI's documented, read-only
/// `/usage` JSON command.
pub trait AntigravityUsageRunner: Sync {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessAntigravityUsageRunner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCommand {
    pub executable: OsString,
    pub allow_extension_fallback: bool,
}

/// Resolve the Codex executable without splitting or interpreting shell syntax.
pub fn codex_command() -> ProviderCommand {
    codex_command_from(env::var_os("VSPARALLEL_CODEX_COMMAND"))
}

/// Resolve the Claude Code executable without splitting or interpreting shell syntax.
pub fn claude_command() -> ProviderCommand {
    claude_command_from(env::var_os("VSPARALLEL_CLAUDE_COMMAND"))
}

/// Resolve the official Antigravity CLI without interpreting shell syntax.
pub fn antigravity_command() -> ProviderCommand {
    provider_command_from(env::var_os("VSPARALLEL_ANTIGRAVITY_COMMAND"), "agy")
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
    let codex_command = codex_command();
    let claude_command = claude_command();
    let antigravity_command = antigravity_command();
    let state_root = crate::state::state_dir_from_environment().ok();
    let codex_runner = ProcessCodexUsageRunner {
        allow_extension_fallback: codex_command.allow_extension_fallback,
    };
    let claude_runner = ProcessClaudeUsageRunner {
        allow_extension_fallback: claude_command.allow_extension_fallback,
    };
    let antigravity_runner = ProcessAntigravityUsageRunner;

    let (mut snapshot, antigravity_result, zed_usage) = thread::scope(|scope| {
        let base = scope.spawn(|| {
            build_usage_snapshot_with(
                &codex_runner,
                codex_command.executable.as_os_str(),
                &claude_runner,
                claude_command.executable.as_os_str(),
                state_root.as_deref(),
                now_ms,
            )
        });
        let antigravity = scope.spawn(|| {
            antigravity_runner.read_rate_limits(antigravity_command.executable.as_os_str())
        });
        let zed = scope.spawn(|| crate::zed_integration::load_zed_usage_from_environment(now_ms));
        (
            base.join()
                .unwrap_or_else(|_| unavailable_usage_snapshot(now_ms)),
            antigravity.join().unwrap_or_else(|_| {
                Err("Antigravity usage worker stopped unexpectedly".to_string())
            }),
            zed.join().ok().flatten(),
        )
    });

    snapshot.antigravity = match antigravity_result {
        Ok(result) => antigravity_provider_view(&result, now_ms).unwrap_or_else(|| {
            unavailable_provider(
                "Antigravity returned no compatible quota buckets. Update Antigravity CLI, then refresh usage.",
            )
        }),
        Err(error) => unavailable_provider(&antigravity_failure_detail(&error)),
    };
    snapshot.zed = zed_usage
        .map(|usage| {
            token_provider(
                usage.total_tokens,
                usage.updated_at_ms,
                "Latest native thread",
                "Local tokens recorded by Zed; this is not plan or billing quota.",
                now_ms,
            )
        })
        .unwrap_or_else(|| {
            unavailable_provider(
                "No native Zed Agent token record is available yet. Start a Zed Agent turn, then refresh.",
            )
        });
    snapshot.gemini = reconcile_gemini_integration_status(
        snapshot.gemini,
        current_gemini_integration_status().ok().as_ref(),
    );
    snapshot
}

fn current_gemini_integration_status(
) -> Result<crate::gemini_integration::GeminiIntegrationStatus, String> {
    let config_dir = crate::gemini_integration::gemini_config_dir_from_environment()?;
    let executable = crate::integration_executable()?;
    crate::gemini_integration::gemini_integration_status(&config_dir, &executable)
}

fn reconcile_gemini_integration_status(
    mut usage: ProviderUsageView,
    status: Option<&crate::gemini_integration::GeminiIntegrationStatus>,
) -> ProviderUsageView {
    let Some(status) = status else {
        return usage;
    };
    let lifecycle_detail = match status.state.as_str() {
        "not_installed" => Some(
            "Gemini usage capture is not installed. Install it in Setup & diagnostics, restart Gemini CLI, then start a new turn.",
        ),
        "disabled" => Some(
            "Gemini usage capture is installed, but Gemini CLI settings disable hooks. Enable hooks in Gemini CLI settings, restart Gemini CLI, then start a new turn.",
        ),
        "stale" => Some(
            "The Gemini usage hook needs repair. Repair it in Setup & diagnostics, restart Gemini CLI, then start a new turn.",
        ),
        "conflict" => Some(
            "The Gemini usage hook name conflicts with another command. Resolve the conflict shown in Setup & diagnostics before starting a new turn.",
        ),
        _ => None,
    };
    let Some(lifecycle_detail) = lifecycle_detail else {
        return usage;
    };

    if usage.token_count.is_none() {
        return unavailable_provider(lifecycle_detail);
    }

    // Preserve the last privacy-filtered token count, but do not present it as
    // current when the hook that refreshes it cannot run.
    usage.state = "stale".to_string();
    usage.detail = lifecycle_detail.to_string();
    usage
}

/// Testable snapshot builder with injected process and state boundaries.
pub fn build_usage_snapshot_with<
    CodexRunner: CodexUsageRunner + ?Sized,
    ClaudeRunner: ClaudeUsageRunner + ?Sized,
>(
    codex_runner: &CodexRunner,
    codex_executable: &OsStr,
    claude_runner: &ClaudeRunner,
    claude_executable: &OsStr,
    state_root: Option<&Path>,
    now_ms: i64,
) -> UsageSnapshot {
    let (codex_result, claude_result) = thread::scope(|scope| {
        let codex = scope.spawn(|| codex_runner.read_rate_limits(codex_executable));
        let claude = scope.spawn(|| claude_runner.read_rate_limits(claude_executable));
        (
            codex
                .join()
                .unwrap_or_else(|_| Err("provider usage worker stopped unexpectedly".to_string())),
            claude
                .join()
                .unwrap_or_else(|_| Err("provider usage worker stopped unexpectedly".to_string())),
        )
    });
    let codex = match codex_result {
        Ok(result) => codex_provider_view(&result, now_ms).unwrap_or_else(|| {
            unavailable_provider(
                "Codex returned no compatible usage windows. Update Codex, then refresh usage.",
            )
        }),
        Err(error) => unavailable_provider(&provider_failure_detail(
            "Codex",
            "VSPARALLEL_CODEX_COMMAND",
            &error,
            "Install or sign in to Codex to view usage limits.",
        )),
    };
    let (claude_live, claude_error) = match claude_result {
        Ok(result) => (
            claude_provider_view(&result, now_ms),
            "Claude Code returned no compatible usage windows. Update Claude Code, then refresh usage."
                .to_string(),
        ),
        Err(error) => (
            None,
            provider_failure_detail(
                "Claude Code",
                "VSPARALLEL_CLAUDE_COMMAND",
                &error,
                "Update or sign in to Claude Code, then refresh usage.",
            ),
        ),
    };
    let claude = claude_live
        .or_else(|| {
            state_root
                .filter(|root| {
                    crate::state::integration_source_is_enabled_at(
                        root,
                        crate::state::IntegrationSource::ClaudeHooks,
                    )
                })
                .map(|root| load_claude_usage(root, now_ms))
                .filter(|view| view.remaining_percent.is_some())
        })
        .unwrap_or_else(|| unavailable_provider(&claude_error));

    let gemini = state_root
        .filter(|root| {
            crate::state::integration_source_is_enabled_at(
                root,
                crate::state::IntegrationSource::GeminiUsage,
            )
        })
        .map(|root| load_gemini_usage(root, now_ms))
        .unwrap_or_else(|| {
            unavailable_provider(
                "Gemini token capture is disabled. Enable it in Setup & diagnostics.",
            )
        });
    let cursor = state_root
        .filter(|root| {
            crate::state::integration_source_is_enabled_at(
                root,
                crate::state::IntegrationSource::CursorHooks,
            )
        })
        .map(|root| load_cursor_usage(root, now_ms))
        .unwrap_or_else(|| {
            unavailable_provider(
                "Cursor Agent context capture is disabled. Enable Cursor monitoring in Setup & diagnostics.",
            )
        });

    UsageSnapshot {
        schema_version: USAGE_SCHEMA_VERSION,
        generated_at_ms: now_ms,
        codex,
        claude,
        gemini,
        antigravity: unavailable_provider(
            "Install Antigravity CLI 1.1.11 or newer to view model quota.",
        ),
        zed: unavailable_provider(
            "No native Zed Agent token record is available yet. Start a Zed Agent turn, then refresh.",
        ),
        cursor,
    }
}

fn unavailable_usage_snapshot(now_ms: i64) -> UsageSnapshot {
    let unavailable = || unavailable_provider("Usage collection stopped unexpectedly.");
    UsageSnapshot {
        schema_version: USAGE_SCHEMA_VERSION,
        generated_at_ms: now_ms,
        codex: unavailable(),
        claude: unavailable(),
        gemini: unavailable(),
        antigravity: unavailable(),
        zed: unavailable(),
        cursor: unavailable(),
    }
}

impl CodexUsageRunner for ProcessCodexUsageRunner {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String> {
        read_codex_rate_limits(executable, self.allow_extension_fallback)
            .map_err(|error| tag_provider_failure(error, self.allow_extension_fallback))
    }
}

fn codex_request_with_extension_fallback(
    executable: &OsStr,
    mut request: impl FnMut(&OsStr) -> Result<Value, String>,
    mut bundled_executable: impl FnMut(bool) -> Option<PathBuf>,
) -> Result<Value, String> {
    if executable != OsStr::new("codex") {
        return request(executable);
    }
    if let Some(bundled) = bundled_executable(false) {
        return match request(bundled.as_os_str()) {
            Ok(result) => Ok(result),
            Err(bundled_error) => request(executable).or(Err(bundled_error)),
        };
    }
    let path_result = request(executable);
    if path_result.is_ok() {
        return path_result;
    }
    if let Some(bundled) = bundled_executable(true) {
        return request(bundled.as_os_str());
    }
    path_result
}

fn read_codex_rate_limits(
    executable: &OsStr,
    allow_extension_fallback: bool,
) -> Result<Value, String> {
    let params = Value::Object(Default::default());
    if allow_extension_fallback {
        codex_app_server_request_resolved(executable, "account/rateLimits/read", params)
    } else {
        codex_app_server_request(executable, "account/rateLimits/read", params)
    }
}

/// Send a Codex app-server request using either a selected non-default
/// executable or, for the `codex` command, the binary bundled with the installed
/// Codex extension in VS Code, Cursor, or Antigravity IDE as a fallback.
pub(crate) fn codex_app_server_request_resolved(
    executable: &OsStr,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    codex_request_with_extension_fallback(
        executable,
        |resolved| codex_app_server_request(resolved, method, params.clone()),
        cached_codex_extension_executable,
    )
}

impl ClaudeUsageRunner for ProcessClaudeUsageRunner {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String> {
        let result = if self.allow_extension_fallback {
            claude_request_with_extension_fallback(
                executable,
                claude_control_usage_request,
                cached_claude_extension_executable,
            )
        } else {
            claude_control_usage_request(executable)
        };
        result.map_err(|error| tag_provider_failure(error, self.allow_extension_fallback))
    }
}

impl AntigravityUsageRunner for ProcessAntigravityUsageRunner {
    fn read_rate_limits(&self, executable: &OsStr) -> Result<Value, String> {
        antigravity_usage_request(executable)
    }
}

#[derive(Debug, Deserialize)]
struct AntigravityCliResponse {
    status: String,
    #[serde(default)]
    command: Option<AntigravityCliCommand>,
}

#[derive(Debug, Deserialize)]
struct AntigravityCliCommand {
    name: String,
    #[serde(default)]
    data: Option<AntigravityCliUsageData>,
}

#[derive(Debug, Deserialize)]
struct AntigravityCliUsageData {
    #[serde(default)]
    groups: Vec<AntigravityCliUsageGroup>,
}

#[derive(Debug, Deserialize)]
struct AntigravityCliUsageGroup {
    #[serde(default)]
    buckets: Vec<AntigravityCliUsageBucket>,
}

#[derive(Debug, Deserialize)]
struct AntigravityCliUsageBucket {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    window: String,
    #[serde(default)]
    remaining_fraction: Option<f64>,
    #[serde(default)]
    reset_time: Option<String>,
}

/// Invoke only the official, read-only Antigravity slash command introduced in
/// CLI 1.1.11. The broad response is immediately narrowed to display-safe
/// bucket fields; authentication data is never read by VSParallel.
fn antigravity_usage_request(executable: &OsStr) -> Result<Value, String> {
    antigravity_usage_request_with_timeout(executable, ANTIGRAVITY_COMMAND_TIMEOUT)
}

fn antigravity_usage_request_with_timeout(
    executable: &OsStr,
    command_timeout: Duration,
) -> Result<Value, String> {
    let mut command = Command::new(executable);
    command
        .args(["-p", "/usage", "--output-format", "json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "could not start Antigravity CLI".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not read Antigravity CLI output".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "could not read Antigravity CLI diagnostics".to_string())?;
    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    let _stdout_reader = thread::spawn(move || {
        let _ = stdout_sender.send(read_capped_bytes(stdout, ANTIGRAVITY_OUTPUT_LIMIT));
    });
    let _stderr_reader = thread::spawn(move || {
        let _ = stderr_sender.send(drain_capped(stderr, ANTIGRAVITY_OUTPUT_LIMIT).map(|_| ()));
    });
    let deadline = Instant::now() + command_timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(PROVIDER_EXIT_POLL_INTERVAL);
            }
            Ok(None) => {
                let _ = terminate_and_reap(&mut child);
                return Err("Antigravity CLI usage request timed out".to_string());
            }
            Err(_) => {
                let _ = terminate_and_reap(&mut child);
                return Err("Antigravity CLI stopped unexpectedly".to_string());
            }
        }
    };
    if !status.success() {
        // A descendant may still own an inherited pipe even though the CLI
        // process has exited. Best-effort group termination keeps that work
        // from lingering; the detached readers never hold up this refresh.
        let _ = terminate_child(&mut child);
        return Err("Antigravity CLI rejected the usage request".to_string());
    }
    let stdout =
        match stdout_receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(stdout)) => stdout,
            Ok(Err(_)) => {
                let _ = terminate_child(&mut child);
                return Err("Antigravity CLI output exceeded its safety limit".to_string());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = terminate_child(&mut child);
                return Err("Antigravity CLI usage request timed out".to_string());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = terminate_child(&mut child);
                return Err("Antigravity CLI output reader stopped unexpectedly".to_string());
            }
        };
    match stderr_receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            let _ = terminate_child(&mut child);
            return Err("Antigravity CLI diagnostics exceeded their safety limit".to_string());
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = terminate_child(&mut child);
            return Err("Antigravity CLI usage request timed out".to_string());
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = terminate_child(&mut child);
            return Err("Antigravity CLI diagnostic reader stopped unexpectedly".to_string());
        }
    }

    parse_antigravity_usage_output(&stdout)
}

fn parse_antigravity_usage_output(stdout: &[u8]) -> Result<Value, String> {
    let response: AntigravityCliResponse = serde_json::from_slice(stdout)
        .map_err(|_| "Antigravity CLI returned malformed usage output".to_string())?;
    if response.status != "SUCCESS" {
        return Err("Antigravity CLI did not return a successful usage result".to_string());
    }
    let command = response
        .command
        .filter(|command| command.name == "usage")
        .ok_or_else(|| "Antigravity CLI returned a different command result".to_string())?;
    let data = command
        .data
        .ok_or_else(|| "Antigravity CLI usage result had no quota data".to_string())?;
    let buckets: Vec<Value> = data
        .groups
        .into_iter()
        .flat_map(|group| group.buckets)
        .take(64)
        .filter_map(|bucket| {
            let remaining_fraction = bucket
                .remaining_fraction
                .filter(|value| value.is_finite())?;
            let name = bounded_display_value(&bucket.name, 128)
                .or_else(|| bounded_display_value(&bucket.id, 128))?;
            let window = bounded_display_value(&bucket.window, 128).unwrap_or_default();
            let reset_time = bucket
                .reset_time
                .as_deref()
                .and_then(|value| bounded_display_value(value, 128));
            Some(json!({
                "name": name,
                "window": window,
                "remainingFraction": remaining_fraction,
                "resetTime": reset_time,
            }))
        })
        .collect();
    Ok(json!({ "buckets": buckets }))
}

fn bounded_display_value(value: &str, maximum_bytes: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= maximum_bytes && !value.chars().any(char::is_control))
        .then(|| value.to_string())
}

fn tag_provider_failure(error: String, automatic: bool) -> String {
    format!(
        "{}{error}",
        if automatic {
            AUTOMATIC_SOURCE_PREFIX
        } else {
            CONFIGURED_SOURCE_PREFIX
        }
    )
}

fn claude_request_with_extension_fallback(
    executable: &OsStr,
    mut request: impl FnMut(&OsStr) -> Result<Value, String>,
    mut bundled_executable: impl FnMut(bool) -> Option<PathBuf>,
) -> Result<Value, String> {
    if executable != OsStr::new("claude") {
        return request(executable);
    }
    if let Some(bundled) = bundled_executable(false) {
        return match request(bundled.as_os_str()) {
            Ok(result) => Ok(result),
            Err(bundled_error) => request(executable).or(Err(bundled_error)),
        };
    }
    let path_result = request(executable);
    if path_result.is_ok() {
        return path_result;
    }
    if let Some(bundled) = bundled_executable(true) {
        return request(bundled.as_os_str());
    }
    path_result
}

fn cached_codex_extension_executable(allow_lookup: bool) -> Option<PathBuf> {
    cached_provider_extension_executable(
        &CODEX_EXTENSION_CACHE,
        allow_lookup,
        locate_codex_extension_executable,
    )
}

fn cached_claude_extension_executable(allow_lookup: bool) -> Option<PathBuf> {
    cached_provider_extension_executable(
        &CLAUDE_EXTENSION_CACHE,
        allow_lookup,
        locate_claude_extension_executable,
    )
}

fn cached_provider_extension_executable(
    storage: &OnceLock<Mutex<ProviderExtensionCache>>,
    allow_lookup: bool,
    locate: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    let cache = storage.get_or_init(|| Mutex::new(ProviderExtensionCache::default()));
    let now = Instant::now();
    {
        let mut cache = cache.lock().ok()?;
        if cache
            .executable
            .as_ref()
            .is_some_and(|executable| executable.is_file())
        {
            return cache.executable.clone();
        }
        cache.executable = None;
        if !allow_lookup
            || cache
                .checked_at
                .is_some_and(|checked| now.duration_since(checked) < PROVIDER_EXTENSION_RETRY_AFTER)
        {
            return None;
        }
        cache.checked_at = Some(now);
    }
    let executable = locate();
    if let Ok(mut cache) = cache.lock() {
        cache.executable = executable.clone();
    }
    executable
}

fn locate_codex_extension_executable() -> Option<PathBuf> {
    locate_provider_extension_executable(
        automatic_extension_cli_lookup_enabled_for_platform(env::consts::OS),
        locate_codex_extension_with_cli,
        locate_codex_extension_from_registry,
    )
}

fn locate_claude_extension_executable() -> Option<PathBuf> {
    locate_provider_extension_executable(
        automatic_extension_cli_lookup_enabled_for_platform(env::consts::OS),
        locate_claude_extension_with_cli,
        locate_claude_extension_from_registry,
    )
}

fn automatic_extension_cli_lookup_enabled_for_platform(platform: &str) -> bool {
    // The bundled launchers for VS Code-compatible macOS applications are shell
    // scripts that start the application's Electron binary as a short-lived Node
    // process. Some editor builds can abort that helper during teardown and make
    // macOS present a native crash report even though VSParallel only requested
    // extension metadata. The bounded local registries below contain the same
    // installed-extension location needed by this automatic fallback, so avoid
    // starting those editor-owned helpers on macOS.
    platform != "macos"
}

fn locate_provider_extension_executable<C, R>(
    allow_cli_lookup: bool,
    locate_with_cli: C,
    locate_from_registry: R,
) -> Option<PathBuf>
where
    C: FnOnce() -> Option<PathBuf>,
    R: FnOnce() -> Option<PathBuf>,
{
    if allow_cli_lookup {
        choose_newest_extension_executable(locate_with_cli(), locate_from_registry())
    } else {
        locate_from_registry()
    }
}

fn choose_newest_extension_executable(
    cli: Option<PathBuf>,
    registry: Option<PathBuf>,
) -> Option<PathBuf> {
    [cli, registry].into_iter().flatten().max_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::UNIX_EPOCH)
    })
}

fn locate_codex_extension_with_cli() -> Option<PathBuf> {
    locate_extension_with_cli(CODEX_EXTENSION_ID, codex_extension_binary)
}

fn locate_claude_extension_with_cli() -> Option<PathBuf> {
    locate_extension_with_cli(CLAUDE_EXTENSION_ID, claude_extension_binary)
}

fn locate_extension_with_cli(
    extension_id: &str,
    binary: fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let commands = [
        OsString::from(crate::opener::code_command()),
        OsString::from(crate::opener::cursor_command()),
        OsString::from(crate::opener::antigravity_ide_command()),
    ];
    locate_extension_with_commands(&ProcessCodeCliRunner, commands, extension_id, binary)
}

fn locate_extension_with_commands(
    runner: &impl CodeCliRunner,
    commands: impl IntoIterator<Item = OsString>,
    extension_id: &str,
    binary: fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let arguments = [
        OsString::from("--locate-extension"),
        OsString::from(extension_id),
    ];
    let mut attempted = Vec::new();
    for command in commands {
        if command.is_empty() || attempted.contains(&command) {
            continue;
        }
        attempted.push(command.clone());
        let Ok(output) = runner.run(command.as_os_str(), &arguments) else {
            continue;
        };
        if output.success {
            if let Some(executable) = extension_executable_from_output(&output.stdout, binary) {
                return Some(executable);
            }
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct EditorExtensionRegistryEntry {
    identifier: EditorExtensionIdentifier,
    #[serde(rename = "relativeLocation")]
    relative_location: Option<String>,
    #[serde(default)]
    metadata: EditorExtensionMetadata,
}

#[derive(Debug, Deserialize)]
struct EditorExtensionIdentifier {
    id: String,
}

#[derive(Debug, Default, Deserialize)]
struct EditorExtensionMetadata {
    #[serde(default, rename = "installedTimestamp")]
    installed_timestamp: u64,
}

/// An editor launcher can be missing from PATH or unusable in a confined
/// package even while its extensions are available. The bounded local VS Code
/// Antigravity IDE, and Cursor registries are a second source for the exact installed
/// extension path.
fn locate_codex_extension_from_registry() -> Option<PathBuf> {
    locate_extension_from_registry(CODEX_EXTENSION_ID, codex_extension_binary)
}

fn locate_claude_extension_from_registry() -> Option<PathBuf> {
    locate_extension_from_registry(CLAUDE_EXTENSION_ID, claude_extension_binary)
}

fn locate_extension_from_registry(
    extension_id: &str,
    binary: fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let home = platform_home_directory()?;
    locate_extension_from_registry_under_home(&home, extension_id, binary)
}

fn locate_extension_from_registry_under_home(
    home: &Path,
    extension_id: &str,
    binary: fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    vs_compatible_extension_directories(home)
        .into_iter()
        .filter_map(|directory| extension_from_registry(&directory, extension_id, binary))
        .max_by_key(|(installed_at, _)| *installed_at)
        .map(|(_, executable)| executable)
}

fn platform_home_directory() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let home = env::var_os("USERPROFILE").or_else(|| {
        let drive = env::var_os("HOMEDRIVE")?;
        let path = env::var_os("HOMEPATH")?;
        Some(PathBuf::from(drive).join(path).into_os_string())
    });
    #[cfg(not(target_os = "windows"))]
    let home = env::var_os("HOME");
    home.filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn vs_compatible_extension_directories(home: &Path) -> [PathBuf; 5] {
    [
        home.join(".vscode").join("extensions"),
        home.join(".vscode-insiders").join("extensions"),
        home.join(".vscode-oss").join("extensions"),
        home.join(".cursor").join("extensions"),
        home.join(".antigravity-ide").join("extensions"),
    ]
}

#[cfg(test)]
fn claude_extension_from_registry(extensions: &Path) -> Option<(u64, PathBuf)> {
    extension_from_registry(extensions, CLAUDE_EXTENSION_ID, claude_extension_binary)
}

#[cfg(test)]
fn codex_extension_from_registry(extensions: &Path) -> Option<(u64, PathBuf)> {
    extension_from_registry(extensions, CODEX_EXTENSION_ID, codex_extension_binary)
}

fn extension_from_registry(
    extensions: &Path,
    extension_id: &str,
    binary: fn(&Path) -> Option<PathBuf>,
) -> Option<(u64, PathBuf)> {
    let registry = extensions.join("extensions.json");
    let metadata = fs::metadata(&registry).ok()?;
    if !metadata.is_file() || metadata.len() > VSCODE_EXTENSION_REGISTRY_LIMIT {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(registry)
        .ok()?
        .take(VSCODE_EXTENSION_REGISTRY_LIMIT + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > VSCODE_EXTENSION_REGISTRY_LIMIT {
        return None;
    }
    let entries: Vec<EditorExtensionRegistryEntry> = serde_json::from_slice(&bytes).ok()?;
    entries
        .into_iter()
        .filter(|entry| entry.identifier.id.eq_ignore_ascii_case(extension_id))
        .filter_map(|entry| {
            let relative = PathBuf::from(entry.relative_location?);
            let mut components = relative.components();
            let root = match (components.next(), components.next()) {
                (Some(std::path::Component::Normal(name)), None) => extensions.join(name),
                _ => return None,
            };
            binary(&root).map(|executable| (entry.metadata.installed_timestamp, executable))
        })
        .max_by_key(|(installed_at, _)| *installed_at)
}

#[cfg(test)]
fn codex_extension_executable_from_output(output: &[u8]) -> Option<PathBuf> {
    extension_executable_from_output(output, codex_extension_binary)
}

#[cfg(test)]
fn claude_extension_executable_from_output(output: &[u8]) -> Option<PathBuf> {
    extension_executable_from_output(output, claude_extension_binary)
}

fn extension_executable_from_output(
    output: &[u8],
    binary: fn(&Path) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let root = PathBuf::from(std::str::from_utf8(output).ok()?.trim());
    if root.as_os_str().is_empty() {
        return None;
    }
    binary(&root)
}

fn codex_extension_binary(root: &Path) -> Option<PathBuf> {
    let binary_name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        std::env::consts::OS
    };
    let architecture = std::env::consts::ARCH;
    codex_extension_binary_for(root, platform, architecture, binary_name)
}

fn codex_extension_binary_for(
    root: &Path,
    platform: &str,
    architecture: &str,
    binary_name: &str,
) -> Option<PathBuf> {
    let bin = root.join("bin");
    let mut candidates = vec![bin
        .join(format!("{platform}-{architecture}"))
        .join(binary_name)];
    if platform == "macos" {
        match architecture {
            "aarch64" => candidates.push(bin.join("macos-x86_64").join(binary_name)),
            "x86_64" => candidates.push(bin.join("macos-aarch64").join(binary_name)),
            _ => {}
        }
    }
    if platform == "windows" && architecture == "aarch64" {
        candidates.push(bin.join("windows-x86_64").join(binary_name));
    }
    candidates.push(bin.join(binary_name));
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn claude_extension_binary(root: &Path) -> Option<PathBuf> {
    let binary_name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let platform = if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        architecture => architecture,
    };
    claude_extension_binary_for(
        root,
        platform,
        architecture,
        binary_name,
        cfg!(all(target_os = "linux", target_env = "musl")),
    )
}

fn claude_extension_binary_for(
    root: &Path,
    platform: &str,
    architecture: &str,
    binary_name: &str,
    musl: bool,
) -> Option<PathBuf> {
    let resources = root.join("resources");
    let platform_architecture = if platform == "linux" && musl {
        format!("{platform}-{architecture}-musl")
    } else {
        format!("{platform}-{architecture}")
    };
    let native_binaries = resources.join("native-binaries");
    let mut candidates = vec![native_binaries
        .join(platform_architecture)
        .join(binary_name)];
    if platform == "darwin" {
        match architecture {
            "arm64" => candidates.push(native_binaries.join("darwin-x64").join(binary_name)),
            "x64" => candidates.push(native_binaries.join("darwin-arm64").join(binary_name)),
            _ => {}
        }
    }
    if platform == "win32" && architecture == "arm64" {
        candidates.push(native_binaries.join("win32-x64").join(binary_name));
    }
    candidates.push(resources.join("native-binary").join(binary_name));
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// Send one initialized request to the installed Codex app-server.
///
/// Integration status and live usage use the same bounded subprocess protocol,
/// so neither path can leave a Codex child running or consume unbounded output.
pub(crate) fn codex_app_server_request(
    executable: &OsStr,
    method: &str,
    params: Value,
) -> Result<Value, String> {
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
        .map_err(|_| "could not start the Codex app-server".to_string())?;
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = terminate_and_reap(&mut child);
            return Err("Codex app-server input was unavailable".to_string());
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            drop(stdin);
            let _ = terminate_and_reap(&mut child);
            return Err("Codex app-server output was unavailable".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            drop(stdin);
            drop(stdout);
            let _ = terminate_and_reap(&mut child);
            return Err("Codex app-server diagnostics were unavailable".to_string());
        }
    };

    let (line_sender, line_receiver) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        pump_protocol_lines(stdout, line_sender, CODEX_LINE_LIMIT, CODEX_OUTPUT_LIMIT)
    });
    let stderr_reader = thread::spawn(move || drain_capped(stderr, CODEX_OUTPUT_LIMIT).map(|_| ()));
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
                "method": method,
                "id": 2,
                "params": params
            }),
        )?;
        wait_for_rpc_response(&line_receiver, 2, deadline)
    })();

    drop(stdin);
    let _ = finish_provider_process(&mut child);
    join_provider_reader(stdout_reader);
    join_provider_reader(stderr_reader);
    protocol_result
}

/// Ask the installed Claude Code process for the subscription usage view used
/// by its Agent SDK. This control method is intentionally isolated here because
/// its upstream name marks it as an evolving compatibility interface.
fn claude_control_usage_request(executable: &OsStr) -> Result<Value, String> {
    // Claude's current full-usage getter also scans its configured project
    // history for attribution summaries. Point only that history/config root at
    // an empty ephemeral directory while leaving credential storage under the
    // provider's own original secure-storage root.
    let isolated_config = TempFileBuilder::new()
        .prefix("vsparallel-claude-usage.")
        .tempdir()
        .map_err(|_| "could not isolate the Claude Code usage query".to_string())?;
    set_private_directory_permissions(isolated_config.path());
    let secure_storage_config = claude_secure_storage_config_from(
        env::var_os("CLAUDE_SECURESTORAGE_CONFIG_DIR"),
        env::var_os("CLAUDE_CONFIG_DIR"),
    );
    let mut command = Command::new(executable);
    command
        .args([
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--tools",
            "",
            "--permission-mode",
            "dontAsk",
            "--no-session-persistence",
            "--safe-mode",
            "--prompt-suggestions",
            "false",
        ])
        .env("CLAUDE_CODE_ENTRYPOINT", "sdk-ts")
        .env("CLAUDE_CONFIG_DIR", isolated_config.path())
        .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", secure_storage_config)
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
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
        .map_err(|_| "could not start Claude Code".to_string())?;
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = terminate_and_reap(&mut child);
            return Err("Claude Code input was unavailable".to_string());
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            drop(stdin);
            let _ = terminate_and_reap(&mut child);
            return Err("Claude Code output was unavailable".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            drop(stdin);
            drop(stdout);
            let _ = terminate_and_reap(&mut child);
            return Err("Claude Code diagnostics were unavailable".to_string());
        }
    };

    let (line_sender, line_receiver) = mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        pump_protocol_lines(stdout, line_sender, CLAUDE_LINE_LIMIT, CLAUDE_OUTPUT_LIMIT)
    });
    let stderr_reader =
        thread::spawn(move || drain_capped(stderr, CLAUDE_OUTPUT_LIMIT).map(|_| ()));
    let deadline = Instant::now() + CLAUDE_PROTOCOL_TIMEOUT;

    let protocol_result = (|| {
        send_claude_control_request(&mut stdin, "vsparallel-initialize", "initialize")?;
        let _ =
            wait_for_claude_control_response(&line_receiver, "vsparallel-initialize", deadline)?;
        send_claude_control_request(&mut stdin, "vsparallel-usage", "get_usage")?;
        wait_for_claude_control_response(&line_receiver, "vsparallel-usage", deadline)
    })();

    drop(stdin);
    let _ = finish_provider_process(&mut child);
    join_provider_reader(stdout_reader);
    join_provider_reader(stderr_reader);
    protocol_result
}

fn claude_secure_storage_config_from(
    secure_storage: Option<OsString>,
    claude_config: Option<OsString>,
) -> OsString {
    secure_storage.or(claude_config).unwrap_or_default()
}

fn send_claude_control_request(
    writer: &mut impl Write,
    request_id: &str,
    subtype: &str,
) -> Result<(), String> {
    serde_json::to_writer(
        &mut *writer,
        &json!({
            "request_id": request_id,
            "type": "control_request",
            "request": {"subtype": subtype}
        }),
    )
    .map_err(|_| "could not encode a Claude Code control request".to_string())?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|_| "could not send a Claude Code control request".to_string())
}

#[derive(Debug, Deserialize)]
struct ClaudeControlMessage {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    response: Option<ClaudeControlResponse>,
}

#[derive(Debug, Deserialize)]
struct ClaudeControlResponse {
    subtype: String,
    request_id: String,
    #[serde(default)]
    response: Option<ClaudeControlResult>,
}

/// Deliberately omits account, session, behavior-attribution, spend, and
/// auxiliary limit fields returned by some Claude Code versions. Serde skips
/// those fields without representing them in the result that leaves the
/// protocol reader.
#[derive(Debug, Default, Deserialize)]
struct ClaudeControlResult {
    #[serde(default)]
    rate_limits: Option<ClaudeControlRateLimits>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct ClaudeControlRateLimits {
    #[serde(default)]
    five_hour: Option<ClaudeControlWindow>,
    #[serde(default)]
    seven_day: Option<ClaudeControlWindow>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ClaudeControlWindow {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<ClaudeControlReset>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ClaudeControlReset {
    Seconds(i64),
    Timestamp(String),
}

fn wait_for_claude_control_response(
    receiver: &mpsc::Receiver<Result<Vec<u8>, String>>,
    expected_request_id: &str,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "Claude Code usage request timed out".to_string())?;
        let line = receiver
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    "Claude Code usage request timed out".to_string()
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    "Claude Code stopped before responding".to_string()
                }
            })??;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let message: ClaudeControlMessage = serde_json::from_slice(&line)
            .map_err(|_| "Claude Code returned malformed output".to_string())?;
        if message.message_type != "control_response" {
            continue;
        }
        let response = match message.response {
            Some(response) if response.request_id == expected_request_id => response,
            _ => continue,
        };
        if response.subtype != "success" {
            return Err("Claude Code rejected the usage request".to_string());
        }
        let result = response
            .response
            .ok_or_else(|| "Claude Code usage response had no result".to_string())?;
        return Ok(json!({"rate_limits": result.rate_limits}));
    }
}

fn send_rpc(writer: &mut impl Write, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, message)
        .map_err(|_| "could not encode a Codex app-server request".to_string())?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|_| "could not send a Codex app-server request".to_string())
}

fn wait_for_rpc_response(
    receiver: &mpsc::Receiver<Result<Vec<u8>, String>>,
    expected_id: i64,
    deadline: Instant,
) -> Result<Value, String> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| "Codex app-server request timed out".to_string())?;
        let line = receiver
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "Codex app-server request timed out".to_string(),
                mpsc::RecvTimeoutError::Disconnected => {
                    "Codex app-server stopped before responding".to_string()
                }
            })??;
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let response: Value = serde_json::from_slice(&line)
            .map_err(|_| "Codex app-server returned malformed output".to_string())?;
        if response.get("id").and_then(Value::as_i64) != Some(expected_id) {
            continue;
        }
        if response.get("error").is_some() {
            return Err("Codex app-server rejected the request".to_string());
        }
        return response
            .get("result")
            .cloned()
            .ok_or_else(|| "Codex app-server response had no result".to_string());
    }
}

fn pump_protocol_lines(
    stdout: impl Read,
    sender: mpsc::Sender<Result<Vec<u8>, String>>,
    line_limit: usize,
    total_limit: usize,
) -> io::Result<()> {
    let mut reader = BufReader::new(stdout);
    let mut total = 0;
    loop {
        match read_protocol_line(&mut reader, &mut total, line_limit, total_limit) {
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
                "provider process output exceeded its safety limit",
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
        "provider diagnostic output exceeded its safety limit",
    ))
}

fn read_capped_bytes(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    reader.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "provider process output exceeded its safety limit",
        ));
    }
    Ok(bytes)
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

fn claude_provider_view(result: &Value, now_ms: i64) -> Option<ProviderUsageView> {
    let limits = result.get("rate_limits")?;
    let mut windows = Vec::with_capacity(2);
    if let Some(window) = limits
        .get("five_hour")
        .and_then(|window| claude_live_window_view(window, 300, "5-hour limit", now_ms))
    {
        windows.push(window);
    }
    if let Some(window) = limits
        .get("seven_day")
        .and_then(|window| claude_live_window_view(window, 10_080, "7-day limit", now_ms))
    {
        windows.push(window);
    }
    provider_from_windows(
        windows,
        now_ms,
        "Live usage limits from Claude Code.",
        "available",
    )
}

fn antigravity_provider_view(result: &Value, now_ms: i64) -> Option<ProviderUsageView> {
    let buckets = result.get("buckets")?.as_array()?;
    let mut windows = Vec::with_capacity(buckets.len().min(64));
    for bucket in buckets.iter().take(64) {
        let remaining_fraction = finite_number(bucket.get("remainingFraction")?)?;
        let remaining_percent = (remaining_fraction * 100.0).clamp(0.0, 100.0);
        let name = bucket
            .get("name")
            .and_then(Value::as_str)
            .and_then(|value| bounded_display_value(value, 128))?;
        let window = bucket
            .get("window")
            .and_then(Value::as_str)
            .and_then(|value| bounded_display_value(value, 128));
        let label = window
            .filter(|value| {
                !name
                    .to_ascii_lowercase()
                    .contains(&value.to_ascii_lowercase())
            })
            .map_or(name.clone(), |value| format!("{name} · {value}"));
        let resets_at_ms = bucket
            .get("resetTime")
            .and_then(Value::as_str)
            .and_then(rfc3339_to_millis);
        if resets_at_ms.is_some_and(|reset| reset <= now_ms) {
            continue;
        }
        windows.push(usage_window(
            label,
            None,
            100.0 - remaining_percent,
            resets_at_ms,
        ));
    }
    provider_from_windows(
        windows,
        now_ms,
        "Live model quota from the official Antigravity CLI.",
        "available",
    )
}

fn claude_live_window_view(
    value: &Value,
    duration_minutes: i64,
    label: &str,
    now_ms: i64,
) -> Option<UsageWindowView> {
    let used_percent = finite_number(value.get("utilization")?)?;
    let resets_at_ms = value.get("resets_at").and_then(claude_reset_to_millis);
    if resets_at_ms.is_some_and(|reset| reset <= now_ms) {
        return None;
    }
    Some(usage_window(
        label.to_string(),
        Some(duration_minutes),
        used_percent,
        resets_at_ms,
    ))
}

fn claude_reset_to_millis(value: &Value) -> Option<i64> {
    if let Some(seconds) = value.as_i64() {
        return seconds_to_millis(seconds);
    }
    let timestamp = OffsetDateTime::parse(value.as_str()?, &Rfc3339).ok()?;
    seconds_to_millis(timestamp.unix_timestamp())?.checked_add(i64::from(timestamp.millisecond()))
}

fn rfc3339_to_millis(value: &str) -> Option<i64> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    seconds_to_millis(timestamp.unix_timestamp())?.checked_add(i64::from(timestamp.millisecond()))
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
    let limiting_window = windows
        .iter()
        .min_by(|left, right| left.remaining_percent.total_cmp(&right.remaining_percent))?;
    let remaining_percent = limiting_window.remaining_percent;
    let metric_label = limiting_window.label.clone();
    Some(ProviderUsageView {
        state: state.to_string(),
        metric_kind: "quota".to_string(),
        remaining_percent: Some(remaining_percent),
        token_count: None,
        metric_label,
        windows,
        updated_at_ms: Some(updated_at_ms),
        detail: detail.to_string(),
    })
}

fn unavailable_provider(detail: &str) -> ProviderUsageView {
    ProviderUsageView {
        state: "unavailable".to_string(),
        metric_kind: "none".to_string(),
        remaining_percent: None,
        token_count: None,
        metric_label: String::new(),
        windows: Vec::new(),
        updated_at_ms: None,
        detail: detail.to_string(),
    }
}

fn token_provider(
    token_count: u64,
    updated_at_ms: i64,
    metric_label: &str,
    detail: &str,
    now_ms: i64,
) -> ProviderUsageView {
    let age_ms = now_ms.saturating_sub(updated_at_ms);
    if updated_at_ms < 0
        || updated_at_ms > now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
        || age_ms > LOCAL_USAGE_EXPIRES_AFTER_MS
    {
        return unavailable_provider(detail);
    }
    ProviderUsageView {
        state: if age_ms > LOCAL_USAGE_STALE_AFTER_MS {
            "stale".to_string()
        } else {
            "available".to_string()
        },
        metric_kind: "tokens".to_string(),
        remaining_percent: None,
        token_count: Some(token_count),
        metric_label: metric_label.to_string(),
        windows: Vec::new(),
        updated_at_ms: Some(updated_at_ms),
        detail: detail.to_string(),
    }
}

fn context_provider(
    remaining_percent: f64,
    updated_at_ms: i64,
    metric_label: &str,
    detail: &str,
    now_ms: i64,
) -> ProviderUsageView {
    if !remaining_percent.is_finite() {
        return unavailable_provider(detail);
    }
    let age_ms = now_ms.saturating_sub(updated_at_ms);
    if updated_at_ms < 0
        || updated_at_ms > now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
        || age_ms > LOCAL_USAGE_EXPIRES_AFTER_MS
    {
        return unavailable_provider(detail);
    }
    let remaining_percent = remaining_percent.clamp(0.0, 100.0);
    ProviderUsageView {
        state: if age_ms > LOCAL_USAGE_STALE_AFTER_MS {
            "stale".to_string()
        } else {
            "available".to_string()
        },
        metric_kind: "context".to_string(),
        remaining_percent: Some(remaining_percent),
        token_count: None,
        metric_label: metric_label.to_string(),
        windows: vec![usage_window(
            metric_label.to_string(),
            None,
            100.0 - remaining_percent,
            None,
        )],
        updated_at_ms: Some(updated_at_ms),
        detail: detail.to_string(),
    }
}

fn provider_failure_detail(
    provider: &str,
    override_variable: &str,
    tagged_error: &str,
    fallback: &str,
) -> String {
    let (source, error) = if let Some(error) = tagged_error.strip_prefix(AUTOMATIC_SOURCE_PREFIX) {
        ("automatic", error)
    } else if let Some(error) = tagged_error.strip_prefix(CONFIGURED_SOURCE_PREFIX) {
        ("configured", error)
    } else {
        ("unknown", tagged_error)
    };
    let normalized = error.to_ascii_lowercase();
    let subject = match source {
        "automatic" => format!(
            "{provider} from the app PATH or a local VS Code-compatible editor extension (VS Code, Cursor, or Antigravity IDE)"
        ),
        "configured" => {
            format!("The {provider} executable selected by {override_variable}")
        }
        _ => provider.to_string(),
    };

    if normalized.contains("could not start") {
        return if source == "unknown" {
            fallback.to_string()
        } else {
            format!("{subject} could not start. Check or update it, sign in, then refresh usage.")
        };
    }
    if normalized.contains("timed out") || normalized.contains("stopped before responding") {
        return format!(
            "{subject} started but did not answer the usage request. Restart or update it, then refresh usage."
        );
    }
    if normalized.contains("rejected") || normalized.contains("no result") {
        return format!(
            "{subject} rejected the usage request. Sign in to the same local account used by VS Code, Cursor, or Antigravity IDE, or update it, then refresh usage."
        );
    }
    if normalized.contains("malformed")
        || normalized.contains("exceeded its safety limit")
        || normalized.contains("output limit")
    {
        return format!(
            "{subject} returned an incompatible usage response. Update it, then refresh usage."
        );
    }
    fallback.to_string()
}

fn antigravity_failure_detail(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("timed out") || normalized.contains("stopped") {
        return "Antigravity CLI did not answer the read-only usage request. Try again after opening or signing in to Antigravity.".to_string();
    }
    if normalized.contains("malformed") || normalized.contains("different command") {
        return "Antigravity CLI returned incompatible usage data. Update Antigravity CLI, then refresh usage.".to_string();
    }
    "Install or update Antigravity CLI to 1.1.11 or newer and sign in to view model quota."
        .to_string()
}

fn codex_command_from(value: Option<OsString>) -> ProviderCommand {
    provider_command_from(value, "codex")
}

fn claude_command_from(value: Option<OsString>) -> ProviderCommand {
    provider_command_from(value, "claude")
}

fn provider_command_from(value: Option<OsString>, default: &str) -> ProviderCommand {
    match value.filter(|command| !command.is_empty()) {
        Some(executable) => ProviderCommand {
            executable,
            allow_extension_fallback: false,
        },
        None => ProviderCommand {
            executable: OsString::from(default),
            allow_extension_fallback: true,
        },
    }
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
    if let Some(state_root) = state_root.filter(|root| {
        crate::state::integration_source_is_enabled_at(
            root,
            crate::state::IntegrationSource::ClaudeHooks,
        )
    }) {
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

/// Fail-open entry point for Gemini CLI's documented `AfterModel` hook.
pub fn run_gemini_usage_stdio() -> i32 {
    let state_root = crate::state::state_dir_from_environment().ok();
    run_gemini_usage(
        io::stdin().lock(),
        io::stdout().lock(),
        state_root.as_deref(),
        crate::state::now_ms(),
    )
}

/// Capture only the stable numeric token total and return a valid, silent hook
/// response. Full request/response content is streamed past `IgnoredAny` by
/// Serde and is never persisted or returned.
pub fn run_gemini_usage<R: Read, W: Write>(
    input: R,
    mut output: W,
    state_root: Option<&Path>,
    captured_at_ms: i64,
) -> i32 {
    if let Some(root) = state_root.filter(|root| {
        crate::state::integration_source_is_enabled_at(
            root,
            crate::state::IntegrationSource::GeminiUsage,
        )
    }) {
        let _ = capture_gemini_usage(input, root, captured_at_ms);
    }
    let _ = output.write_all(b"{\"suppressOutput\":true}\n");
    let _ = output.flush();
    0
}

#[derive(Debug, Deserialize)]
struct GeminiAfterModelInput {
    hook_event_name: String,
    llm_response: GeminiLlmResponse,
}

#[derive(Debug, Deserialize)]
struct GeminiLlmResponse {
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "totalTokenCount")]
    total_token_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GeminiUsageRecord {
    schema_version: u32,
    captured_at_ms: i64,
    total_tokens: u64,
}

fn capture_gemini_usage(
    input: impl Read,
    state_root: &Path,
    captured_at_ms: i64,
) -> Result<(), String> {
    if captured_at_ms < 0 {
        return Err("Gemini usage timestamp was invalid".to_string());
    }
    // AfterModel includes the original request, so a legitimate long-context
    // event can be much larger than the tiny field retained below. Serde still
    // streams ignored request/response content without representing it in our
    // input type; the larger cap bounds total work while accommodating current
    // long-context Gemini models.
    let mut deserializer =
        serde_json::Deserializer::from_reader(input.take((GEMINI_USAGE_INPUT_LIMIT + 1) as u64));
    let payload = GeminiAfterModelInput::deserialize(&mut deserializer)
        .map_err(|_| "Gemini usage input was malformed".to_string())?;
    deserializer
        .end()
        .map_err(|_| "Gemini usage input exceeded its safety limit".to_string())?;
    if payload.hook_event_name != "AfterModel" {
        return Err("Gemini usage input was not an AfterModel event".to_string());
    }
    let total_tokens = payload
        .llm_response
        .usage_metadata
        .map(|usage| usage.total_token_count)
        .ok_or_else(|| "Gemini usage input had no final token total".to_string())?;
    let record = GeminiUsageRecord {
        schema_version: LOCAL_USAGE_RECORD_SCHEMA_VERSION,
        captured_at_ms,
        total_tokens,
    };
    write_local_usage_record(&gemini_record_path(state_root), &record, "Gemini")
}

fn gemini_record_path(state_root: &Path) -> PathBuf {
    state_root
        .join(CLAUDE_RECORD_DIRECTORY)
        .join(GEMINI_RECORD_FILENAME)
}

fn load_gemini_usage(state_root: &Path, now_ms: i64) -> ProviderUsageView {
    let detail = "Latest Gemini CLI model-call tokens; this is not subscription quota. Run /stats model in Gemini CLI for its live quota view.";
    let Some(record) =
        read_local_usage_record::<GeminiUsageRecord>(&gemini_record_path(state_root), now_ms)
    else {
        return unavailable_provider(
            "No Gemini token capture yet. Enable the Gemini usage hook in Setup & diagnostics, then start a new turn. Gemini CLI shows live quota through /stats model.",
        );
    };
    if record.schema_version != LOCAL_USAGE_RECORD_SCHEMA_VERSION {
        return unavailable_provider(
            "The Gemini usage capture is incompatible. Repair the Gemini integration in Setup & diagnostics.",
        );
    }
    token_provider(
        record.total_tokens,
        record.captured_at_ms,
        "Latest model call",
        detail,
        now_ms,
    )
}

/// Fail-open entry point for Cursor Agent's documented custom status line.
pub fn run_cursor_usage_stdio() -> i32 {
    let state_root = crate::state::state_dir_from_environment().ok();
    run_cursor_usage(
        io::stdin().lock(),
        io::stdout().lock(),
        state_root.as_deref(),
        crate::state::now_ms(),
    )
}

pub fn run_cursor_usage<R: Read, W: Write>(
    input: R,
    mut output: W,
    state_root: Option<&Path>,
    captured_at_ms: i64,
) -> i32 {
    let remaining = state_root
        .filter(|root| {
            crate::state::integration_source_is_enabled_at(
                root,
                crate::state::IntegrationSource::CursorHooks,
            )
        })
        .and_then(|root| capture_cursor_usage(input, root, captured_at_ms).ok());
    if let Some(remaining) = remaining {
        let _ = write!(output, "{}% context left", remaining.round() as i64);
    }
    let _ = output.flush();
    0
}

#[derive(Debug, Deserialize)]
struct CursorStatusLineInput {
    context_window: CursorContextWindowInput,
}

#[derive(Debug, Deserialize)]
struct CursorContextWindowInput {
    #[serde(default)]
    used_percentage: Option<f64>,
    #[serde(default)]
    remaining_percentage: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorContextUsageRecord {
    schema_version: u32,
    captured_at_ms: i64,
    remaining_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorTurnUsageRecord {
    schema_version: u32,
    captured_at_ms: i64,
    total_tokens: u64,
}

fn capture_cursor_usage(
    input: impl Read,
    state_root: &Path,
    captured_at_ms: i64,
) -> Result<f64, String> {
    if captured_at_ms < 0 {
        return Err("Cursor usage timestamp was invalid".to_string());
    }
    let mut deserializer =
        serde_json::Deserializer::from_reader(input.take((CURSOR_USAGE_INPUT_LIMIT + 1) as u64));
    let payload = CursorStatusLineInput::deserialize(&mut deserializer)
        .map_err(|_| "Cursor usage input was malformed".to_string())?;
    deserializer
        .end()
        .map_err(|_| "Cursor usage input exceeded its safety limit".to_string())?;
    let remaining = payload
        .context_window
        .remaining_percentage
        .filter(|value| value.is_finite())
        .or_else(|| {
            payload
                .context_window
                .used_percentage
                .filter(|value| value.is_finite())
                .map(|used| 100.0 - used)
        })
        .ok_or_else(|| "Cursor usage input had no context percentage".to_string())?
        .clamp(0.0, 100.0);
    let record = CursorContextUsageRecord {
        schema_version: LOCAL_USAGE_RECORD_SCHEMA_VERSION,
        captured_at_ms,
        remaining_percent: remaining,
    };
    write_local_usage_record(&cursor_record_path(state_root), &record, "Cursor")?;
    Ok(remaining)
}

/// Persist the latest Cursor agent turn total without retaining the response,
/// conversation identifiers, model, workspace, or cache-token breakdowns.
///
/// Cursor reports cache reads and writes as subsets of its input total, so the
/// local metric is deliberately only `input_tokens + output_tokens`.
pub(crate) fn capture_cursor_turn_usage(
    state_root: &Path,
    captured_at_ms: i64,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
) -> Result<(), String> {
    if captured_at_ms < 0 {
        return Err("Cursor turn usage timestamp was invalid".to_string());
    }
    let total_tokens = match (input_tokens, output_tokens) {
        (None, None) => {
            return Err("Cursor turn usage input had no token counts".to_string());
        }
        (input, output) => input
            .unwrap_or(0)
            .checked_add(output.unwrap_or(0))
            .ok_or_else(|| "Cursor turn usage token total overflowed".to_string())?,
    };
    if total_tokens == 0 {
        return Err("Cursor turn usage token total was empty".to_string());
    }

    let record = CursorTurnUsageRecord {
        schema_version: LOCAL_USAGE_RECORD_SCHEMA_VERSION,
        captured_at_ms,
        total_tokens,
    };
    write_local_usage_record(&cursor_turn_record_path(state_root), &record, "Cursor")
}

fn cursor_record_timestamp_is_compatible(captured_at_ms: i64, now_ms: i64) -> bool {
    captured_at_ms >= 0 && captured_at_ms <= now_ms.saturating_add(MAX_FUTURE_SKEW_MS)
}

fn cursor_record_timestamp_is_valid(captured_at_ms: i64, now_ms: i64) -> bool {
    cursor_record_timestamp_is_compatible(captured_at_ms, now_ms)
        && now_ms.saturating_sub(captured_at_ms) <= LOCAL_USAGE_EXPIRES_AFTER_MS
}

fn cursor_context_record_is_compatible(record: &CursorContextUsageRecord, now_ms: i64) -> bool {
    record.schema_version == LOCAL_USAGE_RECORD_SCHEMA_VERSION
        && record.remaining_percent.is_finite()
        && (0.0..=100.0).contains(&record.remaining_percent)
        && cursor_record_timestamp_is_compatible(record.captured_at_ms, now_ms)
}

fn cursor_context_record_is_valid(record: &CursorContextUsageRecord, now_ms: i64) -> bool {
    cursor_context_record_is_compatible(record, now_ms)
        && cursor_record_timestamp_is_valid(record.captured_at_ms, now_ms)
}

fn cursor_context_record_is_fresh(record: &CursorContextUsageRecord, now_ms: i64) -> bool {
    cursor_context_record_is_valid(record, now_ms)
        && now_ms.saturating_sub(record.captured_at_ms) <= LOCAL_USAGE_STALE_AFTER_MS
}

fn cursor_turn_record_is_compatible(record: &CursorTurnUsageRecord, now_ms: i64) -> bool {
    record.schema_version == LOCAL_USAGE_RECORD_SCHEMA_VERSION
        && record.total_tokens > 0
        && cursor_record_timestamp_is_compatible(record.captured_at_ms, now_ms)
}

fn cursor_turn_record_is_valid(record: &CursorTurnUsageRecord, now_ms: i64) -> bool {
    cursor_turn_record_is_compatible(record, now_ms)
        && cursor_record_timestamp_is_valid(record.captured_at_ms, now_ms)
}

fn cursor_record_path(state_root: &Path) -> PathBuf {
    state_root
        .join(CLAUDE_RECORD_DIRECTORY)
        .join(CURSOR_RECORD_FILENAME)
}

fn cursor_turn_record_path(state_root: &Path) -> PathBuf {
    state_root
        .join(CLAUDE_RECORD_DIRECTORY)
        .join(CURSOR_TURN_RECORD_FILENAME)
}

enum CursorUsageObservation {
    Context(CursorContextUsageRecord),
    Turn(CursorTurnUsageRecord),
}

fn load_cursor_usage(state_root: &Path, now_ms: i64) -> ProviderUsageView {
    let context_path = cursor_record_path(state_root);
    let turn_path = cursor_turn_record_path(state_root);
    let parsed_context = read_local_usage_record::<CursorContextUsageRecord>(&context_path, now_ms);
    let parsed_turn = read_local_usage_record::<CursorTurnUsageRecord>(&turn_path, now_ms);
    let incompatible_context = fs::symlink_metadata(&context_path).is_ok()
        && parsed_context
            .as_ref()
            .is_none_or(|record| !cursor_context_record_is_compatible(record, now_ms));
    let incompatible_turn = fs::symlink_metadata(&turn_path).is_ok()
        && parsed_turn
            .as_ref()
            .is_none_or(|record| !cursor_turn_record_is_compatible(record, now_ms));
    let context = parsed_context.filter(|record| cursor_context_record_is_valid(record, now_ms));
    let turn = parsed_turn.filter(|record| cursor_turn_record_is_valid(record, now_ms));

    // A recent CLI context observation is more actionable than a token total,
    // even when the Stop hook ran slightly later. Once context is stale, use
    // the newest valid observation; ties deterministically favor context.
    let observation = match (context, turn) {
        (Some(context), _) if cursor_context_record_is_fresh(&context, now_ms) => {
            Some(CursorUsageObservation::Context(context))
        }
        (Some(context), Some(turn)) if context.captured_at_ms >= turn.captured_at_ms => {
            Some(CursorUsageObservation::Context(context))
        }
        (Some(_), Some(turn)) | (None, Some(turn)) => Some(CursorUsageObservation::Turn(turn)),
        (Some(context), None) => Some(CursorUsageObservation::Context(context)),
        (None, None) => None,
    };

    let Some(observation) = observation else {
        if incompatible_context || incompatible_turn {
            return unavailable_provider(
                "The Cursor usage capture is incompatible. Repair Cursor monitoring in Setup & diagnostics.",
            );
        }
        return unavailable_provider(
            "No Cursor usage capture is available. Review Cursor monitoring status, then start a Cursor turn. Cursor plan quota remains available only inside Cursor.",
        );
    };

    match observation {
        CursorUsageObservation::Context(record) => context_provider(
            record.remaining_percent,
            record.captured_at_ms,
            "Latest CLI context",
            "Cursor Agent CLI context capacity; this is not Composer plan quota.",
            now_ms,
        ),
        CursorUsageObservation::Turn(record) => token_provider(
            record.total_tokens,
            record.captured_at_ms,
            "Latest Cursor turn",
            "Latest local Cursor agent-turn input and output tokens; cache-token fields are breakdowns and are not added. This is not plan or billing quota.",
            now_ms,
        ),
    }
}

fn write_local_usage_record<T: Serialize>(
    path: &Path,
    record: &T,
    provider: &str,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(record)
        .map_err(|_| format!("could not serialize the {provider} usage record"))?;
    atomic_write_bytes(path, &bytes)
}

fn read_local_usage_record<T: for<'de> Deserialize<'de>>(path: &Path, now_ms: i64) -> Option<T> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if is_link_or_reparse_point(&metadata)
        || !metadata.is_file()
        || metadata.len() > LOCAL_USAGE_RECORD_LIMIT
    {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.len() as u64 > LOCAL_USAGE_RECORD_LIMIT {
        return None;
    }
    let value: T = serde_json::from_slice(&bytes).ok()?;
    // Timestamp/schema validation remains in the concrete view constructor;
    // this generic helper only enforces the bounded regular-file boundary.
    let _ = now_ms;
    Some(value)
}

fn atomic_write_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "usage record had no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|_| "could not create the usage directory".to_string())?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|_| "could not inspect the usage directory".to_string())?;
    if is_link_or_reparse_point(&parent_metadata) || !parent_metadata.is_dir() {
        return Err("usage directory was not a regular directory".to_string());
    }
    set_private_directory_permissions(parent);
    reject_unsafe_existing_target(path)?;

    let mut temporary = TempFileBuilder::new()
        .prefix(".vsparallel-usage.")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|_| "could not create a temporary usage record".to_string())?;
    set_private_file_permissions(temporary.path());
    temporary
        .write_all(content)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|_| "could not write the temporary usage record".to_string())?;
    replace_temporary_file(temporary, path)?;
    set_private_file_permissions(path);
    sync_parent(parent);
    Ok(())
}

fn reject_unsafe_existing_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_link_or_reparse_point(&metadata) || !metadata.is_file() => {
            Err("usage record target was not a regular file".to_string())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("could not inspect the usage record".to_string()),
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
        .map_err(|_| "could not atomically replace the usage record".to_string())?;
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
        return Err("could not atomically replace the usage record".to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn nul_terminated_wide_path(path: &Path) -> Result<Vec<u16>, String> {
    let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    if encoded.contains(&0) {
        return Err("usage record path contained an embedded NUL".to_string());
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

fn finish_provider_process(child: &mut Child) -> io::Result<()> {
    let deadline = Instant::now() + PROVIDER_GRACEFUL_EXIT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(PROVIDER_EXIT_POLL_INTERVAL);
            }
            Ok(None) | Err(_) => return terminate_and_reap(child),
        }
    }
}

fn join_provider_reader<T>(reader: thread::JoinHandle<T>) {
    let deadline = Instant::now() + PROVIDER_READER_JOIN_TIMEOUT;
    while !reader.is_finished() && Instant::now() < deadline {
        thread::sleep(PROVIDER_EXIT_POLL_INTERVAL);
    }
    if reader.is_finished() {
        let _ = reader.join();
    }
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
    use crate::companion_integration::CliOutput;
    use std::io::Cursor;
    use tempfile::TempDir;

    #[derive(Clone)]
    struct FakeRunner(Result<Value, String>);

    impl CodexUsageRunner for FakeRunner {
        fn read_rate_limits(&self, _executable: &OsStr) -> Result<Value, String> {
            self.0.clone()
        }
    }

    #[derive(Clone)]
    struct FakeClaudeRunner(Result<Value, String>);

    impl ClaudeUsageRunner for FakeClaudeRunner {
        fn read_rate_limits(&self, _executable: &OsStr) -> Result<Value, String> {
            self.0.clone()
        }
    }

    struct FakeExtensionCliRunner {
        antigravity_root: PathBuf,
        calls: Mutex<Vec<(OsString, Vec<OsString>)>>,
    }

    impl CodeCliRunner for FakeExtensionCliRunner {
        fn run(&self, executable: &OsStr, arguments: &[OsString]) -> io::Result<CliOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((executable.to_owned(), arguments.to_vec()));
            if executable == OsStr::new("antigravity-ide") {
                Ok(CliOutput {
                    success: true,
                    exit_code: Some(0),
                    stdout: format!("{}\n", self.antigravity_root.display()).into_bytes(),
                    stderr: Vec::new(),
                })
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "injected missing editor launcher",
                ))
            }
        }
    }

    fn claude_input(root: &Path, input: &[u8], now_ms: i64) -> (i32, Vec<u8>) {
        let mut output = Vec::new();
        let code = run_claude_statusline(input, &mut output, Some(root), now_ms);
        (code, output)
    }

    fn gemini_input(root: &Path, input: &[u8], now_ms: i64) -> (i32, Vec<u8>) {
        let mut output = Vec::new();
        let code = run_gemini_usage(input, &mut output, Some(root), now_ms);
        (code, output)
    }

    fn cursor_input(root: &Path, input: &[u8], now_ms: i64) -> (i32, Vec<u8>) {
        let mut output = Vec::new();
        let code = run_cursor_usage(input, &mut output, Some(root), now_ms);
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
    fn claude_live_limits_report_percentages_and_rfc3339_resets() {
        let result = json!({
            "subscription_type": "pro",
            "rate_limits_available": true,
            "rate_limits": {
                "five_hour": {
                    "utilization": 23.5,
                    "resets_at": "1970-01-01T00:00:02.345Z"
                },
                "seven_day": {
                    "utilization": 75,
                    "resets_at": 1_800_100_000
                },
                "seven_day_sonnet": {
                    "utilization": 99
                }
            }
        });

        let view = claude_provider_view(&result, 1_000).unwrap();

        assert_eq!(view.state, "available");
        assert_eq!(view.remaining_percent, Some(25.0));
        assert_eq!(view.windows.len(), 2);
        assert_eq!(view.windows[0].label, "5-hour limit");
        assert_eq!(view.windows[0].remaining_percent, 76.5);
        assert_eq!(view.windows[0].resets_at_ms, Some(2_345));
        assert_eq!(view.windows[1].label, "7-day limit");
        assert_eq!(view.windows[1].resets_at_ms, Some(1_800_100_000_000));
        assert_eq!(
            claude_reset_to_millis(&json!("2026-08-05T15:19:59.920164+00:00")),
            Some(1_785_943_199_920)
        );
    }

    #[test]
    fn claude_live_limits_omit_expired_windows() {
        let result = json!({
            "rate_limits": {
                "five_hour": {"utilization": 90, "resets_at": "1970-01-01T00:00:02Z"},
                "seven_day": {"utilization": 40, "resets_at": "2030-01-01T00:00:00Z"}
            }
        });

        let view = claude_provider_view(&result, 3_000).unwrap();

        assert_eq!(view.windows.len(), 1);
        assert_eq!(view.windows[0].label, "7-day limit");
        assert_eq!(view.remaining_percent, Some(60.0));
    }

    #[test]
    fn antigravity_output_is_narrowed_to_quota_buckets() {
        let raw = json!({
            "status":"SUCCESS",
            "account":{"email":"private@example.invalid","token":"private-token"},
            "command":{
                "name":"usage",
                "data":{"groups":[{
                    "privateGroup":"secret-group",
                    "buckets":[
                        {
                            "id":"gemini-pool",
                            "name":"Gemini models",
                            "window":"weekly",
                            "remaining_fraction":0.42,
                            "reset_time":"2030-01-02T03:04:05.678Z",
                            "privateField":"secret-bucket"
                        },
                        {"id":"missing-fraction","name":"Ignored"}
                    ]
                }]}
            }
        });

        let narrowed = parse_antigravity_usage_output(&serde_json::to_vec(&raw).unwrap()).unwrap();
        let serialized = narrowed.to_string();
        for secret in [
            "private@example.invalid",
            "private-token",
            "secret-group",
            "secret-bucket",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert_eq!(narrowed["buckets"].as_array().unwrap().len(), 1);
        let view = antigravity_provider_view(&narrowed, 1_000).unwrap();
        assert_eq!(view.metric_kind, "quota");
        assert_eq!(view.remaining_percent, Some(42.0));
        assert_eq!(view.windows[0].label, "Gemini models · weekly");
        assert_eq!(view.windows[0].resets_at_ms, Some(1_893_553_445_678));

        for invalid in [
            json!({"status":"FAILED","command":{"name":"usage","data":{"groups":[]}}}),
            json!({"status":"SUCCESS","command":{"name":"chat","data":{"groups":[]}}}),
        ] {
            assert!(
                parse_antigravity_usage_output(&serde_json::to_vec(&invalid).unwrap()).is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_reader_timeout_bounds_descendant_held_pipes() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("agy-with-pipe-descendant");
        fs::write(
            &executable,
            "#!/bin/sh\nsleep 30 &\nprintf '%s\\n' '{\"status\":\"SUCCESS\",\"command\":{\"name\":\"usage\",\"data\":{\"groups\":[]}}}'\nexit 0\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).unwrap();

        let started = Instant::now();
        let error = antigravity_usage_request_with_timeout(
            executable.as_os_str(),
            Duration::from_millis(150),
        )
        .unwrap_err();

        assert_eq!(error, "Antigravity CLI usage request timed out");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn gemini_capture_persists_only_latest_model_call_token_total() {
        let temp = TempDir::new().unwrap();
        let input = json!({
            "hook_event_name":"AfterModel",
            "session_id":"private-session",
            "llm_request":{"contents":[{"text":"SECRET PROMPT"}]},
            "llm_response":{
                "candidates":[{"content":{"parts":[{"text":"SECRET RESPONSE"}]}}],
                "usageMetadata":{"totalTokenCount":12_345,"private":"secret-metadata"}
            }
        });

        let (code, output) =
            gemini_input(temp.path(), &serde_json::to_vec(&input).unwrap(), 10_000);
        assert_eq!(code, 0);
        assert_eq!(output, b"{\"suppressOutput\":true}\n");
        let record = fs::read(gemini_record_path(temp.path())).unwrap();
        let persisted = String::from_utf8(record.clone()).unwrap();
        for secret in [
            "private-session",
            "SECRET PROMPT",
            "SECRET RESPONSE",
            "secret-metadata",
        ] {
            assert!(!persisted.contains(secret));
        }
        let value: Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(value["totalTokens"], 12_345);

        let fresh = load_gemini_usage(temp.path(), 11_000);
        assert_eq!(fresh.state, "available");
        assert_eq!(fresh.metric_kind, "tokens");
        assert_eq!(fresh.token_count, Some(12_345));
        assert_eq!(fresh.metric_label, "Latest model call");
        let stale = load_gemini_usage(temp.path(), 10_000 + LOCAL_USAGE_STALE_AFTER_MS + 1);
        assert_eq!(stale.state, "stale");
        let expired = load_gemini_usage(temp.path(), 10_000 + LOCAL_USAGE_EXPIRES_AFTER_MS + 1);
        assert_eq!(expired.state, "unavailable");
        assert_eq!(expired.token_count, None);
    }

    #[test]
    fn gemini_capture_accepts_valid_long_context_events() {
        let temp = TempDir::new().unwrap();
        let input = json!({
            "hook_event_name":"AfterModel",
            "llm_request":{"messages":[{"content":"x".repeat(3 * 1024 * 1024)}]},
            "llm_response":{"usageMetadata":{"totalTokenCount":987_654}}
        });
        let bytes = serde_json::to_vec(&input).unwrap();
        assert!(bytes.len() > 2 * 1024 * 1024);

        let (code, output) = gemini_input(temp.path(), &bytes, 12_000);
        assert_eq!(code, 0);
        assert_eq!(output, b"{\"suppressOutput\":true}\n");
        assert_eq!(
            load_gemini_usage(temp.path(), 12_001).token_count,
            Some(987_654)
        );
    }

    fn gemini_lifecycle_status(state: &str) -> crate::gemini_integration::GeminiIntegrationStatus {
        crate::gemini_integration::GeminiIntegrationStatus {
            state: state.to_string(),
            installed: state == "installed" || state == "disabled",
            config_path: "/tmp/.gemini/settings.json".to_string(),
            backup_path: "/tmp/.gemini/settings.json.vsparallel.bak".to_string(),
            event_states: std::collections::BTreeMap::from([(
                "AfterModel".to_string(),
                if state == "installed" || state == "disabled" {
                    "current"
                } else {
                    state
                }
                .to_string(),
            )]),
            hooks_disabled: state == "disabled",
            message: format!("Gemini lifecycle state: {state}"),
        }
    }

    #[test]
    fn gemini_usage_explains_when_the_capture_hook_is_not_installed() {
        let usage = unavailable_provider("No capture yet.");
        let status = gemini_lifecycle_status("not_installed");

        let reconciled = reconcile_gemini_integration_status(usage, Some(&status));

        assert_eq!(reconciled.state, "unavailable");
        assert!(reconciled.detail.contains("not installed"));
        assert!(reconciled.detail.contains("Setup & diagnostics"));
    }

    #[test]
    fn gemini_usage_keeps_the_last_count_but_marks_it_stale_when_repair_is_needed() {
        let usage = token_provider(42, 10_000, "Latest model call", "captured", 10_001);
        let status = gemini_lifecycle_status("stale");

        let reconciled = reconcile_gemini_integration_status(usage, Some(&status));

        assert_eq!(reconciled.state, "stale");
        assert_eq!(reconciled.token_count, Some(42));
        assert!(reconciled.detail.contains("needs repair"));
    }

    #[test]
    fn cursor_capture_persists_only_context_capacity() {
        let temp = TempDir::new().unwrap();
        let input = json!({
            "session_id":"private-session",
            "workspace":{"current_dir":"/private/project"},
            "model":{"display_name":"private-model"},
            "context_window":{
                "used_percentage":37.5,
                "privateTokens":["SECRET PROMPT","SECRET RESPONSE"]
            }
        });

        let (code, output) =
            cursor_input(temp.path(), &serde_json::to_vec(&input).unwrap(), 20_000);
        assert_eq!(code, 0);
        assert_eq!(output, b"63% context left");
        let record = fs::read(cursor_record_path(temp.path())).unwrap();
        let persisted = String::from_utf8(record.clone()).unwrap();
        for secret in [
            "private-session",
            "/private/project",
            "private-model",
            "SECRET PROMPT",
            "SECRET RESPONSE",
        ] {
            assert!(!persisted.contains(secret));
        }
        let value: Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(value["remainingPercent"], 62.5);

        let view = load_cursor_usage(temp.path(), 21_000);
        assert_eq!(view.state, "available");
        assert_eq!(view.metric_kind, "context");
        assert_eq!(view.remaining_percent, Some(62.5));
        assert_eq!(view.metric_label, "Latest CLI context");
        assert!(view.detail.contains("not Composer plan quota"));
    }

    #[test]
    fn cursor_turn_capture_persists_only_input_and_output_total() {
        let temp = TempDir::new().unwrap();
        capture_cursor_turn_usage(temp.path(), 20_000, Some(191_000), Some(2_345)).unwrap();

        let record = fs::read(cursor_turn_record_path(temp.path())).unwrap();
        let value: Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(value["schemaVersion"], LOCAL_USAGE_RECORD_SCHEMA_VERSION);
        assert_eq!(value["capturedAtMs"], 20_000);
        assert_eq!(value["totalTokens"], 193_345);
        assert!(value.get("remainingPercent").is_none());

        let view = load_cursor_usage(temp.path(), 21_000);
        assert_eq!(view.state, "available");
        assert_eq!(view.metric_kind, "tokens");
        assert_eq!(view.token_count, Some(193_345));
        assert_eq!(view.metric_label, "Latest Cursor turn");
        assert!(view.detail.contains("Cursor agent-turn"));
        assert!(view.detail.contains("not added"));
    }

    #[test]
    fn cursor_selection_prefers_fresh_context_then_newest_record() {
        let temp = TempDir::new().unwrap();
        capture_cursor_turn_usage(temp.path(), 10_000, Some(100), Some(25)).unwrap();
        assert_eq!(
            load_cursor_usage(temp.path(), 10_001).token_count,
            Some(125)
        );

        let (code, output) = cursor_input(
            temp.path(),
            br#"{"context_window":{"remaining_percentage":72.5}}"#,
            20_000,
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"73% context left");
        let context = load_cursor_usage(temp.path(), 20_001);
        assert_eq!(context.metric_kind, "context");
        assert_eq!(context.remaining_percent, Some(72.5));

        capture_cursor_turn_usage(
            temp.path(),
            20_000 + LOCAL_USAGE_STALE_AFTER_MS,
            Some(200),
            Some(50),
        )
        .unwrap();
        let preserved = load_cursor_usage(temp.path(), 20_000 + LOCAL_USAGE_STALE_AFTER_MS);
        assert_eq!(preserved.metric_kind, "context");
        assert_eq!(preserved.remaining_percent, Some(72.5));

        capture_cursor_turn_usage(
            temp.path(),
            20_000 + LOCAL_USAGE_STALE_AFTER_MS + 1,
            Some(200),
            Some(50),
        )
        .unwrap();
        let tokens = load_cursor_usage(temp.path(), 20_000 + LOCAL_USAGE_STALE_AFTER_MS + 1);
        assert_eq!(tokens.metric_kind, "tokens");
        assert_eq!(tokens.token_count, Some(250));
    }

    #[test]
    fn cursor_turn_capture_rejects_empty_or_overflowing_totals_without_clobbering() {
        let temp = TempDir::new().unwrap();
        cursor_input(
            temp.path(),
            br#"{"context_window":{"remaining_percentage":80}}"#,
            1,
        );
        let before = fs::read(cursor_record_path(temp.path())).unwrap();

        assert!(capture_cursor_turn_usage(temp.path(), 1, None, None).is_err());
        assert!(capture_cursor_turn_usage(temp.path(), 1, Some(0), Some(0)).is_err());
        assert!(capture_cursor_turn_usage(temp.path(), 1, Some(u64::MAX), Some(1)).is_err());
        assert_eq!(fs::read(cursor_record_path(temp.path())).unwrap(), before);
        assert!(!cursor_turn_record_path(temp.path()).exists());
    }

    #[test]
    fn cursor_turn_capture_accepts_either_token_counter_on_its_own() {
        let input_only = TempDir::new().unwrap();
        capture_cursor_turn_usage(input_only.path(), 10_000, Some(125), None).unwrap();
        assert_eq!(
            load_cursor_usage(input_only.path(), 10_001).token_count,
            Some(125)
        );

        let output_only = TempDir::new().unwrap();
        capture_cursor_turn_usage(output_only.path(), 10_000, None, Some(25)).unwrap();
        assert_eq!(
            load_cursor_usage(output_only.path(), 10_001).token_count,
            Some(25)
        );
    }

    #[test]
    fn cursor_legacy_context_record_remains_compatible_and_wins_ties() {
        let temp = TempDir::new().unwrap();
        let usage_dir = temp.path().join(CLAUDE_RECORD_DIRECTORY);
        fs::create_dir_all(&usage_dir).unwrap();
        fs::write(
            cursor_record_path(temp.path()),
            br#"{"schemaVersion":1,"capturedAtMs":10000,"remainingPercent":64.5}"#,
        )
        .unwrap();
        capture_cursor_turn_usage(temp.path(), 10_000, Some(100), Some(25)).unwrap();

        let view = load_cursor_usage(temp.path(), 10_000 + LOCAL_USAGE_STALE_AFTER_MS + 1);
        assert_eq!(view.state, "stale");
        assert_eq!(view.metric_kind, "context");
        assert_eq!(view.remaining_percent, Some(64.5));
        assert_eq!(view.updated_at_ms, Some(10_000));
    }

    #[test]
    fn cursor_invalid_record_does_not_hide_valid_other_source() {
        let invalid_context = TempDir::new().unwrap();
        let usage_dir = invalid_context.path().join(CLAUDE_RECORD_DIRECTORY);
        fs::create_dir_all(&usage_dir).unwrap();
        fs::write(
            cursor_record_path(invalid_context.path()),
            br#"{"schemaVersion":999,"capturedAtMs":20000,"remainingPercent":50}"#,
        )
        .unwrap();
        capture_cursor_turn_usage(invalid_context.path(), 19_000, Some(100), Some(25)).unwrap();
        let tokens = load_cursor_usage(invalid_context.path(), 20_000);
        assert_eq!(tokens.metric_kind, "tokens");
        assert_eq!(tokens.token_count, Some(125));

        let invalid_turn = TempDir::new().unwrap();
        cursor_input(
            invalid_turn.path(),
            br#"{"context_window":{"remaining_percentage":70}}"#,
            10_000,
        );
        fs::write(
            cursor_turn_record_path(invalid_turn.path()),
            br#"{"schemaVersion":1,"capturedAtMs":20000,"totalTokens":0}"#,
        )
        .unwrap();
        let context =
            load_cursor_usage(invalid_turn.path(), 10_000 + LOCAL_USAGE_STALE_AFTER_MS + 1);
        assert_eq!(context.metric_kind, "context");
        assert_eq!(context.remaining_percent, Some(70.0));
    }

    #[test]
    fn cursor_expired_records_request_a_new_turn_instead_of_repair() {
        let temp = TempDir::new().unwrap();
        cursor_input(
            temp.path(),
            br#"{"context_window":{"remaining_percentage":70}}"#,
            10_000,
        );
        capture_cursor_turn_usage(temp.path(), 20_000, Some(100), Some(25)).unwrap();

        let view = load_cursor_usage(temp.path(), 20_000 + LOCAL_USAGE_EXPIRES_AFTER_MS + 1);
        assert_eq!(view.state, "unavailable");
        assert!(view.detail.contains("No Cursor usage capture"));
        assert!(!view.detail.contains("incompatible"));
        assert!(!view.detail.contains("Repair"));
    }

    #[test]
    fn disabled_local_usage_hooks_fail_open_without_persisting() {
        let temp = TempDir::new().unwrap();
        crate::state::set_integration_source_enabled_at(
            temp.path(),
            crate::state::IntegrationSource::GeminiUsage,
            false,
        )
        .unwrap();
        crate::state::set_integration_source_enabled_at(
            temp.path(),
            crate::state::IntegrationSource::CursorHooks,
            false,
        )
        .unwrap();

        let (gemini_code, gemini_output) = gemini_input(
            temp.path(),
            br#"{"hook_event_name":"AfterModel","llm_response":{"usageMetadata":{"totalTokenCount":5}}}"#,
            1_000,
        );
        let (cursor_code, cursor_output) = cursor_input(
            temp.path(),
            br#"{"context_window":{"remaining_percentage":50}}"#,
            1_000,
        );
        assert_eq!(gemini_code, 0);
        assert_eq!(gemini_output, b"{\"suppressOutput\":true}\n");
        assert_eq!(cursor_code, 0);
        assert!(cursor_output.is_empty());
        assert!(!gemini_record_path(temp.path()).exists());
        assert!(!cursor_record_path(temp.path()).exists());
        assert!(!cursor_turn_record_path(temp.path()).exists());
    }

    #[test]
    fn snapshot_prefers_live_claude_and_falls_back_to_statusline_cache() {
        let temp = TempDir::new().unwrap();
        let input = br#"{
            "rate_limits": {
                "five_hour": {"used_percentage": 40, "resets_at": 1800000000}
            }
        }"#;
        claude_input(temp.path(), input, 1_000);
        let live = json!({
            "rate_limits": {
                "five_hour": {"utilization": 20, "resets_at": "2030-01-01T00:00:00Z"}
            }
        });

        let fresh = build_usage_snapshot_with(
            &FakeRunner(Err("signed out".to_string())),
            OsStr::new("fake-codex"),
            &FakeClaudeRunner(Ok(live)),
            OsStr::new("fake-claude"),
            Some(temp.path()),
            2_000,
        );
        assert_eq!(fresh.claude.remaining_percent, Some(80.0));
        assert_eq!(fresh.claude.detail, "Live usage limits from Claude Code.");

        let fallback = build_usage_snapshot_with(
            &FakeRunner(Err("signed out".to_string())),
            OsStr::new("fake-codex"),
            &FakeClaudeRunner(Err("unsupported".to_string())),
            OsStr::new("fake-claude"),
            Some(temp.path()),
            2_000,
        );
        assert_eq!(fallback.claude.remaining_percent, Some(60.0));
        assert_eq!(
            fallback.claude.detail,
            "Usage limits captured by Claude Code."
        );

        crate::state::set_integration_source_enabled_at(
            temp.path(),
            crate::state::IntegrationSource::ClaudeHooks,
            false,
        )
        .unwrap();
        let disabled = build_usage_snapshot_with(
            &FakeRunner(Err("signed out".to_string())),
            OsStr::new("fake-codex"),
            &FakeClaudeRunner(Err("unsupported".to_string())),
            OsStr::new("fake-claude"),
            Some(temp.path()),
            2_000,
        );
        assert_eq!(disabled.claude.remaining_percent, None);
        assert_eq!(disabled.claude.state, "unavailable");
    }

    #[test]
    fn snapshot_runner_failures_are_unavailable_not_errors() {
        let temp = TempDir::new().unwrap();
        let snapshot = build_usage_snapshot_with(
            &FakeRunner(Err("signed out".to_string())),
            OsStr::new("fake-codex"),
            &FakeClaudeRunner(Err("signed out".to_string())),
            OsStr::new("fake-claude"),
            Some(temp.path()),
            42_000,
        );

        assert_eq!(snapshot.generated_at_ms, 42_000);
        assert_eq!(snapshot.codex.state, "unavailable");
        assert_eq!(snapshot.codex.remaining_percent, None);
        assert_eq!(snapshot.claude.state, "unavailable");
        assert_eq!(
            snapshot.codex.detail,
            "Install or sign in to Codex to view usage limits."
        );
        assert!(!snapshot.codex.detail.contains("signed out"));
    }

    #[test]
    fn provider_failure_details_report_safe_source_and_category() {
        let automatic =
            tag_provider_failure("could not start the Codex app-server".to_string(), true);
        let detail =
            provider_failure_detail("Codex", "VSPARALLEL_CODEX_COMMAND", &automatic, "fallback");
        assert!(detail.contains("app PATH or a local VS Code-compatible editor extension"));
        assert!(detail.contains("VS Code, Cursor, or Antigravity IDE"));

        let configured = tag_provider_failure("could not start Claude Code".to_string(), false);
        let detail = provider_failure_detail(
            "Claude Code",
            "VSPARALLEL_CLAUDE_COMMAND",
            &configured,
            "fallback",
        );
        assert!(detail.contains("selected by VSPARALLEL_CLAUDE_COMMAND"));

        for (error, expected) in [
            ("Claude Code usage request timed out", "did not answer"),
            (
                "Codex app-server rejected the request",
                "same local account",
            ),
            ("Codex app-server returned malformed output", "incompatible"),
        ] {
            let detail = provider_failure_detail("Provider", "OVERRIDE", error, "fallback");
            assert!(detail.contains(expected));
            if error.contains("rejected") {
                assert!(detail.contains("VS Code, Cursor, or Antigravity IDE"));
            }
        }

        let detail = provider_failure_detail(
            "Provider",
            "OVERRIDE",
            "private provider diagnostic",
            "safe fallback",
        );
        assert_eq!(detail, "safe fallback");
    }

    #[test]
    fn command_override_is_literal_and_empty_values_fall_back() {
        let explicit_codex = codex_command_from(Some(OsString::from("codex")));
        assert_eq!(explicit_codex.executable, "codex");
        assert!(!explicit_codex.allow_extension_fallback);
        let explicit_claude =
            claude_command_from(Some(OsString::from("/opt/Claude Preview/claude")));
        assert_eq!(
            explicit_claude.executable,
            OsString::from("/opt/Claude Preview/claude")
        );
        assert!(!explicit_claude.allow_extension_fallback);

        for default in [
            codex_command_from(Some(OsString::new())),
            codex_command_from(None),
        ] {
            assert_eq!(default.executable, "codex");
            assert!(default.allow_extension_fallback);
        }
        for default in [
            claude_command_from(Some(OsString::new())),
            claude_command_from(None),
        ] {
            assert_eq!(default.executable, "claude");
            assert!(default.allow_extension_fallback);
        }
        assert_eq!(
            claude_secure_storage_config_from(
                Some(OsString::from("/secure")),
                Some(OsString::from("/config")),
            ),
            OsString::from("/secure")
        );
        assert_eq!(
            claude_secure_storage_config_from(None, Some(OsString::from("/config"))),
            OsString::from("/config")
        );
        assert_eq!(
            claude_secure_storage_config_from(None, None),
            OsString::new()
        );
    }

    #[test]
    fn default_codex_command_retries_bundled_extension_but_nondefault_is_literal() {
        let bundled = PathBuf::from("/test/extensions/openai.chatgpt/bin/codex");
        let mut requests = Vec::new();
        let mut lookups = Vec::new();
        let result = codex_request_with_extension_fallback(
            OsStr::new("codex"),
            |executable| {
                requests.push(executable.to_owned());
                if executable == bundled.as_os_str() {
                    Ok(json!({"rateLimits": {}}))
                } else {
                    Err("not on the desktop PATH".to_string())
                }
            },
            |allow_lookup| {
                lookups.push(allow_lookup);
                allow_lookup.then(|| bundled.clone())
            },
        )
        .unwrap();

        assert!(result["rateLimits"].is_object());
        assert_eq!(
            requests,
            [OsString::from("codex"), bundled.into_os_string()]
        );
        assert_eq!(lookups, [false, true]);

        let bundled_failure = PathBuf::from("/test/extensions/openai.chatgpt/bin/broken-codex");
        let error = codex_request_with_extension_fallback(
            OsStr::new("codex"),
            |executable| {
                if executable == OsStr::new("codex") {
                    Err("not on the desktop PATH".to_string())
                } else {
                    Err("bundled Codex could not authenticate".to_string())
                }
            },
            |allow_lookup| allow_lookup.then(|| bundled_failure.clone()),
        )
        .unwrap_err();
        assert_eq!(error, "bundled Codex could not authenticate");

        let override_path = OsStr::new("/opt/Codex Preview/codex");
        let mut override_requests = Vec::new();
        let override_result = codex_request_with_extension_fallback(
            override_path,
            |executable| {
                override_requests.push(executable.to_owned());
                Err("configured command failed".to_string())
            },
            |_| panic!("a non-default Codex command must not trigger extension discovery"),
        );
        assert!(override_result.is_err());
        assert_eq!(override_requests, [override_path.to_owned()]);
    }

    #[test]
    fn default_claude_command_retries_bundled_extension_and_preserves_its_error() {
        let bundled = PathBuf::from("/test/extensions/anthropic.claude-code/claude");
        let mut requests = Vec::new();
        let result = claude_request_with_extension_fallback(
            OsStr::new("claude"),
            |executable| {
                requests.push(executable.to_owned());
                if executable == bundled.as_os_str() {
                    Ok(json!({"limits": {}}))
                } else {
                    Err("not on the desktop PATH".to_string())
                }
            },
            |allow_lookup| allow_lookup.then(|| bundled.clone()),
        )
        .unwrap();
        assert!(result["limits"].is_object());
        assert_eq!(
            requests,
            [OsString::from("claude"), bundled.clone().into_os_string()]
        );

        let error = claude_request_with_extension_fallback(
            OsStr::new("claude"),
            |executable| {
                if executable == OsStr::new("claude") {
                    Err("not on the desktop PATH".to_string())
                } else {
                    Err("bundled Claude could not authenticate".to_string())
                }
            },
            |allow_lookup| allow_lookup.then(|| bundled.clone()),
        )
        .unwrap_err();
        assert_eq!(error, "bundled Claude could not authenticate");

        let override_path = OsStr::new("/opt/Claude Preview/claude");
        let mut override_requests = Vec::new();
        let override_result = claude_request_with_extension_fallback(
            override_path,
            |executable| {
                override_requests.push(executable.to_owned());
                Err("configured command failed".to_string())
            },
            |_| panic!("a non-default Claude command must not trigger extension discovery"),
        );
        assert!(override_result.is_err());
        assert_eq!(override_requests, [override_path.to_owned()]);
    }

    #[test]
    fn codex_extension_location_resolves_its_bundled_binary() {
        let temp = TempDir::new().unwrap();
        let binary_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let platform = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            std::env::consts::OS
        };
        let architecture = std::env::consts::ARCH;
        let binary = temp
            .path()
            .join("bin")
            .join(format!("{platform}-{architecture}"))
            .join(binary_name);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"test binary").unwrap();
        let output = format!("{}\n", temp.path().display());

        assert_eq!(
            codex_extension_executable_from_output(output.as_bytes()),
            Some(binary)
        );
        assert_eq!(codex_extension_executable_from_output(b"\n"), None);
        assert_eq!(
            codex_extension_executable_from_output(b"/missing/extension\n"),
            None
        );
    }

    #[test]
    fn extension_cli_discovery_falls_back_to_antigravity_ide() {
        let temp = TempDir::new().unwrap();
        let binary_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let platform = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            std::env::consts::OS
        };
        let architecture = std::env::consts::ARCH;
        let binary = temp
            .path()
            .join("bin")
            .join(format!("{platform}-{architecture}"))
            .join(binary_name);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"test binary").unwrap();
        let runner = FakeExtensionCliRunner {
            antigravity_root: temp.path().to_owned(),
            calls: Mutex::new(Vec::new()),
        };

        assert_eq!(
            locate_extension_with_commands(
                &runner,
                [OsString::from("code"), OsString::from("antigravity-ide")],
                CODEX_EXTENSION_ID,
                codex_extension_binary,
            ),
            Some(binary)
        );
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [
                (
                    OsString::from("code"),
                    vec![
                        OsString::from("--locate-extension"),
                        OsString::from(CODEX_EXTENSION_ID),
                    ],
                ),
                (
                    OsString::from("antigravity-ide"),
                    vec![
                        OsString::from("--locate-extension"),
                        OsString::from(CODEX_EXTENSION_ID),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn provider_discovery_prefers_the_newer_working_editor_install() {
        let temp = TempDir::new().unwrap();
        let older = temp.path().join("vscode-provider");
        let newer = temp.path().join("antigravity-provider");
        fs::write(&older, b"old").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        fs::write(&newer, b"new").unwrap();

        assert_eq!(
            choose_newest_extension_executable(Some(older), Some(newer.clone())),
            Some(newer)
        );
    }

    #[test]
    fn automatic_provider_extension_cli_lookup_is_disabled_only_on_macos() {
        assert!(!automatic_extension_cli_lookup_enabled_for_platform(
            "macos"
        ));
        assert!(automatic_extension_cli_lookup_enabled_for_platform("linux"));
        assert!(automatic_extension_cli_lookup_enabled_for_platform(
            "windows"
        ));
    }

    #[test]
    fn registry_only_provider_discovery_does_not_invoke_the_editor_cli() {
        let expected = PathBuf::from("/bounded/registry/provider");
        let cli_called = std::cell::Cell::new(false);
        let registry_called = std::cell::Cell::new(false);

        let located = locate_provider_extension_executable(
            false,
            || {
                cli_called.set(true);
                Some(PathBuf::from("/unexpected/cli/provider"))
            },
            || {
                registry_called.set(true);
                Some(expected.clone())
            },
        );

        assert_eq!(located, Some(expected));
        assert!(!cli_called.get());
        assert!(registry_called.get());
    }

    #[test]
    fn codex_extension_location_accepts_the_other_macos_architecture() {
        let temp = TempDir::new().unwrap();
        let binary = temp.path().join("bin").join("macos-aarch64").join("codex");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"test binary").unwrap();

        assert_eq!(
            codex_extension_binary_for(temp.path(), "macos", "x86_64", "codex"),
            Some(binary)
        );
    }

    #[test]
    fn codex_extension_registry_fallback_is_bounded_to_a_child_directory() {
        let temp = TempDir::new().unwrap();
        let extensions = temp.path().join(".vscode").join("extensions");
        let valid_relative = "openai.chatgpt-26.5513-test";
        let binary_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let platform = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            std::env::consts::OS
        };
        let architecture = std::env::consts::ARCH;
        let binary = extensions
            .join(valid_relative)
            .join("bin")
            .join(format!("{platform}-{architecture}"))
            .join(binary_name);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"test binary").unwrap();
        fs::write(
            extensions.join("extensions.json"),
            serde_json::to_vec(&json!([
                {
                    "identifier": {"id": "OPENAI.CHATGPT"},
                    "relativeLocation": valid_relative,
                    "metadata": {"installedTimestamp": 100}
                },
                {
                    "identifier": {"id": "openai.chatgpt"},
                    "relativeLocation": "../outside",
                    "metadata": {"installedTimestamp": 200}
                }
            ]))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            codex_extension_from_registry(&extensions),
            Some((100, binary))
        );
    }

    #[test]
    fn claude_extension_location_resolves_its_bundled_binary() {
        let temp = TempDir::new().unwrap();
        let binary_name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let binary = temp
            .path()
            .join("resources")
            .join("native-binary")
            .join(binary_name);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"test binary").unwrap();
        let output = format!("{}\n", temp.path().display());

        assert_eq!(
            claude_extension_executable_from_output(output.as_bytes()),
            Some(binary)
        );
        assert_eq!(claude_extension_executable_from_output(b"\n"), None);
        assert_eq!(
            claude_extension_executable_from_output(b"/missing/extension\n"),
            None
        );
    }

    #[test]
    fn claude_extension_location_accepts_the_other_macos_architecture() {
        let temp = TempDir::new().unwrap();
        for (process_architecture, bundled_architecture) in [("arm64", "x64"), ("x64", "arm64")] {
            let root = temp.path().join(process_architecture);
            let binary = root
                .join("resources")
                .join("native-binaries")
                .join(format!("darwin-{bundled_architecture}"))
                .join("claude");
            fs::create_dir_all(binary.parent().unwrap()).unwrap();
            fs::write(&binary, b"test binary").unwrap();
            assert_eq!(
                claude_extension_binary_for(&root, "darwin", process_architecture, "claude", false,),
                Some(binary)
            );
        }
    }

    #[test]
    fn claude_extension_registry_fallback_is_bounded_to_a_child_directory() {
        let temp = TempDir::new().unwrap();
        let extensions = temp.path().join(".antigravity-ide").join("extensions");
        let valid_relative = "anthropic.claude-code-2.1.222-test";
        let binary_name = if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        };
        let binary = extensions
            .join(valid_relative)
            .join("resources")
            .join("native-binary")
            .join(binary_name);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"test binary").unwrap();
        fs::write(
            extensions.join("extensions.json"),
            serde_json::to_vec(&json!([
                {
                    "identifier": {"id": "anthropic.claude-code"},
                    "relativeLocation": valid_relative,
                    "metadata": {"installedTimestamp": 100}
                },
                {
                    "identifier": {"id": "anthropic.claude-code"},
                    "relativeLocation": "../outside",
                    "metadata": {"installedTimestamp": 200}
                }
            ]))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            claude_extension_from_registry(&extensions),
            Some((100, binary.clone()))
        );
        assert_eq!(
            locate_extension_from_registry_under_home(
                temp.path(),
                CLAUDE_EXTENSION_ID,
                claude_extension_binary,
            ),
            Some(binary)
        );
        assert_eq!(
            vs_compatible_extension_directories(temp.path()),
            [
                temp.path().join(".vscode").join("extensions"),
                temp.path().join(".vscode-insiders").join("extensions"),
                temp.path().join(".vscode-oss").join("extensions"),
                temp.path().join(".cursor").join("extensions"),
                temp.path().join(".antigravity-ide").join("extensions"),
            ]
        );
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
    fn disabled_claude_integration_suppresses_late_statusline_writes() {
        let temp = TempDir::new().unwrap();
        crate::state::set_integration_source_enabled_at(
            temp.path(),
            crate::state::IntegrationSource::ClaudeHooks,
            false,
        )
        .unwrap();
        let input = br#"{
            "rate_limits": {
                "five_hour": {"used_percentage": 20, "resets_at": 1800000000}
            }
        }"#;

        let (code, output) = claude_input(temp.path(), input, 1_000);

        assert_eq!(code, 0);
        assert!(output.is_empty());
        assert!(!claude_record_path(temp.path()).exists());
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
    fn claude_control_parser_ignores_unrelated_messages() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(br#"{"type":"system","subtype":"init"}"#.to_vec()))
            .unwrap();
        sender
            .send(Ok(
                br#"{"type":"control_response","response":{"subtype":"success","request_id":"other","response":{}}}"#
                    .to_vec(),
            ))
            .unwrap();
        sender
            .send(Ok(
                br#"{"type":"control_response","response":{"subtype":"success","request_id":"usage","response":{"rate_limits":{"five_hour":{"utilization":12,"private":"private-window"},"spend":{"amount":99,"currency":"private-currency"},"extra_usage":{"private":"private-extra"}},"session":{"email":"private@example.test"},"behaviors":{"week":{"skills":["private-skill"]}}}}}"#
                    .to_vec(),
            ))
            .unwrap();

        let result = wait_for_claude_control_response(
            &receiver,
            "usage",
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(
            result
                .pointer("/rate_limits/five_hour/utilization")
                .and_then(Value::as_f64),
            Some(12.0)
        );
        let retained = result.to_string();
        assert!(!retained.contains("private@example.test"));
        assert!(!retained.contains("private-skill"));
        assert!(!retained.contains("private-window"));
        assert!(!retained.contains("private-currency"));
        assert!(!retained.contains("private-extra"));
        assert_eq!(result.as_object().unwrap().len(), 1);
        assert_eq!(result["rate_limits"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn claude_control_parser_rejects_error_responses() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(
                br#"{"type":"control_response","response":{"subtype":"error","request_id":"usage","error":"unsupported"}}"#
                    .to_vec(),
            ))
            .unwrap();

        let result = wait_for_claude_control_response(
            &receiver,
            "usage",
            Instant::now() + Duration::from_secs(1),
        );

        assert!(result.is_err());
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
