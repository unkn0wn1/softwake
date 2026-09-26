//! Passphrase unlock for softwake-ui HUD chat at rest.
//!
//! Metadata: `$XDG_CONFIG_HOME/softwake/ui-vault.json` (no raw key, no chat).
//! Data key stays in process memory (`VaultState`) and optionally in the OS
//! keyring (`softwake` / `ui-data-key`). softwaked never sees the passphrase.

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Mutex;

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use softwake_soul::resolve_config_dir;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::hud_chat;

const META_FILE: &str = "ui-vault.json";
const META_VERSION: u32 = 1;
const KEYRING_SERVICE: &str = "softwake";
const KEYRING_USER: &str = "ui-data-key";
/// OWASP interactive-ish Argon2id: 19 MiB, 2 iterations, parallelism 1.
const ARGON2_M_KIB: u32 = 19 * 1024;
const ARGON2_T: u32 = 2;
const ARGON2_P: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// Vault mode stored in `ui-vault.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VaultMode {
    /// Operator has not chosen yet (first run).
    #[default]
    Unset,
    /// Chats stored as plaintext JSON.
    Plaintext,
    /// Chats sealed with a passphrase-derived key.
    Passphrase,
}

impl VaultMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Plaintext => "plaintext",
            Self::Passphrase => "passphrase",
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "plaintext" => Self::Plaintext,
            "passphrase" => Self::Passphrase,
            _ => Self::Unset,
        }
    }
}

/// 32-byte AEAD key. Zeroized on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct DataKey {
    bytes: [u8; KEY_LEN],
}

impl DataKey {
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self { bytes }
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.bytes
    }

    #[must_use]
    pub fn to_b64(&self) -> String {
        B64.encode(self.bytes)
    }

    /// Decode a base64 key from the OS keyring wrap.
    ///
    /// # Errors
    ///
    /// Bad base64 or wrong length.
    pub fn from_b64(raw: &str) -> Result<Self, String> {
        let bytes = B64.decode(raw.trim()).map_err(|e| e.to_string())?;
        if bytes.len() != KEY_LEN {
            return Err("wrapped key has wrong length".to_owned());
        }
        let mut arr = [0u8; KEY_LEN];
        arr.copy_from_slice(&bytes);
        Ok(Self::from_bytes(arr))
    }
}

/// Process-wide unlocked data key (softwake-ui only).
#[derive(Default)]
pub struct VaultState {
    /// Present after a successful unlock / set / keyring auto-unlock.
    pub key: Mutex<Option<DataKey>>,
}

/// On-disk vault metadata (never stores the raw key or chat bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiVaultMeta {
    pub version: u32,
    pub mode: VaultMode,
    #[serde(default)]
    pub salt_b64: String,
    #[serde(default)]
    pub keyring_wrap: bool,
    #[serde(default)]
    pub verifier_nonce_b64: String,
    #[serde(default)]
    pub verifier_ciphertext_b64: String,
}

impl Default for UiVaultMeta {
    fn default() -> Self {
        Self {
            version: META_VERSION,
            mode: VaultMode::Unset,
            salt_b64: String::new(),
            keyring_wrap: false,
            verifier_nonce_b64: String::new(),
            verifier_ciphertext_b64: String::new(),
        }
    }
}

/// Status snapshot for HUD overlay + Settings General.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // Wire snapshot for JS; each flag is independent.
pub struct UiVaultStatus {
    pub mode: String,
    pub unlocked: bool,
    pub keyring_wrap: bool,
    pub keyring_available: bool,
    pub plaintext_warning: bool,
}

fn config_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())
}

fn meta_path_at(config: &std::path::Path) -> PathBuf {
    config.join(META_FILE)
}

