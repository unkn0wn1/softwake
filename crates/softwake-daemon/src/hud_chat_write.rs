//! Append turns to per-profile HUD chat (plaintext) or the encrypted-vault inbox.

#![cfg_attr(not(feature = "live-http"), allow(dead_code))]

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use softwake_soul::{load_app_config, profile_pack_dir, resolve_config_dir};
use softwake_tools::{InboxTurn, append_hud_inbox, resolve_hud_chat_inbox};

const FILE_NAME: &str = "hud-chat.json";
const MAX_TURNS: usize = 40;

#[derive(Debug, Serialize, Deserialize)]
struct HudChatFile {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turns: Option<Vec<HudTurn>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encrypted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt_b64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    nonce_b64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ciphertext_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HudTurn {
    role: String,
    name: String,
    text: String,
    ts: u64,
    #[serde(default)]
    error: bool,
    #[serde(default)]
    note: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn profile_hud_path(profile_id: &str) -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()?;
    Some(profile_pack_dir(&config, profile_id).join(FILE_NAME))
}

/// Append user + assistant turns for `profile_id`.
pub(crate) fn append_exchange(
    profile_id: &str,
    user_name: &str,
    user_text: &str,
    assistant_name: &str,
    assistant_text: &str,
) {
    let ts = now_ms();
    let turns = [
        InboxTurn {
            role: "user".into(),
            name: user_name.to_owned(),
            text: user_text.to_owned(),
            ts,
            error: false,
            note: String::new(),
        },
        InboxTurn {
            role: "assistant".into(),
            name: assistant_name.to_owned(),
            text: assistant_text.to_owned(),
            ts: ts.saturating_add(1),
            error: false,
            note: String::new(),
        },
    ];
    append_turns(profile_id, &turns);
}

/// Append a single assistant (or notify) line.
pub(crate) fn append_assistant(profile_id: &str, name: &str, text: &str) {
    append_turns(
        profile_id,
        &[InboxTurn {
            role: "assistant".into(),
            name: name.to_owned(),
            text: text.to_owned(),
            ts: now_ms(),
            error: false,
            note: String::new(),
        }],
    );
}

/// Append a single assistant notice (e.g. while-you-were-away) for `profile_id`.
pub(crate) fn append_assistant_notice(profile_id: &str, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let ts = now_ms();
    append_turns(
        profile_id,
        &[InboxTurn {
            role: "assistant".into(),
            name: "Softwake".into(),
            text: trimmed.to_owned(),
            ts,
            error: false,
            note: "while_away".into(),
        }],
    );
}

fn append_turns(profile_id: &str, turns: &[InboxTurn]) {
    let Some(path) = profile_hud_path(profile_id) else {
        return;
    };
    if path.exists() {
        if let Ok(bytes) = fs::read(&path) {
            if let Ok(file) = serde_json::from_slice::<HudChatFile>(&bytes) {
                if file.encrypted == Some(true) {
                    if let Ok(inbox) = resolve_hud_chat_inbox(profile_id) {
                        let _ = append_hud_inbox(&inbox, turns);
                    }
                    return;
                }
                let mut existing = file.turns.unwrap_or_default();
                for turn in turns {
                    existing.push(HudTurn {
                        role: turn.role.clone(),
                        name: turn.name.clone(),
                        text: turn.text.clone(),
                        ts: turn.ts,
                        error: turn.error,
                        note: turn.note.clone(),
                    });
                }
                if existing.len() > MAX_TURNS {
                    let skip = existing.len() - MAX_TURNS;
                    existing = existing.split_off(skip);
                }
                let out = HudChatFile {
                    version: 1,
                    turns: Some(existing),
                    encrypted: None,
                    kdf: None,
                    salt_b64: None,
                    nonce_b64: None,
                    ciphertext_b64: None,
                };
                write_atomic(&path, &out);
                return;
            }
        }
    }
    // Missing or unreadable → plaintext create.
    let mut existing = Vec::new();
    for turn in turns {
        existing.push(HudTurn {
            role: turn.role.clone(),
            name: turn.name.clone(),
            text: turn.text.clone(),
            ts: turn.ts,
            error: turn.error,
            note: turn.note.clone(),
        });
    }
    let out = HudChatFile {
        version: 1,
        turns: Some(existing),
        encrypted: None,
        kdf: None,
        salt_b64: None,
        nonce_b64: None,
        ciphertext_b64: None,
    };
    write_atomic(&path, &out);
}

fn write_atomic(path: &PathBuf, file: &HudChatFile) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(json) = serde_json::to_string_pretty(file) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, format!("{json}\n")).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// Active profile id from config (best-effort).
#[must_use]
pub(crate) fn active_profile_id() -> Option<String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()?;
    let app = load_app_config(&config).ok()?;
    Some(app.active_profile)
}
