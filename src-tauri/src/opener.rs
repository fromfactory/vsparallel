use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::Command;
use std::thread;

pub const CODE_COMMAND_ENV: &str = "VSPARALLEL_CODE_COMMAND";

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
    if let Some(configured) = env::var(CODE_COMMAND_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return configured;
    }

    default_code_command()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn default_code_command() -> String {
    "code".to_string()
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
}
