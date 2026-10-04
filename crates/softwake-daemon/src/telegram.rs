//! Telegram Bot API long-poll + outbound (Messengers).
//!
//! Live HTTPS requires the `live-http` feature. See ADR-0029.

#![cfg_attr(not(feature = "live-http"), allow(dead_code))]

use softwake_providers::{open_store, resolve_secrets_file};
use softwake_soul::{load_profile_meta, profile_pack_dir, resolve_config_dir};
use softwake_tools::{
    ChannelFlags, load_messengers, resolve_messengers_file, save_messengers, wants_ask_fanout,
    wants_timer_push,
};

const API_ROOT: &str = "https://api.telegram.org";
/// Long-poll timeout passed to Telegram `getUpdates`.
pub(crate) const POLL_TIMEOUT_SECS: u64 = 25;

/// Pure decision: should the laptop long-poll Telegram right now?
///
/// - No token → false (caller also checks).
/// - No enabled companion → true (solo laptop).
/// - Companion enabled → only when voice is Awake (presence=present).
#[must_use]
pub(crate) fn laptop_should_own_telegram(
    companion_enabled: bool,
    voice: softwake_state::VoiceState,
) -> bool {
    if !companion_enabled {
        return true;
    }
    matches!(voice, softwake_state::VoiceState::Awake)
}

/// Load the bot token from the secret bag (never logged).
pub(crate) fn load_bot_token() -> Option<String> {
    let path = resolve_secrets_file().ok()?;
    let store = open_store(&path).ok()?;
    let bag = store.load().ok()?;
    bag.telegram_bot_token
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().to_owned())
}

/// One long-poll cycle. Returns updates processed (0 when idle / no token).
///
/// `ask` runs under the caller's runtime lock (may block on provider HTTP).
pub(crate) fn poll_once(ask: &mut dyn FnMut(&str) -> Result<String, String>) -> usize {
    #[cfg(feature = "live-http")]
    {
        poll_once_live(ask)
    }
    #[cfg(not(feature = "live-http"))]
    {
        let _ = ask;
        0
    }
}

#[cfg(feature = "live-http")]
fn poll_once_live(ask: &mut dyn FnMut(&str) -> Result<String, String>) -> usize {
    use std::sync::atomic::{AtomicI64, Ordering};
    static OFFSET: AtomicI64 = AtomicI64::new(0);

    let Some(token) = load_bot_token() else {
        return 0;
    };
    let offset = OFFSET.load(Ordering::Relaxed);
    let Ok(updates) = get_updates(&token, offset) else {
        return 0;
    };
    let mut n = 0;
    for update in updates {
        if update.update_id >= offset {
            OFFSET.store(update.update_id + 1, Ordering::Relaxed);
        }
        if let Some(message) = update.message {
            handle_inbound(&token, message, ask);
            n += 1;
        }
    }
    n
}

#[cfg(feature = "live-http")]
#[derive(Debug, serde::Deserialize)]
struct TgUpdates {
    ok: bool,
    #[serde(default)]
    result: Vec<TgUpdate>,
}

#[cfg(feature = "live-http")]
#[derive(Debug, serde::Deserialize)]
struct TgUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TgMessage>,
}

#[cfg(feature = "live-http")]
#[derive(Debug, serde::Deserialize)]
struct TgMessage {
    #[serde(default)]
    text: Option<String>,
    chat: TgChat,
}

#[cfg(feature = "live-http")]
#[derive(Debug, serde::Deserialize)]
struct TgChat {
    id: i64,
}

#[cfg(feature = "live-http")]
fn get_updates(token: &str, offset: i64) -> Result<Vec<TgUpdate>, String> {
    let url = format!(
        "{API_ROOT}/bot{token}/getUpdates?offset={offset}&timeout={POLL_TIMEOUT_SECS}&allowed_updates=%5B%22message%22%5D"
    );
    let agent = ureq::AgentBuilder::new()
        .timeout_read(std::time::Duration::from_secs(POLL_TIMEOUT_SECS + 10))
        .timeout_connect(std::time::Duration::from_secs(15))
        .build();
    let response = agent.get(&url).call().map_err(|e| e.to_string())?;
    // ureq 2.12 `into_json` needs the optional `json` feature; Softwake keeps
    // ureq lean (same as softwake-providers/live.rs) and parses with serde_json.
    let raw = response.into_string().map_err(|e| e.to_string())?;
    let body: TgUpdates = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if !body.ok {
        return Err("telegram getUpdates not ok".into());
    }
    Ok(body.result)
}

