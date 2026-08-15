//! Production management for the VSParallel VS Code companion extension.
//!
//! The extension's small, dependency-free payload is compiled into the desktop
//! executable.  A VSIX is assembled in memory with a deterministic ZIP writer,
//! so installing the production application does not require Python, Node, npm,
//! or a checked-in generated VSIX.

use serde::Serialize;
use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub const EXTENSION_ID: &str = "vsparallel.vsparallel-companion";
pub const VSCODE_PROFILE_ENV: &str = "VSPARALLEL_VSCODE_PROFILE";
pub const ANTIGRAVITY_IDE_PROFILE_ENV: &str = "VSPARALLEL_ANTIGRAVITY_IDE_PROFILE";
pub const CURSOR_PROFILE_ENV: &str = "VSPARALLEL_CURSOR_PROFILE";

const CONTENT_TYPES: &[u8] = include_bytes!("../../companion/[Content_Types].xml");
const VSIX_MANIFEST: &[u8] = include_bytes!("../../companion/extension.vsixmanifest");
const PACKAGE_JSON: &[u8] = include_bytes!("../../companion/package.json");
const EXTENSION_JS: &[u8] = include_bytes!("../../companion/extension.js");
const ICON_PNG: &[u8] = include_bytes!("../../companion/icon.png");
const README: &[u8] = include_bytes!("../../companion/README.md");
const LICENSE: &[u8] = include_bytes!("../../companion/LICENSE");

const VSIX_ENTRIES: [(&str, &[u8]); 7] = [
    ("[Content_Types].xml", CONTENT_TYPES),
    ("extension.vsixmanifest", VSIX_MANIFEST),
    ("extension/package.json", PACKAGE_JSON),
    ("extension/extension.js", EXTENSION_JS),
    ("extension/icon.png", ICON_PNG),
    ("extension/README.md", README),
    ("extension/LICENSE.txt", LICENSE),
];

static BUNDLED_MANIFEST: OnceLock<Result<BundledManifest, String>> = OnceLock::new();
static BUNDLED_VSIX: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const STATUS_CLI_TIMEOUT: Duration = Duration::from_secs(15);
const CHANGE_CLI_TIMEOUT: Duration = Duration::from_secs(120);
const CLI_POLL_INTERVAL: Duration = Duration::from_millis(25);
const POST_EXIT_PIPE_DRAIN_GRACE: Duration = Duration::from_millis(100);
const CLI_CAPTURE_LIMIT: usize = 256 * 1024;

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn kill_process_group(process_group: i32, signal: i32) -> i32;
}

#[derive(Debug)]
struct BundledManifest {
    version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionStatusState {
    Current,
    DifferentVersion,
    VersionUnknown,
    NotInstalled,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionStatus {
    pub state: CompanionStatusState,
    pub extension_id: String,
    pub bundled_version: Option<String>,
    pub installed_version: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionAction {
    Install,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionOperationResult {
    pub action: CompanionAction,
    pub verified: bool,
    pub message: String,
    pub status: CompanionStatus,
}

#[derive(Debug)]
pub struct CliOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait CodeCliRunner {
    fn run(&self, executable: &OsStr, arguments: &[OsString]) -> io::Result<CliOutput>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessCodeCliRunner;

#[derive(Debug, Clone, Copy)]
struct CliRunLimits {
    status_timeout: Duration,
    change_timeout: Duration,
    poll_interval: Duration,
    output_limit: usize,
}

impl Default for CliRunLimits {
    fn default() -> Self {
        Self {
            status_timeout: STATUS_CLI_TIMEOUT,
            change_timeout: CHANGE_CLI_TIMEOUT,
            poll_interval: CLI_POLL_INTERVAL,
            output_limit: CLI_CAPTURE_LIMIT,
        }
    }
}

impl CodeCliRunner for ProcessCodeCliRunner {
    fn run(&self, executable: &OsStr, arguments: &[OsString]) -> io::Result<CliOutput> {
        self.run_with_limits(executable, arguments, CliRunLimits::default())
    }
}

impl ProcessCodeCliRunner {
    fn run_with_limits(
        &self,
        executable: &OsStr,
        arguments: &[OsString],
        limits: CliRunLimits,
    ) -> io::Result<CliOutput> {
        let (operation, timeout) = cli_operation_timeout(arguments, limits);
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        let mut child = command.spawn()?;
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = terminate_and_reap(&mut child);
                return Err(io::Error::other("VS Code stdout pipe was unavailable"));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                drop(stdout);
                let _ = terminate_and_reap(&mut child);
                return Err(io::Error::other("VS Code stderr pipe was unavailable"));
            }
        };

        let mut stdout_capture = CliStreamCapture::spawn(stdout, limits.output_limit);
        let mut stderr_capture = CliStreamCapture::spawn(stderr, limits.output_limit);
        let started = Instant::now();
        #[cfg(unix)]
        let process_group_id = child.id();
        let mut status = None;
        #[cfg(unix)]
        let mut process_group_terminated = false;
        #[cfg(unix)]
        let mut child_exit_observed_at = None;

        loop {
            stdout_capture.poll("stdout");
            stderr_capture.poll("stderr");

            if status.is_none() {
                match child.try_wait() {
                    Ok(Some(exit_status)) => {
                        status = Some(exit_status);
                        #[cfg(unix)]
                        {
                            child_exit_observed_at = Some(Instant::now());
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = terminate_and_reap(&mut child);
                        return Err(error);
                    }
                }
            }

            if let Some(exit_status) = status {
                if stdout_capture.is_finished() && stderr_capture.is_finished() {
                    return Ok(CliOutput {
                        success: exit_status.success(),
                        exit_code: exit_status.code(),
                        stdout: stdout_capture.into_result("stdout")?,
                        stderr: stderr_capture.into_result("stderr")?,
                    });
                }

                // A CLI wrapper can exit while one of its descendants still
                // owns an inherited pipe. The child was placed in its own
                // process group specifically so those writers can be closed
                // without signalling VSParallel itself. First allow normal EOF
                // and reader-channel delivery a short grace period so a
                // persistent descendant that closed its inherited streams is
                // not killed because of a scheduling race. This remains best
                // effort: a deliberately detached writer is still bounded by
                // the original command deadline below.
                #[cfg(unix)]
                if !process_group_terminated
                    && child_exit_observed_at.is_some_and(|observed_at| {
                        observed_at.elapsed() >= POST_EXIT_PIPE_DRAIN_GRACE
                    })
                {
                    let _ = terminate_process_group(process_group_id);
                    process_group_terminated = true;
                }
            }

            let elapsed = started.elapsed();
            if elapsed >= timeout {
                if status.is_none() {
                    terminate_and_reap(&mut child)?;
                }
                #[cfg(unix)]
                if !process_group_terminated {
                    let _ = terminate_process_group(process_group_id);
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "VS Code {operation} command timed out after {}",
                        duration_label(timeout)
                    ),
                ));
            }

            let mut sleep_for = limits.poll_interval.min(timeout.saturating_sub(elapsed));
            #[cfg(unix)]
            if !process_group_terminated {
                if let Some(observed_at) = child_exit_observed_at {
                    sleep_for = sleep_for
                        .min(POST_EXIT_PIPE_DRAIN_GRACE.saturating_sub(observed_at.elapsed()));
                }
            }
            thread::sleep(sleep_for);
        }
    }
}

fn cli_operation_timeout(arguments: &[OsString], limits: CliRunLimits) -> (&'static str, Duration) {
    if arguments.first().is_some_and(|argument| {
        argument == "--install-extension" || argument == "--uninstall-extension"
    }) {
        ("change", limits.change_timeout)
    } else {
        ("status", limits.status_timeout)
    }
}

fn read_capped(mut reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut captured = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(captured);
        }
        let remaining = limit.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
}

struct CliStreamCapture {
    receiver: Receiver<io::Result<Vec<u8>>>,
    result: Option<io::Result<Vec<u8>>>,
}

impl CliStreamCapture {
    fn spawn(reader: impl Read + Send + 'static, limit: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let _ = sender.send(read_capped(reader, limit));
        });
        Self {
            receiver,
            result: None,
        }
    }

    fn poll(&mut self, stream: &str) {
        if self.result.is_some() {
            return;
        }
        match self.receiver.try_recv() {
            Ok(result) => self.result = Some(result),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.result = Some(Err(io::Error::other(format!(
                    "VS Code {stream} reader stopped unexpectedly"
                ))));
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.result.is_some()
    }

    fn into_result(self, stream: &str) -> io::Result<Vec<u8>> {
        self.result.unwrap_or_else(|| {
            Err(io::Error::other(format!(
                "VS Code {stream} reader did not finish"
            )))
        })
    }
}