/// Load vault meta from an explicit Softwake config root.
#[must_use]
pub fn load_meta_at(config: &std::path::Path) -> UiVaultMeta {
    let path = meta_path_at(config);
    let Ok(bytes) = fs::read(&path) else {
        return UiVaultMeta::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Persist vault meta under an explicit Softwake config root.
pub fn save_meta_at(config: &std::path::Path, meta: &UiVaultMeta) -> Result<(), String> {
    let path = meta_path_at(config);
    let body = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
    atomic_write(&path, &body)
}

fn atomic_write(path: &std::path::Path, body: &[u8]) -> Result<(), String> {
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

/// Load vault meta, or defaults when missing.
#[must_use]
pub fn load_meta() -> UiVaultMeta {
    let Ok(config) = config_dir() else {
        return UiVaultMeta::default();
    };
    load_meta_at(&config)
}

fn save_meta(meta: &UiVaultMeta) -> Result<(), String> {
    let config = config_dir()?;
    save_meta_at(&config, meta)
}

/// Derive a 32-byte data key with Argon2id.
///
/// # Errors
///
/// Argon2 parameter / hash failure.
pub fn derive_key(passphrase: &str, salt: &[u8]) -> Result<DataKey, String> {
    if passphrase.is_empty() {
        return Err("passphrase must not be empty".to_owned());
    }
    if salt.len() != SALT_LEN {
        return Err("salt must be 16 bytes".to_owned());
    }
    let params =
        Params::new(ARGON2_M_KIB, ARGON2_T, ARGON2_P, Some(KEY_LEN)).map_err(|e| e.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| e.to_string())?;
    Ok(DataKey::from_bytes(out))
}

fn random_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    salt
}

fn random_nonce() -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Seal arbitrary bytes with ChaCha20-Poly1305. Returns `(nonce_b64, ciphertext_b64)`.
///
/// # Errors
///
/// AEAD encrypt failure.
pub fn seal_bytes(key: &DataKey, plain: &[u8]) -> Result<(String, String), String> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    let nonce = random_nonce();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain)
        .map_err(|_| "encrypt failed".to_owned())?;
    Ok((B64.encode(nonce), B64.encode(ct)))
}

/// Open sealed bytes.
///
/// # Errors
///
/// Bad base64, wrong key, or tampered ciphertext.
pub fn open_bytes(key: &DataKey, nonce_b64: &str, ciphertext_b64: &str) -> Result<Vec<u8>, String> {
    let nonce_raw = B64.decode(nonce_b64.trim()).map_err(|e| e.to_string())?;
    if nonce_raw.len() != NONCE_LEN {
        return Err("nonce has wrong length".to_owned());
    }
    let ct = B64
        .decode(ciphertext_b64.trim())
        .map_err(|e| e.to_string())?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key.as_bytes()));
    cipher
        .decrypt(Nonce::from_slice(&nonce_raw), ct.as_ref())
        .map_err(|_| "decrypt failed (wrong passphrase or tampered data)".to_owned())
}

fn make_verifier(key: &DataKey) -> Result<(String, String), String> {
    seal_bytes(key, b"softwake-ui-vault-v1")
}

fn check_verifier(key: &DataKey, meta: &UiVaultMeta) -> Result<(), String> {
    if meta.verifier_nonce_b64.is_empty() || meta.verifier_ciphertext_b64.is_empty() {
        return Err("vault verifier is missing".to_owned());
    }
    let plain = open_bytes(key, &meta.verifier_nonce_b64, &meta.verifier_ciphertext_b64)?;
    if plain.as_slice() != b"softwake-ui-vault-v1" {
        return Err("vault verifier mismatch".to_owned());
    }
    Ok(())
}

fn keyring_available() -> bool {
    let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, "ui-data-key-probe") else {
        return false;
    };
    if entry.set_password("ok").is_err() {
        return false;
    }
    let ok = matches!(entry.get_password().as_deref(), Ok("ok"));
    let _ = entry.delete_credential();
    ok
}

fn keyring_store(key: &DataKey) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("keyring unavailable: {e}"))?;
    entry
        .set_password(&key.to_b64())
        .map_err(|e| format!("keyring store failed: {e}"))
}

