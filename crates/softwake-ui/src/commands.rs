//! Window commands. Each one opens the daemon socket, sends one command, and
//! returns the status or the daemon's error text.

use serde::Serialize;
use softwake_ipc::{Client, Command, Status, VoiceState, resolve_socket_path};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

fn call(command: Command) -> Result<Status, String> {
    let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
    let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
    client.call(command).map_err(|error| error.to_string())
}

fn connect() -> Result<Client, String> {
    let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
    Client::connect(&path).map_err(|error| error.to_string())
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
    let mut client = connect()?;
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
    let mut client = connect()?;
    client
        .cancel_tool(&pending_id)
        .map_err(|error| error.to_string())
}

/// Enter awake from sleep when the soul pack is valid (`ctl wake`).
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn wake() -> Result<Status, String> {
    let mut client = connect()?;
    client.call_wake().map_err(|error| error.to_string())
}

/// Send one typed ask while awake. Assistant text is [`Status::message`].
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes this command argument as an owned String"
)]
pub fn ask(text: String) -> Result<Status, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("ask text is blank".to_owned());
    }
    let mut client = connect()?;
    client.call_ask(trimmed).map_err(|error| error.to_string())
}

/// HUD particle level plus voice state.
///
/// Prefers [`Status::capture_level`] from the daemon when PCM was scored.
/// Falls back to a local sine while capture runs and no level is on the wire
/// ([ADR 0016](../../docs/ADR-0016-capture-level-hud.md)).
#[derive(Debug, Clone, Serialize)]
pub struct HudSnapshot {
    /// Voice state spelling: sleep, awake, or hibernate.
    pub state: String,
    /// Whether capture is running.
    pub capture_running: bool,
    /// Level in `0.0..=1.0` for particle bloom.
    pub level: f64,
    /// `true` when the UI sine fallback is in use; `false` when the daemon
    /// supplied [`Status::capture_level`].
    pub level_mocked: bool,
}

/// Status plus a listening level for the HUD capsule.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn hud_snapshot() -> Result<HudSnapshot, String> {
    let status = call(Command::GetStatus)?;
    let (level, level_mocked) = match status.capture_level {
        Some(level) => (f64::from(level).clamp(0.0, 1.0), false),
        None => (mock_level(status.capture_running), true),
    };
    Ok(HudSnapshot {
        state: status.state.as_str().to_owned(),
        capture_running: status.capture_running,
        level,
        level_mocked,
    })
}

fn mock_level(capture_running: bool) -> f64 {
    if !capture_running {
        return 0.02;
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    // Slow sine so blooms visibly breathe while listening.
    let wave = (secs * 1.7).sin();
    (0.28 + 0.55 * (0.5 + 0.5 * wave)).clamp(0.05, 0.95)
}

/// Arm press-to-talk. From sleep this wakes first. Hibernate is refused.
///
/// Runs off the UI thread so a slow wake does not freeze the HUD chrome.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub async fn hud_talk_start() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut client = connect()?;
        client.call_talk_start().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("talk start task failed: {error}"))?
}

/// Release press-to-talk, transcribe, ask, and speak when Eve is configured.
///
/// The socket read timeout is raised for this call because STT, ask, and TTS
/// share one round trip. The daemon still bounds each HTTP call. Work runs on
/// a blocking pool so the HUD can paint a released mic button and a thinking
/// line while the round trip is in flight.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub async fn hud_talk_stop() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut client = connect()?;
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(90)))
            .map_err(|error| error.to_string())?;
        client.call_talk_stop().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("talk stop task failed: {error}"))?
}

/// Submit from the HUD: ask while awake; wake then ask from sleep; refuse hibernate.
///
/// # Errors
///
/// Returns the daemon or socket error as text, or a clear operator sentence.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes this command argument as an owned String"
)]
pub async fn hud_ask(text: String) -> Result<Status, String> {
    let trimmed = text.trim().to_owned();
    if trimmed.is_empty() {
        return Err("ask text is blank".to_owned());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut client = connect()?;
        let status = client
            .call(Command::GetStatus)
            .map_err(|error| error.to_string())?;
        match status.state {
            VoiceState::Awake => client.call_ask(&trimmed).map_err(|error| error.to_string()),
            VoiceState::Sleep => {
                client.call_wake().map_err(|error| error.to_string())?;
                client.call_ask(&trimmed).map_err(|error| error.to_string())
            }
            VoiceState::Hibernate => Err(
                "Softwake is hibernating — leave hibernate from Settings (Wake) or the tray first"
                    .to_owned(),
            ),
        }
    })
    .await
    .map_err(|error| format!("ask task failed: {error}"))?
}

/// Resize and re-anchor the HUD capsule (collapsed bloom vs expanded ask strip).
///
/// # Errors
///
/// Returns a sentence when the HUD window is missing or the window API fails.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects an owned AppHandle into commands that touch windows"
)]
pub fn hud_set_layout(app: tauri::AppHandle, expanded: bool) -> Result<(), String> {
    crate::set_hud_layout(&app, expanded)
}

/// Begin a native window drag for the HUD capsule.
///
/// # Errors
///
/// Returns a sentence when the HUD window is missing or the drag API fails.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects an owned AppHandle into commands that touch windows"
)]
pub fn hud_start_drag(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("hud")
        .ok_or_else(|| "HUD window is not open".to_owned())?;
    window.start_dragging().map_err(|error| error.to_string())
}

/// Persist the current HUD top-left so expand/collapse stops re-anchoring to BR.
///
/// # Errors
///
/// Returns a sentence when the window or config write fails.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects an owned AppHandle into commands that touch windows"
)]
pub fn hud_save_position(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("hud")
        .ok_or_else(|| "HUD window is not open".to_owned())?;
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let scale = window.scale_factor().map_err(|error| error.to_string())?;
    let logical = position.to_logical::<f64>(scale);
    crate::hud_pos::save(crate::hud_pos::HudPosition {
        x: logical.x,
        y: logical.y,
    })?;
    crate::assert_hud_on_top(&app);
    Ok(())
}

/// Clear a saved HUD placement and park at primary bottom-right again.
///
/// # Errors
///
/// Returns a sentence when clear or re-layout fails.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects an owned AppHandle into commands that touch windows"
)]
pub fn hud_reset_position(app: tauri::AppHandle) -> Result<(), String> {
    crate::hud_pos::clear()?;
    crate::set_hud_layout(&app, false)
}