fn duration_label(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{} seconds", duration.as_secs())
    } else {
        format!("{} ms", duration.as_millis())
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
fn terminate_process_group(process_group_id: u32) -> io::Result<()> {
    const SIGKILL: i32 = 9;
    let process_group = i32::try_from(process_group_id)
        .map_err(|_| io::Error::other("VS Code process identifier was out of range"))?;
    // SAFETY: the child was spawned into a process group whose ID is its PID.
    // A negative PID asks POSIX `kill` to signal only that process group.
    if unsafe { kill_process_group(-process_group, SIGKILL) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn terminate_child(child: &mut Child) -> io::Result<()> {
    terminate_process_group(child.id()).or_else(|_| child.kill())
}

#[cfg(not(unix))]
fn terminate_child(child: &mut Child) -> io::Result<()> {
    child.kill()
}

/// Returns the version embedded in the desktop executable's companion payload.
pub fn bundled_companion_version() -> Result<&'static str, String> {
    bundled_manifest().map(|manifest| manifest.version.as_str())
}

/// Returns a complete VSIX assembled solely from assets embedded in the binary.
pub fn bundled_vsix_bytes() -> Result<&'static [u8], String> {
    match BUNDLED_VSIX.get_or_init(build_bundled_vsix) {
        Ok(bytes) => Ok(bytes.as_slice()),
        Err(error) => Err(error.clone()),
    }
}

/// Returns whether an editor CLI command has a concrete local executable.
/// This distinguishes an unavailable executable from an executable whose
/// status command failed, so global removal can report the right verification
/// outcome without attempting to launch a command that cannot exist.
pub(crate) fn command_is_available(executable: &OsStr) -> bool {
    if executable.is_empty() {
        return false;
    }
    let path = Path::new(executable);
    let explicit = path.is_absolute()
        || path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty());
    if explicit {
        return path.is_file();
    }

    let Some(search_path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&search_path).any(|directory| command_candidates(&directory, executable))
}

