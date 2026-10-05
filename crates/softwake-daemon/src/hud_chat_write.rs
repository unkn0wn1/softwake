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

#[cfg(test)]
pub(crate) static HISTORY_CONFIG_OVERRIDE: std::sync::Mutex<Option<PathBuf>> =
    std::sync::Mutex::new(None);

/// One test at a time may point HUD writes at a temp directory.
#[cfg(test)]
pub(crate) fn hold_history_override_tests() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn profile_hud_path(profile_id: &str) -> Option<PathBuf> {
    #[cfg(test)]
    {
        if let Ok(guard) = HISTORY_CONFIG_OVERRIDE.lock() {
            if let Some(root) = guard.as_ref() {
                return Some(root.join("profiles").join(profile_id).join(FILE_NAME));
            }
        }
    }
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()?;
    Some(profile_pack_dir(&config, profile_id).join(FILE_NAME))
}

/// Append one turn to a profile's HUD chat.
///
/// In tests this writes only while [`HISTORY_CONFIG_OVERRIDE`] is set, so
/// room and voice tests cannot touch the operator's real profile chat.
pub(crate) fn append_role_turn(profile_id: &str, role: &str, name: &str, text: &str) {
    let profile_id = profile_id.trim();
    let text = text.trim();
    if profile_id.is_empty() || text.is_empty() {
        return;
    }
    #[cfg(test)]
    {
        let active = HISTORY_CONFIG_OVERRIDE
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false);
        if !active {
            return;
        }
    }
    let role = if role.eq_ignore_ascii_case("assistant") {
        "assistant"
    } else {
        "user"
    };
    append_turns(
        profile_id,
        &[InboxTurn {
            role: role.to_owned(),
            name: name.to_owned(),
            text: text.to_owned(),
            ts: now_ms(),
            error: false,
            note: String::new(),
        }],
    );
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

/// True when a test has pointed HUD writes at a temp config.
///
/// The room play path appends a line once and does not rewrite it, so this
/// guard stays with the test helpers below.
#[cfg(test)]
fn history_writes_allowed() -> bool {
    HISTORY_CONFIG_OVERRIDE
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}

/// Replace plaintext HUD turns whose text equals `old_text`.
///
/// Encrypted vault chats are left alone. The room play path no longer calls
/// this. Tests keep it so a matching plaintext turn can still be edited.
#[cfg(test)]
pub(crate) fn replace_role_turn(profile_id: &str, old_text: &str, new_text: &str) {
    let old_text = old_text.trim();
    let new_text = new_text.trim();
    if old_text.is_empty() || new_text.is_empty() || old_text == new_text {
        return;
    }
    edit_turns(profile_id, old_text, Some(new_text));
}

/// Delete plaintext HUD turns whose text equals `text`.
#[cfg(test)]
pub(crate) fn delete_role_turn(profile_id: &str, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    edit_turns(profile_id, text, None);
}

