mod antigravity_integration;
mod claude_integration;
mod codex_integration;
mod companion_integration;
mod opener;
mod state;
mod tray;
mod usage;

pub use antigravity_integration::{run_antigravity_hook_stdio, AntigravityHookEvent};
pub use claude_integration::run_claude_hook_stdio;
pub use codex_integration::run_codex_hook_stdio;
pub use usage::{run_claude_statusline_stdio, CLAUDE_STATUSLINE_ARGUMENT};

use companion_integration::{CompanionOperationResult, CompanionStatus, CompanionStatusState};
use opener::{
    antigravity_ide_command, code_command, open_editor_with, ProcessWorkspaceLauncher,
    WorkspaceLaunchMode,
};
use serde::Serialize;
use state::{now_ms, Diagnostics, Snapshot, StateStore};
use std::ffi::OsStr;
use std::path::PathBuf;
#[cfg(any(target_os = "macos", test))]
use std::path::{Component, Path};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::window::{Color, Effect, EffectState, EffectsBuilder};
use tauri::{LogicalSize, Manager, PhysicalPosition, PhysicalRect, PhysicalSize};

const INTEGRATION_SCHEMA_VERSION: u32 = 1;
const WINDOW_CHROME_SCHEMA_VERSION: u32 = 1;
const MAIN_WINDOW_MIN_WIDTH: f64 = 520.0;
const MAIN_WINDOW_MIN_HEIGHT: f64 = 360.0;
const FLOATING_PANEL_WIDTH: f64 = 400.0;
const FLOATING_PANEL_HEIGHT: f64 = 440.0;
const FLOATING_PANEL_MIN_WIDTH: f64 = 360.0;
const FLOATING_PANEL_MIN_HEIGHT: f64 = 220.0;
const FLOATING_PANEL_MARGIN: f64 = 16.0;
const FLOATING_PANEL_READY_ATTEMPTS: usize = 30;
const FLOATING_PANEL_READY_INTERVAL: Duration = Duration::from_millis(20);
const FLOATING_PANEL_WATCHDOG_DEADLINES_MS: [u64; 5] = [100, 350, 800, 1_600, 3_000];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WindowChromeState {
    schema_version: u32,
    platform: String,
    custom_controls: bool,
    maximized: bool,
    fullscreen: bool,
    focused: bool,
    floating: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowSizeAction {
    Maximize,
    Restore,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy)]
struct MacosWindowBehavior {
    collection_behavior: objc2_app_kit::NSWindowCollectionBehavior,
    hides_on_deactivate: bool,
}

#[derive(Debug, Clone)]
struct NormalWindowState {
    position: PhysicalPosition<i32>,
    inner_size: PhysicalSize<u32>,
    maximized: bool,
    fullscreen: bool,
    resizable: bool,
    decorated: bool,
    always_on_top: bool,
    #[cfg(target_os = "macos")]
    minimizable: bool,
    #[cfg(target_os = "macos")]
    macos_behavior: MacosWindowBehavior,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowGeometry {
    position: PhysicalPosition<i32>,
    inner_size: PhysicalSize<u32>,
}

#[derive(Debug, Clone, Copy)]
struct FloatingPanelPlacement {
    work_area: PhysicalRect<i32, u32>,
    scale_factor: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum WindowPresentationMode {
    #[default]
    Full,
    EnteringFloating,
    Floating,
    Restoring,
}

#[derive(Debug)]
struct WindowPresentationInner {
    mode: WindowPresentationMode,
    normal: Option<NormalWindowState>,
    last_normal_geometry: Option<WindowGeometry>,
    theme: String,
    generation: u64,
    panel_hidden: bool,
}

impl Default for WindowPresentationInner {
    fn default() -> Self {
        Self {
            mode: WindowPresentationMode::Full,
            normal: None,
            last_normal_geometry: None,
            theme: "dark".to_string(),
            generation: 0,
            panel_hidden: false,
        }
    }
}

#[derive(Debug, Default)]
struct WindowPresentationState {
    inner: Mutex<WindowPresentationInner>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CompanionIntegrationView {
    state: String,
    label: String,
    detail: String,
    installed_version: Option<String>,
    target_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct LifecycleIntegrationView {
    state: String,
    label: String,
    detail: String,
    config_path: Option<String>,
    review_required: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct IntegrationStatusView {
    schema_version: u32,
    companion: CompanionIntegrationView,
    antigravity_ide: CompanionIntegrationView,
    antigravity: LifecycleIntegrationView,
    codex: LifecycleIntegrationView,
    claude: LifecycleIntegrationView,
    requires_restart: bool,
}

#[tauri::command]
async fn get_snapshot() -> Result<Snapshot, String> {
    run_background(current_snapshot).await
}

#[tauri::command]
fn is_release_build() -> bool {
    !cfg!(debug_assertions)
}

#[tauri::command]
async fn get_usage() -> Result<usage::UsageSnapshot, String> {
    run_background(|| Ok(usage::get_usage_snapshot())).await
}

fn current_snapshot() -> Result<Snapshot, String> {
    Ok(StateStore::from_environment()?.snapshot(now_ms()))
}

#[tauri::command]
async fn get_diagnostics() -> Result<Diagnostics, String> {
    run_background(|| {
        let command = code_command();
        let antigravity_command = antigravity_ide_command();
        Ok(StateStore::from_environment()?.diagnostics(now_ms(), command, antigravity_command))
    })
    .await
}

#[tauri::command]
async fn get_integration_status() -> Result<IntegrationStatusView, String> {
    run_background(|| Ok(build_integration_status(false))).await
}

#[tauri::command]
async fn install_companion() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let command = code_command();
        let result = companion_integration::install_companion(OsStr::new(&command))?;
        let status = verified_companion_status(result)?;
        Ok(build_integration_status_with_companion(status, true))
    })
    .await
}

#[tauri::command]
async fn uninstall_companion() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let command = code_command();
        let result = companion_integration::uninstall_companion(OsStr::new(&command))?;
        let status = verified_companion_status(result)?;
        Ok(build_integration_status_with_companion(status, true))
    })
    .await
}

#[tauri::command]
async fn install_antigravity_ide_companion() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let command = antigravity_ide_command();
        let result = companion_integration::install_companion_for_editor(
            OsStr::new(&command),
            "Antigravity IDE",
            companion_integration::ANTIGRAVITY_IDE_PROFILE_ENV,
        )?;
        let status = verified_companion_status(result)?;
        Ok(build_integration_status_with_antigravity_ide_companion(
            status, true,
        ))
    })
    .await
}

#[tauri::command]
async fn uninstall_antigravity_ide_companion() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let command = antigravity_ide_command();
        let result = companion_integration::uninstall_companion_for_editor(
            OsStr::new(&command),
            "Antigravity IDE",
            companion_integration::ANTIGRAVITY_IDE_PROFILE_ENV,
        )?;
        let status = verified_companion_status(result)?;
        Ok(build_integration_status_with_antigravity_ide_companion(
            status, true,
        ))
    })
    .await
}

#[tauri::command]
async fn install_codex_hooks() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let codex_home = codex_integration::codex_home_from_environment()?;
        let executable = integration_executable()?;
        codex_integration::install_codex_integration(&codex_home, &executable)?;
        Ok(build_integration_status(true))
    })
    .await
}

#[tauri::command]
async fn uninstall_codex_hooks() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let codex_home = codex_integration::codex_home_from_environment()?;
        let executable = integration_executable()?;
        codex_integration::uninstall_codex_integration(&codex_home, &executable)?;
        Ok(build_integration_status(true))
    })
    .await
}

#[tauri::command]
async fn install_claude_hooks() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let claude_config_dir = claude_integration::claude_config_dir_from_environment()?;
        let executable = integration_executable()?;
        claude_integration::install_claude_integration(&claude_config_dir, &executable)?;
        Ok(build_integration_status(true))
    })
    .await
}

#[tauri::command]
async fn uninstall_claude_hooks() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let claude_config_dir = claude_integration::claude_config_dir_from_environment()?;
        let executable = integration_executable()?;
        claude_integration::uninstall_claude_integration(&claude_config_dir, &executable)?;
        Ok(build_integration_status(true))
    })
    .await
}

#[tauri::command]
async fn install_antigravity_hooks() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let config_dir = antigravity_integration::antigravity_config_dir_from_environment()?;
        let executable = integration_executable()?;
        let change =
            antigravity_integration::install_antigravity_integration(&config_dir, &executable)?;
        verify_antigravity_change(&change, true)?;
        Ok(build_integration_status(true))
    })
    .await
}

#[tauri::command]
async fn uninstall_antigravity_hooks() -> Result<IntegrationStatusView, String> {
    run_background(|| {
        let config_dir = antigravity_integration::antigravity_config_dir_from_environment()?;
        let executable = integration_executable()?;
        let change =
            antigravity_integration::uninstall_antigravity_integration(&config_dir, &executable)?;
        verify_antigravity_change(&change, false)?;
        Ok(build_integration_status(true))
    })
    .await
}

fn verify_antigravity_change(
    change: &antigravity_integration::AntigravityIntegrationChange,
    expected_installed: bool,
) -> Result<(), String> {
    if change.status.installed == expected_installed
        && (expected_installed || change.status.state == "not_installed")
    {
        Ok(())
    } else {
        Err(change.status.message.clone())
    }
}

async fn run_background<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("the background operation stopped unexpectedly: {error}"))?
}

fn build_integration_status(requires_restart: bool) -> IntegrationStatusView {
    build_integration_status_with_companions(None, None, requires_restart)
}

fn build_integration_status_with_companion(
    companion_status: CompanionStatus,
    requires_restart: bool,
) -> IntegrationStatusView {
    build_integration_status_with_companions(Some(companion_status), None, requires_restart)
}

fn build_integration_status_with_antigravity_ide_companion(
    companion_status: CompanionStatus,
    requires_restart: bool,
) -> IntegrationStatusView {
    build_integration_status_with_companions(None, Some(companion_status), requires_restart)
}

fn build_integration_status_with_companions(
    companion_status: Option<CompanionStatus>,
    antigravity_ide_status: Option<CompanionStatus>,
    requires_restart: bool,
) -> IntegrationStatusView {
    let companion_status = companion_status
        .unwrap_or_else(|| companion_integration::companion_status(OsStr::new(&code_command())));
    let antigravity_ide_status = antigravity_ide_status.unwrap_or_else(|| {
        companion_integration::companion_status_for_editor(
            OsStr::new(&antigravity_ide_command()),
            "Antigravity IDE",
            companion_integration::ANTIGRAVITY_IDE_PROFILE_ENV,
        )
    });
    let companion = companion_view(companion_status);
    let antigravity_ide = companion_view_for(antigravity_ide_status, "Antigravity IDE");
    let antigravity = antigravity_view();
    let codex = codex_view();
    let claude = claude_view();
    IntegrationStatusView {
        schema_version: INTEGRATION_SCHEMA_VERSION,
        companion,
        antigravity_ide,
        antigravity,
        codex,
        claude,
        requires_restart,
    }
}

