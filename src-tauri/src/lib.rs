mod claude_integration;
mod codex_integration;
mod companion_integration;
mod opener;
mod state;
mod tray;

pub use claude_integration::run_claude_hook_stdio;
pub use codex_integration::run_codex_hook_stdio;

use companion_integration::{CompanionOperationResult, CompanionStatus, CompanionStatusState};
use opener::{code_command, open_with, ProcessWorkspaceLauncher, WorkspaceLaunchMode};
use serde::Serialize;
use state::{now_ms, Diagnostics, Snapshot, StateStore};
use std::ffi::OsStr;
use std::path::PathBuf;
use tauri::Manager;

const INTEGRATION_SCHEMA_VERSION: u32 = 1;
const WINDOW_CHROME_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WindowChromeState {
    schema_version: u32,
    platform: String,
    custom_controls: bool,
    maximized: bool,
    fullscreen: bool,
    focused: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowSizeAction {
    Maximize,
    Restore,
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
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct IntegrationStatusView {
    schema_version: u32,
    companion: CompanionIntegrationView,
    codex: LifecycleIntegrationView,
    claude: LifecycleIntegrationView,
    requires_restart: bool,
}

#[tauri::command]
async fn get_snapshot() -> Result<Snapshot, String> {
    run_background(current_snapshot).await
}

fn current_snapshot() -> Result<Snapshot, String> {
    Ok(StateStore::from_environment()?.snapshot(now_ms()))
}

#[tauri::command]
async fn get_diagnostics() -> Result<Diagnostics, String> {
    run_background(|| {
        let command = code_command();
        Ok(StateStore::from_environment()?.diagnostics(now_ms(), command))
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
    let command = code_command();
    let companion_status = companion_integration::companion_status(OsStr::new(&command));
    build_integration_status_with_companion(companion_status, requires_restart)
}

fn build_integration_status_with_companion(
    companion_status: CompanionStatus,
    requires_restart: bool,
) -> IntegrationStatusView {
    let companion = companion_view(companion_status);
    let codex = codex_view();
    let claude = claude_view();
    IntegrationStatusView {
        schema_version: INTEGRATION_SCHEMA_VERSION,
        companion,
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
    let (state, label, fallback_detail) = match status.state {
        CompanionStatusState::Current => (
            "installed",
            "Installed",
            "The VS Code companion is installed and current.",
        ),
        CompanionStatusState::DifferentVersion => (
            "outdated",
            "Update available",
            "The installed VS Code companion differs from the version bundled with VSParallel.",
        ),
        CompanionStatusState::VersionUnknown => (
            "repair_needed",
            "Repair needed",
            "VS Code reports the companion, but its installed version could not be verified.",
        ),
        CompanionStatusState::NotInstalled => (
            "not_installed",
            "Not installed",
            "The VS Code companion is not installed.",
        ),
        CompanionStatusState::Unavailable => (
            "unavailable",
            "VS Code unavailable",
            "VSParallel could not query the VS Code extension installation.",
        ),
    };
    CompanionIntegrationView {
        state: state.to_string(),
        label: label.to_string(),
        detail: status.detail.unwrap_or_else(|| fallback_detail.to_string()),
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
            let (state, label) = match status.state.as_str() {
                "installed" => ("installed", "Installed · review required"),
                "not_installed" => ("not_installed", "Not installed"),
                "stale" => ("repair_needed", "Update available"),
                "partial" => ("repair_needed", "Repair needed"),
                _ => ("unavailable", "Status unavailable"),
            };
            LifecycleIntegrationView {
                state: state.to_string(),
                label: label.to_string(),
                detail: status.message,
                config_path: Some(status.config_path),
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
        label: "Review required".to_string(),
        detail: error,
        config_path,
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
    }
}

fn integration_executable() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(app_image) = std::env::var_os("APPIMAGE").filter(|value| !value.is_empty()) {
        return validate_integration_executable(PathBuf::from(app_image), "APPIMAGE");
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate the VSParallel executable: {error}"))?;
    validate_integration_executable(executable, "current executable")
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

#[tauri::command]
async fn open_workspace(instance_id: String) -> Result<(), String> {
    run_background(move || {
        let store = StateStore::from_environment()?;
        let target = store
            .find_open_target(&instance_id, now_ms())
            .ok_or_else(|| {
                "the selected VS Code workspace is no longer available or has no local open target"
                    .to_string()
            })?;
        open_with(
            &ProcessWorkspaceLauncher,
            &code_command(),
            &target,
            WorkspaceLaunchMode::NewWindow,
        )
    })
    .await
}

fn activate_tray_workspace(instance_id: &str) -> Result<(), String> {
    let target = StateStore::from_environment()?
        .find_active_open_target(instance_id, now_ms())
        .ok_or_else(|| {
            "the selected VS Code workspace is no longer active or has no local open target"
                .to_string()
        })?;
    open_with(
        &ProcessWorkspaceLauncher,
        &code_command(),
        &target,
        WorkspaceLaunchMode::PreferExisting,
    )
}

#[tauri::command]
fn hide_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = main_window(&app)?;
    window
        .minimize()
        .map_err(|error| format!("could not minimize VSParallel: {error}"))
}

fn show_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    let window = main_window(app)?;
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

fn read_window_chrome_state(window: &tauri::WebviewWindow) -> Result<WindowChromeState, String> {
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
    })
}

#[tauri::command]
fn get_window_chrome_state(app: tauri::AppHandle) -> Result<WindowChromeState, String> {
    read_window_chrome_state(&main_window(&app)?)
}

#[tauri::command]
fn toggle_window_maximize(app: tauri::AppHandle) -> Result<WindowChromeState, String> {
    let window = main_window(&app)?;
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
    read_window_chrome_state(&window)
}

#[tauri::command]
fn close_window(app: tauri::AppHandle) -> Result<(), String> {
    main_window(&app)?
        .close()
        .map_err(|error| format!("could not close VSParallel: {error}"))
}

fn window_background(theme: &str) -> Result<tauri::window::Color, String> {
    match theme {
        "dark" => Ok(tauri::window::Color(17, 17, 20, 255)),
        "light" => Ok(tauri::window::Color(252, 252, 253, 255)),
        _ => Err("window theme must be either dark or light".to_string()),
    }
}

#[tauri::command]
fn set_window_chrome_theme(theme: String, app: tauri::AppHandle) -> Result<(), String> {
    main_window(&app)?
        .set_background_color(Some(window_background(&theme)?))
        .map_err(|error| format!("could not update the VSParallel window theme: {error}"))
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
        .setup(|app| {
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
            if tray_available && window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_diagnostics,
            get_integration_status,
            install_companion,
            uninstall_companion,
            install_codex_hooks,
            uninstall_codex_hooks,
            install_claude_hooks,
            uninstall_claude_hooks,
            open_workspace,
            hide_window,
            get_window_chrome_state,
            toggle_window_maximize,
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
            bundled_version: Some("0.2.0".to_string()),
            installed_version: None,
            detail: None,
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
            window_background("dark"),
            Ok(tauri::window::Color(17, 17, 20, 255))
        );
        assert_eq!(
            window_background("light"),
            Ok(tauri::window::Color(252, 252, 253, 255))
        );
        assert!(window_background("system").is_err());
    }

    #[test]
    fn window_size_action_toggles_between_maximize_and_restore() {
        assert_eq!(window_size_action(false), WindowSizeAction::Maximize);
        assert_eq!(window_size_action(true), WindowSizeAction::Restore);
    }

    #[test]
    fn maps_companion_states_to_the_versioned_setup_contract() {
        let current = companion_view(CompanionStatus {
            installed_version: Some("0.2.0".to_string()),
            ..companion_status(CompanionStatusState::Current)
        });
        assert_eq!(current.state, "installed");
        assert_eq!(current.target_version.as_deref(), Some("0.2.0"));

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
    fn setup_contract_serializes_both_optional_lifecycle_integrations() {
        let lifecycle = LifecycleIntegrationView {
            state: "not_installed".to_string(),
            label: "Not installed".to_string(),
            detail: "Optional lifecycle monitoring is not installed.".to_string(),
            config_path: Some("/config/settings.json".to_string()),
        };
        let status = IntegrationStatusView {
            schema_version: INTEGRATION_SCHEMA_VERSION,
            companion: companion_view(companion_status(CompanionStatusState::NotInstalled)),
            codex: lifecycle.clone(),
            claude: lifecycle,
            requires_restart: false,
        };

        let serialized = serde_json::to_value(status).unwrap();
        assert_eq!(serialized["schemaVersion"], INTEGRATION_SCHEMA_VERSION);
        assert_eq!(serialized["codex"]["state"], "not_installed");
        assert_eq!(serialized["claude"]["state"], "not_installed");
        assert!(serialized["companion"].is_object());
    }
}