#[cfg(test)]
fn edit_turns(profile_id: &str, old_text: &str, new_text: Option<&str>) {
    if !history_writes_allowed() {
        return;
    }
    let Some(path) = profile_hud_path(profile_id) else {
        return;
    };
    let Ok(bytes) = fs::read(&path) else {
        return;
    };
    let Ok(file) = serde_json::from_slice::<HudChatFile>(&bytes) else {
        return;
    };
    if file.encrypted == Some(true) {
        return;
    }
    let Some(mut turns) = file.turns else {
        return;
    };
    let mut changed = false;
    if let Some(new_text) = new_text {
        for turn in &mut turns {
            if turn.text == old_text {
                new_text.clone_into(&mut turn.text);
                changed = true;
            }
        }
    } else {
        let before = turns.len();
        turns.retain(|turn| turn.text != old_text);
        changed = turns.len() != before;
    }
    if !changed {
        return;
    }
    write_atomic(
        &path,
        &HudChatFile {
            version: file.version,
            turns: Some(turns),
            encrypted: file.encrypted,
            kdf: file.kdf,
            salt_b64: file.salt_b64,
            nonce_b64: file.nonce_b64,
            ciphertext_b64: file.ciphertext_b64,
        },
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

/// Write a private note into both profiles' chats. The message text matches.
///
/// `sender_id` is omitted when the caller has no profile pack. The recipient
/// still gets the note.
pub(crate) fn append_dm_histories(
    sender_id: Option<&str>,
    sender_name: &str,
    recipient_id: &str,
    recipient_name: &str,
    text: &str,
    reply: Option<&str>,
) {
    let (sender_turns, recipient_turns) =
        dm_history_turns(sender_name, recipient_name, text, reply, now_ms());
    if let Some(sender_id) = sender_id {
        append_turns(sender_id, &sender_turns);
    }
    append_turns(recipient_id, &recipient_turns);
}

/// Same message text on both sides. A reply, when present, is on both sides too.
fn dm_history_turns(
    sender_name: &str,
    recipient_name: &str,
    text: &str,
    reply: Option<&str>,
    ts: u64,
) -> (Vec<InboxTurn>, Vec<InboxTurn>) {
    let mut sender_turns = vec![InboxTurn {
        role: "assistant".into(),
        name: sender_name.to_owned(),
        text: text.to_owned(),
        ts,
        error: false,
        note: String::new(),
    }];
    let mut recipient_turns = vec![InboxTurn {
        role: "user".into(),
        name: sender_name.to_owned(),
        text: text.to_owned(),
        ts,
        error: false,
        note: String::new(),
    }];
    if let Some(reply) = reply.map(str::trim).filter(|reply| !reply.is_empty()) {
        sender_turns.push(InboxTurn {
            role: "user".into(),
            name: recipient_name.to_owned(),
            text: reply.to_owned(),
            ts: ts.saturating_add(1),
            error: false,
            note: String::new(),
        });
        recipient_turns.push(InboxTurn {
            role: "assistant".into(),
            name: recipient_name.to_owned(),
            text: reply.to_owned(),
            ts: ts.saturating_add(1),
            error: false,
            note: String::new(),
        });
    }
    (sender_turns, recipient_turns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_histories_carry_the_same_message_and_reply() {
        let (sender, recipient) =
            dm_history_turns("Sara", "Sally", "ping the notes", Some("got it"), 10);
        assert_eq!(sender[0].text, "ping the notes");
        assert_eq!(recipient[0].text, "ping the notes");
        assert_eq!(sender[0].role, "assistant");
        assert_eq!(recipient[0].role, "user");
        assert_eq!(recipient[0].name, "Sara");
        assert_eq!(sender[1].text, "got it");
        assert_eq!(recipient[1].text, "got it");
        assert_eq!(sender[1].name, "Sally");
        assert_eq!(recipient[1].name, "Sally");
    }

    #[test]
    fn replace_and_delete_hit_only_the_matching_turn() {
        let _hold = super::hold_history_override_tests();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "softwake-hud-recheck-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp");
        *HISTORY_CONFIG_OVERRIDE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(root.clone());
        append_role_turn("sally", "assistant", "Sally", "[room standup] Sally: draft");
        append_role_turn("sally", "user", "Joi", "[room standup] Joi: other");
        replace_role_turn(
            "sally",
            "[room standup] Sally: draft",
            "[room standup] Sally: final",
        );
        let body = std::fs::read_to_string(root.join("profiles/sally/hud-chat.json")).expect("hud");
        assert!(body.contains("Sally: final"), "{body}");
        assert!(!body.contains("Sally: draft"), "{body}");
        assert!(body.contains("Joi: other"), "{body}");
        delete_role_turn("sally", "[room standup] Joi: other");
        let body = std::fs::read_to_string(root.join("profiles/sally/hud-chat.json")).expect("hud");
        assert!(!body.contains("Joi: other"), "{body}");
        assert!(body.contains("Sally: final"), "{body}");
        *HISTORY_CONFIG_OVERRIDE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let _ = std::fs::remove_dir_all(&root);
    }
}