fn command_candidates(directory: &Path, executable: &OsStr) -> bool {
    let exact = directory.join(executable);
    if exact.is_file() {
        return true;
    }
    #[cfg(windows)]
    {
        if Path::new(executable).extension().is_some() {
            return false;
        }
        let extensions =
            env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
        return extensions
            .to_string_lossy()
            .split(';')
            .filter(|extension| !extension.is_empty())
            .any(|extension| {
                let mut candidate = executable.to_os_string();
                candidate.push(extension);
                directory.join(candidate).is_file()
            });
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Queries VS Code with `--list-extensions --show-versions`.
///
/// Operational failures are represented as `Unavailable`, rather than making a
/// setup screen fail to render.
pub fn companion_status(executable: &OsStr) -> CompanionStatus {
    companion_status_with(
        &ProcessCodeCliRunner,
        executable,
        profile_from_environment(VSCODE_PROFILE_ENV).as_deref(),
    )
}

/// Queries a VS Code-compatible editor while keeping operational diagnostics
/// specific to the product whose CLI was invoked.
pub fn companion_status_for_editor(
    executable: &OsStr,
    editor_name: &str,
    profile_environment: &str,
) -> CompanionStatus {
    relabel_status(
        companion_status_with(
            &ProcessCodeCliRunner,
            executable,
            profile_from_environment(profile_environment).as_deref(),
        ),
        editor_name,
    )
}

/// Installs (or updates) the embedded companion using VS Code's supported CLI.
pub fn install_companion(executable: &OsStr) -> Result<CompanionOperationResult, String> {
    install_companion_with(
        &ProcessCodeCliRunner,
        executable,
        &env::temp_dir(),
        profile_from_environment(VSCODE_PROFILE_ENV).as_deref(),
    )
}

/// Installs the companion into a VS Code-compatible editor.
pub fn install_companion_for_editor(
    executable: &OsStr,
    editor_name: &str,
    profile_environment: &str,
) -> Result<CompanionOperationResult, String> {
    install_companion_with(
        &ProcessCodeCliRunner,
        executable,
        &env::temp_dir(),
        profile_from_environment(profile_environment).as_deref(),
    )
    .map(|result| relabel_operation(result, editor_name))
    .map_err(|error| relabel_editor_name(error, editor_name))
}

/// Uninstalls the companion by its exact extension identifier.
pub fn uninstall_companion(executable: &OsStr) -> Result<CompanionOperationResult, String> {
    uninstall_companion_with(
        &ProcessCodeCliRunner,
        executable,
        profile_from_environment(VSCODE_PROFILE_ENV).as_deref(),
    )
}

/// Removes the companion from a VS Code-compatible editor.
pub fn uninstall_companion_for_editor(
    executable: &OsStr,
    editor_name: &str,
    profile_environment: &str,
) -> Result<CompanionOperationResult, String> {
    uninstall_companion_with(
        &ProcessCodeCliRunner,
        executable,
        profile_from_environment(profile_environment).as_deref(),
    )
    .map(|result| relabel_operation(result, editor_name))
    .map_err(|error| relabel_editor_name(error, editor_name))
}

fn profile_from_environment(key: &str) -> Option<OsString> {
    env::var_os(key).filter(|value| !value.is_empty())
}

fn with_profile(mut arguments: Vec<OsString>, profile: Option<&OsStr>) -> Vec<OsString> {
    if let Some(profile) = profile.filter(|profile| !profile.is_empty()) {
        arguments.push(OsString::from("--profile"));
        arguments.push(profile.to_owned());
    }
    arguments
}

fn relabel_operation(
    mut result: CompanionOperationResult,
    editor_name: &str,
) -> CompanionOperationResult {
    result.message = relabel_editor_name(result.message, editor_name);
    result.status = relabel_status(result.status, editor_name);
    result
}

fn relabel_status(mut status: CompanionStatus, editor_name: &str) -> CompanionStatus {
    status.detail = status
        .detail
        .map(|detail| relabel_editor_name(detail, editor_name));
    status
}

fn relabel_editor_name(message: String, editor_name: &str) -> String {
    message.replace("VS Code", editor_name)
}

fn bundled_manifest() -> Result<&'static BundledManifest, String> {
    match BUNDLED_MANIFEST.get_or_init(parse_bundled_manifest) {
        Ok(manifest) => Ok(manifest),
        Err(error) => Err(error.clone()),
    }
}

fn parse_bundled_manifest() -> Result<BundledManifest, String> {
    let value: serde_json::Value = serde_json::from_slice(PACKAGE_JSON)
        .map_err(|error| format!("the embedded companion package.json is invalid: {error}"))?;
    let publisher = value
        .get("publisher")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "the embedded companion package has no publisher".to_string())?;
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "the embedded companion package has no name".to_string())?;
    let identifier = format!("{publisher}.{name}");
    if identifier != EXTENSION_ID {
        return Err(format!(
            "the embedded companion identifier is `{identifier}`, expected `{EXTENSION_ID}`"
        ));
    }

    let version = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| !version.trim().is_empty())
        .ok_or_else(|| "the embedded companion package has no version".to_string())?;
    if version.len() > 64 || version.chars().any(char::is_control) {
        return Err("the embedded companion package has an invalid version".to_string());
    }

    Ok(BundledManifest {
        version: version.to_string(),
    })
}

fn build_bundled_vsix() -> Result<Vec<u8>, String> {
    // Validate the identity and version before exposing an installable package.
    let manifest = bundled_manifest()?;
    validate_vsix_manifest_identity(VSIX_MANIFEST, &manifest.version)?;

    let mut archive = Vec::new();
    let mut central_directory = Vec::new();

    for (name, contents) in VSIX_ENTRIES {
        let name_bytes = name.as_bytes();
        let name_len = u16::try_from(name_bytes.len())
            .map_err(|_| format!("embedded VSIX entry name is too long: {name}"))?;
        let size = u32::try_from(contents.len())
            .map_err(|_| format!("embedded VSIX entry is too large: {name}"))?;
        let local_offset = u32::try_from(archive.len())
            .map_err(|_| "the embedded VSIX is too large".to_string())?;
        let crc = crc32(contents);

        // Local file header. Method 0 (stored) is universally supported by ZIP
        // readers and keeps this packager dependency-free.
        push_u32(&mut archive, 0x0403_4b50);
        push_u16(&mut archive, 20);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0); // fixed 00:00:00
        push_u16(&mut archive, 0x5c21); // fixed 2026-01-01
        push_u32(&mut archive, crc);
        push_u32(&mut archive, size);
        push_u32(&mut archive, size);
        push_u16(&mut archive, name_len);
        push_u16(&mut archive, 0);
        archive.extend_from_slice(name_bytes);
        archive.extend_from_slice(contents);

        // Matching central-directory record.
        push_u32(&mut central_directory, 0x0201_4b50);
        push_u16(&mut central_directory, 0x0314); // Unix, ZIP 2.0
        push_u16(&mut central_directory, 20);
        push_u16(&mut central_directory, 0);
        push_u16(&mut central_directory, 0);
        push_u16(&mut central_directory, 0);
        push_u16(&mut central_directory, 0x5c21);
        push_u32(&mut central_directory, crc);
        push_u32(&mut central_directory, size);
        push_u32(&mut central_directory, size);
        push_u16(&mut central_directory, name_len);
        push_u16(&mut central_directory, 0);
        push_u16(&mut central_directory, 0);
        push_u16(&mut central_directory, 0);
        push_u16(&mut central_directory, 0);
        push_u32(&mut central_directory, 0o100644 << 16);
        push_u32(&mut central_directory, local_offset);
        central_directory.extend_from_slice(name_bytes);
    }

    let central_offset =
        u32::try_from(archive.len()).map_err(|_| "the embedded VSIX is too large".to_string())?;
    let central_size = u32::try_from(central_directory.len())
        .map_err(|_| "the embedded VSIX directory is too large".to_string())?;
    archive.extend_from_slice(&central_directory);

    let entry_count = u16::try_from(VSIX_ENTRIES.len())
        .map_err(|_| "the embedded VSIX has too many entries".to_string())?;
    push_u32(&mut archive, 0x0605_4b50);
    push_u16(&mut archive, 0);
    push_u16(&mut archive, 0);
    push_u16(&mut archive, entry_count);
    push_u16(&mut archive, entry_count);
    push_u32(&mut archive, central_size);
    push_u32(&mut archive, central_offset);
    push_u16(&mut archive, 0);

    Ok(archive)
}

