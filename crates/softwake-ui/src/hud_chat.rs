//! Per-profile HUD chat history (display only — not model context).
//!
//! File: `$XDG_CONFIG_HOME/softwake/profiles/<id>/hud-chat.json`
//! Cap 40 turns (matches HUD `MAX_TURNS`). Softwaked never reads this file.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use softwake_soul::{
    ensure_migrated, list_profiles, load_app_config, profile_pack_dir, resolve_config_dir,
};

use crate::ui_vault::{self, DataKey, VaultMode};

/// Cap matches the in-memory HUD ring.
pub const MAX_TURNS: usize = 40;
const FILE_NAME: &str = "hud-chat.json";
const FILE_VERSION: u32 = 1;

/// One bubble in the HUD log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudTurn {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Display name ("You" or profile agent name).
    pub name: String,
    /// Bubble body.
    pub text: String,
    /// Unix milliseconds.
    pub ts: u64,
    /// True when the bubble is an error line.
    #[serde(default)]
    pub error: bool,
    /// Optional secondary note under the body.
    #[serde(default)]
    pub note: String,
}

/// On-disk shape: plaintext turns **or** encrypted envelope (not both).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudChatFile {
    /// Schema version.
    pub version: u32,
    /// Plaintext turns when unlocked / plaintext mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turns: Option<Vec<HudTurn>>,
    /// Envelope flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
    /// KDF id (`argon2id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdf: Option<String>,
    /// Base64 salt (unused on chat file — salt lives in ui-vault.json; kept for
    /// per-file independence if a future revision re-keys per profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt_b64: Option<String>,
    /// Base64 12-byte nonce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce_b64: Option<String>,
    /// Base64 AEAD ciphertext of the turns JSON payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ciphertext_b64: Option<String>,
}

impl Default for HudChatFile {
    fn default() -> Self {
        Self {
            version: FILE_VERSION,
            turns: Some(Vec::new()),
            encrypted: None,
            kdf: None,
            salt_b64: None,
            nonce_b64: None,
            ciphertext_b64: None,
        }
    }
}

/// Truncate to the newest [`MAX_TURNS`] turns.
#[must_use]
pub fn truncate_turns(mut turns: Vec<HudTurn>) -> Vec<HudTurn> {
    if turns.len() > MAX_TURNS {
        let skip = turns.len() - MAX_TURNS;
        turns = turns.split_off(skip);
    }
    turns
}

fn config_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())
}

/// Resolve which profile id to use (`None` / empty → active).
pub fn resolve_profile_id(profile_id: Option<String>) -> Result<String, String> {
    let config = config_dir()?;
    ensure_migrated(&config).map_err(|e| e.to_string())?;
    let app = load_app_config(&config).map_err(|e| e.to_string())?;
    let id = profile_id.unwrap_or_default();
    let id = id.trim();
    if id.is_empty() {
        return Ok(app.active_profile);
    }
    let dir = profile_pack_dir(&config, id);
    if !dir.is_dir() {
        return Err(format!("unknown profile `{id}`"));
    }
    Ok(id.to_owned())
}

/// Absolute path for one profile's HUD chat file.
#[must_use]
pub fn chat_path(config: &Path, profile_id: &str) -> PathBuf {
    profile_pack_dir(config, profile_id).join(FILE_NAME)
}

/// Load turns from an explicit Softwake config root.
pub fn load_turns_at(
    config: &Path,
    profile_id: &str,
    key: Option<&DataKey>,
) -> Result<Vec<HudTurn>, String> {
    let path = chat_path(config, profile_id);
    let Some(file) = read_raw(&path)? else {
        return Ok(Vec::new());
    };
    if file.encrypted == Some(true) {
        let key = key.ok_or_else(|| "chat vault is locked".to_owned())?;
        let nonce_b64 = file
            .nonce_b64
            .as_deref()
            .ok_or_else(|| "encrypted chat is missing nonce".to_owned())?;
        let ct_b64 = file
            .ciphertext_b64
            .as_deref()
            .ok_or_else(|| "encrypted chat is missing ciphertext".to_owned())?;
        let plain = ui_vault::open_bytes(key, nonce_b64, ct_b64)?;
        return turns_from_payload(&plain);
    }
    Ok(truncate_turns(file.turns.unwrap_or_default()))
}