#[cfg(feature = "live-http")]
fn handle_inbound(
    token: &str,
    message: TgMessage,
    ask: &mut dyn FnMut(&str) -> Result<String, String>,
) {
    let Some(text) = message
        .text
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    else {
        return;
    };
    let chat_id = message.chat.id.to_string();
    let profile_id = resolve_profile_for_chat(&chat_id);
    bind_chat_id_if_needed(&profile_id, &chat_id);

    let ask_text = if text.to_ascii_lowercase().starts_with("/start") {
        "Hello".to_owned()
    } else {
        text
    };

    let agent_name = profile_agent_name(&profile_id);
    let reply_text = match ask(&ask_text) {
        Ok(r) => r,
        Err(e) => format!("(Softwake could not answer: {e})"),
    };

    crate::hud_chat_write::append_exchange(&profile_id, "You", &ask_text, &agent_name, &reply_text);

    let file = load_messengers_for(&profile_id);
    let _ = send_text(token, &chat_id, &reply_text);
    if file.telegram.voice {
        let _ = send_voice_mp3(token, &chat_id, &reply_text, &profile_id);
    }
}

fn resolve_profile_for_chat(chat_id: &str) -> String {
    if let Ok(Some(id)) = softwake_tools::find_profile_for_telegram_chat(chat_id) {
        return id;
    }
    crate::hud_chat_write::active_profile_id().unwrap_or_else(|| "default".into())
}

fn bind_chat_id_if_needed(profile_id: &str, chat_id: &str) {
    let Ok(path) = resolve_messengers_file(profile_id) else {
        return;
    };
    let Ok(mut file) = load_messengers(&path) else {
        return;
    };
    let needs = file
        .telegram
        .chat_id
        .as_ref()
        .is_none_or(|c| c.trim() != chat_id);
    if needs {
        file.telegram.chat_id = Some(chat_id.to_owned());
        file.telegram.enabled = true;
        let _ = save_messengers(&path, &file);
    }
}

fn load_messengers_for(profile_id: &str) -> softwake_tools::MessengersFile {
    resolve_messengers_file(profile_id)
        .ok()
        .and_then(|p| load_messengers(&p).ok())
        .unwrap_or_default()
}

fn profile_agent_name(profile_id: &str) -> String {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let Ok(config) = resolve_config_dir(xdg.as_deref(), home.as_deref()) else {
        return "Softwake".into();
    };
    let pack = profile_pack_dir(&config, profile_id);
    let name = load_profile_meta(&pack).name;
    if name.trim().is_empty() {
        "Softwake".into()
    } else {
        name
    }
}

/// Fan-out a timer / schedule line for `profile_id`.
pub(crate) fn fanout_timer(profile_id: &str, text: &str) {
    let file = load_messengers_for(profile_id);
    if !wants_timer_push(&file.telegram.flags()) || !file.telegram.is_bound() {
        return;
    }
    let Some(token) = load_bot_token() else {
        return;
    };
    let Some(chat_id) = file.telegram.chat_id.clone() else {
        return;
    };
    let agent = profile_agent_name(profile_id);
    crate::hud_chat_write::append_assistant(profile_id, &agent, text);
    let _ = send_text(&token, &chat_id, text);
    if file.telegram.voice {
        let _ = send_voice_mp3(&token, &chat_id, text, profile_id);
    }
}

/// Fan-out after a HUD / typed ask (`receive_all` channels only).
pub(crate) fn fanout_ask_reply(profile_id: &str, text: &str) {
    let file = load_messengers_for(profile_id);
    if !wants_ask_fanout(&file.telegram.flags()) || !file.telegram.is_bound() {
        return;
    }
    let Some(token) = load_bot_token() else {
        return;
    };
    let Some(chat_id) = file.telegram.chat_id.clone() else {
        return;
    };
    let _ = send_text(&token, &chat_id, text);
    if file.telegram.voice {
        let _ = send_voice_mp3(&token, &chat_id, text, profile_id);
    }
}

