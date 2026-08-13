use serde::Serialize;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Command;
use std::thread;

pub const CODE_COMMAND_ENV: &str = "VSPARALLEL_CODE_COMMAND";
pub const CURSOR_COMMAND_ENV: &str = "VSPARALLEL_CURSOR_COMMAND";
pub const ANTIGRAVITY_IDE_COMMAND_ENV: &str = "VSPARALLEL_ANTIGRAVITY_IDE_COMMAND";

/// A trusted editor identity reported by a bundled workspace companion.
///
/// Heartbeats select from this closed enum; they never provide an executable
/// path. Command paths remain local application configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EditorKind {
    #[serde(rename = "vscode")]
    VsCode,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "antigravity_ide")]
    AntigravityIde,
    #[serde(rename = "antigravity_2")]
    Antigravity2,
}

impl EditorKind {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code",
            Self::Cursor => "Cursor",
            Self::AntigravityIde => "Antigravity IDE",
            Self::Antigravity2 => "Antigravity 2.0",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceLaunchMode {
    /// Pass only the exact workspace target. VS Code can then focus an existing
    /// matching window, subject to its settings and the platform's focus rules.
    PreferExisting,
    /// Force the exact workspace target to open in a separate VS Code window.
    NewWindow,
}

pub trait WorkspaceLauncher {
    fn launch(
        &self,
        executable: &OsStr,
        target: &Path,
        mode: WorkspaceLaunchMode,
    ) -> io::Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessWorkspaceLauncher;

impl WorkspaceLauncher for ProcessWorkspaceLauncher {
    fn launch(
        &self,
        executable: &OsStr,
        target: &Path,
        mode: WorkspaceLaunchMode,
    ) -> io::Result<()> {
        let mut child = Command::new(executable)
            .args(launch_arguments(mode, target))
            .spawn()?;
        thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

fn launch_arguments(mode: WorkspaceLaunchMode, target: &Path) -> Vec<OsString> {
    let mut arguments = Vec::with_capacity(2);
    if mode == WorkspaceLaunchMode::NewWindow {
        arguments.push(OsString::from("--new-window"));
    }
    arguments.push(target.as_os_str().to_owned());
    arguments
}

pub fn code_command() -> String {
    configured_command(CODE_COMMAND_ENV).unwrap_or_else(default_code_command)
}

pub fn cursor_command() -> String {
    configured_command(CURSOR_COMMAND_ENV).unwrap_or_else(default_cursor_command)
}

pub fn antigravity_ide_command() -> String {
    configured_command(ANTIGRAVITY_IDE_COMMAND_ENV).unwrap_or_else(default_antigravity_ide_command)
}

/// Resolve a trusted editor to its locally configured command. A missing
/// editor is a legacy companion heartbeat and retains the historical
/// `VSPARALLEL_CODE_COMMAND` behavior.
pub fn command_for_editor(editor: Option<EditorKind>) -> Option<String> {
    match editor {
        Some(EditorKind::Cursor) => Some(cursor_command()),
        Some(EditorKind::AntigravityIde) => Some(antigravity_ide_command()),
        Some(EditorKind::VsCode) | None => Some(code_command()),
        Some(EditorKind::Antigravity2) => None,
    }
}

fn configured_command(environment_variable: &str) -> Option<String> {
    env::var(environment_variable)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn default_code_command() -> String {
    "code".to_string()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn default_cursor_command() -> String {
    if let Some(home) = env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let candidates = [
            home.join(".local").join("bin").join("cursor"),
            home.join(".local")
                .join("share")
                .join("cursor")
                .join("cursor"),
            home.join("Applications").join("Cursor.AppImage"),
            home.join("Applications").join("cursor.AppImage"),
        ];
        if let Some(command) = candidates.into_iter().find(|candidate| candidate.is_file()) {
            return command.to_string_lossy().into_owned();
        }
    }

    for candidate in ["/usr/local/bin/cursor", "/usr/bin/cursor"] {
        let candidate = std::path::PathBuf::from(candidate);
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }

    "cursor".to_string()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn default_antigravity_ide_command() -> String {
    if let Some(home) = env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let candidates = [
            home.join("Applications")
                .join("antigravity-ide")
                .join("Antigravity IDE")
                .join("bin")
                .join("antigravity-ide"),
            home.join("Applications")
                .join("Antigravity IDE")
                .join("bin")
                .join("antigravity-ide"),
        ];
        if let Some(command) = candidates.into_iter().find(|candidate| candidate.is_file()) {
            return command.to_string_lossy().into_owned();
        }
    }

    "antigravity-ide".to_string()
}

#[cfg(target_os = "macos")]
fn default_code_command() -> String {
    let system = std::path::PathBuf::from(
        "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
    );
    if system.is_file() {
        return system.to_string_lossy().into_owned();
    }

    if let Some(home) = env::var_os("HOME") {
        let user = std::path::PathBuf::from(home)
            .join("Applications")
            .join("Visual Studio Code.app")
            .join("Contents")
            .join("Resources")
            .join("app")
            .join("bin")
            .join("code");
        if user.is_file() {
            return user.to_string_lossy().into_owned();
        }
    }

    "code".to_string()
}

#[cfg(target_os = "macos")]
fn default_cursor_command() -> String {
    let system =
        std::path::PathBuf::from("/Applications/Cursor.app/Contents/Resources/app/bin/cursor");
    if system.is_file() {
        return system.to_string_lossy().into_owned();
    }

    if let Some(home) = env::var_os("HOME") {
        let user = std::path::PathBuf::from(home)
            .join("Applications")
            .join("Cursor.app")
            .join("Contents")
            .join("Resources")
            .join("app")
            .join("bin")
            .join("cursor");
        if user.is_file() {
            return user.to_string_lossy().into_owned();
        }
    }

    "cursor".to_string()
}

#[cfg(target_os = "macos")]
fn default_antigravity_ide_command() -> String {
    let system = std::path::PathBuf::from(
        "/Applications/Antigravity IDE.app/Contents/Resources/app/bin/antigravity-ide",
    );
    if system.is_file() {
        return system.to_string_lossy().into_owned();
    }

    if let Some(home) = env::var_os("HOME") {
        let user = std::path::PathBuf::from(home)
            .join("Applications")
            .join("Antigravity IDE.app")
            .join("Contents")
            .join("Resources")
            .join("app")
            .join("bin")
            .join("antigravity-ide");
        if user.is_file() {
            return user.to_string_lossy().into_owned();
        }
    }

    "antigravity-ide".to_string()
}

#[cfg(target_os = "windows")]
fn default_code_command() -> String {
    let path_directories = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    let install_roots = [
        env::var_os("LOCALAPPDATA").map(|root| {
            std::path::PathBuf::from(root)
                .join("Programs")
                .join("Microsoft VS Code")
        }),
        env::var_os("ProgramFiles")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Microsoft VS Code")),
        env::var_os("ProgramFiles(x86)")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Microsoft VS Code")),
    ];

    find_windows_code_executable(
        path_directories.iter().map(std::path::PathBuf::as_path),
        install_roots
            .iter()
            .flatten()
            .map(std::path::PathBuf::as_path),
    )
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_else(|| "Code.exe".to_string())
}

#[cfg(target_os = "windows")]
fn default_cursor_command() -> String {
    let path_directories = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    let install_roots = [
        env::var_os("LOCALAPPDATA").map(|root| {
            std::path::PathBuf::from(root)
                .join("Programs")
                .join("cursor")
        }),
        env::var_os("LOCALAPPDATA").map(|root| {
            std::path::PathBuf::from(root)
                .join("Programs")
                .join("Cursor")
        }),
        env::var_os("ProgramFiles")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Cursor")),
        env::var_os("ProgramFiles(x86)")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Cursor")),
    ];

    find_windows_cursor_executable(
        path_directories.iter().map(std::path::PathBuf::as_path),
        install_roots
            .iter()
            .flatten()
            .map(std::path::PathBuf::as_path),
    )
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_else(|| "Cursor.exe".to_string())
}

#[cfg(target_os = "windows")]
fn default_antigravity_ide_command() -> String {
    let path_directories = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    let install_roots = [
        env::var_os("LOCALAPPDATA").map(|root| {
            std::path::PathBuf::from(root)
                .join("Programs")
                .join("Antigravity IDE")
        }),
        env::var_os("ProgramFiles")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Antigravity IDE")),
        env::var_os("ProgramFiles(x86)")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Antigravity IDE")),
    ];

    find_windows_antigravity_ide_executable(
        path_directories.iter().map(std::path::PathBuf::as_path),
        install_roots
            .iter()
            .flatten()
            .map(std::path::PathBuf::as_path),
    )
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_else(|| "Antigravity IDE.exe".to_string())
}

#[cfg(any(target_os = "windows", test))]
fn find_windows_code_executable<'a>(
    path_directories: impl IntoIterator<Item = &'a Path>,
    install_roots: impl IntoIterator<Item = &'a Path>,
) -> Option<std::path::PathBuf> {
    for directory in path_directories {
        let direct = directory.join("Code.exe");
        if direct.is_file() {
            return Some(direct);
        }

        let command_launcher = directory.join("code.cmd");
        if command_launcher.is_file() {
            if let Some(root) = directory.parent() {
                let native = root.join("Code.exe");
                if native.is_file() {
                    return Some(native);
                }
            }
        }
    }

    install_roots
        .into_iter()
        .map(|root| root.join("Code.exe"))
        .find(|candidate| candidate.is_file())
}

#[cfg(any(target_os = "windows", test))]
fn find_windows_cursor_executable<'a>(
    path_directories: impl IntoIterator<Item = &'a Path>,
    install_roots: impl IntoIterator<Item = &'a Path>,
) -> Option<std::path::PathBuf> {
    const EXECUTABLE_NAMES: [&str; 2] = ["Cursor.exe", "cursor.exe"];

    for directory in path_directories {
        for name in EXECUTABLE_NAMES {
            let direct = directory.join(name);
            if direct.is_file() {
                return Some(direct);
            }
        }

        if directory.join("cursor.cmd").is_file() {
            // Cursor commonly adds `resources/app/bin` to PATH, while the
            // native executable lives at the installation root.
            for root in directory.ancestors().skip(1).take(4) {
                for name in EXECUTABLE_NAMES {
                    let native = root.join(name);
                    if native.is_file() {
                        return Some(native);
                    }
                }
            }
        }
    }

    for root in install_roots {
        for name in EXECUTABLE_NAMES {
            let candidate = root.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(any(target_os = "windows", test))]
fn find_windows_antigravity_ide_executable<'a>(
    path_directories: impl IntoIterator<Item = &'a Path>,
    install_roots: impl IntoIterator<Item = &'a Path>,
) -> Option<std::path::PathBuf> {
    const EXECUTABLE_NAMES: [&str; 2] = ["Antigravity IDE.exe", "antigravity-ide.exe"];

    for directory in path_directories {
        for name in EXECUTABLE_NAMES {
            let direct = directory.join(name);
            if direct.is_file() {
                return Some(direct);
            }
        }

        if directory.join("antigravity-ide.cmd").is_file() {
            if let Some(root) = directory.parent() {
                for name in EXECUTABLE_NAMES {
                    let native = root.join(name);
                    if native.is_file() {
                        return Some(native);
                    }
                }
            }
        }
    }

    for root in install_roots {
        for name in EXECUTABLE_NAMES {
            let candidate = root.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn open_with<L: WorkspaceLauncher>(
    launcher: &L,
    executable: &str,
    target: &Path,
    mode: WorkspaceLaunchMode,
) -> Result<(), String> {
    if executable.trim().is_empty() {
        return Err("the VS Code command is empty".to_string());
    }
    if !target.is_absolute() {
        return Err("the workspace target is not an absolute local path".to_string());
    }
    if !target.exists() {
        return Err(format!(
            "the workspace target no longer exists: {}",
            target.display()
        ));
    }

    launcher
        .launch(OsStr::new(executable), target, mode)
        .map_err(|error| {
            let option = match mode {
                WorkspaceLaunchMode::PreferExisting => "",
                WorkspaceLaunchMode::NewWindow => " --new-window",
            };
            format!(
                "could not start `{executable}{option} {}`: {error}",
                target.display()
            )
        })
}

/// Open a target with the locally configured command for a trusted editor.
/// Returns the resolved command so callers can use the same process identity
/// for platform-specific post-launch handling.
pub fn open_editor_with<L: WorkspaceLauncher>(
    launcher: &L,
    editor: Option<EditorKind>,
    target: &Path,
    mode: WorkspaceLaunchMode,
) -> Result<String, String> {
    let executable = command_for_editor(editor).ok_or_else(|| {
        let name = editor.map_or("This editor", EditorKind::display_name);
        format!("{name} does not expose a supported workspace launcher")
    })?;
    open_with(launcher, &executable, target, mode)?;
    Ok(executable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;
    use tempfile::TempDir;

    #[derive(Default)]
    struct RecordingLauncher {
        call: Mutex<Option<(OsString, std::path::PathBuf, WorkspaceLaunchMode)>>,
        failure: Mutex<Option<io::ErrorKind>>,
    }

    impl WorkspaceLauncher for RecordingLauncher {
        fn launch(
            &self,
            executable: &OsStr,
            target: &Path,
            mode: WorkspaceLaunchMode,
        ) -> io::Result<()> {
            if let Some(kind) = *self.failure.lock().unwrap() {
                return Err(io::Error::new(kind, "injected failure"));
            }
            *self.call.lock().unwrap() = Some((executable.to_owned(), target.to_path_buf(), mode));
            Ok(())
        }
    }

    #[test]
    fn passes_executable_path_and_explicit_mode_separately() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("project with spaces");
        std::fs::create_dir_all(&target).unwrap();
        let launcher = RecordingLauncher::default();

        open_with(
            &launcher,
            "/opt/Visual Studio Code/code",
            &target,
            WorkspaceLaunchMode::PreferExisting,
        )
        .unwrap();

        let call = launcher.call.lock().unwrap().clone().unwrap();
        assert_eq!(call.0, OsString::from("/opt/Visual Studio Code/code"));
        assert_eq!(call.1, target);
        assert_eq!(call.2, WorkspaceLaunchMode::PreferExisting);
    }

    #[test]
    fn editor_kinds_have_stable_protocol_values_and_labels() {
        let cases = [
            (EditorKind::VsCode, "vscode", "VS Code"),
            (EditorKind::Cursor, "cursor", "Cursor"),
            (
                EditorKind::AntigravityIde,
                "antigravity_ide",
                "Antigravity IDE",
            ),
            (EditorKind::Antigravity2, "antigravity_2", "Antigravity 2.0"),
        ];
        for (editor, serialized, label) in cases {
            assert_eq!(serde_json::to_value(editor).unwrap(), serialized);
            assert_eq!(editor.display_name(), label);
        }
    }

    #[test]
    fn trusted_editor_selects_only_its_local_command() {
        assert_eq!(
            command_for_editor(Some(EditorKind::VsCode)),
            Some(code_command())
        );
        assert_eq!(
            command_for_editor(Some(EditorKind::Cursor)),
            Some(cursor_command())
        );
        assert_eq!(
            command_for_editor(Some(EditorKind::AntigravityIde)),
            Some(antigravity_ide_command())
        );
        assert_eq!(command_for_editor(None), Some(code_command()));
        assert_eq!(command_for_editor(Some(EditorKind::Antigravity2)), None);
    }

    #[test]
    fn source_specific_launch_uses_selected_editor_and_legacy_uses_default() {
        let temp = TempDir::new().unwrap();
        let launcher = RecordingLauncher::default();

        let cursor_command = open_editor_with(
            &launcher,
            Some(EditorKind::Cursor),
            temp.path(),
            WorkspaceLaunchMode::PreferExisting,
        )
        .unwrap();
        assert_eq!(cursor_command, super::cursor_command());
        assert_eq!(
            launcher.call.lock().unwrap().as_ref().unwrap().0,
            OsString::from(super::cursor_command())
        );

        let antigravity_command = open_editor_with(
            &launcher,
            Some(EditorKind::AntigravityIde),
            temp.path(),
            WorkspaceLaunchMode::PreferExisting,
        )
        .unwrap();
        assert_eq!(antigravity_command, antigravity_ide_command());
        assert_eq!(
            launcher.call.lock().unwrap().as_ref().unwrap().0,
            OsString::from(antigravity_ide_command())
        );

        let default_command =
            open_editor_with(&launcher, None, temp.path(), WorkspaceLaunchMode::NewWindow).unwrap();
        assert_eq!(default_command, code_command());
        let call = launcher.call.lock().unwrap().clone().unwrap();
        assert_eq!(call.0, OsString::from(code_command()));
        assert_eq!(call.2, WorkspaceLaunchMode::NewWindow);
    }

    #[test]
    fn antigravity_two_is_explicitly_not_openable() {
        let temp = TempDir::new().unwrap();
        let launcher = RecordingLauncher::default();
        let error = open_editor_with(
            &launcher,
            Some(EditorKind::Antigravity2),
            temp.path(),
            WorkspaceLaunchMode::PreferExisting,
        )
        .unwrap_err();

        assert!(error.contains("Antigravity 2.0"));
        assert!(error.contains("does not expose a supported workspace launcher"));
        assert!(launcher.call.lock().unwrap().is_none());
    }

    #[test]
    fn prefer_existing_passes_only_the_exact_target() {
        let target = Path::new("/work/project with spaces");
        assert_eq!(
            launch_arguments(WorkspaceLaunchMode::PreferExisting, target),
            vec![target.as_os_str().to_owned()]
        );
    }

    #[test]
    fn new_window_passes_the_force_flag_before_the_exact_target() {
        let target = Path::new("/work/project with spaces");
        assert_eq!(
            launch_arguments(WorkspaceLaunchMode::NewWindow, target),
            vec![
                OsString::from("--new-window"),
                target.as_os_str().to_owned()
            ]
        );
    }

    #[test]
    fn rejects_missing_and_relative_targets_without_launching() {
        let launcher = RecordingLauncher::default();
        assert!(open_with(
            &launcher,
            "code",
            Path::new("relative/repo"),
            WorkspaceLaunchMode::PreferExisting,
        )
        .is_err());
        assert!(open_with(
            &launcher,
            "code",
            Path::new("/definitely/missing/vsparallel"),
            WorkspaceLaunchMode::NewWindow,
        )
        .is_err());
        assert!(launcher.call.lock().unwrap().is_none());
    }

    #[test]
    fn reports_launcher_failure() {
        let temp = TempDir::new().unwrap();
        let launcher = RecordingLauncher::default();
        *launcher.failure.lock().unwrap() = Some(io::ErrorKind::NotFound);
        let error = open_with(
            &launcher,
            "missing-code",
            temp.path(),
            WorkspaceLaunchMode::PreferExisting,
        )
        .unwrap_err();
        assert!(error.contains("missing-code"));
        assert!(error.contains("injected failure"));
        assert!(!error.contains("--new-window"));
    }

    #[test]
    fn new_window_failure_reports_the_selected_mode() {
        let temp = TempDir::new().unwrap();
        let launcher = RecordingLauncher::default();
        *launcher.failure.lock().unwrap() = Some(io::ErrorKind::NotFound);
        let error = open_with(
            &launcher,
            "missing-code",
            temp.path(),
            WorkspaceLaunchMode::NewWindow,
        )
        .unwrap_err();
        assert!(error.contains("--new-window"));
    }

    #[test]
    fn resolves_native_windows_code_without_invoking_a_batch_shell() {
        let temp = TempDir::new().unwrap();
        let install_root = temp.path().join("Microsoft VS Code");
        let bin = install_root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("code.cmd"), b"launcher").unwrap();
        let executable = install_root.join("Code.exe");
        std::fs::write(&executable, b"native executable").unwrap();

        assert_eq!(
            find_windows_code_executable([bin.as_path()], std::iter::empty()),
            Some(executable)
        );
    }

    #[test]
    fn resolves_native_windows_cursor_without_invoking_a_batch_shell() {
        let temp = TempDir::new().unwrap();
        let install_root = temp.path().join("Cursor");
        let bin = install_root.join("resources").join("app").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("cursor.cmd"), b"launcher").unwrap();
        let executable = install_root.join("Cursor.exe");
        std::fs::write(&executable, b"native executable").unwrap();

        assert_eq!(
            find_windows_cursor_executable([bin.as_path()], std::iter::empty()),
            Some(executable)
        );
    }

    #[test]
    fn resolves_native_windows_antigravity_ide_without_invoking_a_batch_shell() {
        let temp = TempDir::new().unwrap();
        let install_root = temp.path().join("Antigravity IDE");
        let bin = install_root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("antigravity-ide.cmd"), b"launcher").unwrap();
        let executable = install_root.join("Antigravity IDE.exe");
        std::fs::write(&executable, b"native executable").unwrap();

        assert_eq!(
            find_windows_antigravity_ide_executable([bin.as_path()], std::iter::empty()),
            Some(executable)
        );
    }
}