fn verified_companion_status(result: CompanionOperationResult) -> Result<CompanionStatus, String> {
    if result.verified {
        Ok(result.status)
    } else {
        Err(result.message)
    }
}

fn companion_view(status: CompanionStatus) -> CompanionIntegrationView {
    companion_view_for(status, "VS Code")
}

fn companion_view_for(status: CompanionStatus, editor_name: &str) -> CompanionIntegrationView {
    let (state, label, fallback_detail): (&str, String, String) = match status.state {
        CompanionStatusState::Current => (
            "installed",
            "Installed".to_string(),
            format!("The {editor_name} companion is installed and current."),
        ),
        CompanionStatusState::DifferentVersion => (
            "outdated",
            "Update available".to_string(),
            format!(
                "The installed {editor_name} companion differs from the version bundled with VSParallel."
            ),
        ),
        CompanionStatusState::VersionUnknown => (
            "repair_needed",
            "Repair needed".to_string(),
            format!(
                "{editor_name} reports the companion, but its installed version could not be verified."
            ),
        ),
        CompanionStatusState::NotInstalled => (
            "not_installed",
            "Not installed".to_string(),
            format!("The {editor_name} companion is not installed."),
        ),
        CompanionStatusState::Unavailable => (
            "unavailable",
            format!("{editor_name} unavailable"),
            format!("VSParallel could not query the {editor_name} extension installation."),
        ),
    };
    CompanionIntegrationView {
        state: state.to_string(),
        label,
        detail: status.detail.unwrap_or(fallback_detail),
        installed_version: status.installed_version,
        target_version: status.bundled_version,
    }
}

fn codex_view() -> LifecycleIntegrationView {
    let codex_home = match codex_integration::codex_home_from_environment() {
        Ok(path) => path,
        Err(error) => return unavailable_codex_view(error, None),
    };
    let config_path = Some(codex_home.join("hooks.json").to_string_lossy().into_owned());
    let executable = match integration_executable() {
        Ok(path) => path,
        Err(error) => return unavailable_codex_view(error, config_path),
    };
    match codex_integration::codex_integration_status(&codex_home, &executable) {
        Ok(status) => {
            let (state, label, detail, review_required) = match status.state.as_str() {
                "installed" => {
                    let codex_command = usage::codex_command();
                    match codex_integration::codex_hook_review_status(
                        &codex_home,
                        &executable,
                        codex_command.executable.as_os_str(),
                        codex_command.allow_extension_fallback,
                    ) {
                        Ok(codex_integration::CodexHookReviewStatus::Trusted) => (
                            "installed",
                            "Installed · trusted",
                            "Codex has trusted the installed user-level VSParallel handlers. Workspace settings can still disable hooks."
                                .to_string(),
                            Some(false),
                        ),
                        Ok(codex_integration::CodexHookReviewStatus::ReviewRequired) => (
                            "installed",
                            "Installed · review required",
                            status.message.clone(),
                            Some(true),
                        ),
                        Err(_) => (
                            "installed",
                            "Installed",
                            "Codex activity monitoring is installed. VSParallel could not verify its review status; check /hooks in Codex if needed."
                                .to_string(),
                            None,
                        ),
                    }
                }
                "not_installed" => (
                    "not_installed",
                    "Not installed",
                    status.message.clone(),
                    None,
                ),
                "stale" => (
                    "repair_needed",
                    "Update available",
                    status.message.clone(),
                    None,
                ),
                "partial" => (
                    "repair_needed",
                    "Repair needed",
                    status.message.clone(),
                    None,
                ),
                _ => (
                    "unavailable",
                    "Status unavailable",
                    status.message.clone(),
                    None,
                ),
            };
            LifecycleIntegrationView {
                state: state.to_string(),
                label: label.to_string(),
                detail,
                config_path: Some(status.config_path),
                review_required,
            }
        }
        Err(error) => unavailable_codex_view(
            format!("VSParallel could not safely read the Codex hook configuration: {error}"),
            config_path,
        ),
    }
}

fn unavailable_codex_view(error: String, config_path: Option<String>) -> LifecycleIntegrationView {
    LifecycleIntegrationView {
        state: "unavailable".to_string(),
        label: "Status unavailable".to_string(),
        detail: error,
        config_path,
        review_required: None,
    }
}

fn claude_view() -> LifecycleIntegrationView {
    let claude_config_dir = match claude_integration::claude_config_dir_from_environment() {
        Ok(path) => path,
        Err(error) => return unavailable_claude_view(error, None),
    };
    let config_path = Some(
        claude_config_dir
            .join("settings.json")
            .to_string_lossy()
            .into_owned(),
    );
    let executable = match integration_executable() {
        Ok(path) => path,
        Err(error) => return unavailable_claude_view(error, config_path),
    };
    match claude_integration::claude_integration_status(&claude_config_dir, &executable) {
        Ok(status) => {
            let (state, label) = if status.hooks_disabled && status.installed {
                ("repair_needed", "Installed · hooks disabled")
            } else if status.hooks_disabled && status.state == "not_installed" {
                ("not_installed", "Not installed · hooks disabled")
            } else if status.hooks_disabled {
                ("repair_needed", "Repair needed · hooks disabled")
            } else {
                match status.state.as_str() {
                    "installed" => ("installed", "Installed"),
                    "not_installed" => ("not_installed", "Not installed"),
                    "stale" => ("repair_needed", "Update available"),
                    "partial" => ("repair_needed", "Repair needed"),
                    _ => ("unavailable", "Status unavailable"),
                }
            };
            LifecycleIntegrationView {
                state: state.to_string(),
                label: label.to_string(),
                detail: status.message,
                config_path: Some(status.config_path),
                review_required: None,
            }
        }
        Err(error) => unavailable_claude_view(
            format!("VSParallel could not safely read the Claude Code hook configuration: {error}"),
            config_path,
        ),
    }
}

fn unavailable_claude_view(error: String, config_path: Option<String>) -> LifecycleIntegrationView {
    LifecycleIntegrationView {
        state: "unavailable".to_string(),
        label: "Review required".to_string(),
        detail: error,
        config_path,
        review_required: None,
    }
}

fn antigravity_view() -> LifecycleIntegrationView {
    let config_dir = match antigravity_integration::antigravity_config_dir_from_environment() {
        Ok(path) => path,
        Err(error) => return unavailable_antigravity_view(error, None),
    };
    let config_path = Some(config_dir.join("hooks.json").to_string_lossy().into_owned());
    let executable = match integration_executable() {
        Ok(path) => path,
        Err(error) => return unavailable_antigravity_view(error, config_path),
    };
    match antigravity_integration::antigravity_integration_status(&config_dir, &executable) {
        Ok(status) => {
            let (state, label, detail) = if status.hooks_disabled && status.installed {
                (
                    "repair_needed",
                    "Installed · hooks disabled".to_string(),
                    status.message,
                )
            } else if status.hooks_disabled && status.state == "not_installed" {
                (
                    "not_installed",
                    "Not installed · hooks disabled".to_string(),
                    status.message,
                )
            } else if status.hooks_disabled {
                (
                    "repair_needed",
                    "Repair needed · hooks disabled".to_string(),
                    status.message,
                )
            } else {
                match status.state.as_str() {
                    "installed" => {
                        let (label, detail) = antigravity_installed_copy();
                        ("installed", label, detail)
                    }
                    "not_installed" => {
                        ("not_installed", "Not installed".to_string(), status.message)
                    }
                    "stale" => (
                        "repair_needed",
                        "Update available".to_string(),
                        status.message,
                    ),
                    "partial" => ("repair_needed", "Repair needed".to_string(), status.message),
                    _ => (
                        "unavailable",
                        "Status unavailable".to_string(),
                        status.message,
                    ),
                }
            };
            LifecycleIntegrationView {
                state: state.to_string(),
                label,
                detail,
                config_path: Some(status.config_path),
                review_required: None,
            }
        }
        Err(error) => unavailable_antigravity_view(
            format!("VSParallel could not safely read the Antigravity hook configuration: {error}"),
            config_path,
        ),
    }
}

fn antigravity_installed_copy() -> (String, String) {
    const FIRST_TURN_GUIDANCE: &str = "Opening a Project or workspace alone does not run lifecycle hooks; start a new Antigravity 2.0 or Antigravity IDE agent turn. A workspace-level .agents/hooks.json can override this global hook.";
    let root = match state::state_dir_from_environment() {
        Ok(root) => root,
        Err(_) => {
            return (
                "Installed · observation unavailable".to_string(),
                format!(
                    "Antigravity activity monitoring is configured, but its local execution-health record could not be read. {FIRST_TURN_GUIDANCE}"
                ),
            )
        }
    };
    antigravity_installed_copy_from_root(&root, now_ms(), FIRST_TURN_GUIDANCE)
}

fn antigravity_installed_copy_from_root(
    root: &std::path::Path,
    now: i64,
    first_turn_guidance: &str,
) -> (String, String) {
    let observations = [
        (
            "Antigravity 2.0",
            antigravity_integration::antigravity_two_hook_observation(root, now),
        ),
        (
            "Antigravity IDE",
            antigravity_integration::antigravity_ide_hook_observation(root, now),
        ),
    ];
    let mut latest: Option<(&str, antigravity_integration::AntigravityHookObservation)> = None;
    let mut read_failed = false;
    for (surface, result) in observations {
        match result {
            Ok(Some(observation))
                if latest.as_ref().is_none_or(|(_, current)| {
                    observation.observed_at_ms > current.observed_at_ms
                }) =>
            {
                latest = Some((surface, observation));
            }
            Ok(_) => {}
            Err(_) => read_failed = true,
        }
    }

    if read_failed {
        return (
            "Installed · observation unavailable".to_string(),
            format!(
                "Antigravity activity monitoring is configured, but at least one local surface execution-health record could not be read. {first_turn_guidance}"
            ),
        );
    }

    match latest {
        Some((surface, observation))
            if observation.outcome
                == antigravity_integration::AntigravityHookOutcome::Recorded =>
        {
            (
                "Installed · event observed".to_string(),
                format!(
                    "{surface} activity monitoring is installed; the latest {} event recorded {} workspace path{}. Hook rows show recent agent activity, not a live window.",
                    observation.event,
                    observation.workspace_count,
                    if observation.workspace_count == 1 { "" } else { "s" },
                ),
            )
        }
        Some((surface, observation)) => (
            "Installed · hook issue".to_string(),
            format!(
                "{surface} ran the hook, but {}; {}",
                observation.outcome.user_message(),
                first_turn_guidance
            ),
        ),
        None => (
            "Installed · awaiting agent turn".to_string(),
            format!("Antigravity activity monitoring is configured. {first_turn_guidance}"),
        ),
    }
}

fn unavailable_antigravity_view(
    error: String,
    config_path: Option<String>,
) -> LifecycleIntegrationView {
    LifecycleIntegrationView {
        state: "unavailable".to_string(),
        label: "Status unavailable".to_string(),
        detail: error,
        config_path,
        review_required: None,
    }
}