/// Persist turns under an explicit Softwake config root.
pub fn save_turns_at(
    config: &Path,
    profile_id: &str,
    turns: Vec<HudTurn>,
    mode: VaultMode,
    key: Option<&DataKey>,
) -> Result<(), String> {
    let path = chat_path(config, profile_id);
    let turns = truncate_turns(turns);
    match mode {
        VaultMode::Plaintext | VaultMode::Unset => {
            let file = HudChatFile {
                version: FILE_VERSION,
                turns: Some(turns),
                encrypted: None,
                kdf: None,
                salt_b64: None,
                nonce_b64: None,
                ciphertext_b64: None,
            };
            let body = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
            atomic_write(&path, &body)
        }
        VaultMode::Passphrase => {
            let key = key.ok_or_else(|| "chat vault is locked".to_owned())?;
            let plain = payload_bytes(&turns)?;
            let (nonce_b64, ciphertext_b64) = ui_vault::seal_bytes(key, &plain)?;
            let file = HudChatFile {
                version: FILE_VERSION,
                turns: None,
                encrypted: Some(true),
                kdf: Some("argon2id".to_owned()),
                salt_b64: None,
                nonce_b64: Some(nonce_b64),
                ciphertext_b64: Some(ciphertext_b64),
            };
            let body = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
            atomic_write(&path, &body)
        }
    }
}

/// Re-encrypt every profile under an explicit config root.
pub fn migrate_all_profiles_at(
    config: &Path,
    mode: VaultMode,
    key: Option<&DataKey>,
) -> Result<(), String> {
    let profiles = list_profiles(config).map_err(|e| e.to_string())?;
    for meta in profiles {
        let path = chat_path(config, &meta.id);
        let turns = match read_raw(&path)? {
            None => Vec::new(),
            Some(file) if file.encrypted == Some(true) => {
                let k = key.ok_or_else(|| "chat vault is locked".to_owned())?;
                let nonce = file
                    .nonce_b64
                    .as_deref()
                    .ok_or_else(|| "encrypted chat is missing nonce".to_owned())?;
                let ct = file
                    .ciphertext_b64
                    .as_deref()
                    .ok_or_else(|| "encrypted chat is missing ciphertext".to_owned())?;
                let plain = ui_vault::open_bytes(k, nonce, ct)?;
                turns_from_payload(&plain)?
            }
            Some(file) => truncate_turns(file.turns.unwrap_or_default()),
        };
        save_turns_at(config, &meta.id, turns, mode, key)?;
    }
    Ok(())
}

fn read_raw(path: &Path) -> Result<Option<HudChatFile>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let file: HudChatFile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(Some(file))
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
        file.write_all(body).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn payload_bytes(turns: &[HudTurn]) -> Result<Vec<u8>, String> {
    let wrapped = serde_json::json!({
        "version": FILE_VERSION,
        "turns": turns,
    });
    serde_json::to_vec(&wrapped).map_err(|e| e.to_string())
}

fn turns_from_payload(bytes: &[u8]) -> Result<Vec<HudTurn>, String> {
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        turns: Vec<HudTurn>,
    }
    let payload: Payload = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    Ok(truncate_turns(payload.turns))
}

/// Load turns for a profile. Requires `key` when the file (or vault) is encrypted.
pub fn load_turns(profile_id: &str, key: Option<&DataKey>) -> Result<Vec<HudTurn>, String> {
    let config = config_dir()?;
    ensure_migrated(&config).map_err(|e| e.to_string())?;
    load_turns_at(&config, profile_id, key)
}

/// Persist turns for a profile (plaintext or sealed with `key`).
pub fn save_turns(
    profile_id: &str,
    turns: Vec<HudTurn>,
    mode: VaultMode,
    key: Option<&DataKey>,
) -> Result<(), String> {
    let config = config_dir()?;
    ensure_migrated(&config).map_err(|e| e.to_string())?;
    save_turns_at(&config, profile_id, turns, mode, key)
}

/// Re-encrypt (or leave plaintext) every profile's chat file after a vault mode change.
pub fn migrate_all_profiles(mode: VaultMode, key: Option<&DataKey>) -> Result<(), String> {
    let config = config_dir()?;
    ensure_migrated(&config).map_err(|e| e.to_string())?;
    migrate_all_profiles_at(&config, mode, key)
}

/// Snapshot for the HUD / Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudChatSnapshot {
    /// Profile id these turns belong to.
    pub profile_id: String,
    /// Ordered turns (oldest first).
    pub turns: Vec<HudTurn>,
}

/// Load HUD chat for a profile (active when `profile_id` is null/empty).
#[tauri::command]
pub fn hud_chat_snapshot(
    profile_id: Option<String>,
    state: tauri::State<'_, ui_vault::VaultState>,
) -> Result<HudChatSnapshot, String> {
    let id = resolve_profile_id(profile_id)?;
    let meta = ui_vault::load_meta();
    let key_guard = state
        .key
        .lock()
        .map_err(|_| "vault state poisoned".to_owned())?;
    let key = key_guard.as_ref();
    if meta.mode == VaultMode::Passphrase && key.is_none() {
        return Err("chat vault is locked".to_owned());
    }
    let turns = load_turns(&id, key)?;
    Ok(HudChatSnapshot {
        profile_id: id,
        turns,
    })
}

