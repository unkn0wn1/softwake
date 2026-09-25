//! Softwake settings window, system tray, and always-on-top HUD.
//!
//! Buttons and the status line call [`softwake_ipc`]. Provider Settings call
//! [`softwake_providers`] in this process. Email Settings call
//! [`softwake_connectors`] and the secret bag in this process. The Profiles pane reads and writes
//! the soul pack through [`softwake_soul`] in this process. This crate does
//! not decide whether a voice-state transition is legal.

mod commands;
mod email;
mod oauth_open;
mod pack;
mod profiles;
mod providers;
mod tray;

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

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
            commands::hud_talk_start,
            commands::hud_talk_stop,
            commands::hud_set_layout,
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
            providers::provider_set_tts_voice,
            providers::provider_opt_in_plaintext,
            email::email_snapshot,
            email::email_save,
            email::email_clear_password,
            email::email_test,
            profiles::profiles_snapshot,
            profiles::profile_create,
            profiles::profile_rename,
            profiles::profile_set_active,
            profiles::pack_snapshot,
            profiles::pack_save,
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

/// Collapsed HUD: bloom capsule only.
const HUD_COLLAPSED_W: f64 = 220.0;
const HUD_COLLAPSED_H: f64 = 96.0;
/// Expanded HUD: bloom + type strip + reply.
const HUD_EXPANDED_W: f64 = 320.0;
const HUD_EXPANDED_H: f64 = 176.0;
const HUD_MARGIN: f64 = 16.0;

fn hud_logical_size(expanded: bool) -> (f64, f64) {
    if expanded {
        (HUD_EXPANDED_W, HUD_EXPANDED_H)
    } else {
        (HUD_COLLAPSED_W, HUD_COLLAPSED_H)
    }
}

/// Bottom-right of the primary monitor work area (falls back to full monitor).
///
/// Uses [`tauri::Monitor::position`] / work area so multi-monitor layouts do not
/// place the capsule on the wrong screen when primary is not at `(0, 0)`.
fn primary_bottom_right(app: &AppHandle, width: f64, height: f64) -> Option<(f64, f64)> {
    let monitor = app.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let origin_x = f64::from(area.position.x) / scale;
    let origin_y = f64::from(area.position.y) / scale;
    let work_w = f64::from(area.size.width) / scale;
    let work_h = f64::from(area.size.height) / scale;
    let x = origin_x + work_w - width - HUD_MARGIN;
    let y = origin_y + work_h - height - HUD_MARGIN;
    Some((x, y))
}

/// Resize the HUD and re-anchor to the primary bottom-right corner.
pub(crate) fn set_hud_layout(app: &AppHandle, expanded: bool) -> Result<(), String> {
    let (width, height) = hud_logical_size(expanded);
    let window = app
        .get_webview_window("hud")
        .ok_or_else(|| "HUD window is not open".to_owned())?;
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| error.to_string())?;
    if let Some((x, y)) = primary_bottom_right(app, width, height) {
        window
            .set_position(LogicalPosition::new(x, y))
            .map_err(|error| error.to_string())?;
    }
    let _ = window.set_always_on_top(true);
    Ok(())
}

fn open_hud(app: &AppHandle) -> tauri::Result<()> {
    let (width, height) = hud_logical_size(false);
    let mut builder = WebviewWindowBuilder::new(app, "hud", WebviewUrl::App("hud.html".into()))
        .title("Softwake")
        .inner_size(width, height)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .visible(true)
        .focused(false);

    if let Some((x, y)) = primary_bottom_right(app, width, height) {
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
        "hud_talk_start",
        "hud_talk_stop",
        "hud_set_layout",
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
        "provider_set_tts_voice",
        "provider_opt_in_plaintext",
        "email_snapshot",
        "email_save",
        "email_clear_password",
        "email_test",
        "profiles_snapshot",
        "profile_create",
        "profile_rename",
        "profile_set_active",
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
        "allow-hud-talk-start",
        "allow-hud-talk-stop",
        "allow-hud-set-layout",
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
        "allow-provider-set-tts-voice",
        "allow-provider-opt-in-plaintext",
        "allow-email-snapshot",
        "allow-email-save",
        "allow-email-clear-password",
        "allow-email-test",
        "allow-profiles-snapshot",
        "allow-profile-create",
        "allow-profile-rename",
        "allow-profile-set-active",
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