fn integration_executable() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(app_image) = std::env::var_os("APPIMAGE").filter(|value| !value.is_empty()) {
        return validate_integration_executable(PathBuf::from(app_image), "APPIMAGE");
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate the VSParallel executable: {error}"))?;
    #[cfg(target_os = "macos")]
    validate_macos_integration_location(&executable)?;
    validate_integration_executable(executable, "current executable")
}

#[cfg(any(target_os = "macos", test))]
fn validate_macos_integration_location(path: &Path) -> Result<(), String> {
    let mounted_volume = path.starts_with(Path::new("/Volumes"));
    let app_translocation = path.components().any(
        |component| matches!(component, Component::Normal(value) if value == "AppTranslocation"),
    );
    if mounted_volume || app_translocation {
        return Err(
            "VSParallel is running from a temporary macOS location. Copy VSParallel.app to /Applications, relaunch it there, then use Repair for the Antigravity, Codex, and Claude Code lifecycle integrations."
                .to_string(),
        );
    }
    Ok(())
}

fn validate_integration_executable(path: PathBuf, source: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "the {source} path must be absolute: {}",
            path.display()
        ));
    }
    if !path.is_file() {
        return Err(format!(
            "the {source} path is not an available file: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn window_presentation(
    state: &WindowPresentationState,
) -> Result<MutexGuard<'_, WindowPresentationInner>, String> {
    state
        .inner
        .lock()
        .map_err(|_| "the VSParallel window presentation state is unavailable".to_string())
}

fn is_floating_panel(state: &WindowPresentationState) -> Result<bool, String> {
    Ok(window_presentation(state)?.mode == WindowPresentationMode::Floating)
}

fn is_compact_window(state: &WindowPresentationState) -> Result<bool, String> {
    Ok(matches!(
        window_presentation(state)?.mode,
        WindowPresentationMode::EnteringFloating | WindowPresentationMode::Floating
    ))
}

fn is_full_window(state: &WindowPresentationState) -> Result<bool, String> {
    Ok(window_presentation(state)?.mode == WindowPresentationMode::Full)
}

fn advance_window_generation(presentation: &mut WindowPresentationInner) -> u64 {
    presentation.generation = presentation.generation.wrapping_add(1);
    presentation.generation
}

fn floating_panel_is_expected(
    state: &WindowPresentationState,
    generation: u64,
) -> Result<bool, String> {
    let presentation = window_presentation(state)?;
    Ok(presentation.mode == WindowPresentationMode::Floating
        && presentation.generation == generation
        && !presentation.panel_hidden)
}

fn current_window_theme(state: &WindowPresentationState) -> Result<String, String> {
    Ok(window_presentation(state)?.theme.clone())
}

fn window_geometry(window: &tauri::WebviewWindow) -> Result<WindowGeometry, String> {
    Ok(WindowGeometry {
        position: window
            .outer_position()
            .map_err(|error| format!("could not read the VSParallel window position: {error}"))?,
        inner_size: window
            .inner_size()
            .map_err(|error| format!("could not read the VSParallel window size: {error}"))?,
    })
}

fn refresh_full_window_geometry(
    presentation: &mut WindowPresentationInner,
    geometry: WindowGeometry,
    scale_factor: f64,
) -> bool {
    if presentation.mode != WindowPresentationMode::Full {
        return false;
    }
    let minimum: PhysicalSize<u32> =
        LogicalSize::new(MAIN_WINDOW_MIN_WIDTH, MAIN_WINDOW_MIN_HEIGHT).to_physical(scale_factor);
    const NATIVE_ROUNDING_TOLERANCE: u32 = 4;
    if geometry
        .inner_size
        .width
        .saturating_add(NATIVE_ROUNDING_TOLERANCE)
        < minimum.width
        || geometry
            .inner_size
            .height
            .saturating_add(NATIVE_ROUNDING_TOLERANCE)
            < minimum.height
    {
        return false;
    }

    presentation.last_normal_geometry = Some(geometry);
    if let Some(normal) = presentation.normal.as_mut() {
        // A snapshot retained after a partial restore still owns the non-geometry properties
        // that may need another repair pass. Only merge verified full-window geometry here.
        normal.position = geometry.position;
        normal.inner_size = geometry.inner_size;
    }
    true
}

fn remember_normal_window_geometry(
    window: &tauri::WebviewWindow,
    state: &WindowPresentationState,
) -> Result<(), String> {
    if !is_full_window(state)?
        || window
            .is_maximized()
            .map_err(|error| format!("could not read the VSParallel maximized state: {error}"))?
        || window
            .is_fullscreen()
            .map_err(|error| format!("could not read the VSParallel full-screen state: {error}"))?
    {
        return Ok(());
    }
    let scale_factor = window
        .scale_factor()
        .map_err(|error| format!("could not read the VSParallel display scale: {error}"))?;
    let geometry = window_geometry(window)?;
    // Native move/resize events can arrive after a panel transition has started. Recheck the
    // mode under the same final lock that updates the snapshot so a late compact event cannot
    // overwrite the full-window recovery geometry.
    let mut presentation = window_presentation(state)?;
    let _ = refresh_full_window_geometry(&mut presentation, geometry, scale_factor);
    Ok(())
}

fn capture_normal_window(
    window: &tauri::WebviewWindow,
    state: &WindowPresentationState,
) -> Result<NormalWindowState, String> {
    let maximized = window
        .is_maximized()
        .map_err(|error| format!("could not read the VSParallel maximized state: {error}"))?;
    let fullscreen = window
        .is_fullscreen()
        .map_err(|error| format!("could not read the VSParallel full-screen state: {error}"))?;
    let current_geometry = window_geometry(window)?;
    let geometry = if maximized || fullscreen {
        window_presentation(state)?
            .last_normal_geometry
            .unwrap_or(current_geometry)
    } else {
        current_geometry
    };
    #[cfg(target_os = "macos")]
    let macos_behavior = capture_macos_window_behavior(window)?;
    Ok(NormalWindowState {
        position: geometry.position,
        inner_size: geometry.inner_size,
        maximized,
        fullscreen,
        resizable: window
            .is_resizable()
            .map_err(|error| format!("could not read the VSParallel resize state: {error}"))?,
        decorated: window
            .is_decorated()
            .map_err(|error| format!("could not read the VSParallel decorations: {error}"))?,
        always_on_top: window
            .is_always_on_top()
            .map_err(|error| format!("could not read the VSParallel stacking state: {error}"))?,
        #[cfg(target_os = "macos")]
        minimizable: window
            .is_minimizable()
            .map_err(|error| format!("could not read the VSParallel minimize state: {error}"))?,
        #[cfg(target_os = "macos")]
        macos_behavior,
    })
}

fn clamp_i64_to_i32(value: i64) -> i32 {
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn floating_panel_position(
    work_area: PhysicalRect<i32, u32>,
    panel_size: PhysicalSize<u32>,
    margin: u32,
) -> PhysicalPosition<i32> {
    let left = i64::from(work_area.position.x);
    let top = i64::from(work_area.position.y);
    let width = i64::from(work_area.size.width);
    let height = i64::from(work_area.size.height);
    let panel_width = i64::from(panel_size.width);
    let panel_height = i64::from(panel_size.height);
    let margin = i64::from(margin);

    let x = if panel_width.saturating_add(margin.saturating_mul(2)) <= width {
        left.saturating_add(width)
            .saturating_sub(panel_width)
            .saturating_sub(margin)
    } else {
        left
    };
    let y = if panel_height.saturating_add(margin.saturating_mul(2)) <= height {
        top.saturating_add(margin)
    } else {
        top
    };

    PhysicalPosition::new(clamp_i64_to_i32(x), clamp_i64_to_i32(y))
}

fn monitor_placement(monitor: &tauri::Monitor) -> FloatingPanelPlacement {
    FloatingPanelPlacement {
        work_area: *monitor.work_area(),
        scale_factor: monitor.scale_factor(),
    }
}

fn capture_floating_panel_placement(
    window: &tauri::WebviewWindow,
) -> Result<Option<FloatingPanelPlacement>, String> {
    use std::sync::mpsc;

    let (sender, receiver) = mpsc::sync_channel(1);
    let main_thread_window = window.clone();
    window
        .run_on_main_thread(move || {
            // On GTK, Tauri's monitor wrapper contains a GDK object. Although the wrapper is
            // marked Send, converting it into `tauri::Monitor` reads GDK properties on the
            // caller's thread. Keep the lookup and conversion on GTK's main thread and send only
            // plain geometry back to the async command worker.
            let placement = main_thread_window
                .current_monitor()
                .map_err(|error| format!("could not identify the VSParallel display: {error}"))
                .and_then(|current| match current {
                    Some(monitor) => Ok(Some(monitor_placement(&monitor))),
                    None => main_thread_window
                        .primary_monitor()
                        .map(|primary| primary.as_ref().map(monitor_placement))
                        .map_err(|error| {
                            format!("could not identify the primary display: {error}")
                        }),
                });
            let _ = sender.send(placement);
        })
        .map_err(|error| format!("could not schedule the VSParallel display lookup: {error}"))?;
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| {
            format!("VSParallel did not report its display geometry in time: {error}")
        })?
}

fn floating_window_effects(
    theme: &str,
) -> Result<tauri::utils::config::WindowEffectsConfig, String> {
    let color = match theme {
        "dark" => Color(18, 18, 22, 190),
        "light" => Color(244, 244, 248, 200),
        _ => return Err("window theme must be either dark or light".to_string()),
    };
    Ok(EffectsBuilder::new()
        .effects([Effect::Popover, Effect::Acrylic])
        .state(EffectState::Active)
        .radius(14.0)
        .color(color)
        .build())
}

fn native_window_theme(theme: &str) -> Result<tauri::Theme, String> {
    match theme {
        "dark" => Ok(tauri::Theme::Dark),
        "light" => Ok(tauri::Theme::Light),
        _ => Err("window theme must be either dark or light".to_string()),
    }
}

fn apply_floating_window_effect(window: &tauri::WebviewWindow, theme: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // macOS installs one persistent vibrancy view during setup. Reapplying the effect would
        // stack NSVisualEffectViews because Tauri 2.11 cannot clear them.
        let _ = (window, theme);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        window
            .set_effects(floating_window_effects(theme)?)
            .map_err(|error| format!("could not apply the floating panel glass effect: {error}"))
    }
}

fn clear_floating_window_effect(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "windows")]
    let _ = window.set_effects(None);
    #[cfg(not(target_os = "windows"))]
    let _ = window;
}

#[cfg(target_os = "macos")]
fn capture_macos_window_behavior(
    window: &tauri::WebviewWindow,
) -> Result<MacosWindowBehavior, String> {
    use objc2_app_kit::NSWindow;
    use std::sync::mpsc;

    let native_window = window
        .ns_window()
        .map_err(|error| format!("could not access the native VSParallel window: {error}"))?
        as usize;
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            // SAFETY: Tauri owns this NSWindow for the lifetime of the WebviewWindow. The raw
            // pointer is only dereferenced on AppKit's main thread and is not retained here.
            let behavior = unsafe {
                let native_window = &*(native_window as *const NSWindow);
                MacosWindowBehavior {
                    collection_behavior: native_window.collectionBehavior(),
                    hides_on_deactivate: native_window.hidesOnDeactivate(),
                }
            };
            let _ = sender.send(behavior);
        })
        .map_err(|error| {
            format!("could not schedule reading the macOS window behavior: {error}")
        })?;
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| format!("macOS did not report its original window behavior: {error}"))
}

