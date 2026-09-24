//! Window commands. Each one opens the daemon socket, sends one command, and
//! returns the status or the daemon's error text.

use softwake_ipc::{Client, Command, Status, resolve_socket_path};

fn call(command: Command) -> Result<Status, String> {
    let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
    let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
    client.call(command).map_err(|error| error.to_string())
}

/// Current voice state.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn status() -> Result<Status, String> {
    call(Command::GetStatus)
}

/// Ask the daemon to hibernate.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn hibernate() -> Result<Status, String> {
    call(Command::Hibernate)
}

/// Ask the daemon to leave hibernate. The daemon lands in sleep.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn resume() -> Result<Status, String> {
    call(Command::WakeFromUi)
}

/// Ask the daemon to sleep from awake.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn sleep() -> Result<Status, String> {
    call(Command::Sleep)
}

/// Ask the daemon to record a soul reload for the next awake session.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn reload_soul() -> Result<Status, String> {
    call(Command::ReloadSoul)
}