fn validate_vsix_manifest_identity(bytes: &[u8], expected_version: &str) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("the embedded VSIX manifest is not UTF-8: {error}"))?;
    let identity = text
        .split_once("<Identity ")
        .and_then(|(_, rest)| rest.split_once(" />"))
        .map(|(identity, _)| identity)
        .ok_or_else(|| "the embedded VSIX manifest has no Identity element".to_string())?;
    for (attribute, expected) in [
        ("Id", "vsparallel-companion"),
        ("Publisher", "vsparallel"),
        ("Version", expected_version),
    ] {
        let needle = format!("{attribute}=\"{expected}\"");
        if !identity.split_whitespace().any(|part| part == needle) {
            return Err(format!(
                "the embedded VSIX manifest {attribute} does not match the companion package"
            ));
        }
    }
    Ok(())
}

fn companion_status_with<R: CodeCliRunner>(
    runner: &R,
    executable: &OsStr,
    profile: Option<&OsStr>,
) -> CompanionStatus {
    let bundled_version = match bundled_companion_version() {
        Ok(version) => Some(version.to_string()),
        Err(error) => return unavailable_status(None, error),
    };
    if executable.is_empty() {
        return unavailable_status(
            bundled_version,
            "the VS Code command is empty; configure a VS Code executable".to_string(),
        );
    }

    let arguments = with_profile(
        vec![
            OsString::from("--list-extensions"),
            OsString::from("--show-versions"),
        ],
        profile,
    );
    let output = match runner.run(executable, &arguments) {
        Ok(output) => output,
        Err(error) => {
            return unavailable_status(
                bundled_version,
                format!("could not run the VS Code command: {error}"),
            )
        }
    };
    if !output.success {
        return unavailable_status(
            bundled_version,
            cli_failure_detail("query installed extensions", &output),
        );
    }

    status_from_extension_list(
        &output.stdout,
        bundled_version.expect("version was set above"),
    )
}

fn status_from_extension_list(output: &[u8], bundled_version: String) -> CompanionStatus {
    let text = String::from_utf8_lossy(output);
    let mut versions = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let (identifier, version) = line
            .split_once('@')
            .map_or((line, None), |(identifier, version)| {
                (identifier, Some(version.trim()))
            });
        if identifier.trim().eq_ignore_ascii_case(EXTENSION_ID) {
            versions.push(version.filter(|version| !version.is_empty()));
        }
    }

    if versions.is_empty() {
        return CompanionStatus {
            state: CompanionStatusState::NotInstalled,
            extension_id: EXTENSION_ID.to_string(),
            bundled_version: Some(bundled_version),
            installed_version: None,
            detail: None,
        };
    }

    let known: HashSet<&str> = versions.iter().filter_map(|version| *version).collect();
    if versions.iter().any(|version| version.is_none()) || known.len() != 1 {
        return CompanionStatus {
            state: CompanionStatusState::VersionUnknown,
            extension_id: EXTENSION_ID.to_string(),
            bundled_version: Some(bundled_version),
            installed_version: None,
            detail: Some(
                "VS Code listed the companion, but its installed version was ambiguous".to_string(),
            ),
        };
    }

    let installed_version = known.into_iter().next().expect("one version was found");
    let state = if installed_version == bundled_version {
        CompanionStatusState::Current
    } else {
        CompanionStatusState::DifferentVersion
    };
    CompanionStatus {
        state,
        extension_id: EXTENSION_ID.to_string(),
        bundled_version: Some(bundled_version),
        installed_version: Some(installed_version.to_string()),
        detail: None,
    }
}

fn install_companion_with<R: CodeCliRunner>(
    runner: &R,
    executable: &OsStr,
    temporary_directory: &Path,
    profile: Option<&OsStr>,
) -> Result<CompanionOperationResult, String> {
    validate_executable(executable)?;
    let package = TemporaryVsix::create(temporary_directory, bundled_vsix_bytes()?)?;
    let arguments = with_profile(
        vec![
            OsString::from("--install-extension"),
            package.path().as_os_str().to_owned(),
            OsString::from("--force"),
        ],
        profile,
    );
    let output = runner
        .run(executable, &arguments)
        .map_err(|error| format!("could not run the VS Code install command: {error}"))?;
    if !output.success {
        return Err(cli_failure_detail(
            "install the VSParallel companion",
            &output,
        ));
    }

    // Query only after the CLI has returned and finished reading the VSIX. The
    // TemporaryVsix guard removes the package on every return path.
    let status = companion_status_with(runner, executable, profile);
    let verified = status.state == CompanionStatusState::Current;
    let message = if verified {
        "VSParallel Companion is installed and current".to_string()
    } else {
        "VS Code accepted the install command, but the installed version could not be verified"
            .to_string()
    };
    Ok(CompanionOperationResult {
        action: CompanionAction::Install,
        verified,
        message,
        status,
    })
}

fn uninstall_companion_with<R: CodeCliRunner>(
    runner: &R,
    executable: &OsStr,
    profile: Option<&OsStr>,
) -> Result<CompanionOperationResult, String> {
    validate_executable(executable)?;

    // Make uninstall idempotent. This also avoids presenting VS Code's
    // "extension is not installed" exit code as a user-facing failure.
    let before = companion_status_with(runner, executable, profile);
    if before.state == CompanionStatusState::NotInstalled {
        return Ok(CompanionOperationResult {
            action: CompanionAction::Uninstall,
            verified: true,
            message: "VSParallel Companion is already uninstalled".to_string(),
            status: before,
        });
    }

    let arguments = with_profile(
        vec![
            OsString::from("--uninstall-extension"),
            OsString::from(EXTENSION_ID),
        ],
        profile,
    );
    let output = runner
        .run(executable, &arguments)
        .map_err(|error| format!("could not run the VS Code uninstall command: {error}"))?;
    if !output.success {
        return Err(cli_failure_detail(
            "uninstall the VSParallel companion",
            &output,
        ));
    }

    let status = companion_status_with(runner, executable, profile);
    let verified = status.state == CompanionStatusState::NotInstalled;
    let message = if verified {
        "VSParallel Companion is uninstalled".to_string()
    } else {
        "VS Code accepted the uninstall command, but removal could not be verified".to_string()
    };
    Ok(CompanionOperationResult {
        action: CompanionAction::Uninstall,
        verified,
        message,
        status,
    })
}

