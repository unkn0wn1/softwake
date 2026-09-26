//! Start `softwaked serve` when the UI opens and nothing is listening.
//!
//! Packaged builds place `softwaked` next to `softwake-ui` (Tauri
//! `externalBin`). A systemd --user unit from `scripts/install-linux.sh`
//! already keeps the daemon up; this path is for the `AppImage`, the Windows
//! installer/portable layout, and a bare `softwake-ui` on PATH.
//!
//! Set `SOFTWAKE_NO_AUTOSTART=1` to skip spawning (tests / operators who
//! manage the daemon themselves).

use softwake_ipc::{Client, resolve_socket_path};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// Try to connect; if that fails, spawn `softwaked serve` from a sibling
/// binary (or PATH) and retry briefly.
pub fn ensure_daemon_running() {
    if std::env::var_os("SOFTWAKE_NO_AUTOSTART").is_some() {
        return;
    }
    if daemon_is_up() {
        return;
    }
    let Some(bin) = find_softwaked() else {
        eprintln!(
            "softwake-ui: softwaked not found next to this binary or on PATH; start softwaked serve first"
        );
        return;
    };
    match spawn_serve(&bin) {
        Ok(()) => wait_until_up(),
        Err(error) => eprintln!("softwake-ui: failed to start {}: {error}", bin.display()),
    }
}

fn daemon_is_up() -> bool {
    let Ok(path) = resolve_socket_path(None) else {
        return false;
    };
    Client::connect(&path).is_ok()
}

fn wait_until_up() {
    for _ in 0..50 {
        if daemon_is_up() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("softwake-ui: softwaked serve was started but is not accepting clients yet");
}

fn spawn_serve(bin: &Path) -> std::io::Result<()> {
    Command::new(bin)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

fn find_softwaked() -> Option<PathBuf> {
    let name = softwaked_file_name();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
            // Some AppImage layouts keep helpers one directory up from a nested bin.
            if let Some(parent) = dir.parent() {
                let nested = parent.join("bin").join(name);
                if is_executable_file(&nested) {
                    return Some(nested);
                }
                let sibling = parent.join(name);
                if is_executable_file(&sibling) {
                    return Some(sibling);
                }
            }
        }
    }
    which_on_path(name)
}

fn softwaked_file_name() -> &'static str {
    if cfg!(windows) {
        "softwaked.exe"
    } else {
        "softwaked"
    }
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::softwaked_file_name;

    #[test]
    fn softwaked_name_matches_platform() {
        let name = softwaked_file_name();
        assert!(name.starts_with("softwaked"));
    }
}