fn keyring_load() -> Result<Option<DataKey>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("keyring unavailable: {e}"))?;
    match entry.get_password() {
        Ok(raw) => Ok(Some(DataKey::from_b64(&raw)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring load failed: {e}")),
    }
}

fn keyring_clear() {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
}

fn status_from(meta: &UiVaultMeta, unlocked: bool) -> UiVaultStatus {
    UiVaultStatus {
        mode: meta.mode.as_str().to_owned(),
        unlocked,
        keyring_wrap: meta.keyring_wrap,
        keyring_available: keyring_available(),
        plaintext_warning: meta.mode == VaultMode::Plaintext || meta.mode == VaultMode::Unset,
    }
}

fn is_unlocked(state: &VaultState) -> bool {
    state.key.lock().is_ok_and(|g| g.is_some())
}

/// Vault status for HUD + Settings.
#[tauri::command]
pub fn ui_vault_status(state: tauri::State<'_, VaultState>) -> UiVaultStatus {
    let meta = load_meta();
    let unlocked = match meta.mode {
        VaultMode::Passphrase => is_unlocked(&state),
        VaultMode::Plaintext | VaultMode::Unset => true,
    };
    status_from(&meta, unlocked)
}

/// First-run: stay in plaintext until a passphrase is set.
#[tauri::command]
pub fn ui_vault_skip_plaintext(
    state: tauri::State<'_, VaultState>,
) -> Result<UiVaultStatus, String> {
    let mut meta = load_meta();
    if meta.mode == VaultMode::Passphrase {
        return Err("vault already uses a passphrase".to_owned());
    }
    meta.version = META_VERSION;
    meta.mode = VaultMode::Plaintext;
    meta.salt_b64.clear();
    meta.verifier_nonce_b64.clear();
    meta.verifier_ciphertext_b64.clear();
    meta.keyring_wrap = false;
    save_meta(&meta)?;
    keyring_clear();
    if let Ok(mut guard) = state.key.lock() {
        *guard = None;
    }
    Ok(status_from(&meta, true))
}

/// Set (or change) the passphrase; encrypts existing HUD chats.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn ui_vault_set_passphrase(
    passphrase: String,
    keyring_wrap: bool,
    state: tauri::State<'_, VaultState>,
) -> Result<UiVaultStatus, String> {
    let passphrase = passphrase.trim();
    if passphrase.len() < 4 {
        return Err("passphrase must be at least 4 characters".to_owned());
    }
    let salt = random_salt();
    let key = derive_key(passphrase, &salt)?;
    let (verifier_nonce_b64, verifier_ciphertext_b64) = make_verifier(&key)?;
    let meta = UiVaultMeta {
        version: META_VERSION,
        mode: VaultMode::Passphrase,
        salt_b64: B64.encode(salt),
        keyring_wrap,
        verifier_nonce_b64,
        verifier_ciphertext_b64,
    };
    // Encrypt chats before committing meta so a mid-failure leaves plaintext readable.
    hud_chat::migrate_all_profiles(VaultMode::Passphrase, Some(&key))?;
    save_meta(&meta)?;
    if keyring_wrap {
        let _ = keyring_store(&key);
    } else {
        keyring_clear();
    }
    if let Ok(mut guard) = state.key.lock() {
        *guard = Some(key);
    }
    Ok(status_from(&meta, true))
}

/// Unlock with a passphrase.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn ui_vault_unlock(
    passphrase: String,
    state: tauri::State<'_, VaultState>,
) -> Result<UiVaultStatus, String> {
    let meta = load_meta();
    if meta.mode != VaultMode::Passphrase {
        return Err("vault is not passphrase-locked".to_owned());
    }
    let salt = B64
        .decode(meta.salt_b64.trim())
        .map_err(|e| e.to_string())?;
    if salt.len() != SALT_LEN {
        return Err("vault salt is invalid".to_owned());
    }
    let key = derive_key(passphrase.trim(), &salt)?;
    check_verifier(&key, &meta)?;
    if meta.keyring_wrap {
        let _ = keyring_store(&key);
    }
    if let Ok(mut guard) = state.key.lock() {
        *guard = Some(key);
    }
    Ok(status_from(&meta, true))
}

/// Try OS keyring auto-unlock.
#[tauri::command]
pub fn ui_vault_try_keyring(state: tauri::State<'_, VaultState>) -> Result<UiVaultStatus, String> {
    let meta = load_meta();
    if meta.mode != VaultMode::Passphrase {
        return Ok(status_from(&meta, true));
    }
    if !meta.keyring_wrap {
        return Ok(status_from(&meta, is_unlocked(&state)));
    }
    match keyring_load()? {
        Some(key) => {
            check_verifier(&key, &meta)?;
            if let Ok(mut guard) = state.key.lock() {
                *guard = Some(key);
            }
            Ok(status_from(&meta, true))
        }
        None => Ok(status_from(&meta, false)),
    }
}

