//! Per-profile messenger channel bindings (`messengers.json`).
//!
//! Non-secret flags + Telegram chat id. Bot token stays in the secret bag.
//! See [ADR-0029](../../docs/ADR-0029-messengers-telegram.md).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use softwake_soul::{load_app_config, profile_pack_dir, resolve_config_dir};

/// File name under a profile pack directory.
pub const MESSENGERS_FILE_NAME: &str = "messengers.json";

/// Pending HUD turns when the vault is encrypted.
pub const HUD_CHAT_INBOX_FILE_NAME: &str = "hud-chat-inbox.json";

/// Channel id for the local HUD surface.
pub const CHANNEL_DESKTOP: &str = "desktop";

/// Channel id for Telegram Bot API.
pub const CHANNEL_TELEGRAM: &str = "telegram";

/// On-disk messengers document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessengersFile {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Local HUD surface flags.
    #[serde(default)]
    pub desktop: ChannelFlags,
    /// Telegram binding for this profile.
    #[serde(default)]
    pub telegram: TelegramChannel,
}

fn one() -> u32 {
    1
}

impl Default for MessengersFile {
    fn default() -> Self {
        Self {
            version: 1,
            desktop: ChannelFlags {
                default: true,
                receive_all: true,
                voice: true,
            },
            telegram: TelegramChannel::default(),
        }
    }
}

/// Per-channel delivery flags (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChannelFlags {
    /// Primary channel for this profile.
    #[serde(default)]
    pub default: bool,
    /// Receives timer fires and receive-all agent pushes.
    #[serde(default)]
    pub receive_all: bool,
    /// Outbound includes TTS when the channel supports it.
    #[serde(default)]
    pub voice: bool,
}

/// Telegram row: flags + optional bound chat id.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors Settings checkboxes for one channel"
)]
pub struct TelegramChannel {
    /// Operator enabled the channel (token may still be missing).
    #[serde(default)]
    pub enabled: bool,
    /// Primary channel.
    #[serde(default)]
    pub default: bool,
    /// Fan-out for timers / receive-all pushes.
    #[serde(default)]
    pub receive_all: bool,
    /// Send TTS audio with text.
    #[serde(default)]
    pub voice: bool,
    /// Bound Telegram chat id (stringified integer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
}

impl TelegramChannel {
    /// Flags view.
    #[must_use]
    pub fn flags(&self) -> ChannelFlags {
        ChannelFlags {
            default: self.default,
            receive_all: self.receive_all,
            voice: self.voice,
        }
    }

    /// Whether outbound can target this binding.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.enabled
            && self
                .chat_id
                .as_ref()
                .is_some_and(|id| !id.trim().is_empty())
    }
}

/// Errors loading or saving messengers.json.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MessengersError {
    /// Config root could not be resolved.
    #[error("Softwake config directory is unavailable")]
    NoConfigDir,
    /// IO or JSON failure.
    #[error("{0}")]
    Io(String),
}

impl From<softwake_soul::SoulError> for MessengersError {
    fn from(value: softwake_soul::SoulError) -> Self {
        Self::Io(value.to_string())
    }
}

fn config_dir() -> Result<PathBuf, MessengersError> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|_| MessengersError::NoConfigDir)
}

/// Resolve `profiles/<id>/messengers.json`.
///
/// # Errors
///
/// Returns [`MessengersError::NoConfigDir`] when config cannot be resolved.
pub fn resolve_messengers_file(profile_id: &str) -> Result<PathBuf, MessengersError> {
    let config = config_dir()?;
    Ok(profile_pack_dir(&config, profile_id).join(MESSENGERS_FILE_NAME))
}

/// Resolve inbox path for encrypted HUD merge.
///
/// # Errors
///
/// Returns [`MessengersError::NoConfigDir`] when config cannot be resolved.
pub fn resolve_hud_chat_inbox(profile_id: &str) -> Result<PathBuf, MessengersError> {
    let config = config_dir()?;
    Ok(profile_pack_dir(&config, profile_id).join(HUD_CHAT_INBOX_FILE_NAME))
}

/// Resolve messengers for the active profile.
///
/// # Errors
///
/// Config resolution failures.
pub fn resolve_active_messengers_file() -> Result<PathBuf, MessengersError> {
    let config = config_dir()?;
    let app = load_app_config(&config).unwrap_or_default();
    Ok(profile_pack_dir(&config, &app.active_profile).join(MESSENGERS_FILE_NAME))
}

/// Load messengers from `path`. Missing file → defaults.
///
/// # Errors
///
/// IO/JSON errors.
pub fn load_messengers(path: &Path) -> Result<MessengersFile, MessengersError> {
    if !path.exists() {
        return Ok(MessengersFile::default());
    }
    let raw = fs::read_to_string(path).map_err(|e| MessengersError::Io(e.to_string()))?;
    let mut file: MessengersFile =
        serde_json::from_str(&raw).map_err(|e| MessengersError::Io(e.to_string()))?;
    if file.version == 0 {
        file.version = 1;
    }
    Ok(file)
}

