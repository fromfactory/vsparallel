use serde::Serialize;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
#[cfg(any(test, target_os = "windows", target_os = "macos"))]
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::thread;

pub const CODE_COMMAND_ENV: &str = "VSPARALLEL_CODE_COMMAND";
pub const CURSOR_COMMAND_ENV: &str = "VSPARALLEL_CURSOR_COMMAND";
pub const ANTIGRAVITY_IDE_COMMAND_ENV: &str = "VSPARALLEL_ANTIGRAVITY_IDE_COMMAND";
pub const ZED_COMMAND_ENV: &str = "VSPARALLEL_ZED_COMMAND";
const MAX_WORKSPACE_TARGETS: usize = 64;

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
    #[serde(rename = "zed")]
    Zed,
}

impl EditorKind {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::VsCode => "VS Code",
            Self::Cursor => "Cursor",
            Self::AntigravityIde => "Antigravity IDE",
            Self::Antigravity2 => "Antigravity 2.0",
            Self::Zed => "Zed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceLaunchMode {
    /// Ask the selected editor to prefer an existing exact-target window,
    /// subject to its settings and the platform's focus rules.
    PreferExisting,
    /// Force the exact workspace target to open in a separate editor window.
    NewWindow,
}

pub trait WorkspaceLauncher {
    fn launch(
        &self,
        executable: &OsStr,
        targets: &[PathBuf],
        mode: WorkspaceLaunchMode,
        editor: Option<EditorKind>,
    ) -> io::Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessWorkspaceLauncher;

impl WorkspaceLauncher for ProcessWorkspaceLauncher {
    fn launch(
        &self,
        executable: &OsStr,
        targets: &[PathBuf],
        mode: WorkspaceLaunchMode,
        editor: Option<EditorKind>,
    ) -> io::Result<()> {
        let mut command = workspace_launch_command(executable, editor);
        let mut child = command
            .args(launch_arguments(editor, mode, targets))
            .spawn()?;
        thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn workspace_launch_command(executable: &OsStr, editor: Option<EditorKind>) -> Command {
    if let Some(gui_executable) = standard_macos_editor_gui_executable(executable, editor)
        .filter(|candidate| candidate.is_file())
    {
        // The standard shell launchers run the bundle executable with
        // ELECTRON_RUN_AS_NODE to execute the editor CLI. Opening a workspace
        // does not need that short-lived Node process, and older Electron
        // builds can abort while its ESM worker is shutting down. Ask Launch
        // Services to start the bundle's normal GUI entry point instead.
        return macos_gui_launch_command(&gui_executable);
    }

    Command::new(executable)
}

#[cfg(not(target_os = "macos"))]
fn workspace_launch_command(executable: &OsStr, _editor: Option<EditorKind>) -> Command {
    Command::new(executable)
}

#[cfg(any(test, target_os = "macos"))]
fn macos_gui_launch_command(gui_executable: &Path) -> Command {
    let mut command = Command::new("/usr/bin/open");
    command
        // `-n` makes LaunchServices deliver these arguments even when another
        // instance owns Electron's single-instance lock. Omitting `-g` lets
        // the selected editor remain the final focus-affecting action.
        .args(["-n", "-a"])
        .arg(gui_executable)
        .arg("--args")
        // Preserve the caller's normal environment while ensuring neither
        // `open` nor the editor bundle starts in Electron's Node mode.
        .env_remove("ELECTRON_RUN_AS_NODE");
    command
}

#[cfg(any(test, target_os = "macos"))]
fn standard_macos_editor_gui_executable(
    executable: &OsStr,
    editor: Option<EditorKind>,
) -> Option<PathBuf> {
    let (cli_name, gui_name) = match editor {
        Some(EditorKind::Cursor) => ("cursor", "Cursor"),
        Some(EditorKind::AntigravityIde) => ("antigravity-ide", "Electron"),
        Some(EditorKind::VsCode) | None => ("code", "Electron"),
        Some(EditorKind::Antigravity2 | EditorKind::Zed) => return None,
    };
    let executable = Path::new(executable);
    if executable.file_name()? != OsStr::new(cli_name) {
        return None;
    }

    let bin = executable.parent()?;
    let app_resources = bin.parent()?;
    let resources = app_resources.parent()?;
    let contents = resources.parent()?;
    let app_bundle = contents.parent()?;
    if bin.file_name()? != OsStr::new("bin")
        || app_resources.file_name()? != OsStr::new("app")
        || resources.file_name()? != OsStr::new("Resources")
        || contents.file_name()? != OsStr::new("Contents")
        || app_bundle.extension()? != OsStr::new("app")
    {
        return None;
    }

    Some(app_bundle.join("Contents").join("MacOS").join(gui_name))
}

fn launch_arguments(
    editor: Option<EditorKind>,
    mode: WorkspaceLaunchMode,
    targets: &[PathBuf],
) -> Vec<OsString> {
    let mut arguments = Vec::with_capacity(targets.len().saturating_add(1));
    match (editor, mode) {
        (Some(EditorKind::Zed), WorkspaceLaunchMode::PreferExisting) => {
            arguments.push(OsString::from("--existing"));
        }
        (Some(EditorKind::Zed), WorkspaceLaunchMode::NewWindow) => {
            arguments.push(OsString::from("--new"));
        }
        (_, WorkspaceLaunchMode::NewWindow) => {
            arguments.push(OsString::from("--new-window"));
        }
        (_, WorkspaceLaunchMode::PreferExisting) => {}
    }
    arguments.extend(targets.iter().map(|target| target.as_os_str().to_owned()));
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

pub fn zed_command() -> String {
    configured_command(ZED_COMMAND_ENV).unwrap_or_else(default_zed_command)
}

/// Resolve a trusted editor to its locally configured command. A missing
/// editor is a legacy companion heartbeat and retains the historical
/// `VSPARALLEL_CODE_COMMAND` behavior.
pub fn command_for_editor(editor: Option<EditorKind>) -> Option<String> {
    match editor {
        Some(EditorKind::Cursor) => Some(cursor_command()),
        Some(EditorKind::AntigravityIde) => Some(antigravity_ide_command()),
        Some(EditorKind::Zed) => Some(zed_command()),
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

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn default_zed_command() -> String {
    if let Some(home) = env::var_os("HOME") {
        for name in ["zed", "zeditor", "zedit", "zed-editor"] {
            let candidate = std::path::PathBuf::from(&home)
                .join(".local")
                .join("bin")
                .join(name);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    for root in ["/usr/local/bin", "/usr/bin"] {
        for name in ["zed", "zeditor", "zedit", "zed-editor"] {
            let candidate = std::path::PathBuf::from(root).join(name);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    "zed".to_string()
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

#[cfg(target_os = "macos")]
fn default_zed_command() -> String {
    for root in ["/Applications", "/System/Applications"] {
        let candidate = std::path::PathBuf::from(root)
            .join("Zed.app")
            .join("Contents")
            .join("MacOS")
            .join("cli");
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    if let Some(home) = env::var_os("HOME") {
        let candidate = std::path::PathBuf::from(home)
            .join("Applications")
            .join("Zed.app")
            .join("Contents")
            .join("MacOS")
            .join("cli");
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    "zed".to_string()
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

#[cfg(target_os = "windows")]
fn default_zed_command() -> String {
    let path_directories = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    let install_roots = [
        env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Programs").join("Zed")),
        env::var_os("ProgramFiles")
            .map(std::path::PathBuf::from)
            .map(|root| root.join("Zed")),
    ];
    find_windows_zed_executable(
        path_directories.iter().map(std::path::PathBuf::as_path),
        install_roots
            .iter()
            .flatten()
            .map(std::path::PathBuf::as_path),
    )
    .map(|path| path.to_string_lossy().into_owned())
    .unwrap_or_else(|| "zed.exe".to_string())
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

#[cfg(any(target_os = "windows", test))]
fn find_windows_zed_executable<'a>(
    path_directories: impl IntoIterator<Item = &'a Path>,
    install_roots: impl IntoIterator<Item = &'a Path>,
) -> Option<std::path::PathBuf> {
    const EXECUTABLE_NAMES: [&str; 3] = ["zed.exe", "Zed.exe", "zed-editor.exe"];
    for directory in path_directories {
        for name in EXECUTABLE_NAMES {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
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

#[cfg(test)]
fn open_with<L: WorkspaceLauncher>(
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
        .launch(
            OsStr::new(executable),
            std::slice::from_ref(&target.to_path_buf()),
            mode,
            None,
        )
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
#[cfg(test)]
pub fn open_editor_with<L: WorkspaceLauncher>(
    launcher: &L,
    editor: Option<EditorKind>,
    target: &Path,
    mode: WorkspaceLaunchMode,
) -> Result<String, String> {
    open_editor_targets_with(
        launcher,
        editor,
        std::slice::from_ref(&target.to_path_buf()),
        mode,
    )
}

/// Open one exact editor workspace. Zed multi-root workspaces are passed as
/// one ordered argument vector so reopening does not silently drop roots.
pub fn open_editor_targets_with<L: WorkspaceLauncher>(
    launcher: &L,
    editor: Option<EditorKind>,
    targets: &[PathBuf],
    mode: WorkspaceLaunchMode,
) -> Result<String, String> {
    let executable = command_for_editor(editor).ok_or_else(|| {
        let name = editor.map_or("This editor", EditorKind::display_name);
        format!("{name} does not expose a supported workspace launcher")
    })?;
    if executable.trim().is_empty() {
        return Err("the editor command is empty".to_string());
    }
    if targets.is_empty() || targets.len() > MAX_WORKSPACE_TARGETS {
        return Err("the workspace target list is empty or exceeds its safety bound".to_string());
    }
    if editor != Some(EditorKind::Zed) && targets.len() != 1 {
        return Err("this editor does not support a multi-root launch target".to_string());
    }
    for target in targets {
        if !target.is_absolute() {
            return Err("the workspace target is not an absolute local path".to_string());
        }
        if !target.exists() {
            return Err(format!(
                "the workspace target no longer exists: {}",
                target.display()
            ));
        }
    }
    launcher
        .launch(OsStr::new(&executable), targets, mode, editor)
        .map_err(|error| {
            let option = match (editor, mode) {
                (Some(EditorKind::Zed), WorkspaceLaunchMode::PreferExisting) => " --existing",
                (_, WorkspaceLaunchMode::PreferExisting) => "",
                (Some(EditorKind::Zed), WorkspaceLaunchMode::NewWindow) => " --new",
                (_, WorkspaceLaunchMode::NewWindow) => " --new-window",
            };
            let target = targets
                .first()
                .map(|target| target.display().to_string())
                .unwrap_or_else(|| "<missing>".to_string());
            let additional = targets.len().saturating_sub(1);
            let suffix = if additional == 0 {
                String::new()
            } else {
                format!(" (+{additional} roots)")
            };
            format!("could not start `{executable}{option} {target}{suffix}`: {error}")
        })?;
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
        call: Mutex<Option<(OsString, Vec<PathBuf>, WorkspaceLaunchMode)>>,
        failure: Mutex<Option<io::ErrorKind>>,
    }

    impl WorkspaceLauncher for RecordingLauncher {
        fn launch(
            &self,
            executable: &OsStr,
            targets: &[PathBuf],
            mode: WorkspaceLaunchMode,
            editor: Option<EditorKind>,
        ) -> io::Result<()> {
            if let Some(kind) = *self.failure.lock().unwrap() {
                return Err(io::Error::new(kind, "injected failure"));
            }
            let _ = editor;
            *self.call.lock().unwrap() = Some((executable.to_owned(), targets.to_vec(), mode));
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
        assert_eq!(call.1, vec![target]);
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
            (EditorKind::Zed, "zed", "Zed"),
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
        assert_eq!(
            command_for_editor(Some(EditorKind::Zed)),
            Some(zed_command())
        );
        assert_eq!(command_for_editor(None), Some(code_command()));
        assert_eq!(command_for_editor(Some(EditorKind::Antigravity2)), None);
    }

    #[test]
    fn standard_macos_bundle_clis_resolve_to_gui_entry_points() {
        let cases = [
            (
                Some(EditorKind::VsCode),
                "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                "/Applications/Visual Studio Code.app/Contents/MacOS/Electron",
            ),
            (
                None,
                "/Users/example/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                "/Users/example/Applications/Visual Studio Code.app/Contents/MacOS/Electron",
            ),
            (
                Some(EditorKind::Cursor),
                "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
                "/Applications/Cursor.app/Contents/MacOS/Cursor",
            ),
            (
                Some(EditorKind::AntigravityIde),
                "/Applications/Antigravity IDE.app/Contents/Resources/app/bin/antigravity-ide",
                "/Applications/Antigravity IDE.app/Contents/MacOS/Electron",
            ),
        ];

        for (editor, cli, expected) in cases {
            assert_eq!(
                standard_macos_editor_gui_executable(OsStr::new(cli), editor),
                Some(PathBuf::from(expected))
            );
        }
    }

    #[test]
    fn macos_gui_launch_uses_launch_services_without_node_mode() {
        let gui = Path::new("/Applications/Antigravity IDE.app/Contents/MacOS/Electron");
        let command = macos_gui_launch_command(gui);

        assert_eq!(command.get_program(), OsStr::new("/usr/bin/open"));
        assert_eq!(
            command.get_args().map(OsStr::to_owned).collect::<Vec<_>>(),
            vec![
                OsString::from("-n"),
                OsString::from("-a"),
                gui.as_os_str().to_owned(),
                OsString::from("--args"),
            ]
        );
        assert!(command.get_envs().any(|(name, value)| {
            name == OsStr::new("ELECTRON_RUN_AS_NODE") && value.is_none()
        }));
    }

    #[test]
    fn macos_gui_resolution_preserves_nonstandard_and_non_gui_launchers() {
        assert_eq!(
            standard_macos_editor_gui_executable(
                OsStr::new("/usr/local/bin/antigravity-ide"),
                Some(EditorKind::AntigravityIde),
            ),
            None
        );
        assert_eq!(
            standard_macos_editor_gui_executable(
                OsStr::new(
                    "/Applications/Custom.app/Contents/Resources/app/bin/custom-antigravity"
                ),
                Some(EditorKind::AntigravityIde),
            ),
            None
        );
        assert_eq!(
            standard_macos_editor_gui_executable(
                OsStr::new("/Applications/Cursor.app/Contents/Resources/app/bin/cursor"),
                Some(EditorKind::VsCode),
            ),
            None
        );
        assert_eq!(
            standard_macos_editor_gui_executable(
                OsStr::new("/Applications/Zed.app/Contents/MacOS/cli"),
                Some(EditorKind::Zed),
            ),
            None
        );
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
            launch_arguments(
                Some(EditorKind::VsCode),
                WorkspaceLaunchMode::PreferExisting,
                &[target.to_path_buf()]
            ),
            vec![target.as_os_str().to_owned()]
        );
    }

    #[test]
    fn new_window_passes_the_force_flag_before_the_exact_target() {
        let target = Path::new("/work/project with spaces");
        assert_eq!(
            launch_arguments(
                Some(EditorKind::VsCode),
                WorkspaceLaunchMode::NewWindow,
                &[target.to_path_buf()]
            ),
            vec![
                OsString::from("--new-window"),
                target.as_os_str().to_owned()
            ]
        );
    }

    #[test]
    fn zed_new_window_uses_its_documented_cli_flag() {
        let target = Path::new("/work/project with spaces");
        assert_eq!(
            launch_arguments(
                Some(EditorKind::Zed),
                WorkspaceLaunchMode::NewWindow,
                &[target.to_path_buf()]
            ),
            vec![OsString::from("--new"), target.as_os_str().to_owned()]
        );
    }

    #[test]
    fn zed_prefer_existing_uses_its_documented_cli_flag() {
        let target = Path::new("/work/project with spaces");
        assert_eq!(
            launch_arguments(
                Some(EditorKind::Zed),
                WorkspaceLaunchMode::PreferExisting,
                &[target.to_path_buf()]
            ),
            vec![OsString::from("--existing"), target.as_os_str().to_owned()]
        );
    }

    #[test]
    fn zed_multi_root_launch_preserves_saved_order() {
        let first = PathBuf::from("/work/first");
        let second = PathBuf::from("/work/second");
        assert_eq!(
            launch_arguments(
                Some(EditorKind::Zed),
                WorkspaceLaunchMode::NewWindow,
                &[first.clone(), second.clone()],
            ),
            vec![
                OsString::from("--new"),
                first.into_os_string(),
                second.into_os_string(),
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

    #[test]
    fn resolves_native_windows_zed_without_invoking_a_batch_shell() {
        let temp = TempDir::new().unwrap();
        let install_root = temp.path().join("Zed");
        std::fs::create_dir_all(&install_root).unwrap();
        let executable = install_root.join("zed.exe");
        std::fs::write(&executable, b"native executable").unwrap();

        assert_eq!(
            find_windows_zed_executable(std::iter::empty(), [install_root.as_path()]),
            Some(executable)
        );
    }
}
