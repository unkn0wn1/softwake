//! Softwake settings window, system tray, and always-on-top HUD.
//!
//! Buttons and the status line call [`softwake_ipc`]. Provider Settings call
//! [`softwake_providers`] in this process. Email Settings call
//! [`softwake_connectors`] and the secret bag in this process. The Profiles pane reads and writes
//! the soul pack through [`softwake_soul`] in this process. This crate does
//! not decide whether a voice-state transition is legal.

mod commands;
mod email;
mod hud_pos;
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
            commands::hud_start_drag,
            commands::hud_save_position,
            commands::hud_reset_position,
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
            match window.label() {
                "main" => {
                    match event {
                        WindowEvent::CloseRequested { api, .. } => {
                            // Keep the tray + HUD alive; Settings can reopen from the tray.
                            api.prevent_close();
                            let _ = window.hide();
                        }
                        WindowEvent::Focused(true) | WindowEvent::Moved(_) => {
                            // Opening Settings must not leave a centred HUD.
                            reassert_hud_placement(window.app_handle());
                        }
                        _ => {}
                    }
                }
                "hud" => {
                    if matches!(event, WindowEvent::Focused(true) | WindowEvent::Moved(_)) {
                        assert_hud_on_top(window.app_handle());
                    }
                }
                _ => {}
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

/// Bottom-right of a monitor work area (logical pixels).
fn bottom_right_on(monitor: &tauri::Monitor, width: f64, height: f64) -> (f64, f64) {
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let origin_x = f64::from(area.position.x) / scale;
    let origin_y = f64::from(area.position.y) / scale;
    let work_w = f64::from(area.size.width) / scale;
    let work_h = f64::from(area.size.height) / scale;
    let x = origin_x + work_w - width - HUD_MARGIN;
    let y = origin_y + work_h - height - HUD_MARGIN;
    (x, y)
}

/// Primary bottom-right, else first monitor, else a coarse fallback (not center).
fn default_hud_position<R: tauri::Runtime>(
    app: &AppHandle<R>,
    width: f64,
    height: f64,
) -> (f64, f64) {
    if let Ok(Some(monitor)) = app.primary_monitor() {
        return bottom_right_on(&monitor, width, height);
    }
    if let Ok(monitors) = app.available_monitors() {
        if let Some(monitor) = monitors.first() {
            return bottom_right_on(monitor, width, height);
        }
    }
    // Last resort: park away from the typical centered default.
    (HUD_MARGIN * 4.0, HUD_MARGIN * 4.0)
}

/// Keep the capsule above Settings and other normal windows.
pub(crate) fn assert_hud_on_top<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("hud") {
        let _ = window.set_always_on_top(true);
        let _ = window.unminimize();
        // show() is a no-op when already visible; helps after focus races on Linux.
        let _ = window.show();
    }
}

/// Resize the HUD. Re-anchor to primary BR only when the operator has not dragged it.
pub(crate) fn set_hud_layout<R: tauri::Runtime>(
    app: &AppHandle<R>,
    expanded: bool,
) -> Result<(), String> {
    let (width, height) = hud_logical_size(expanded);
    let window = app
        .get_webview_window("hud")
        .ok_or_else(|| "HUD window is not open".to_owned())?;
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| error.to_string())?;
    if hud_pos::load().is_none() {
        let (x, y) = default_hud_position(app, width, height);
        window
            .set_position(LogicalPosition::new(x, y))
            .map_err(|error| error.to_string())?;
    }
    assert_hud_on_top(app);
    Ok(())
}

fn open_hud<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let (width, height) = hud_logical_size(false);
    let (x, y) = match hud_pos::load() {
        Some(pos) => (pos.x, pos.y),
        None => default_hud_position(app, width, height),
    };
    let builder = WebviewWindowBuilder::new(app, "hud", WebviewUrl::App("hud.html".into()))
        .title("Softwake")
        .inner_size(width, height)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .transparent(true)
        .visible(true)
        .focused(false)
        .position(x, y);

    let window = builder.build()?;
    // Wayland / some compositors ignore builder.position — force after create.
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.set_position(LogicalPosition::new(x, y));
    let _ = window.set_always_on_top(true);
    let _ = window.show();
    reassert_hud_placement(app);
    // Second pass after the compositor maps the window (avoids centred spawn
    // over Settings on Linux).
    let delayed = app.clone();
    let _ = std::thread::Builder::new()
        .name("softwake-hud-place".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            reassert_hud_placement(&delayed);
            std::thread::sleep(std::time::Duration::from_millis(400));
            reassert_hud_placement(&delayed);
        });
    Ok(())
}

/// Park at saved coords or primary bottom-right. Never leave a centred default.
pub(crate) fn reassert_hud_placement<R: tauri::Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window("hud") else {
        return;
    };
    let (width, height) = window
        .inner_size()
        .ok()
        .and_then(|size| {
            let scale = window.scale_factor().ok()?;
            let logical = size.to_logical::<f64>(scale);
            Some((logical.width, logical.height))
        })
        .unwrap_or_else(|| hud_logical_size(false));
    let (x, y) = match hud_pos::load() {
        Some(pos) => (pos.x, pos.y),
        None => default_hud_position(app, width, height),
    };
    let _ = window.set_position(LogicalPosition::new(x, y));
    assert_hud_on_top(app);
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
        "hud_start_drag",
        "hud_save_position",
        "hud_reset_position",
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
        "allow-hud-start-drag",
        "allow-hud-save-position",
        "allow-hud-reset-position",
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