fn validate_executable(executable: &OsStr) -> Result<(), String> {
    if executable.is_empty() {
        Err("the VS Code command is empty; configure a VS Code executable".to_string())
    } else {
        Ok(())
    }
}

fn unavailable_status(bundled_version: Option<String>, detail: String) -> CompanionStatus {
    CompanionStatus {
        state: CompanionStatusState::Unavailable,
        extension_id: EXTENSION_ID.to_string(),
        bundled_version,
        installed_version: None,
        detail: Some(detail),
    }
}

fn cli_failure_detail(action: &str, output: &CliOutput) -> String {
    let code = output
        .exit_code
        .map_or_else(|| "unknown".to_string(), |code| code.to_string());
    let detail = concise_cli_output(&output.stderr)
        .or_else(|| concise_cli_output(&output.stdout))
        .map_or_else(String::new, |detail| format!(": {detail}"));
    format!("VS Code could not {action} (exit code {code}){detail}")
}

fn concise_cli_output(bytes: &[u8]) -> Option<String> {
    const LIMIT: usize = 1024;
    let text = String::from_utf8_lossy(bytes);
    let normalized: String = text
        .chars()
        .map(|character| {
            if character.is_control() && !character.is_whitespace() {
                '�'
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return None;
    }

    let mut shortened = normalized.chars().take(LIMIT).collect::<String>();
    if normalized.chars().count() > LIMIT {
        shortened.push('…');
    }
    Some(shortened)
}

struct TemporaryVsix {
    path: PathBuf,
}

impl TemporaryVsix {
    fn create(directory: &Path, contents: &[u8]) -> Result<Self, String> {
        if !directory.is_dir() {
            return Err(format!(
                "the temporary directory is unavailable: {}",
                directory.display()
            ));
        }

        for _ in 0..32 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let filename = format!(
                "vsparallel-companion-{}-{timestamp}-{sequence}.vsix",
                std::process::id()
            );
            let path = directory.join(filename);
            match create_new_file(&path, contents) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    // The path may have been created before a later write or
                    // sync failed. Never leave that partial package behind.
                    let _ = fs::remove_file(&path);
                    return Err(format!(
                        "could not create the temporary companion package: {error}"
                    ));
                }
            }
        }
        Err("could not allocate a unique temporary companion package".to_string())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryVsix {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_new_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn push_u16(destination: &mut Vec<u8>, value: u16) {
    destination.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(destination: &mut Vec<u8>, value: u32) {
    destination.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Debug)]
    enum ScriptedResult {
        Output(CliOutput),
        IoError(io::ErrorKind),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedCall {
        executable: OsString,
        arguments: Vec<OsString>,
    }

    struct ScriptedRunner {
        results: Mutex<VecDeque<ScriptedResult>>,
        calls: Mutex<Vec<RecordedCall>>,
        captured_package: Mutex<Option<Vec<u8>>>,
        captured_package_path: Mutex<Option<PathBuf>>,
    }

    impl ScriptedRunner {
        fn new(results: Vec<ScriptedResult>) -> Self {
            Self {
                results: Mutex::new(results.into()),
                calls: Mutex::new(Vec::new()),
                captured_package: Mutex::new(None),
                captured_package_path: Mutex::new(None),
            }
        }

        fn calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CodeCliRunner for ScriptedRunner {
        fn run(&self, executable: &OsStr, arguments: &[OsString]) -> io::Result<CliOutput> {
            self.calls.lock().unwrap().push(RecordedCall {
                executable: executable.to_owned(),
                arguments: arguments.to_vec(),
            });
            if arguments
                .first()
                .is_some_and(|value| value == "--install-extension")
            {
                let path = PathBuf::from(arguments.get(1).expect("install path argument"));
                assert!(path.exists(), "temporary VSIX must exist during CLI call");
                *self.captured_package.lock().unwrap() = Some(fs::read(&path)?);
                *self.captured_package_path.lock().unwrap() = Some(path);
            }

            match self
                .results
                .lock()
                .unwrap()
                .pop_front()
                .expect("scripted result")
            {
                ScriptedResult::Output(output) => Ok(output),
                ScriptedResult::IoError(kind) => Err(io::Error::new(kind, "injected failure")),
            }
        }
    }

    fn successful(stdout: impl Into<Vec<u8>>) -> ScriptedResult {
        ScriptedResult::Output(CliOutput {
            success: true,
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: Vec::new(),
        })
    }

    fn failed(code: i32, stderr: impl Into<Vec<u8>>) -> ScriptedResult {
        ScriptedResult::Output(CliOutput {
            success: false,
            exit_code: Some(code),
            stdout: Vec::new(),
            stderr: stderr.into(),
        })
    }

    #[test]
    fn process_runner_uses_separate_status_and_change_timeouts() {
        let limits = CliRunLimits::default();
        assert_eq!(
            cli_operation_timeout(&[OsString::from("--list-extensions")], limits),
            ("status", STATUS_CLI_TIMEOUT)
        );
        assert_eq!(
            cli_operation_timeout(&[OsString::from("--install-extension")], limits),
            ("change", CHANGE_CLI_TIMEOUT)
        );
        assert_eq!(
            cli_operation_timeout(&[OsString::from("--uninstall-extension")], limits),
            ("change", CHANGE_CLI_TIMEOUT)
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_drains_both_streams_but_caps_captured_output() {
        let arguments = vec![
            OsString::from("-c"),
            OsString::from(
                "i=0; while [ \"$i\" -lt 500 ]; do printf 'stdout-line-0123456789\\n'; printf 'stderr-line-0123456789\\n' >&2; i=$((i + 1)); done",
            ),
        ];
        let limits = CliRunLimits {
            status_timeout: Duration::from_secs(5),
            change_timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(5),
            output_limit: 1024,
        };

        let output = ProcessCodeCliRunner
            .run_with_limits(OsStr::new("/bin/sh"), &arguments, limits)
            .unwrap();

        assert!(output.success);
        assert_eq!(output.stdout.len(), limits.output_limit);
        assert_eq!(output.stderr.len(), limits.output_limit);
        assert!(output.stdout.starts_with(b"stdout-line-"));
        assert!(output.stderr.starts_with(b"stderr-line-"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exited_process_does_not_wait_for_descendant_inherited_pipes() {
        let temporary_directory = TempDir::new().unwrap();
        let pid_file = temporary_directory.path().join("descendant-pid");
        let arguments = vec![
            OsString::from("-c"),
            OsString::from(
                "sleep 30 & descendant=$!; printf '%s\n' \"$descendant\" > \"$1\"; printf 'direct stdout\n'; printf 'direct stderr\n' >&2; exit 0",
            ),
            OsString::from("vsparallel-inherited-pipe-test"),
            pid_file.as_os_str().to_owned(),
        ];
        let limits = CliRunLimits {
            status_timeout: Duration::from_secs(2),
            change_timeout: Duration::from_secs(2),
            poll_interval: Duration::from_millis(5),
            output_limit: 1024,
        };
        let started = Instant::now();

        let output = ProcessCodeCliRunner
            .run_with_limits(OsStr::new("/bin/sh"), &arguments, limits)
            .unwrap();

        assert!(output.success);
        assert_eq!(output.stdout, b"direct stdout\n");
        assert_eq!(output.stderr, b"direct stderr\n");
        assert!(started.elapsed() >= POST_EXIT_PIPE_DRAIN_GRACE);
        assert!(started.elapsed() < Duration::from_secs(1));
        let descendant_pid: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..50 {
            if !Path::new(&format!("/proc/{descendant_pid}")).exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("descendant retaining VS Code output pipes was not terminated");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn post_exit_grace_preserves_a_descendant_that_closed_inherited_pipes() {
        let temporary_directory = TempDir::new().unwrap();
        let pid_file = temporary_directory
            .path()
            .join("persistent-descendant-pids");
        let arguments = vec![
            OsString::from("-c"),
            OsString::from(
                "sleep 30 >/dev/null 2>&1 & descendant=$!; printf '%s %s\n' \"$$\" \"$descendant\" > \"$1\"; printf 'complete\n'; exit 0",
            ),
            OsString::from("vsparallel-persistent-descendant-test"),
            pid_file.as_os_str().to_owned(),
        ];
        let limits = CliRunLimits {
            status_timeout: Duration::from_secs(2),
            change_timeout: Duration::from_secs(2),
            poll_interval: Duration::from_millis(5),
            output_limit: 1024,
        };

        let output = ProcessCodeCliRunner
            .run_with_limits(OsStr::new("/bin/sh"), &arguments, limits)
            .unwrap();
        let process_ids: Vec<u32> = fs::read_to_string(&pid_file)
            .unwrap()
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        assert_eq!(process_ids.len(), 2);
        let process_group_id = process_ids[0];
        let descendant_pid = process_ids[1];
        let descendant_survived = Path::new(&format!("/proc/{descendant_pid}")).exists();
        let cleanup = terminate_process_group(process_group_id);

        assert!(output.success);
        assert_eq!(output.stdout, b"complete\n");
        assert!(output.stderr.is_empty());
        assert!(
            descendant_survived,
            "a persistent descendant with closed output pipes should not be terminated"
        );
        cleanup.expect("the persistent test descendant should be terminated");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipe_drain_respects_the_process_deadline_for_a_detached_writer() {
        let setsid = Path::new("/usr/bin/setsid");
        assert!(setsid.is_file(), "the Linux timeout test requires setsid");
        let temporary_directory = TempDir::new().unwrap();
        let pid_file = temporary_directory.path().join("detached-pid");
        let arguments = vec![
            OsString::from("-c"),
            OsString::from(
                "/usr/bin/setsid /bin/sh -c 'printf \"%s\\n\" \"$$\" > \"$1\"; sleep 30' vsparallel-detached \"$1\" & while [ ! -s \"$1\" ]; do sleep 0.01; done; exit 0",
            ),
            OsString::from("vsparallel-drain-timeout-test"),
            pid_file.as_os_str().to_owned(),
        ];
        let limits = CliRunLimits {
            status_timeout: Duration::from_millis(500),
            change_timeout: Duration::from_millis(500),
            poll_interval: Duration::from_millis(5),
            output_limit: 1024,
        };
        let started = Instant::now();

        let result =
            ProcessCodeCliRunner.run_with_limits(OsStr::new("/bin/sh"), &arguments, limits);
        let elapsed = started.elapsed();
        let detached_pid: u32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let cleanup = terminate_process_group(detached_pid);

        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("status command timed out"));
        assert!(elapsed >= limits.status_timeout);
        assert!(elapsed < Duration::from_secs(3));
        cleanup.expect("the detached test writer should be terminated");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timed_out_process_is_killed_with_its_children_and_reaped() {
        let temporary_directory = TempDir::new().unwrap();
        let pid_file = temporary_directory.path().join("pids");
        let arguments = vec![
            OsString::from("-c"),
            OsString::from(
                "sleep 30 & child=$!; printf '%s %s\\n' \"$$\" \"$child\" > \"$1\"; wait",
            ),
            OsString::from("vsparallel-timeout-test"),
            pid_file.as_os_str().to_owned(),
        ];
        let limits = CliRunLimits {
            status_timeout: Duration::from_millis(500),
            change_timeout: Duration::from_millis(500),
            poll_interval: Duration::from_millis(5),
            output_limit: 1024,
        };
        let started = Instant::now();

        let error = ProcessCodeCliRunner
            .run_with_limits(OsStr::new("/bin/sh"), &arguments, limits)
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("status command timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
        let process_ids: Vec<u32> = fs::read_to_string(&pid_file)
            .unwrap()
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        assert_eq!(process_ids.len(), 2);
        for _ in 0..50 {
            if process_ids
                .iter()
                .all(|process_id| !Path::new(&format!("/proc/{process_id}")).exists())
            {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed-out VS Code process group was not fully reaped");
    }

    #[test]
    fn embedded_identity_and_version_come_from_package_json() {
        assert_eq!(bundled_companion_version().unwrap(), "0.4.2");
        let value: serde_json::Value = serde_json::from_slice(PACKAGE_JSON).unwrap();
        assert_eq!(
            format!(
                "{}.{}",
                value["publisher"].as_str().unwrap(),
                value["name"].as_str().unwrap()
            ),
            EXTENSION_ID
        );
        assert!(validate_vsix_manifest_identity(VSIX_MANIFEST, "0.4.2").is_ok());
        assert!(validate_vsix_manifest_identity(VSIX_MANIFEST, "9.9.9").is_err());
    }

    #[test]
    fn embedded_vsix_contains_only_the_seven_expected_files_with_valid_crc() {
        let entries = read_stored_entries(bundled_vsix_bytes().unwrap());
        let expected: Vec<(&str, &[u8])> = VSIX_ENTRIES.into_iter().collect();
        assert_eq!(entries.len(), expected.len());
        for ((actual_name, actual_contents, actual_crc), (name, contents)) in
            entries.iter().zip(expected)
        {
            assert_eq!(actual_name, name);
            assert_eq!(actual_contents, contents);
            assert_eq!(*actual_crc, crc32(contents));
        }
    }

    #[cfg(unix)]
    #[test]
    fn temporary_vsix_is_owner_only_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TempDir::new().unwrap();
        let package = TemporaryVsix::create(directory.path(), b"private package").unwrap();
        let path = package.path().to_path_buf();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        drop(package);
        assert!(!path.exists());
    }

    #[test]
    fn status_reports_current_version_and_uses_exact_cli_arguments() {
        let runner = ScriptedRunner::new(vec![successful(
            b"unrelated.extension@9.0.0\nvsparallel.vsparallel-companion@0.4.2\n".to_vec(),
        )]);
        let status = companion_status_with(&runner, OsStr::new("/opt/code with spaces"), None);

        assert_eq!(status.state, CompanionStatusState::Current);
        assert_eq!(status.installed_version.as_deref(), Some("0.4.2"));
        assert_eq!(
            runner.calls(),
            vec![RecordedCall {
                executable: OsString::from("/opt/code with spaces"),
                arguments: vec![
                    OsString::from("--list-extensions"),
                    OsString::from("--show-versions")
                ],
            }]
        );
    }

    #[test]
    fn named_profile_is_applied_to_status_install_and_verification() {
        let runner = ScriptedRunner::new(vec![
            successful(b"installed\n".to_vec()),
            successful(b"vsparallel.vsparallel-companion@0.4.2\n".to_vec()),
        ]);
        let temporary_directory = TempDir::new().unwrap();
        let result = install_companion_with(
            &runner,
            OsStr::new("antigravity-ide"),
            temporary_directory.path(),
            Some(OsStr::new("Agents & Tools")),
        )
        .unwrap();

        assert!(result.verified);
        let calls = runner.calls();
        assert_eq!(
            calls[0].arguments[calls[0].arguments.len() - 2],
            "--profile"
        );
        assert_eq!(calls[0].arguments.last().unwrap(), "Agents & Tools");
        assert_eq!(
            calls[1].arguments[calls[1].arguments.len() - 2],
            "--profile"
        );
        assert_eq!(calls[1].arguments.last().unwrap(), "Agents & Tools");
    }

    #[test]
    fn named_profile_is_applied_to_uninstall_and_verification() {
        let runner = ScriptedRunner::new(vec![
            successful(b"vsparallel.vsparallel-companion@0.4.2\n".to_vec()),
            successful(b"uninstalled\n".to_vec()),
            successful(b"other.extension@1.0.0\n".to_vec()),
        ]);

        let result =
            uninstall_companion_with(&runner, OsStr::new("code"), Some(OsStr::new("Work")))
                .unwrap();

        assert!(result.verified);
        for call in runner.calls() {
            assert_eq!(call.arguments[call.arguments.len() - 2], "--profile");
            assert_eq!(call.arguments.last().unwrap(), "Work");
        }
    }

    #[test]
    fn status_distinguishes_different_unknown_and_absent_versions() {
        let different = ScriptedRunner::new(vec![successful(
            b"vsparallel.vsparallel-companion@0.0.9\n".to_vec(),
        )]);
        let status = companion_status_with(&different, OsStr::new("code"), None);
        assert_eq!(status.state, CompanionStatusState::DifferentVersion);
        assert_eq!(status.installed_version.as_deref(), Some("0.0.9"));

        let unknown = ScriptedRunner::new(vec![successful(
            b"vsparallel.vsparallel-companion\n".to_vec(),
        )]);
        assert_eq!(
            companion_status_with(&unknown, OsStr::new("code"), None).state,
            CompanionStatusState::VersionUnknown
        );

        let absent = ScriptedRunner::new(vec![successful(
            b"vsparallel.vsparallel-companion-extra@0.1.0\nother.extension@1.0.0\n".to_vec(),
        )]);
        assert_eq!(
            companion_status_with(&absent, OsStr::new("code"), None).state,
            CompanionStatusState::NotInstalled
        );
    }

    #[test]
    fn status_is_unavailable_for_empty_missing_or_failing_cli() {
        let unused = ScriptedRunner::new(Vec::new());
        let empty = companion_status_with(&unused, OsStr::new(""), None);
        assert_eq!(empty.state, CompanionStatusState::Unavailable);
        assert!(unused.calls().is_empty());

        let missing = ScriptedRunner::new(vec![ScriptedResult::IoError(io::ErrorKind::NotFound)]);
        let status = companion_status_with(&missing, OsStr::new("missing-code"), None);
        assert_eq!(status.state, CompanionStatusState::Unavailable);
        assert!(status.detail.unwrap().contains("injected failure"));

        let failure = ScriptedRunner::new(vec![failed(23, b"profile unavailable".to_vec())]);
        let status = companion_status_with(&failure, OsStr::new("code"), None);
        assert_eq!(status.state, CompanionStatusState::Unavailable);
        let detail = status.detail.unwrap();
        assert!(detail.contains("exit code 23"));
        assert!(detail.contains("profile unavailable"));
    }

    #[test]
    fn editor_specific_views_never_mislabel_antigravity_as_vs_code() {
        let status = CompanionStatus {
            state: CompanionStatusState::Unavailable,
            extension_id: EXTENSION_ID.to_string(),
            bundled_version: Some("0.4.2".to_string()),
            installed_version: None,
            detail: Some("could not run the VS Code command".to_string()),
        };
        let relabeled = relabel_status(status, "Antigravity IDE");
        assert_eq!(
            relabeled.detail.as_deref(),
            Some("could not run the Antigravity IDE command")
        );

        let result = CompanionOperationResult {
            action: CompanionAction::Install,
            verified: false,
            message: "VS Code accepted the install command".to_string(),
            status: relabeled,
        };
        let relabeled = relabel_operation(result, "Antigravity IDE");
        assert_eq!(
            relabeled.message,
            "Antigravity IDE accepted the install command"
        );
    }

    #[test]
    fn install_passes_a_real_embedded_vsix_as_one_argument_and_removes_it() {
        let runner = ScriptedRunner::new(vec![
            successful(b"installed\n".to_vec()),
            successful(b"vsparallel.vsparallel-companion@0.4.2\n".to_vec()),
        ]);
        let root = TempDir::new().unwrap();
        let temporary_directory = root.path().join("temporary packages with spaces");
        fs::create_dir(&temporary_directory).unwrap();

        let result = install_companion_with(
            &runner,
            OsStr::new("/Applications/Visual Studio Code/code"),
            &temporary_directory,
            None,
        )
        .unwrap();

        assert!(result.verified);
        assert_eq!(result.status.state, CompanionStatusState::Current);
        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].arguments[0], "--install-extension");
        assert_eq!(calls[0].arguments[2], "--force");
        let package_path = PathBuf::from(&calls[0].arguments[1]);
        assert!(package_path.starts_with(&temporary_directory));
        assert_eq!(
            package_path.extension().and_then(OsStr::to_str),
            Some("vsix")
        );
        assert!(!package_path.exists(), "temporary VSIX must be removed");
        assert_eq!(
            runner.captured_package.lock().unwrap().as_deref(),
            Some(bundled_vsix_bytes().unwrap())
        );
    }

    #[test]
    fn failed_install_is_reported_and_still_removes_the_temporary_package() {
        let runner = ScriptedRunner::new(vec![failed(9, b"cannot install\0details".to_vec())]);
        let temporary_directory = TempDir::new().unwrap();

        let error = install_companion_with(
            &runner,
            OsStr::new("code"),
            temporary_directory.path(),
            None,
        )
        .unwrap_err();

        assert!(error.contains("exit code 9"));
        assert!(error.contains("cannot install�details"));
        let path = runner
            .captured_package_path
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        assert!(!path.exists(), "temporary VSIX must be removed on error");
    }

    #[test]
    fn install_reports_success_without_overclaiming_when_verification_fails() {
        let runner = ScriptedRunner::new(vec![
            successful(b"installed\n".to_vec()),
            ScriptedResult::IoError(io::ErrorKind::PermissionDenied),
        ]);
        let temporary_directory = TempDir::new().unwrap();

        let result = install_companion_with(
            &runner,
            OsStr::new("code"),
            temporary_directory.path(),
            None,
        )
        .unwrap();

        assert!(!result.verified);
        assert_eq!(result.status.state, CompanionStatusState::Unavailable);
        assert!(result.message.contains("could not be verified"));
    }

    #[test]
    fn uninstall_uses_the_exact_extension_id_and_verifies_removal() {
        let runner = ScriptedRunner::new(vec![
            successful(b"vsparallel.vsparallel-companion@0.4.2\n".to_vec()),
            successful(b"uninstalled\n".to_vec()),
            successful(b"other.extension@1.0.0\n".to_vec()),
        ]);

        let result = uninstall_companion_with(&runner, OsStr::new("code"), None).unwrap();

        assert!(result.verified);
        let calls = runner.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[1].arguments,
            vec![
                OsString::from("--uninstall-extension"),
                OsString::from(EXTENSION_ID)
            ]
        );
    }

    #[test]
    fn uninstall_is_idempotent_when_extension_is_already_absent() {
        let runner = ScriptedRunner::new(vec![successful(b"other.extension@1.0.0\n".to_vec())]);

        let result = uninstall_companion_with(&runner, OsStr::new("code"), None).unwrap();

        assert!(result.verified);
        assert_eq!(result.status.state, CompanionStatusState::NotInstalled);
        assert_eq!(runner.calls().len(), 1);
    }

    #[test]
    fn invalid_temporary_directory_and_empty_executable_do_not_launch() {
        let runner = ScriptedRunner::new(Vec::new());
        let root = TempDir::new().unwrap();
        let missing = root.path().join("missing");
        assert!(install_companion_with(&runner, OsStr::new("code"), &missing, None).is_err());
        assert!(install_companion_with(&runner, OsStr::new(""), root.path(), None).is_err());
        assert!(uninstall_companion_with(&runner, OsStr::new(""), None).is_err());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn explicit_command_availability_distinguishes_missing_optional_editors() {
        let root = TempDir::new().unwrap();
        let missing = root.path().join("missing-editor-cli");
        let available = root.path().join("available-editor-cli");
        fs::write(&available, b"test").unwrap();

        assert!(!command_is_available(OsStr::new("")));
        assert!(!command_is_available(missing.as_os_str()));
        assert!(command_is_available(available.as_os_str()));
    }

    fn read_stored_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>, u32)> {
        let mut offset = 0;
        let mut entries = Vec::new();
        while read_u32(bytes, offset) == Some(0x0403_4b50) {
            assert_eq!(read_u16(bytes, offset + 8), Some(0));
            let crc = read_u32(bytes, offset + 14).unwrap();
            let size = usize::try_from(read_u32(bytes, offset + 18).unwrap()).unwrap();
            let name_len = usize::from(read_u16(bytes, offset + 26).unwrap());
            let extra_len = usize::from(read_u16(bytes, offset + 28).unwrap());
            let name_start = offset + 30;
            let data_start = name_start + name_len + extra_len;
            let name =
                String::from_utf8(bytes[name_start..name_start + name_len].to_vec()).unwrap();
            let contents = bytes[data_start..data_start + size].to_vec();
            entries.push((name, contents, crc));
            offset = data_start + size;
        }
        assert_eq!(read_u32(bytes, offset), Some(0x0201_4b50));
        entries
    }

    fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes(
            bytes.get(offset..offset + 2)?.try_into().ok()?,
        ))
    }

    fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }
}