/// Whether desktop should speak for a timer fire.
#[must_use]
pub(crate) fn desktop_wants_timer_voice(profile_id: &str) -> bool {
    let file = load_messengers_for(profile_id);
    wants_timer_push(&ChannelFlags {
        default: file.desktop.default,
        receive_all: file.desktop.receive_all,
        voice: file.desktop.voice,
    }) && file.desktop.voice
}

/// Whether desktop should speak for a normal ask (HUD surface).
#[must_use]
pub(crate) fn desktop_wants_ask_voice(profile_id: &str) -> bool {
    load_messengers_for(profile_id).desktop.voice
}

fn send_text(token: &str, chat_id: &str, text: &str) -> Result<(), String> {
    #[cfg(feature = "live-http")]
    {
        let url = format!("{API_ROOT}/bot{token}/sendMessage");
        let body = serde_json::json!({
            "chat_id": chat_id,
            "text": clip(text, 4000),
        });
        let agent = ureq::AgentBuilder::new()
            .timeout_read(std::time::Duration::from_secs(30))
            .timeout_connect(std::time::Duration::from_secs(15))
            .build();
        let _ = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (token, chat_id, text);
        Err("live-http disabled".into())
    }
}

fn send_voice_mp3(token: &str, chat_id: &str, text: &str, profile_id: &str) -> Result<(), String> {
    #[cfg(feature = "live-http")]
    {
        let mp3 = synthesize_mp3(text, &profile_tts_voice(profile_id))?;
        let url = format!("{API_ROOT}/bot{token}/sendAudio");
        let boundary = "----softwakeTelegramBoundary";
        let mut body = Vec::new();
        push_field(&mut body, boundary, "chat_id", chat_id);
        push_file(
            &mut body,
            boundary,
            "audio",
            "softwake.mp3",
            "audio/mpeg",
            &mp3,
        );
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let agent = ureq::AgentBuilder::new()
            .timeout_read(std::time::Duration::from_secs(60))
            .timeout_connect(std::time::Duration::from_secs(15))
            .build();
        let _ = agent
            .post(&url)
            .set(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send_bytes(&body)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (token, chat_id, text, profile_id);
        Err("live-http disabled".into())
    }
}

#[cfg(feature = "live-http")]
fn push_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

#[cfg(feature = "live-http")]
fn push_file(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
}

/// This profile's `tts_voice`. Empty falls back to Settings, then eve.
#[cfg(feature = "live-http")]
fn profile_tts_voice(profile_id: &str) -> String {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let Ok(config) = resolve_config_dir(xdg.as_deref(), home.as_deref()) else {
        return String::new();
    };
    load_profile_meta(&profile_pack_dir(&config, profile_id)).tts_voice
}

fn synthesize_mp3(text: &str, profile_voice: &str) -> Result<Vec<u8>, String> {
    #[cfg(feature = "live-http")]
    {
        use softwake_providers::{family_speaks_xai, tts_synthesize};
        let ready = crate::chat::load_disk_chat()?;
        let provider = ready.prepared.provider;
        if !family_speaks_xai(provider) {
            return Err("TTS requires xAI".into());
        }
        let Some(voice) =
            crate::talk::resolve_spoken_voice(provider, profile_voice, ready.prepared_tts_voice())
        else {
            return Err("no TTS voice".into());
        };
        let transport = softwake_providers::live::LiveTransport::bounded(crate::chat::CHAT_TIMEOUT);
        tts_synthesize(
            &transport,
            provider,
            &ready.prepared.api_base,
            &ready.bearer,
            text,
            voice,
            ready.tts_speed,
        )
        .map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (text, profile_voice);
        Err("live-http disabled".into())
    }
}

fn clip(text: &str, max: usize) -> String {
    let mut out = String::new();
    for (i, ch) in text.chars().enumerate() {
        if i >= max {
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_limits_chars() {
        assert_eq!(clip("abcd", 2), "ab");
    }
}

#[cfg(test)]
mod sticky_tests {
    use super::laptop_should_own_telegram;
    use softwake_state::VoiceState;

    #[test]
    fn laptop_should_own_matrix() {
        assert!(laptop_should_own_telegram(false, VoiceState::Sleep));
        assert!(laptop_should_own_telegram(false, VoiceState::Hibernate));
        assert!(laptop_should_own_telegram(true, VoiceState::Awake));
        assert!(!laptop_should_own_telegram(true, VoiceState::Sleep));
        assert!(!laptop_should_own_telegram(true, VoiceState::Hibernate));
    }
}
