//! Thin window over the daemon socket.
//!
//! Buttons and the status line call [`softwake_ipc`]. Provider Settings call
//! [`softwake_providers`] in this process. The General pane reads and writes
//! the soul pack through [`softwake_soul`] in this process. This crate does
//! not decide whether a voice-state transition is legal.

mod commands;
mod pack;
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
            providers::provider_set_base_url,
            providers::provider_clear_cred,
            providers::provider_oauth_start,
            providers::provider_oauth_poll,
            providers::provider_oauth_sign_out,
            providers::provider_test,
            providers::provider_set_model,
            pack::pack_snapshot,
            pack::pack_save,
        ])
        .run(tauri::generate_context!())
        .expect("softwake-ui failed to start");
}

#[cfg(test)]
mod tests {
    const COMMANDS: &[&str] = &[
        "status",
        "hibernate",
        "resume",
        "sleep",
        "reload_soul",
        "confirm_tool",
        "cancel_tool",
        "provider_snapshot",
        "provider_select",
        "provider_set_key",
        "provider_set_base_url",
        "provider_clear_cred",
        "provider_oauth_start",
        "provider_oauth_poll",
        "provider_oauth_sign_out",
        "provider_test",
        "provider_set_model",
        "pack_snapshot",
        "pack_save",
    ];

    const PERMISSIONS: &[&str] = &[
        "allow-status",
        "allow-hibernate",
        "allow-resume",
        "allow-sleep",
        "allow-reload-soul",
        "allow-confirm-tool",
        "allow-cancel-tool",
        "allow-provider-snapshot",
        "allow-provider-select",
        "allow-provider-set-key",
        "allow-provider-set-base-url",
        "allow-provider-clear-cred",
        "allow-provider-oauth-start",
        "allow-provider-oauth-poll",
        "allow-provider-oauth-sign-out",
        "allow-provider-test",
        "allow-provider-set-model",
        "allow-pack-snapshot",
        "allow-pack-save",
    ];

    #[test]
    fn capability_allows_window_commands() {
        let permissions = include_str!("../permissions/commands.toml");
        let capability = include_str!("../capabilities/default.json");
        for command in COMMANDS {
            let needle = format!("\"{command}\"");
            assert!(
                permissions.contains(needle.as_str()),
                "permissions missing {command}"
            );
        }
        for permission in PERMISSIONS {
            let needle = format!("\"{permission}\"");
            assert!(
                capability.contains(needle.as_str()),
                "capability missing {permission}"
            );
        }
    }
}
