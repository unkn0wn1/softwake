//! Softwake settings window, system tray, and always-on-top HUD.
//!
//! Buttons and the status line call [`softwake_ipc`]. Provider Settings call
//! [`softwake_providers`] in this process. The General pane reads and writes
//! the soul pack through [`softwake_soul`] in this process. This crate does
//! not decide whether a voice-state transition is legal.

mod commands;
mod oauth_open;
mod pack;
mod providers;
mod tray;

use tauri::{WebviewUrl, WebviewWindowBuilder, WindowEvent};

/// Open the Softwake tray, HUD capsule, and Settings window.
///
/// # Panics
///
/// Panics when the window runtime cannot start. That ends the process; it is
/// not a voice-state error.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::status,
            commands::hibernate,
            commands::resume,
            commands::sleep,
            commands::reload_soul,
            commands::confirm_tool,
            commands::cancel_tool,
            commands::wake,
            commands::ask,
            commands::hud_snapshot,
            commands::hud_ask,
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
            providers::provider_set_voice_model,
            providers::provider_opt_in_plaintext,
            pack::pack_snapshot,
            pack::pack_save,
        ])
        .setup(|app| {
            tray::install(app.handle())?;
            open_hud(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Keep the tray + HUD alive; Settings can reopen from the tray.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("softwake-ui failed to start");
}

fn open_hud(app: &tauri::AppHandle) -> tauri::Result<()> {
    const HUD_W: f64 = 320.0;
    const HUD_H: f64 = 120.0;
    const MARGIN: f64 = 16.0;

    let mut builder = WebviewWindowBuilder::new(app, "hud", WebviewUrl::App("hud.html".into()))
        .title("Softwake")
        .inner_size(HUD_W, HUD_H)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .visible(true)
        .focused(false);

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let size = monitor.size();
        let scale = monitor.scale_factor();
        let work_w = f64::from(size.width) / scale;
        let x = work_w - HUD_W - MARGIN;
        let y = MARGIN;
        builder = builder.position(x, y);
    }

    builder.build()?;
    Ok(())
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
        "wake",
        "ask",
        "hud_snapshot",
        "hud_ask",
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
        "provider_set_voice_model",
        "provider_opt_in_plaintext",
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
        "allow-wake",
        "allow-ask",
        "allow-hud-snapshot",
        "allow-hud-ask",
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
        "allow-provider-set-voice-model",
        "allow-provider-opt-in-plaintext",
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
        assert!(
            capability.contains("\"hud\""),
            "capability missing hud window"
        );
        assert!(
            capability.contains("opener:allow-open-url"),
            "capability missing opener:allow-open-url"
        );
        assert!(
            capability.contains("https://auth.x.ai"),
            "capability missing https://auth.x.ai"
        );
        assert!(
            capability.contains("https://auth.x.ai/*"),
            "capability missing https://auth.x.ai/*"
        );
        assert!(
            !capability.contains("opener:default"),
            "capability must not grant opener:default"
        );
        assert!(
            !capability.contains("allow-open-path"),
            "capability must not grant allow-open-path"
        );
    }
}
