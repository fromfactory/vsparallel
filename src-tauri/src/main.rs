#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
const VSCODE_SNAP_GUI_ENVIRONMENT: &[&str] = &[
    "GDK_PIXBUF_MODULEDIR",
    "GDK_PIXBUF_MODULE_FILE",
    "GIO_LAUNCHED_DESKTOP_FILE",
    "GIO_LAUNCHED_DESKTOP_FILE_PID",
    "GIO_MODULE_DIR",
    "GTK_EXE_PREFIX",
    "GTK_IM_MODULE_FILE",
    "GTK_MODULES",
    "GTK_PATH",
];

#[cfg(target_os = "linux")]
#[derive(Debug, Eq, PartialEq)]
enum DesktopEnvironmentChange {
    Set(&'static str, std::ffi::OsString),
    Remove(&'static str),
}

#[cfg(target_os = "linux")]
fn vscode_snap_environment_changes(
    original_xdg_data_dirs: Option<std::ffi::OsString>,
) -> Vec<DesktopEnvironmentChange> {
    let Some(original_xdg_data_dirs) = original_xdg_data_dirs.filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    std::iter::once(DesktopEnvironmentChange::Set(
        "XDG_DATA_DIRS",
        original_xdg_data_dirs,
    ))
    .chain(
        VSCODE_SNAP_GUI_ENVIRONMENT
            .iter()
            .copied()
            .map(DesktopEnvironmentChange::Remove),
    )
    .collect()
}

#[cfg(target_os = "linux")]
fn prepare_desktop_environment() {
    // An integrated terminal inside the VS Code Snap inherits lookup paths for
    // modules built against the Snap's GTK stack. VSParallel is a host-built
    // Tauri binary, so loading those modules can abort the process when GTK is
    // first reconfigured (for example, while entering the floating panel).
    // Apply this before Tauri starts any threads or initializes GTK.
    for change in
        vscode_snap_environment_changes(std::env::var_os("XDG_DATA_DIRS_VSCODE_SNAP_ORIG"))
    {
        match change {
            DesktopEnvironmentChange::Set(name, value) => std::env::set_var(name, value),
            DesktopEnvironmentChange::Remove(name) => std::env::remove_var(name),
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn prepare_desktop_environment() {}

fn main() {
    match std::env::args_os().nth(1).as_deref() {
        Some(argument) if argument == std::ffi::OsStr::new("codex-hook") => {
            std::process::exit(vsparallel_lib::run_codex_hook_stdio());
        }
        Some(argument) if argument == std::ffi::OsStr::new("claude-hook") => {
            std::process::exit(vsparallel_lib::run_claude_hook_stdio());
        }
        Some(argument) if argument == std::ffi::OsStr::new("cursor-hook") => {
            let event = std::env::args_os()
                .nth(2)
                .and_then(|argument| argument.into_string().ok())
                .as_deref()
                .and_then(vsparallel_lib::CursorHookEvent::from_cli_argument);
            match event {
                Some(event) => std::process::exit(vsparallel_lib::run_cursor_hook_stdio(event)),
                None => std::process::exit(2),
            }
        }
        Some(argument) if argument == std::ffi::OsStr::new("antigravity-hook") => {
            let event = std::env::args_os()
                .nth(2)
                .and_then(|argument| argument.into_string().ok())
                .as_deref()
                .and_then(vsparallel_lib::AntigravityHookEvent::from_cli_argument);
            std::process::exit(event.map_or(0, vsparallel_lib::run_antigravity_hook_stdio));
        }
        Some(argument)
            if argument == std::ffi::OsStr::new(vsparallel_lib::CLAUDE_STATUSLINE_ARGUMENT) =>
        {
            std::process::exit(vsparallel_lib::run_claude_statusline_stdio());
        }
        Some(argument)
            if argument == std::ffi::OsStr::new(vsparallel_lib::GEMINI_USAGE_ARGUMENT) =>
        {
            std::process::exit(vsparallel_lib::run_gemini_usage_stdio());
        }
        Some(argument)
            if argument == std::ffi::OsStr::new(vsparallel_lib::CURSOR_USAGE_ARGUMENT) =>
        {
            std::process::exit(vsparallel_lib::run_cursor_usage_stdio());
        }
        _ => {}
    }
    prepare_desktop_environment();
    vsparallel_lib::run();
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn ignores_non_snap_and_empty_original_environment() {
        assert!(vscode_snap_environment_changes(None).is_empty());
        assert!(vscode_snap_environment_changes(Some(std::ffi::OsString::new())).is_empty());
    }

    #[test]
    fn restores_host_data_dirs_and_removes_snap_gui_lookups() {
        let host_data_dirs = std::ffi::OsString::from("/usr/local/share:/usr/share");
        let changes = vscode_snap_environment_changes(Some(host_data_dirs.clone()));

        assert_eq!(
            changes.first(),
            Some(&DesktopEnvironmentChange::Set(
                "XDG_DATA_DIRS",
                host_data_dirs
            ))
        );
        assert_eq!(
            changes[1..],
            VSCODE_SNAP_GUI_ENVIRONMENT
                .iter()
                .copied()
                .map(DesktopEnvironmentChange::Remove)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn development_wrapper_removes_the_same_snap_gui_lookups() {
        let wrapper = include_str!("../../scripts/run-dev.sh");
        assert!(wrapper.contains("XDG_DATA_DIRS=$XDG_DATA_DIRS_VSCODE_SNAP_ORIG"));
        for name in VSCODE_SNAP_GUI_ENVIRONMENT {
            assert!(wrapper.contains(name), "run-dev.sh does not remove {name}");
        }
    }
}
