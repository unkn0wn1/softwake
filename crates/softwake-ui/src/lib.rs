//! Thin window over the daemon socket.
//!
//! Buttons and the status line call [`softwake_ipc`]. Provider Settings call
//! [`softwake_providers`] in this process. This crate does not decide whether
//! a voice-state transition is legal.

mod commands;
mod providers;

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
            providers::provider_snapshot,
            providers::provider_select,
            providers::provider_set_key,
            providers::provider_clear_cred,
            providers::provider_oauth_start,
            providers::provider_oauth_poll,
            providers::provider_oauth_sign_out,
            providers::provider_test,
            providers::provider_set_model,
        ])
        .run(tauri::generate_context!())
        .expect("softwake-ui failed to start");
}