/// Persist HUD chat turns for a profile (active when `profile_id` is null/empty).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri owns JSON args.
pub fn hud_chat_save(
    profile_id: Option<String>,
    turns: Vec<HudTurn>,
    state: tauri::State<'_, ui_vault::VaultState>,
) -> Result<HudChatSnapshot, String> {
    let id = resolve_profile_id(profile_id)?;
    let meta = ui_vault::load_meta();
    let key_guard = state
        .key
        .lock()
        .map_err(|_| "vault state poisoned".to_owned())?;
    let key = key_guard.as_ref();
    if meta.mode == VaultMode::Passphrase && key.is_none() {
        return Err("chat vault is locked".to_owned());
    }
    let mode = if meta.mode == VaultMode::Unset {
        VaultMode::Plaintext
    } else {
        meta.mode
    };
    save_turns(&id, turns.clone(), mode, key)?;
    Ok(HudChatSnapshot {
        profile_id: id,
        turns: truncate_turns(turns),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_vault::{self, DataKey, VaultMode};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempConfig {
        path: PathBuf,
    }

    impl TempConfig {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("softwake-ui-hudchat-{}-{n}", std::process::id()));
            fs::create_dir_all(path.join("profiles").join("default")).expect("dirs");
            fs::write(
                path.join("softwake.json"),
                br#"{"version":1,"active_profile":"default"}"#,
            )
            .expect("cfg");
            fs::write(
                path.join("profiles").join("default").join("profile.json"),
                br#"{"version":1,"name":"Default"}"#,
            )
            .expect("meta");
            Self { path }
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn sample_turns() -> Vec<HudTurn> {
        vec![
            HudTurn {
                role: "user".into(),
                name: "You".into(),
                text: "hi".into(),
                ts: 1,
                error: false,
                note: String::new(),
            },
            HudTurn {
                role: "assistant".into(),
                name: "Sally".into(),
                text: "hello".into(),
                ts: 2,
                error: false,
                note: String::new(),
            },
        ]
    }

    #[test]
    fn truncate_keeps_newest_forty() {
        let turns: Vec<_> = (0..45)
            .map(|i| HudTurn {
                role: "user".into(),
                name: "You".into(),
                text: format!("{i}"),
                ts: i,
                error: false,
                note: String::new(),
            })
            .collect();
        let kept = truncate_turns(turns);
        assert_eq!(kept.len(), 40);
        assert_eq!(kept[0].ts, 5);
        assert_eq!(kept[39].ts, 44);
    }

    #[test]
    fn plaintext_round_trip() {
        let tmp = TempConfig::new();
        save_turns_at(
            &tmp.path,
            "default",
            sample_turns(),
            VaultMode::Plaintext,
            None,
        )
        .expect("save");
        let loaded = load_turns_at(&tmp.path, "default", None).expect("load");
        assert_eq!(loaded, sample_turns());
        let raw = fs::read_to_string(chat_path(&tmp.path, "default")).expect("read");
        assert!(raw.contains("\"hi\""));
        assert!(!raw.contains("ciphertext"));
    }

    #[test]
    fn encrypted_round_trip_and_wrong_key_fails() {
        let tmp = TempConfig::new();
        let salt = [9u8; 16];
        let key = ui_vault::derive_key("correct horse", &salt).expect("derive");
        save_turns_at(
            &tmp.path,
            "default",
            sample_turns(),
            VaultMode::Passphrase,
            Some(&key),
        )
        .expect("save");
        let loaded = load_turns_at(&tmp.path, "default", Some(&key)).expect("load");
        assert_eq!(loaded, sample_turns());
        let raw = fs::read_to_string(chat_path(&tmp.path, "default")).expect("read");
        assert!(raw.contains("ciphertext_b64"));
        assert!(!raw.contains("\"hi\""));
        let bad = DataKey::from_bytes([0u8; 32]);
        assert!(load_turns_at(&tmp.path, "default", Some(&bad)).is_err());
    }

    #[test]
    fn migrate_plaintext_to_encrypted() {
        let tmp = TempConfig::new();
        save_turns_at(
            &tmp.path,
            "default",
            sample_turns(),
            VaultMode::Plaintext,
            None,
        )
        .expect("plain");
        let salt = [3u8; 16];
        let key = ui_vault::derive_key("secret", &salt).expect("derive");
        migrate_all_profiles_at(&tmp.path, VaultMode::Passphrase, Some(&key)).expect("migrate");
        let loaded = load_turns_at(&tmp.path, "default", Some(&key)).expect("load");
        assert_eq!(loaded, sample_turns());
    }
}
