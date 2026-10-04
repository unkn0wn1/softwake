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

/// Turn voice test mode on or off for the running daemon.
///
/// Phrases and the short state voice still run. Microphone speech is not sent
/// to chat. The flag clears when that daemon process exits.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub fn set_voice_test(enabled: bool) -> Result<Status, String> {
    let mut client = connect()?;
    client
        .set_voice_test(enabled)
        .map_err(|error| error.to_string())
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

/// Show or focus the Settings window. Same path as the tray Settings item.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects an owned AppHandle into commands that touch windows"
)]
pub fn show_settings(app: tauri::AppHandle) {
    crate::tray::show_settings(&app);
}

/// Turn 2-way listening on or off.
///
/// ON enters awake (from hibernate: Resume to sleep, then wake). OFF enters
/// sleep and does **not** hibernate, so the wake word can still fire and a
/// typed HUD ask can wake. The choice is stored in `ui-prefs.json` `two_way`.
///
/// # Errors
///
/// Returns the daemon or socket error as text. A failed wake does not store ON.
#[tauri::command]
pub fn hud_set_two_way(enabled: bool) -> Result<Status, String> {
    let mut client = connect()?;
    let status = client
        .call(Command::GetStatus)
        .map_err(|error| error.to_string())?;
    let next = if enabled {
        match status.state {
            VoiceState::Awake => status,
            VoiceState::Hibernate => {
                let slept = client
                    .call(Command::WakeFromUi)
                    .map_err(|error| error.to_string())?;
                if slept.state == VoiceState::Awake {
                    slept
                } else {
                    client.call_wake().map_err(|error| error.to_string())?
                }
            }
            VoiceState::Sleep => client.call_wake().map_err(|error| error.to_string())?,
        }
    } else if status.state == VoiceState::Awake {
        client
            .call(Command::Sleep)
            .map_err(|error| error.to_string())?
    } else {
        // Already sleep, or hibernate. Do not leave hibernate just to sleep.
        status
    };
    let mut prefs = crate::ui_prefs::load();
    prefs.two_way = enabled;
    crate::ui_prefs::save(&prefs)?;
    Ok(next)
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "HUD wire mirrors Status talking/auto_listening flags"
)]
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
    /// Last retained voice/ask line (PTT, typed ask, or free speech).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Short detail (speech note, free speech, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Press-to-talk armed.
    pub talking: bool,
    /// Awake energy-gated listen without holding PTT.
    pub auto_listening: bool,
    /// Operator mic mute latch.
    pub mic_muted: bool,
    /// Turn phase wire value (`listening` / `thinking` / `calling_tools` / `speaking` / `awaiting_approve`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Daemon build stamp when the daemon reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    /// Confirm-gated tool waiting for Approve or Deny. Omitted when nothing is waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_tool: Option<softwake_ipc::PendingTool>,
    /// Estimated awake context tokens (char/4). Omitted when asleep / unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_used: Option<u32>,
    /// Resolved context window limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u32>,
    /// Auto-compact threshold percent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_compact_at: Option<u8>,
    /// True when the latest ask compacted older turns.
    pub context_compacted: bool,
    /// Operator speech for the open profile chat. Omitted when none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator_said: Option<String>,
    /// Generation of `operator_said`. `0` means none.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub operator_said_seq: u64,
}

/// UI (+ optional daemon) build stamp for Settings / HUD chrome.
#[derive(Debug, Clone, Serialize)]
pub struct AppBuildInfo {
    /// Crate version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// Short git sha baked at compile time.
    pub git_sha: String,
    /// Local build timestamp string.
    pub built_at: String,
    /// One-line label: `0.1.0 · abc1234 · stamp`.
    pub label: String,
}

/// Softwake UI build stamp (always available; no daemon required).
#[tauri::command]
pub fn app_build_info() -> AppBuildInfo {
    let version = env!("SOFTWAKE_UI_VERSION").to_owned();
    let git_sha = env!("SOFTWAKE_UI_GIT_SHA").to_owned();
    let built_at = env!("SOFTWAKE_UI_BUILT_AT").to_owned();
    let label = format!("{version} · {git_sha} · {built_at}");
    AppBuildInfo {
        version,
        git_sha,
        built_at,
        label,
    }
}

/// Status plus a listening level for the HUD capsule.
///
/// Runs on the blocking pool so a contended daemon lock (STT/ask/TTS holding
/// `Runtime`) cannot freeze the Tauri main thread and stall bloom rAF.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub async fn hud_snapshot() -> Result<HudSnapshot, String> {
    tauri::async_runtime::spawn_blocking(|| {
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
            message: status.message,
            detail: status.detail,
            talking: status.talking,
            auto_listening: status.auto_listening,
            mic_muted: status.mic_muted,
            phase: status.phase.clone(),
            build: status.build.clone(),
            pending_tool: status.pending_tool,
            context_used: status.context_used,
            context_limit: status.context_limit,
            context_compact_at: status.context_compact_at,
            context_compacted: status.context_compacted,
            operator_said: status.operator_said,
            operator_said_seq: status.operator_said_seq,
        })
    })
    .await
    .map_err(|error| format!("hud snapshot task failed: {error}"))?
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
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

