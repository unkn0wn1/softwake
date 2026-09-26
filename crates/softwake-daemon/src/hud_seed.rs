//! Seed the awake model session from per-profile HUD chat history.
//!
//! Reads plaintext `hud-chat.json` only. Encrypted vault files are skipped;
//! softwake-ui can push decrypted turns over IPC after unlock.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use softwake_session::{SessionMessage, TextStubSession};
use softwake_soul::{load_app_config, profile_pack_dir, resolve_config_dir};

/// Soft cap on seeded character mass (Unicode scalars). Newest turns win.
pub(crate) const HUD_SEED_MAX_CHARS: usize = 12_000;

const FILE_NAME: &str = "hud-chat.json";

#[derive(Debug, Deserialize)]
struct HudChatFile {
    #[serde(default)]
    turns: Option<Vec<HudTurnWire>>,
    #[serde(default)]
    encrypted: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct HudTurnWire {
    role: String,
    text: String,
    #[serde(default)]
    error: bool,
}

/// One turn from softwake-ui (decrypted HUD history).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SeedTurn {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Turn body.
    pub text: String,
    /// Skip when true.
    #[serde(default)]
    pub error: bool,
}

/// Load plaintext HUD turns for the active profile (oldest-first).
#[must_use]
pub(crate) fn load_plaintext_hud_turns() -> Vec<SessionMessage> {
    let Some(path) = active_hud_chat_path() else {
        return Vec::new();
    };
    let Ok(bytes) = fs::read(&path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_slice::<HudChatFile>(&bytes) else {
        return Vec::new();
    };
    if file.encrypted == Some(true) {
        return Vec::new();
    }
    wire_to_messages(file.turns.unwrap_or_default())
}

/// Convert IPC seed turns into session messages.
#[must_use]
pub(crate) fn seed_turns_to_messages(turns: Vec<SeedTurn>) -> Vec<SessionMessage> {
    wire_to_messages(
        turns
            .into_iter()
            .map(|turn| HudTurnWire {
                role: turn.role,
                text: turn.text,
                error: turn.error,
            })
            .collect(),
    )
}

fn wire_to_messages(turns: Vec<HudTurnWire>) -> Vec<SessionMessage> {
    let mut out = Vec::new();
    for turn in turns {
        if turn.error {
            continue;
        }
        let text = turn.text.trim();
        if text.is_empty() {
            continue;
        }
        match turn.role.trim().to_ascii_lowercase().as_str() {
            "user" => out.push(SessionMessage::user(text)),
            "assistant" => out.push(SessionMessage::assistant(text)),
            _ => {}
        }
    }
    out
}

/// Apply turns into an open empty session under the character budget.
pub(crate) fn seed_session(session: &mut TextStubSession, turns: Vec<SessionMessage>) -> usize {
    session.seed_turns_if_empty(turns, HUD_SEED_MAX_CHARS)
}

fn active_hud_chat_path() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()?;
    let app = load_app_config(&config).ok()?;
    Some(profile_pack_dir(&config, &app.active_profile).join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;
    use softwake_session::MessageRole;

    #[test]
    fn skips_errors_and_unknown_roles() {
        let messages = wire_to_messages(vec![
            HudTurnWire {
                role: "user".into(),
                text: "hi".into(),
                error: false,
            },
            HudTurnWire {
                role: "assistant".into(),
                text: "boom".into(),
                error: true,
            },
            HudTurnWire {
                role: "system".into(),
                text: "nope".into(),
                error: false,
            },
            HudTurnWire {
                role: "assistant".into(),
                text: "hello".into(),
                error: false,
            },
        ]);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[1].content, "hello");
    }
}