/// Atomically write messengers JSON.
///
/// # Errors
///
/// IO errors.
pub fn save_messengers(path: &Path, file: &MessengersFile) -> Result<(), MessengersError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| MessengersError::Io(e.to_string()))?;
    }
    let json =
        serde_json::to_string_pretty(file).map_err(|e| MessengersError::Io(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, format!("{json}\n")).map_err(|e| MessengersError::Io(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path).map_err(|e| MessengersError::Io(e.to_string()))
}

/// Whether `flags` should receive a timer / receive-all push.
#[must_use]
pub fn wants_timer_push(flags: &ChannelFlags) -> bool {
    flags.receive_all || flags.default
}

/// Whether `flags` should receive a HUD-ask fan-out (receive-all only).
#[must_use]
pub fn wants_ask_fanout(flags: &ChannelFlags) -> bool {
    flags.receive_all
}

/// One pending HUD turn in the encrypted-vault inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxTurn {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Display name.
    #[serde(default)]
    pub name: String,
    /// Body.
    pub text: String,
    /// Unix ms.
    pub ts: u64,
    /// Error bubble.
    #[serde(default)]
    pub error: bool,
    /// Secondary note.
    #[serde(default)]
    pub note: String,
}

/// Inbox document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct HudChatInbox {
    /// Schema version.
    #[serde(default = "one")]
    pub version: u32,
    /// Pending turns (oldest first).
    #[serde(default)]
    pub turns: Vec<InboxTurn>,
}

/// Append turns to the inbox file (create if missing).
///
/// # Errors
///
/// IO/JSON errors.
pub fn append_hud_inbox(path: &Path, turns: &[InboxTurn]) -> Result<(), MessengersError> {
    if turns.is_empty() {
        return Ok(());
    }
    let mut file = if path.exists() {
        let raw = fs::read_to_string(path).map_err(|e| MessengersError::Io(e.to_string()))?;
        serde_json::from_str(&raw).unwrap_or_default()
    } else {
        HudChatInbox {
            version: 1,
            turns: Vec::new(),
        }
    };
    file.version = 1;
    file.turns.extend_from_slice(turns);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| MessengersError::Io(e.to_string()))?;
    }
    let json =
        serde_json::to_string_pretty(&file).map_err(|e| MessengersError::Io(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, format!("{json}\n")).map_err(|e| MessengersError::Io(e.to_string()))?;
    fs::rename(&tmp, path).map_err(|e| MessengersError::Io(e.to_string()))
}

/// Take and clear the inbox (missing → empty).
///
/// # Errors
///
/// IO errors on delete after read.
pub fn take_hud_inbox(path: &Path) -> Result<Vec<InboxTurn>, MessengersError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(path).map_err(|e| MessengersError::Io(e.to_string()))?;
    let file: HudChatInbox = serde_json::from_str(&raw).unwrap_or_default();
    let _ = fs::remove_file(path);
    Ok(file.turns)
}

/// Find a profile whose `telegram.chat_id` matches `chat_id`.
///
/// # Errors
///
/// Config resolution failures.
pub fn find_profile_for_telegram_chat(chat_id: &str) -> Result<Option<String>, MessengersError> {
    let chat_id = chat_id.trim();
    if chat_id.is_empty() {
        return Ok(None);
    }
    let config = config_dir()?;
    let profiles = softwake_soul::list_profiles(&config).unwrap_or_default();
    for meta in profiles {
        let path = profile_pack_dir(&config, &meta.id).join(MESSENGERS_FILE_NAME);
        let file = load_messengers(&path)?;
        if file
            .telegram
            .chat_id
            .as_ref()
            .is_some_and(|id| id.trim() == chat_id)
        {
            return Ok(Some(meta.id));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("softwake-messengers-{nanos}"));
        fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn defaults_desktop_receive_and_round_trip() {
        let dir = temp_dir();
        let path = dir.join(MESSENGERS_FILE_NAME);
        let file = MessengersFile::default();
        assert!(file.desktop.default);
        assert!(file.desktop.receive_all);
        assert!(file.desktop.voice);
        assert!(!file.telegram.enabled);
        save_messengers(&path, &file).expect("save");
        let loaded = load_messengers(&path).expect("load");
        assert_eq!(loaded, file);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn timer_push_includes_default_or_receive_all() {
        assert!(wants_timer_push(&ChannelFlags {
            default: true,
            receive_all: false,
            voice: false,
        }));
        assert!(wants_timer_push(&ChannelFlags {
            default: false,
            receive_all: true,
            voice: false,
        }));
        assert!(!wants_timer_push(&ChannelFlags::default()));
        assert!(!wants_ask_fanout(&ChannelFlags {
            default: true,
            receive_all: false,
            voice: true,
        }));
        assert!(wants_ask_fanout(&ChannelFlags {
            default: false,
            receive_all: true,
            voice: false,
        }));
    }

    #[test]
    fn inbox_append_and_take() {
        let dir = temp_dir();
        let path = dir.join(HUD_CHAT_INBOX_FILE_NAME);
        append_hud_inbox(
            &path,
            &[InboxTurn {
                role: "user".into(),
                name: "You".into(),
                text: "hi".into(),
                ts: 1,
                error: false,
                note: String::new(),
            }],
        )
        .expect("append");
        append_hud_inbox(
            &path,
            &[InboxTurn {
                role: "assistant".into(),
                name: "Softwake".into(),
                text: "hello".into(),
                ts: 2,
                error: false,
                note: String::new(),
            }],
        )
        .expect("append2");
        let turns = take_hud_inbox(&path).expect("take");
        assert_eq!(turns.len(), 2);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
