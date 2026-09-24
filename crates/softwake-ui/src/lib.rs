//! Thin window over the daemon socket.
//!
//! Buttons and the status line call [`softwake_ipc`]. This crate does not
//! decide whether a transition is legal.

mod commands;

/// Open the Softwake window.
///
/// # Panics
///
/// Panics when the window runtime cannot start. That ends the process; it is
/// not a voice-state error.
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::hibernate,
            commands::resume,
            commands::sleep,
            commands::reload_soul,
            commands::confirm_tool,
            commands::cancel_tool,
        ])
        .run(tauri::generate_context!())
        .expect("softwake-ui failed to start");
}