/// Lock: zeroize the in-memory key (keyring wrap left in place for next auto-unlock).
#[tauri::command]
#[allow(clippy::unnecessary_wraps)] // Match other vault commands' Result shape for JS.
pub fn ui_vault_lock(state: tauri::State<'_, VaultState>) -> Result<UiVaultStatus, String> {
    let meta = load_meta();
    if let Ok(mut guard) = state.key.lock() {
        *guard = None;
    }
    let unlocked = meta.mode != VaultMode::Passphrase;
    Ok(status_from(&meta, unlocked))
}

/// Update the keyring-wrap preference (requires unlocked passphrase mode).
#[tauri::command]
pub fn ui_vault_set_keyring_wrap(
    enabled: bool,
    state: tauri::State<'_, VaultState>,
) -> Result<UiVaultStatus, String> {
    let mut meta = load_meta();
    if meta.mode != VaultMode::Passphrase {
        return Err("keyring wrap requires passphrase mode".to_owned());
    }
    let guard = state
        .key
        .lock()
        .map_err(|_| "vault state poisoned".to_owned())?;
    let Some(key) = guard.as_ref() else {
        return Err("chat vault is locked".to_owned());
    };
    meta.keyring_wrap = enabled;
    if enabled {
        keyring_store(key)?;
    } else {
        keyring_clear();
    }
    save_meta(&meta)?;
    Ok(status_from(&meta, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempConfig {
        path: PathBuf,
    }

    impl TempConfig {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("softwake-ui-vault-{}-{n}", std::process::id()));
            fs::create_dir_all(path.join("profiles").join("default")).expect("dirs");
            fs::write(
                path.join("softwake.json"),
                br#"{"version":1,"active_profile":"default"}"#,
            )
            .expect("cfg");
            Self { path }
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn derive_and_seal_round_trip() {
        let salt = [7u8; 16];
        let key = derive_key("test-pass", &salt).expect("derive");
        let (n, c) = seal_bytes(&key, b"hello vault").expect("seal");
        let plain = open_bytes(&key, &n, &c).expect("open");
        assert_eq!(plain, b"hello vault");
        let bad = DataKey::from_bytes([1u8; 32]);
        assert!(open_bytes(&bad, &n, &c).is_err());
    }

    #[test]
    fn wrong_passphrase_fails_verifier() {
        let tmp = TempConfig::new();
        let salt = random_salt();
        let key = derive_key("good-pass", &salt).expect("derive");
        let (vn, vc) = make_verifier(&key).expect("ver");
        let meta = UiVaultMeta {
            version: 1,
            mode: VaultMode::Passphrase,
            salt_b64: B64.encode(salt),
            keyring_wrap: false,
            verifier_nonce_b64: vn,
            verifier_ciphertext_b64: vc,
        };
        save_meta_at(&tmp.path, &meta).expect("save");
        let loaded = load_meta_at(&tmp.path);
        let salt2 = B64.decode(&loaded.salt_b64).expect("salt");
        let bad = derive_key("bad-pass", &salt2).expect("derive bad");
        assert!(check_verifier(&bad, &loaded).is_err());
        check_verifier(&key, &loaded).expect("good");
        assert_eq!(VaultMode::parse("plaintext"), VaultMode::Plaintext);
        assert_eq!(VaultMode::parse("passphrase"), VaultMode::Passphrase);
        assert_eq!(VaultMode::parse(""), VaultMode::Unset);
        assert_eq!(VaultMode::Passphrase.as_str(), "passphrase");
    }

    #[test]
    fn skip_plaintext_writes_meta() {
        let tmp = TempConfig::new();
        let mut meta = load_meta_at(&tmp.path);
        assert_eq!(meta.mode, VaultMode::Unset);
        meta.mode = VaultMode::Plaintext;
        save_meta_at(&tmp.path, &meta).expect("save");
        let loaded = load_meta_at(&tmp.path);
        assert_eq!(loaded.mode, VaultMode::Plaintext);
    }
}