/// Socket wait for one typed HUD ask.
///
/// The daemon chat budget is 120s, and speech synthesis uses that same budget
/// before the reply is returned. This waits for both, plus a short wake.
const HUD_ASK_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(270);

/// Socket wait for press-to-talk release.
///
/// Speech-to-text, chat, and speech synthesis each use the 120s daemon budget
/// before this round trip returns.
const HUD_TALK_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(390);

/// Tell the daemon which HUD room composer is open.
///
/// `None` or blank returns finished speech to the active profile. Does not
/// restart the Voice Agent session.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub async fn hud_set_open_room(room_id: Option<String>) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut client = connect()?;
        client
            .call_set_open_room(room_id.as_deref())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("set open room task failed: {error}"))?
}

/// Release press-to-talk, transcribe, ask, and speak when Eve is configured.
///
/// When a room composer is open the daemon posts the transcript to that room
/// instead of asking the active profile. The socket read timeout is raised
/// for this call because STT and ask share one round trip and each may take
/// the full daemon chat budget. Work runs on a blocking pool so the HUD can
/// paint a released mic button and a thinking line while the round trip is
/// in flight.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub async fn hud_talk_stop() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut client = connect()?;
        client
            .set_read_timeout(Some(HUD_TALK_READ_TIMEOUT))
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
        client
            .set_read_timeout(Some(HUD_ASK_READ_TIMEOUT))
            .map_err(|error| error.to_string())?;
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
                "Softwake is hibernating — leave hibernate from Settings (Resume) first".to_owned(),
            ),
        }
    })
    .await
    .map_err(|error| format!("ask task failed: {error}"))?
}

/// Abort an in-flight ask / chat stream (Escape or Cancel).
///
/// Sets the daemon cancel flag without waiting for the ask lock.
///
/// # Errors
///
/// Returns the daemon or socket error as text.
#[tauri::command]
pub async fn hud_cancel_ask() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let mut client = connect()?;
        client.call_cancel_ask().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("cancel ask task failed: {error}"))?
}

/// Mute or unmute mic listening while keeping typed ask.
///
/// Persists the preference in ui-prefs and tells the daemon via `SetMicMute`.
///
/// # Errors
///
/// Returns the daemon or prefs error as text.
#[tauri::command]
pub async fn hud_set_mic_mute(muted: bool) -> Result<Status, String> {
    let _ = crate::ui_prefs::ui_prefs_set_hud_mic_muted(muted)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut client = connect()?;
        client
            .set_mic_mute(muted)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("mic mute task failed: {error}"))?
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

/// Re-read `hud_shrunk_px` and resize only when the HUD is already the shrunk square.
///
/// An expanded main window is left alone. The next shrink reads the new size.
///
/// # Errors
///
/// Returns a sentence when the HUD window is missing or the window API fails.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects an owned AppHandle into commands that touch windows"
)]
pub fn hud_apply_shrunk_size(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("hud")
        .ok_or_else(|| "HUD window is not open".to_owned())?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let size = window
        .inner_size()
        .map_err(|error| error.to_string())?
        .to_logical::<f64>(scale);
    let shrunk_max = f64::from(crate::ui_prefs::HUD_SHRUNK_PX_MAX) + 12.0;
    if size.width <= shrunk_max && size.height <= shrunk_max {
        crate::set_hud_layout(&app, false)?;
    }
    Ok(())
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

/// Persist the current expanded HUD size into ui-prefs.json.
///
/// # Errors
///
/// Returns a sentence when the window or config write fails.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri injects an owned AppHandle into commands that touch windows"
)]
pub fn hud_save_size(app: tauri::AppHandle) -> Result<(), String> {
    crate::save_hud_size(&app)
}

/// Push HUD turns into the awake model session (encrypted-history path).
///
/// No-op on the daemon when the session already has messages. Softwaked also
/// loads plaintext `hud-chat.json` itself on wake.
///
/// # Errors
///
/// Daemon refusal or IPC failure.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri owns JSON args.
pub fn hud_seed_session(turns: Vec<softwake_ipc::SeedChatTurn>) -> Result<String, String> {
    let mut client = connect()?;
    let status = client
        .call_seed_chat(turns)
        .map_err(|error| error.to_string())?;
    Ok(status.message.unwrap_or_else(|| "seeded".to_owned()))
}

/// Best-effort trim matching turns from the open model session after HUD delete.
#[tauri::command]
pub fn hud_drop_session_turns(turns: Vec<softwake_ipc::SeedChatTurn>) -> Result<String, String> {
    let mut client = connect()?;
    let status = client
        .call_drop_chat_turns(turns)
        .map_err(|error| error.to_string())?;
    Ok(status
        .message
        .unwrap_or_else(|| "session turns updated".to_owned()))
}