#[cfg(target_os = "macos")]
fn set_macos_floating_behavior(window: &tauri::WebviewWindow) -> Result<(), String> {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    let native_window = window
        .ns_window()
        .map_err(|error| format!("could not access the native VSParallel window: {error}"))?
        as usize;
    window
        .run_on_main_thread(move || {
            // SAFETY: Tauri owns this NSWindow for the lifetime of the WebviewWindow. The raw
            // pointer is only dereferenced on AppKit's main thread and is not retained here.
            unsafe {
                let native_window = &*(native_window as *const NSWindow);
                let mut behavior = native_window.collectionBehavior();
                // These option groups are mutually exclusive. Preserve unrelated native flags
                // and restore this exact original mask when the panel returns to full mode.
                behavior.remove(
                    NSWindowCollectionBehavior::MoveToActiveSpace
                        | NSWindowCollectionBehavior::Primary
                        | NSWindowCollectionBehavior::Auxiliary
                        | NSWindowCollectionBehavior::FullScreenPrimary
                        | NSWindowCollectionBehavior::FullScreenNone,
                );
                behavior.insert(
                    NSWindowCollectionBehavior::CanJoinAllSpaces
                        | NSWindowCollectionBehavior::FullScreenAuxiliary
                        | NSWindowCollectionBehavior::CanJoinAllApplications,
                );
                native_window.setCollectionBehavior(behavior);
                native_window.setHidesOnDeactivate(false);
                // Unlike `show`, this orders the panel above other apps without making it key
                // and taking keyboard focus back from the selected editor.
                native_window.orderFrontRegardless();
            }
        })
        .map_err(|error| format!("could not schedule the macOS panel behavior: {error}"))
}

#[cfg(not(target_os = "macos"))]
fn set_macos_floating_behavior(_window: &tauri::WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn restore_macos_window_behavior(
    window: &tauri::WebviewWindow,
    normal: &NormalWindowState,
) -> Result<(), String> {
    use objc2_app_kit::NSWindow;
    use std::sync::mpsc;

    let native_window = window
        .ns_window()
        .map_err(|error| format!("could not access the native VSParallel window: {error}"))?
        as usize;
    let original = normal.macos_behavior;
    let (sender, receiver) = mpsc::sync_channel(1);
    window
        .run_on_main_thread(move || {
            // SAFETY: Tauri owns this NSWindow for the lifetime of the WebviewWindow. The raw
            // pointer is only dereferenced on AppKit's main thread and is not retained here.
            unsafe {
                let native_window = &*(native_window as *const NSWindow);
                native_window.setCollectionBehavior(original.collection_behavior);
                native_window.setHidesOnDeactivate(original.hides_on_deactivate);
            }
            let _ = sender.send(());
        })
        .map_err(|error| {
            format!("could not schedule restoring the macOS window behavior: {error}")
        })?;
    receiver
        .recv_timeout(Duration::from_secs(1))
        .map_err(|error| format!("macOS did not restore its original window behavior: {error}"))
}

#[cfg(not(target_os = "macos"))]
fn restore_macos_window_behavior(
    _window: &tauri::WebviewWindow,
    _normal: &NormalWindowState,
) -> Result<(), String> {
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn normalized_windows_executable_name(value: &str) -> String {
    let file_name = value
        .trim()
        .trim_matches('"')
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(value);
    let stem = match file_name.rsplit_once('.') {
        Some((stem, extension))
            if ["exe", "cmd", "bat"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate)) =>
        {
            stem
        }
        _ => file_name,
    };
    stem.chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(any(target_os = "windows", test))]
fn windows_editor_process_matches(actual: &str, expected: &str) -> bool {
    let actual = normalized_windows_executable_name(actual);
    let expected = normalized_windows_executable_name(expected);
    if actual.is_empty() || expected.is_empty() {
        return false;
    }

    actual == expected
        || matches!(
            (actual.as_str(), expected.as_str()),
            ("vscodium", "codium") | ("codium", "vscodium")
        )
}

#[cfg(target_os = "windows")]
fn foreground_window_matches_editor(
    foreground: windows::Win32::Foundation::HWND,
    editor_executable: &str,
) -> bool {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let mut process_id = 0;
    // SAFETY: foreground is a borrowed live top-level HWND and process_id is valid output storage.
    unsafe { GetWindowThreadProcessId(foreground, Some(&mut process_id)) };
    if process_id == 0 {
        return false;
    }

    // SAFETY: the handle requests read-only process identity access and is closed below.
    let Ok(process) =
        (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) })
    else {
        return false;
    };
    let mut path = vec![0_u16; 32_768];
    let mut path_length = path.len() as u32;
    // SAFETY: path provides path_length writable UTF-16 elements and process is a live handle.
    let query_result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut path_length,
        )
    };
    // SAFETY: process was returned by OpenProcess in this function and is no longer used.
    let _ = unsafe { CloseHandle(process) };
    if query_result.is_err() {
        return false;
    }

    windows_editor_process_matches(
        &String::from_utf16_lossy(&path[..path_length as usize]),
        editor_executable,
    )
}

#[cfg(target_os = "windows")]
fn move_panel_to_foreground_desktop(
    window: &tauri::WebviewWindow,
    editor_executable: &str,
) -> Result<bool, String> {
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let panel = window
        .hwnd()
        .map_err(|error| format!("could not access the native VSParallel window: {error}"))?;
    // SAFETY: GetForegroundWindow has no preconditions and returns a borrowed HWND.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.0.is_null()
        || foreground == panel
        || !foreground_window_matches_editor(foreground, editor_executable)
    {
        return Ok(false);
    }

    // The watchdog runs on a blocking-runtime worker. Initialize COM for that worker when
    // needed; RPC_E_CHANGED_MODE means the thread already has a usable apartment.
    // SAFETY: COM initialization and uninitialization are balanced on this same thread.
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() && initialized != RPC_E_CHANGED_MODE {
        return Err(format!(
            "could not initialize Windows desktop services: {initialized}"
        ));
    }
    let owns_com_initialization = initialized.is_ok();

    let result = (|| {
        // SAFETY: both HWND values are live top-level windows. The documented public virtual
        // desktop manager moves only VSParallel and does not activate either application.
        unsafe {
            let manager: IVirtualDesktopManager =
                CoCreateInstance(&VirtualDesktopManager, None, CLSCTX_ALL).map_err(|error| {
                    format!("could not open Windows virtual desktop services: {error}")
                })?;
            let desktop = manager.GetWindowDesktopId(foreground).map_err(|error| {
                format!("could not identify the foreground Windows desktop: {error}")
            })?;
            manager
                .MoveWindowToDesktop(panel, &desktop)
                .map_err(|error| {
                    format!("could not move VSParallel to the active desktop: {error}")
                })
        }
    })();

    if owns_com_initialization {
        // SAFETY: this balances the successful CoInitializeEx call above on the same thread.
        unsafe { CoUninitialize() };
    }
    result.map(|()| true)
}

#[cfg(not(target_os = "windows"))]
fn move_panel_to_foreground_desktop(
    _window: &tauri::WebviewWindow,
    _editor_executable: &str,
) -> Result<bool, String> {
    Ok(true)
}

#[cfg(target_os = "windows")]
fn show_floating_panel_without_focus(window: &tauri::WebviewWindow) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    let panel = window
        .hwnd()
        .map_err(|error| format!("could not access the native VSParallel window: {error}"))?;
    // SAFETY: SetWindowPos receives VSParallel's live HWND and changes only visibility/z-order.
    unsafe {
        SetWindowPos(
            panel,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    }
    .map_err(|error| format!("could not show the VSParallel panel without focus: {error}"))
}

