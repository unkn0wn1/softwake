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

/// Ask the daemon to re-read the soul pack. It applies on the next awake.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn reload_soul() -> Result<Status, String> {
    call(Command::ReloadSoul)
}

/// Run the pending confirm-gated tool.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes this command argument as an owned String"
)]
pub fn confirm_tool(pending_id: String) -> Result<Status, String> {
    let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
    let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
    client
        .confirm_tool(&pending_id)
        .map_err(|error| error.to_string())
}

/// Drop the pending confirmation without running the tool.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes this command argument as an owned String"
)]
pub fn cancel_tool(pending_id: String) -> Result<Status, String> {
    let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
    let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
    client
        .cancel_tool(&pending_id)
        .map_err(|error| error.to_string())
}
