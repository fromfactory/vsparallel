#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    match std::env::args_os().nth(1).as_deref() {
        Some(argument) if argument == std::ffi::OsStr::new("codex-hook") => {
            std::process::exit(vsparallel_lib::run_codex_hook_stdio());
        }
        Some(argument) if argument == std::ffi::OsStr::new("claude-hook") => {
            std::process::exit(vsparallel_lib::run_claude_hook_stdio());
        }
        _ => {}
    }
    vsparallel_lib::run();
}