#[cfg(target_os = "macos")]
fn show_floating_panel_without_focus(window: &tauri::WebviewWindow) -> Result<(), String> {
    set_macos_floating_behavior(window)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn show_floating_panel_without_focus(window: &tauri::WebviewWindow) -> Result<(), String> {
    window
        .show()
        .map_err(|error| format!("could not show the VSParallel panel: {error}"))
}

async fn wait_for_restored_window_state(window: &tauri::WebviewWindow) -> Result<(), String> {
    const POLL_ATTEMPTS: usize = 60;
    const POLL_INTERVAL: Duration = Duration::from_millis(50);
    let window = window.clone();
    run_background(move || {
        for _ in 0..POLL_ATTEMPTS {
            let fullscreen = window.is_fullscreen().map_err(|error| {
                format!("could not read the VSParallel full-screen state: {error}")
            })?;
            let maximized = window.is_maximized().map_err(|error| {
                format!("could not read the VSParallel maximized state: {error}")
            })?;
            if !fullscreen && !maximized {
                return Ok(());
            }
            if maximized {
                window.unmaximize().map_err(|error| {
                    format!("could not restore the window for the panel: {error}")
                })?;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        Err("VSParallel timed out while leaving full screen for the floating panel".to_string())
    })
    .await
}

fn floating_panel_size_is_ready(size: PhysicalSize<u32>, scale_factor: f64) -> bool {
    let expected: PhysicalSize<u32> =
        LogicalSize::new(FLOATING_PANEL_WIDTH, FLOATING_PANEL_HEIGHT).to_physical(scale_factor);
    size.width.abs_diff(expected.width) <= 4 && size.height.abs_diff(expected.height) <= 4
}

async fn wait_for_floating_panel_ready(window: &tauri::WebviewWindow) -> Result<(), String> {
    let scale_factor = window
        .scale_factor()
        .map_err(|error| format!("could not read the VSParallel display scale: {error}"))?;
    let window = window.clone();
    run_background(move || {
        for _ in 0..FLOATING_PANEL_READY_ATTEMPTS {
            let visible = window
                .is_visible()
                .map_err(|error| format!("could not read the VSParallel visibility: {error}"))?;
            let minimized = window.is_minimized().map_err(|error| {
                format!("could not read the VSParallel minimized state: {error}")
            })?;
            let always_on_top = window.is_always_on_top().map_err(|error| {
                format!("could not read the VSParallel stacking state: {error}")
            })?;
            let size = window
                .inner_size()
                .map_err(|error| format!("could not read the VSParallel panel size: {error}"))?;
            #[cfg(target_os = "macos")]
            let minimizable = window.is_minimizable().map_err(|error| {
                format!("could not read the VSParallel panel minimize state: {error}")
            })?;
            #[cfg(not(target_os = "macos"))]
            let minimizable = true;

            if visible
                && !minimized
                && always_on_top
                && minimizable
                && floating_panel_size_is_ready(size, scale_factor)
            {
                // Keep a small compositor turn between the sticky-window request and the editor's
                // focus request. This closes the cross-desktop race on X11 and macOS Spaces.
                std::thread::sleep(Duration::from_millis(40));
                return Ok(());
            }

            if minimized {
                window.unminimize().map_err(|error| {
                    format!("could not unminimize the VSParallel panel: {error}")
                })?;
            }
            if !visible {
                show_floating_panel_without_focus(&window)?;
            }
            if !always_on_top {
                window.set_always_on_top(true).map_err(|error| {
                    format!("could not keep the VSParallel panel above the editor: {error}")
                })?;
            }
            #[cfg(target_os = "macos")]
            if !minimizable {
                window.set_minimizable(true).map_err(|error| {
                    format!("could not make the VSParallel panel minimizable: {error}")
                })?;
            }
            std::thread::sleep(FLOATING_PANEL_READY_INTERVAL);
        }

        let visible = window
            .is_visible()
            .map_err(|error| format!("could not read the VSParallel visibility: {error}"))?;
        let minimized = window
            .is_minimized()
            .map_err(|error| format!("could not read the VSParallel minimized state: {error}"))?;
        #[cfg(target_os = "macos")]
        let minimizable = window.is_minimizable().map_err(|error| {
            format!("could not read the VSParallel panel minimize state: {error}")
        })?;
        #[cfg(not(target_os = "macos"))]
        let minimizable = true;
        if !visible || minimized || !minimizable {
            return Err(
                "VSParallel could not make the floating panel visible and minimizable before opening the editor"
                    .to_string(),
            );
        }

        // Some compositors do not report or honor topmost/size hints. Visibility, plus native
        // minimization on macOS, is the hard requirement; the watchdog continues best-effort
        // stacking repair afterward.
        Ok(())
    })
    .await
}

fn apply_floating_panel_presentation(
    window: &tauri::WebviewWindow,
    state: &WindowPresentationState,
    placement: Option<&FloatingPanelPlacement>,
) -> Result<(), String> {
    // Decorations and placement are compositor capabilities rather than requirements. The
    // compact CSS surface remains usable if either hint is ignored (notably on Wayland).
    let _ = window.set_decorations(false);
    window
        .set_min_size(Some(LogicalSize::new(
            FLOATING_PANEL_MIN_WIDTH,
            FLOATING_PANEL_MIN_HEIGHT,
        )))
        .map_err(|error| format!("could not relax the panel size limit: {error}"))?;
    window
        .set_resizable(false)
        .map_err(|error| format!("could not fix the floating panel size: {error}"))?;
    #[cfg(target_os = "macos")]
    // Tao's runtime borderless mask drops AppKit's Miniaturizable bit. The readiness loop also
    // verifies this after the asynchronous decoration update has settled.
    window
        .set_minimizable(true)
        .map_err(|error| format!("could not make the floating panel minimizable: {error}"))?;
    let panel_size = LogicalSize::new(FLOATING_PANEL_WIDTH, FLOATING_PANEL_HEIGHT);
    window
        .set_size(panel_size)
        .map_err(|error| format!("could not resize the floating panel: {error}"))?;

    if let Some(placement) = placement {
        let scale = placement.scale_factor;
        let physical_size: PhysicalSize<u32> = panel_size.to_physical(scale);
        let physical_margin = (FLOATING_PANEL_MARGIN * scale).round().max(0.0) as u32;
        let position = floating_panel_position(placement.work_area, physical_size, physical_margin);
        let _ = window.set_position(position);
    }

    let theme = current_window_theme(state)?;
    window
        .set_theme(Some(native_window_theme(&theme)?))
        .map_err(|error| format!("could not apply the floating panel theme: {error}"))?;
    window
        .set_background_color(Some(window_background(&theme, true)?))
        .map_err(|error| format!("could not make the floating panel transparent: {error}"))?;
    apply_floating_window_effect(window, &theme)?;
    window
        .set_always_on_top(true)
        .map_err(|error| format!("could not keep the floating panel above the editor: {error}"))?;
    window
        .set_visible_on_all_workspaces(true)
        .map_err(|error| {
            format!("could not keep the floating panel on the active desktop: {error}")
        })?;
    set_macos_floating_behavior(window)?;
    window
        .unminimize()
        .map_err(|error| format!("could not unminimize the floating panel: {error}"))?;
    show_floating_panel_without_focus(window)
}

async fn enter_floating_panel(
    window: &tauri::WebviewWindow,
    state: &WindowPresentationState,
) -> Result<u64, String> {
    let current_mode = window_presentation(state)?.mode;
    let (normal, placement, generation) = match current_mode {
        WindowPresentationMode::Full => {
            // A previous best-effort restore may have left its recovery snapshot in place. Reuse
            // it instead of ever treating compact geometry as the user's normal window state.
            let saved_normal = window_presentation(state)?.normal.clone();
            let normal = match saved_normal {
                Some(normal) => normal,
                None => capture_normal_window(window, state)?,
            };
            let placement = capture_floating_panel_placement(window)?;
            let generation = {
                let mut presentation = window_presentation(state)?;
                if presentation.mode != WindowPresentationMode::Full {
                    return Err("the VSParallel panel transition was superseded".to_string());
                }
                presentation.normal = Some(normal.clone());
                presentation.mode = WindowPresentationMode::EnteringFloating;
                presentation.panel_hidden = false;
                advance_window_generation(&mut presentation)
            };
            (normal, placement, generation)
        }
        WindowPresentationMode::Floating => {
            let (normal, generation) = {
                let mut presentation = window_presentation(state)?;
                let normal = presentation.normal.clone().ok_or_else(|| {
                    "the saved VSParallel window state is unavailable".to_string()
                })?;
                presentation.panel_hidden = false;
                let generation = advance_window_generation(&mut presentation);
                (normal, generation)
            };
            // A repeated workspace switch is a repair pass. Preserve the user's panel position
            // while reapplying visibility, size, stickiness, and topmost state.
            (normal, None, generation)
        }
        WindowPresentationMode::EnteringFloating => {
            return Err("VSParallel is already preparing the floating panel".to_string());
        }
        WindowPresentationMode::Restoring => {
            return Err("VSParallel is restoring its full window".to_string());
        }
    };

    let transition = async {
        if current_mode == WindowPresentationMode::Full {
            if normal.fullscreen {
                window.set_fullscreen(false).map_err(|error| {
                    format!("could not leave full screen for the panel: {error}")
                })?;
            }
            if normal.maximized {
                window.unmaximize().map_err(|error| {
                    format!("could not restore the window for the panel: {error}")
                })?;
            }
            if normal.fullscreen || normal.maximized {
                wait_for_restored_window_state(window).await?;
            }
        }

        apply_floating_panel_presentation(window, state, placement.as_ref())?;
        wait_for_floating_panel_ready(window).await
    }
    .await;

    if let Err(error) = transition {
        let restore_error = restore_full_window_state(window, state, false).err();
        return Err(match restore_error {
            Some(restore_error) => format!(
                "{error}; VSParallel also had trouble restoring its full window: {restore_error}"
            ),
            None => error,
        });
    }

    let completion = {
        let mut presentation = window_presentation(state)?;
        if presentation.generation == generation && !presentation.panel_hidden {
            presentation.mode = WindowPresentationMode::Floating;
            Some((generation, false))
        } else if presentation.panel_hidden
            && matches!(
                presentation.mode,
                WindowPresentationMode::EnteringFloating | WindowPresentationMode::Floating
            )
        {
            // Hide may be invoked while the native transition is still settling. Complete the
            // transition, then honor that newer user intent instead of leaving an Entering state.
            presentation.mode = WindowPresentationMode::Floating;
            Some((presentation.generation, true))
        } else {
            None
        }
    };
    let Some((effective_generation, remain_hidden)) = completion else {
        // Restore or a second activation won while native setters were settling. Reapply the
        // latest full-window intent after those setters so compact transparency/topmost state
        // cannot be left behind with a Full presentation flag.
        let restore_error = restore_full_window_state(window, state, false).err();
        return Err(match restore_error {
            Some(error) => format!(
                "the VSParallel panel transition was superseded, and its full window could not be fully restored: {error}"
            ),
            None => "the VSParallel panel transition was superseded".to_string(),
        });
    };
    if remain_hidden {
        window
            .minimize()
            .map_err(|error| format!("could not finish hiding the floating panel: {error}"))?;
    }
    Ok(effective_generation)
}

fn record_first_window_error<T, E: std::fmt::Display>(
    first_error: &mut Option<String>,
    context: &str,
    result: Result<T, E>,
) {
    if first_error.is_none() {
        if let Err(error) = result {
            *first_error = Some(format!("{context}: {error}"));
        }
    }
}

fn restore_full_window_state(
    window: &tauri::WebviewWindow,
    state: &WindowPresentationState,
    focus: bool,
) -> Result<(), String> {
    let (normal, theme) = {
        let mut presentation = window_presentation(state)?;
        presentation.mode = WindowPresentationMode::Restoring;
        presentation.panel_hidden = false;
        advance_window_generation(&mut presentation);
        (presentation.normal.clone(), presentation.theme.clone())
    };
    let mut first_error = None;

    // Show early and again after geometry restoration. Even when one cosmetic operation fails,
    // VSParallel must never be left as an invisible transparent window.
    record_first_window_error(
        &mut first_error,
        "could not unminimize VSParallel",
        window.unminimize(),
    );
    record_first_window_error(&mut first_error, "could not show VSParallel", window.show());

    if let Some(normal) = normal {
        record_first_window_error(
            &mut first_error,
            "could not restore the VSParallel stacking mode",
            window.set_always_on_top(normal.always_on_top),
        );
        record_first_window_error(
            &mut first_error,
            "could not restore the VSParallel desktop behavior",
            window.set_visible_on_all_workspaces(false),
        );
        record_first_window_error(
            &mut first_error,
            "could not restore the macOS window behavior",
            restore_macos_window_behavior(window, &normal),
        );
        clear_floating_window_effect(window);
        match window_background(&theme, false) {
            Ok(background) => record_first_window_error(
                &mut first_error,
                "could not restore the VSParallel window background",
                window.set_background_color(Some(background)),
            ),
            Err(error) => record_first_window_error::<(), _>(
                &mut first_error,
                "could not resolve the VSParallel window background",
                Err(error),
            ),
        }
        record_first_window_error(
            &mut first_error,
            "could not restore the VSParallel decorations",
            window.set_decorations(normal.decorated),
        );
        record_first_window_error(
            &mut first_error,
            "could not restore VSParallel resizing",
            window.set_resizable(normal.resizable),
        );
        #[cfg(target_os = "macos")]
        record_first_window_error(
            &mut first_error,
            "could not restore VSParallel minimization",
            window.set_minimizable(normal.minimizable),
        );
        record_first_window_error(
            &mut first_error,
            "could not restore the VSParallel size limit",
            window.set_min_size(Some(LogicalSize::new(
                MAIN_WINDOW_MIN_WIDTH,
                MAIN_WINDOW_MIN_HEIGHT,
            ))),
        );
        record_first_window_error(
            &mut first_error,
            "could not restore the VSParallel window size",
            window.set_size(normal.inner_size),
        );
        let _ = window.set_position(normal.position);
        if normal.maximized {
            record_first_window_error(
                &mut first_error,
                "could not restore the maximized window",
                window.maximize(),
            );
        }
        if normal.fullscreen {
            record_first_window_error(
                &mut first_error,
                "could not restore the full-screen window",
                window.set_fullscreen(true),
            );
        }
    }

    record_first_window_error(
        &mut first_error,
        "could not unminimize VSParallel after restoring it",
        window.unminimize(),
    );
    record_first_window_error(
        &mut first_error,
        "could not show VSParallel after restoring it",
        window.show(),
    );
    {
        let mut presentation = window_presentation(state)?;
        if first_error.is_none() {
            presentation.normal = None;
        }
        presentation.mode = WindowPresentationMode::Full;
        presentation.panel_hidden = false;
    }
    if focus {
        // Some window managers legitimately deny focus requests. The window is still fully
        // restored, visible, and interactive in that case.
        let _ = window.set_focus();
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct FloatingPanelReconcileResult {
    active: bool,
    desktop_move: Result<bool, String>,
}

fn reconcile_floating_panel(
    app: &tauri::AppHandle,
    generation: u64,
    editor_executable: &str,
    move_desktop: bool,
) -> Result<FloatingPanelReconcileResult, String> {
    let window = main_window(app)?;
    let presentation = app.state::<WindowPresentationState>();
    if !floating_panel_is_expected(&presentation, generation)? {
        return Ok(FloatingPanelReconcileResult {
            active: false,
            desktop_move: Ok(false),
        });
    }

    // Windows shell COM can occasionally wait on Explorer. Keep that work on this watchdog
    // worker instead of blocking Tauri's UI thread or holding the presentation-state lock.
    // The guarded setter batch below checks the generation again before repairing visibility.
    let desktop_move = if move_desktop {
        move_panel_to_foreground_desktop(&window, editor_executable)
    } else {
        Ok(true)
    };
    #[cfg(target_os = "macos")]
    let native_minimized = window
        .is_minimized()
        .map_err(|error| format!("could not read the VSParallel panel minimize state: {error}"))?;
    #[cfg(target_os = "macos")]
    let mut presentation = window_presentation(&presentation)?;
    #[cfg(not(target_os = "macos"))]
    let presentation = window_presentation(&presentation)?;
    if presentation.mode != WindowPresentationMode::Floating
        || presentation.generation != generation
        || presentation.panel_hidden
    {
        return Ok(FloatingPanelReconcileResult {
            active: false,
            desktop_move: Ok(false),
        });
    }
    #[cfg(target_os = "macos")]
    if native_minimized {
        // Cmd-M and native AppKit minimize actions bypass the frontend Hide command. Treat the
        // native state as newer user intent so this visibility watchdog cannot undo it.
        presentation.panel_hidden = true;
        advance_window_generation(&mut presentation);
        return Ok(FloatingPanelReconcileResult {
            active: false,
            desktop_move: Ok(false),
        });
    }

    // These Tauri setters enqueue native event-loop messages when called from this watchdog
    // worker. Keep the intent guard only while enqueuing the batch: Hide/Restore either wins
    // before this check or queues its newer native intent afterward. Avoid wrapping the batch in
    // a main-thread callback; nested Xlib/XCB access can abort Linux clients that did not call
    // XInitThreads.
    window
        .set_always_on_top(true)
        .map_err(|error| format!("could not repair the floating panel stacking: {error}"))?;
    let _ = window.set_visible_on_all_workspaces(true);
    set_macos_floating_behavior(&window)?;
    #[cfg(not(target_os = "macos"))]
    window
        .unminimize()
        .map_err(|error| format!("could not repair the floating panel minimized state: {error}"))?;
    show_floating_panel_without_focus(&window)?;
    Ok(FloatingPanelReconcileResult {
        active: true,
        desktop_move,
    })
}

fn schedule_floating_panel_watchdog(
    app: tauri::AppHandle,
    generation: u64,
    editor_executable: String,
) {
    std::mem::drop(tauri::async_runtime::spawn_blocking(move || {
        let mut previous_deadline = 0;
        let mut last_error = None;
        let mut move_desktop = cfg!(target_os = "windows");
        for deadline in FLOATING_PANEL_WATCHDOG_DEADLINES_MS {
            std::thread::sleep(Duration::from_millis(deadline - previous_deadline));
            previous_deadline = deadline;
            match reconcile_floating_panel(&app, generation, &editor_executable, move_desktop) {
                Ok(result) if !result.active => return,
                Ok(result) => match result.desktop_move {
                    Ok(true) => {
                        move_desktop = false;
                        last_error = None;
                    }
                    Ok(false) => {}
                    Err(error) => last_error = Some(error),
                },
                Err(error) => last_error = Some(error),
            }
        }

        if let Some(error) = last_error {
            if app
                .try_state::<WindowPresentationState>()
                .and_then(|presentation| floating_panel_is_expected(&presentation, generation).ok())
                == Some(true)
            {
                eprintln!("VSParallel could not fully reconcile its floating panel: {error}");
            }
        }
    }));
}

#[tauri::command]
async fn open_workspace(
    instance_id: String,
    app: tauri::AppHandle,
) -> Result<WindowChromeState, String> {
    let (open_target, launch_mode) = run_background(move || {
        let store = StateStore::from_environment()?;
        let now = now_ms();
        if let Some(target) = store.find_active_workspace_open_target(&instance_id, now) {
            return Ok((target, WorkspaceLaunchMode::PreferExisting));
        }
        store
            .find_workspace_open_target(&instance_id, now)
            .map(|target| (target, WorkspaceLaunchMode::NewWindow))
            .ok_or_else(|| {
                "the selected workspace is no longer available or has no supported local open target"
                    .to_string()
            })
    })
    .await?;

    let window = main_window(&app)?;
    let presentation = app.state::<WindowPresentationState>();
    let panel_generation = enter_floating_panel(&window, &presentation).await?;

    let launch_result = run_background(move || {
        open_editor_with(
            &ProcessWorkspaceLauncher,
            open_target.editor,
            &open_target.path,
            launch_mode,
        )
    })
    .await;
    let watchdog_editor_executable = match launch_result {
        Ok(command) => command,
        Err(error) => {
            let restore_result = restore_full_window_state(&window, &presentation, true);
            return match restore_result {
                Ok(()) => Err(error),
                Err(restore_error) => Err(format!(
                    "{error}; VSParallel also could not restore its full window: {restore_error}"
                )),
            };
        }
    };

    // A compatible editor may activate an existing window on another desktop or full-screen
    // Space after its CLI process has already returned. A bounded, non-focusing watchdog follows that delayed
    // activation and repairs visibility without taking keyboard focus away from the editor.
    schedule_floating_panel_watchdog(app.clone(), panel_generation, watchdog_editor_executable);

    // The editor launch is deliberately the final focus-affecting action. Reading state here
    // does not bring VSParallel back in front.
    read_window_chrome_state(&window, &presentation)
}

fn activate_tray_workspace(instance_id: &str) -> Result<(), String> {
    let target = StateStore::from_environment()?
        .find_active_workspace_open_target(instance_id, now_ms())
        .ok_or_else(|| {
            "the selected workspace is no longer active or has no supported local open target"
                .to_string()
        })?;
    open_editor_with(
        &ProcessWorkspaceLauncher,
        target.editor,
        &target.path,
        WorkspaceLaunchMode::PreferExisting,
    )
    .map(|_| ())
}

fn mark_floating_panel_hidden(
    state: &WindowPresentationState,
    hidden: bool,
) -> Result<bool, String> {
    let mut presentation = window_presentation(state)?;
    let floating = matches!(
        presentation.mode,
        WindowPresentationMode::EnteringFloating | WindowPresentationMode::Floating
    );
    if floating {
        presentation.panel_hidden = hidden;
        advance_window_generation(&mut presentation);
    }
    Ok(floating)
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = main_window(&app)?;
    let presentation = app.state::<WindowPresentationState>();
    let floating = mark_floating_panel_hidden(&presentation, true)?;
    let result = window
        .minimize()
        .map_err(|error| format!("could not minimize VSParallel: {error}"));
    if result.is_err() && floating {
        let _ = mark_floating_panel_hidden(&presentation, false);
    }
    result
}

fn show_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = main_window(app)?;
    if let Some(presentation) = app.try_state::<WindowPresentationState>() {
        restore_full_window_state(&window, &presentation, true)
    } else {
        window
            .unminimize()
            .map_err(|error| format!("could not restore VSParallel: {error}"))?;
        window
            .show()
            .map_err(|error| format!("could not show VSParallel: {error}"))?;
        window
            .set_focus()
            .map_err(|error| format!("could not focus VSParallel: {error}"))
    }
}

fn uses_custom_window_controls(platform: &str, decorated: bool) -> bool {
    platform != "macos" && !decorated
}

fn window_size_action(maximized: bool) -> WindowSizeAction {
    if maximized {
        WindowSizeAction::Restore
    } else {
        WindowSizeAction::Maximize
    }
}

fn main_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "the VSParallel window is not available".to_string())
}

fn read_window_chrome_state(
    window: &tauri::WebviewWindow,
    presentation: &WindowPresentationState,
) -> Result<WindowChromeState, String> {
    let platform = std::env::consts::OS;
    let decorated = window
        .is_decorated()
        .map_err(|error| format!("could not read the VSParallel window decorations: {error}"))?;
    let maximized = window
        .is_maximized()
        .map_err(|error| format!("could not read the VSParallel maximized state: {error}"))?;
    let fullscreen = window
        .is_fullscreen()
        .map_err(|error| format!("could not read the VSParallel full-screen state: {error}"))?;
    let focused = window
        .is_focused()
        .map_err(|error| format!("could not read the VSParallel focus state: {error}"))?;

    Ok(WindowChromeState {
        schema_version: WINDOW_CHROME_SCHEMA_VERSION,
        platform: platform.to_string(),
        custom_controls: uses_custom_window_controls(platform, decorated),
        maximized,
        fullscreen,
        focused,
        floating: is_floating_panel(presentation)?,
    })
}

#[tauri::command]
fn get_window_chrome_state(app: tauri::AppHandle) -> Result<WindowChromeState, String> {
    read_window_chrome_state(&main_window(&app)?, &app.state::<WindowPresentationState>())
}

#[tauri::command]
fn toggle_window_maximize(app: tauri::AppHandle) -> Result<WindowChromeState, String> {
    let window = main_window(&app)?;
    let presentation = app.state::<WindowPresentationState>();
    if !is_full_window(&presentation)? {
        return Err("restore the full VSParallel window before maximizing it".to_string());
    }
    if window
        .is_fullscreen()
        .map_err(|error| format!("could not read the VSParallel full-screen state: {error}"))?
    {
        return Err(
            "VSParallel cannot be maximized or restored while it is full screen".to_string(),
        );
    }
    let maximized = window
        .is_maximized()
        .map_err(|error| format!("could not read the VSParallel maximized state: {error}"))?;
    match window_size_action(maximized) {
        WindowSizeAction::Maximize => window.maximize(),
        WindowSizeAction::Restore => window.unmaximize(),
    }
    .map_err(|error| format!("could not maximize or restore VSParallel: {error}"))?;
    read_window_chrome_state(&window, &presentation)
}

#[tauri::command]
fn restore_full_window(app: tauri::AppHandle) -> Result<WindowChromeState, String> {
    let window = main_window(&app)?;
    let presentation = app.state::<WindowPresentationState>();
    restore_full_window_state(&window, &presentation, true)?;
    read_window_chrome_state(&window, &presentation)
}

#[tauri::command]
fn close_window(app: tauri::AppHandle) -> Result<(), String> {
    main_window(&app)?
        .close()
        .map_err(|error| format!("could not close VSParallel: {error}"))
}

fn window_background(theme: &str, floating: bool) -> Result<Color, String> {
    if floating {
        return match theme {
            "dark" | "light" => Ok(Color(0, 0, 0, 0)),
            _ => Err("window theme must be either dark or light".to_string()),
        };
    }
    match theme {
        "dark" => Ok(Color(17, 17, 20, 255)),
        "light" => Ok(Color(252, 252, 253, 255)),
        _ => Err("window theme must be either dark or light".to_string()),
    }
}

#[tauri::command]
fn set_window_chrome_theme(theme: String, app: tauri::AppHandle) -> Result<(), String> {
    // Validate before persisting so an unsupported token cannot poison future restores.
    window_background(&theme, false)?;
    let presentation = app.state::<WindowPresentationState>();
    let floating = {
        let mut presentation = window_presentation(&presentation)?;
        presentation.theme.clone_from(&theme);
        matches!(
            presentation.mode,
            WindowPresentationMode::EnteringFloating | WindowPresentationMode::Floating
        )
    };
    let window = main_window(&app)?;
    window
        .set_theme(Some(native_window_theme(&theme)?))
        .map_err(|error| format!("could not update the VSParallel native theme: {error}"))?;
    window
        .set_background_color(Some(window_background(&theme, floating)?))
        .map_err(|error| format!("could not update the VSParallel window theme: {error}"))?;
    if floating {
        apply_floating_window_effect(&window, &theme)?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // This must be the first plugin: a second launch restores the existing window instead
        // of leaving another background process and an indistinguishable stale tray icon.
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _cwd| {
                if let Err(error) = show_main_window(app) {
                    eprintln!("VSParallel could not restore its existing window: {error}");
                }
            },
        ))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            app.manage(WindowPresentationState::default());
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                if let Ok(effects) = floating_window_effects("dark") {
                    let _ = window.set_effects(effects);
                }
                let _ = remember_normal_window_geometry(
                    &window,
                    &app.state::<WindowPresentationState>(),
                );
            }
            let tray_available = match tray::setup(app) {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("VSParallel tray unavailable: {error}");
                    false
                }
            };
            app.manage(tray::TrayAvailability::new(tray_available));
            Ok(())
        })
        .on_window_event(|window, event| {
            let tray_available = window
                .app_handle()
                .state::<tray::TrayAvailability>()
                .is_available();
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    let presentation = window.app_handle().try_state::<WindowPresentationState>();
                    let floating = presentation
                        .as_ref()
                        .and_then(|state| is_compact_window(state).ok())
                        .unwrap_or(false);
                    if floating {
                        // A native close gesture must not destroy the only window while it is the
                        // switcher panel. Treat it as the panel's recoverable Hide action instead.
                        api.prevent_close();
                        if let Some(presentation) = presentation {
                            let _ = mark_floating_panel_hidden(&presentation, true);
                        }
                        let _ = window.minimize();
                    } else if tray_available {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
            if window.label() == "main"
                && matches!(
                    event,
                    tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
                )
            {
                if let Some(presentation) =
                    window.app_handle().try_state::<WindowPresentationState>()
                {
                    if let Some(webview_window) = window.app_handle().get_webview_window("main") {
                        let _ = remember_normal_window_geometry(&webview_window, &presentation);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            is_release_build,
            get_snapshot,
            get_usage,
            get_diagnostics,
            get_integration_status,
            install_companion,
            uninstall_companion,
            install_antigravity_ide_companion,
            uninstall_antigravity_ide_companion,
            install_codex_hooks,
            uninstall_codex_hooks,
            install_claude_hooks,
            uninstall_claude_hooks,
            install_antigravity_hooks,
            uninstall_antigravity_hooks,
            open_workspace,
            hide_window,
            get_window_chrome_state,
            toggle_window_maximize,
            restore_full_window,
            close_window,
            set_window_chrome_theme
        ])
        .build(tauri::generate_context!())
        .expect("error while building VSParallel")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = _event
            {
                if let Err(error) = show_main_window(_app) {
                    eprintln!("VSParallel could not restore its window: {error}");
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use companion_integration::{CompanionAction, CompanionStatusState};
    use tempfile::TempDir;

    fn companion_status(state: CompanionStatusState) -> CompanionStatus {
        CompanionStatus {
            state,
            extension_id: companion_integration::EXTENSION_ID.to_string(),
            bundled_version: Some("0.4.0".to_string()),
            installed_version: None,
            detail: None,
        }
    }

    fn test_normal_window_state(geometry: WindowGeometry) -> NormalWindowState {
        NormalWindowState {
            position: geometry.position,
            inner_size: geometry.inner_size,
            maximized: false,
            fullscreen: false,
            resizable: true,
            decorated: false,
            always_on_top: false,
            #[cfg(target_os = "macos")]
            minimizable: true,
            #[cfg(target_os = "macos")]
            macos_behavior: MacosWindowBehavior {
                collection_behavior: objc2_app_kit::NSWindowCollectionBehavior::Default,
                hides_on_deactivate: false,
            },
        }
    }

    #[test]
    fn window_chrome_policy_keeps_macos_native_and_customizes_undecorated_desktops() {
        assert!(!uses_custom_window_controls("macos", true));
        assert!(!uses_custom_window_controls("macos", false));
        assert!(uses_custom_window_controls("windows", false));
        assert!(uses_custom_window_controls("linux", false));
        assert!(!uses_custom_window_controls("windows", true));
        assert!(!uses_custom_window_controls("linux", true));
    }

    #[test]
    fn window_background_accepts_only_supported_theme_tokens() {
        assert_eq!(
            window_background("dark", false),
            Ok(tauri::window::Color(17, 17, 20, 255))
        );
        assert_eq!(
            window_background("light", false),
            Ok(tauri::window::Color(252, 252, 253, 255))
        );
        assert_eq!(
            window_background("dark", true),
            Ok(tauri::window::Color(0, 0, 0, 0))
        );
        assert_eq!(
            window_background("light", true),
            Ok(tauri::window::Color(0, 0, 0, 0))
        );
        assert!(window_background("system", false).is_err());
        assert!(window_background("system", true).is_err());
    }

    #[test]
    fn floating_panel_uses_native_glass_candidates_for_macos_and_windows() {
        let dark = floating_window_effects("dark").unwrap();
        assert_eq!(dark.effects, vec![Effect::Popover, Effect::Acrylic]);
        assert_eq!(dark.state, Some(EffectState::Active));
        assert_eq!(dark.radius, Some(14.0));
        assert_eq!(dark.color, Some(Color(18, 18, 22, 190)));

        let light = floating_window_effects("light").unwrap();
        assert_eq!(light.color, Some(Color(244, 244, 248, 200)));
        assert!(floating_window_effects("system").is_err());
    }

    #[test]
    fn native_window_theme_matches_the_resolved_css_theme() {
        assert_eq!(native_window_theme("dark"), Ok(tauri::Theme::Dark));
        assert_eq!(native_window_theme("light"), Ok(tauri::Theme::Light));
        assert!(native_window_theme("system").is_err());
    }

    #[test]
    fn floating_panel_position_uses_the_top_right_of_negative_origin_work_areas() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(-1920, 24),
            size: PhysicalSize::new(1920, 1056),
        };
        assert_eq!(
            floating_panel_position(work_area, PhysicalSize::new(400, 440), 16),
            PhysicalPosition::new(-416, 40)
        );
    }

    #[test]
    fn floating_panel_position_stays_recoverable_on_tiny_work_areas() {
        let work_area = PhysicalRect {
            position: PhysicalPosition::new(100, -50),
            size: PhysicalSize::new(320, 200),
        };
        assert_eq!(
            floating_panel_position(work_area, PhysicalSize::new(400, 440), 16),
            PhysicalPosition::new(100, -50)
        );
    }

    #[test]
    fn floating_panel_readiness_allows_only_small_native_rounding_differences() {
        assert!(floating_panel_size_is_ready(
            PhysicalSize::new(400, 440),
            1.0
        ));
        assert!(floating_panel_size_is_ready(
            PhysicalSize::new(796, 884),
            2.0
        ));
        assert!(!floating_panel_size_is_ready(
            PhysicalSize::new(760, 840),
            2.0
        ));
    }

    #[test]
    fn full_window_geometry_refreshes_a_retained_recovery_snapshot() {
        let original = WindowGeometry {
            position: PhysicalPosition::new(10, 20),
            inner_size: PhysicalSize::new(680, 560),
        };
        let compact = WindowGeometry {
            position: PhysicalPosition::new(900, 30),
            inner_size: PhysicalSize::new(400, 440),
        };
        let recovered = WindowGeometry {
            position: PhysicalPosition::new(80, 90),
            inner_size: PhysicalSize::new(720, 600),
        };
        let mut presentation = WindowPresentationInner::default();
        let mut normal = test_normal_window_state(original);
        normal.maximized = true;
        normal.fullscreen = true;
        normal.resizable = false;
        normal.decorated = true;
        normal.always_on_top = true;
        presentation.normal = Some(normal);
        presentation.last_normal_geometry = Some(original);

        assert!(!refresh_full_window_geometry(
            &mut presentation,
            compact,
            1.0
        ));
        assert_eq!(presentation.last_normal_geometry, Some(original));

        assert!(refresh_full_window_geometry(
            &mut presentation,
            recovered,
            1.0
        ));
        assert_eq!(presentation.last_normal_geometry, Some(recovered));
        let normal = presentation.normal.unwrap();
        assert_eq!(normal.position, recovered.position);
        assert_eq!(normal.inner_size, recovered.inner_size);
        assert!(normal.maximized);
        assert!(normal.fullscreen);
        assert!(!normal.resizable);
        assert!(normal.decorated);
        assert!(normal.always_on_top);
    }

    #[test]
    fn late_panel_events_cannot_replace_full_window_geometry() {
        let original = WindowGeometry {
            position: PhysicalPosition::new(10, 20),
            inner_size: PhysicalSize::new(680, 560),
        };
        let late_event = WindowGeometry {
            position: PhysicalPosition::new(40, 50),
            inner_size: PhysicalSize::new(800, 640),
        };
        for mode in [
            WindowPresentationMode::EnteringFloating,
            WindowPresentationMode::Floating,
            WindowPresentationMode::Restoring,
        ] {
            let mut presentation = WindowPresentationInner {
                mode,
                normal: Some(test_normal_window_state(original)),
                last_normal_geometry: Some(original),
                ..WindowPresentationInner::default()
            };
            assert!(!refresh_full_window_geometry(
                &mut presentation,
                late_event,
                1.0
            ));
            assert_eq!(presentation.last_normal_geometry, Some(original));
            assert_eq!(presentation.normal.unwrap().position, original.position);
        }
    }

    #[test]
    fn healthy_full_window_events_continue_to_update_saved_geometry() {
        let geometry = WindowGeometry {
            position: PhysicalPosition::new(-300, 120),
            inner_size: PhysicalSize::new(640, 480),
        };
        let mut presentation = WindowPresentationInner::default();
        assert!(refresh_full_window_geometry(
            &mut presentation,
            geometry,
            1.0
        ));
        assert_eq!(presentation.last_normal_geometry, Some(geometry));
        assert!(presentation.normal.is_none());
    }

    #[test]
    fn full_window_geometry_allows_only_native_hidpi_rounding() {
        let mut presentation = WindowPresentationInner::default();
        let rounded = WindowGeometry {
            position: PhysicalPosition::new(0, 0),
            inner_size: PhysicalSize::new(1_036, 716),
        };
        assert!(refresh_full_window_geometry(
            &mut presentation,
            rounded,
            2.0
        ));

        let too_small = WindowGeometry {
            position: PhysicalPosition::new(0, 0),
            inner_size: PhysicalSize::new(1_035, 716),
        };
        assert!(!refresh_full_window_geometry(
            &mut presentation,
            too_small,
            2.0
        ));
        assert_eq!(presentation.last_normal_geometry, Some(rounded));
    }

    #[test]
    fn explicit_presentation_mode_does_not_confuse_saved_geometry_with_readiness() {
        let state = WindowPresentationState::default();
        {
            let mut presentation = window_presentation(&state).unwrap();
            presentation.normal = Some(test_normal_window_state(WindowGeometry {
                position: PhysicalPosition::new(10, 20),
                inner_size: PhysicalSize::new(680, 560),
            }));
            presentation.mode = WindowPresentationMode::EnteringFloating;
        }

        assert!(!is_floating_panel(&state).unwrap());
        assert!(is_compact_window(&state).unwrap());
        assert!(!is_full_window(&state).unwrap());
        window_presentation(&state).unwrap().mode = WindowPresentationMode::Restoring;
        assert!(!is_floating_panel(&state).unwrap());
        assert!(!is_compact_window(&state).unwrap());
        assert!(!is_full_window(&state).unwrap());
        window_presentation(&state).unwrap().mode = WindowPresentationMode::Full;
        assert!(is_full_window(&state).unwrap());
    }

    #[test]
    fn hiding_the_panel_invalidates_a_pending_visibility_watchdog() {
        let state = WindowPresentationState::default();
        let generation = {
            let mut presentation = window_presentation(&state).unwrap();
            presentation.mode = WindowPresentationMode::Floating;
            advance_window_generation(&mut presentation)
        };
        assert!(floating_panel_is_expected(&state, generation).unwrap());

        assert!(mark_floating_panel_hidden(&state, true).unwrap());
        assert!(!floating_panel_is_expected(&state, generation).unwrap());
    }

    #[test]
    fn watchdog_retries_cover_delayed_editor_activation_without_running_forever() {
        assert_eq!(FLOATING_PANEL_WATCHDOG_DEADLINES_MS[0], 100);
        assert!(FLOATING_PANEL_WATCHDOG_DEADLINES_MS
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
        assert!(
            FLOATING_PANEL_WATCHDOG_DEADLINES_MS
                .last()
                .copied()
                .unwrap()
                <= 3_000
        );
    }

    #[test]
    fn windows_editor_identity_uses_exact_normalized_names() {
        assert!(windows_editor_process_matches(
            r#"C:\Program Files\Microsoft VS Code\Code.exe"#,
            "Code.exe"
        ));
        assert!(windows_editor_process_matches(
            r#"C:\Program Files\Microsoft VS Code Insiders\Code - Insiders.exe"#,
            "code-insiders.cmd"
        ));
        assert!(windows_editor_process_matches("VSCodium.exe", "codium"));
        assert!(!windows_editor_process_matches("Codex.exe", "code"));
        assert!(!windows_editor_process_matches("", "code"));
    }

    #[test]
    fn window_size_action_toggles_between_maximize_and_restore() {
        assert_eq!(window_size_action(false), WindowSizeAction::Maximize);
        assert_eq!(window_size_action(true), WindowSizeAction::Restore);
    }

    #[test]
    fn maps_companion_states_to_the_versioned_setup_contract() {
        let current = companion_view(CompanionStatus {
            installed_version: Some("0.4.0".to_string()),
            ..companion_status(CompanionStatusState::Current)
        });
        assert_eq!(current.state, "installed");
        assert_eq!(current.target_version.as_deref(), Some("0.4.0"));

        assert_eq!(
            companion_view(companion_status(CompanionStatusState::DifferentVersion)).state,
            "outdated"
        );
        assert_eq!(
            companion_view(companion_status(CompanionStatusState::VersionUnknown)).state,
            "repair_needed"
        );
        assert_eq!(
            companion_view(companion_status(CompanionStatusState::NotInstalled)).state,
            "not_installed"
        );
        assert_eq!(
            companion_view(companion_status(CompanionStatusState::Unavailable)).state,
            "unavailable"
        );
    }

    #[test]
    fn integration_executable_must_be_an_absolute_existing_file() {
        assert!(validate_integration_executable(PathBuf::from("relative"), "test").is_err());

        let temp = TempDir::new().unwrap();
        assert!(validate_integration_executable(temp.path().join("missing"), "test").is_err());
        let executable = temp.path().join("VSParallel test executable");
        std::fs::write(&executable, b"test").unwrap();
        assert_eq!(
            validate_integration_executable(executable.clone(), "test").unwrap(),
            executable
        );
    }

    #[test]
    fn macos_hook_installation_rejects_transient_app_locations() {
        for path in [
            "/Volumes/VSParallel/VSParallel.app/Contents/MacOS/vsparallel",
            "/private/var/folders/xy/random/AppTranslocation/UUID/d/VSParallel.app/Contents/MacOS/vsparallel",
        ] {
            let error = validate_macos_integration_location(Path::new(path)).unwrap_err();
            assert!(error.contains("/Applications"));
            assert!(error.contains("Repair"));
        }

        assert!(validate_macos_integration_location(Path::new(
            "/Applications/VSParallel.app/Contents/MacOS/vsparallel"
        ))
        .is_ok());
        assert!(validate_macos_integration_location(Path::new(
            "/System/Volumes/Data/Applications/VSParallel.app/Contents/MacOS/vsparallel"
        ))
        .is_ok());
    }

    #[test]
    fn an_unverified_companion_change_is_not_reported_as_success() {
        let result = CompanionOperationResult {
            action: CompanionAction::Install,
            verified: false,
            message: "VS Code accepted the command, but verification failed".to_string(),
            status: companion_status(CompanionStatusState::Unavailable),
        };
        let error = verified_companion_status(result).unwrap_err();
        assert!(error.contains("verification failed"));
    }

    #[test]
    fn antigravity_hook_changes_are_verified_before_setup_reports_success() {
        let change = |state: &str, installed: bool, message: &str| {
            antigravity_integration::AntigravityIntegrationChange {
                changed: false,
                migrated: false,
                status: antigravity_integration::AntigravityIntegrationStatus {
                    state: state.to_string(),
                    installed,
                    config_path: "/config/hooks.json".to_string(),
                    backup_path: "/config/hooks.json.vsparallel.bak".to_string(),
                    event_states: std::collections::BTreeMap::new(),
                    hooks_disabled: false,
                    message: message.to_string(),
                },
            }
        };

        assert!(verify_antigravity_change(&change("installed", true, "installed"), true).is_ok());
        assert!(
            verify_antigravity_change(&change("not_installed", false, "not installed"), false,)
                .is_ok()
        );
        let error = verify_antigravity_change(
            &change("conflict", false, "Rename the unrelated entry"),
            false,
        )
        .unwrap_err();
        assert!(error.contains("Rename"));
    }

    #[test]
    fn antigravity_setup_uses_the_latest_two_or_ide_hook_receipt() {
        let temp = TempDir::new().unwrap();
        let health = temp.path().join("antigravity-hook-health");
        std::fs::create_dir(&health).unwrap();
        std::fs::write(
            health.join("antigravity-2.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "event": "pre-invocation",
                "surface": "antigravity_2",
                "outcome": "no_workspace",
                "observedAtMs": 10,
                "workspaceCount": 0
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            health.join("antigravity-ide.json"),
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "event": "stop",
                "surface": "antigravity_ide",
                "outcome": "recorded",
                "observedAtMs": 20,
                "workspaceCount": 1
            }))
            .unwrap(),
        )
        .unwrap();

        let (label, detail) = antigravity_installed_copy_from_root(temp.path(), 20, "start a turn");

        assert_eq!(label, "Installed · event observed");
        assert!(detail.contains("Antigravity IDE"));
        assert!(detail.contains("latest stop event"));
        assert!(!detail.contains("Antigravity 2.0 ran the hook"));

        std::fs::write(health.join("antigravity-2.json"), b"{not json").unwrap();
        let (label, detail) = antigravity_installed_copy_from_root(temp.path(), 20, "start a turn");
        assert_eq!(label, "Installed · observation unavailable");
        assert!(detail.contains("at least one local surface"));
    }

    #[test]
    fn setup_contract_serializes_both_optional_lifecycle_integrations() {
        let lifecycle = LifecycleIntegrationView {
            state: "not_installed".to_string(),
            label: "Not installed".to_string(),
            detail: "Optional lifecycle monitoring is not installed.".to_string(),
            config_path: Some("/config/settings.json".to_string()),
            review_required: None,
        };
        let mut codex = lifecycle.clone();
        codex.review_required = Some(false);
        let status = IntegrationStatusView {
            schema_version: INTEGRATION_SCHEMA_VERSION,
            companion: companion_view(companion_status(CompanionStatusState::NotInstalled)),
            antigravity_ide: companion_view_for(
                companion_status(CompanionStatusState::NotInstalled),
                "Antigravity IDE",
            ),
            antigravity: lifecycle.clone(),
            codex,
            claude: lifecycle,
            requires_restart: false,
        };

        let serialized = serde_json::to_value(status).unwrap();
        assert_eq!(serialized["schemaVersion"], INTEGRATION_SCHEMA_VERSION);
        assert_eq!(serialized["codex"]["state"], "not_installed");
        assert_eq!(serialized["codex"]["reviewRequired"], false);
        assert!(serialized["claude"]["reviewRequired"].is_null());
        assert_eq!(serialized["claude"]["state"], "not_installed");
        assert!(serialized["companion"].is_object());
        assert!(serialized["antigravityIde"].is_object());
        assert_eq!(serialized["antigravity"]["state"], "not_installed");
    }
}
