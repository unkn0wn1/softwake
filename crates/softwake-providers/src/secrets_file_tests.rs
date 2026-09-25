use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::FileSecretStore;
use crate::oauth::OAuthTokenSet;
use crate::secrets::{
    PLAINTEXT_WARNING, POINTER_IDENTITY_MESSAGE, SecretBag, SecretStore, SecretStoreError,
    UNSUPPORTED_BACKEND_MESSAGE,
};

const SENTINEL: &str = "sentinel-secret-7c1e";

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "softwake-providers-secrets-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temp");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).expect("meta").permissions().mode() & 0o777
}

#[test]
fn round_trip_writes_version_2_opt_in_and_mode_0600() {
    let dir = TempDir::new();
    let nested = dir.path.join("created");
    let path = nested.join("secrets.json");
    let store = FileSecretStore::new(&path).expect("store");
    assert!(store.load().expect("missing").xai_api_key.is_none());
    assert!(!path.exists());
    store
        .update(|bag| {
            bag.xai_api_key = Some("xai-secret".to_owned());
            bag.openai_api_key = Some("sk-test".to_owned());
            bag.openrouter_api_key = Some("or-test".to_owned());
            bag.openai_compatible_api_key = Some("compat-test".to_owned());
            bag.xai_oauth = Some(OAuthTokenSet {
                access_token: "a".to_owned(),
                refresh_token: "r".to_owned(),
                expires_at_ms: 9,
                token_type: "Bearer".to_owned(),
            });
        })
        .expect("save");
    let raw = std::fs::read_to_string(&path).expect("read");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
    assert_eq!(value["version"], 2);
    assert_eq!(value["plaintext"], true);
    assert_eq!(value["backend"], "plaintext");
    assert_eq!(value["plaintext_opt_in"], true);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&nested), 0o700);
    let loaded = store.load().expect("load");
    assert_eq!(loaded.version, 2);
    assert!(loaded.plaintext);
    assert_eq!(loaded.xai_api_key.as_deref(), Some("xai-secret"));
    assert_eq!(loaded.openai_api_key.as_deref(), Some("sk-test"));
    assert_eq!(loaded.openrouter_api_key.as_deref(), Some("or-test"));
    assert_eq!(
        loaded.openai_compatible_api_key.as_deref(),
        Some("compat-test")
    );
    assert_eq!(
        loaded
            .xai_oauth
            .as_ref()
            .map(|tokens| tokens.refresh_token.as_str()),
        Some("r")
    );
    assert_eq!(store.report().message, PLAINTEXT_WARNING);
}

#[test]
fn version_1_loads_tokens_by_equality_and_forces_plaintext() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"version":1,"plaintext":false,"xai_oauth":{{"access_token":"{SENTINEL}","refresh_token":"refresh-{SENTINEL}","expires_at_ms":9,"token_type":"Bearer"}}}}"#
        ),
    )
    .expect("write");
    let loaded = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect("load");
    assert_eq!(loaded.version, 1);
    assert!(loaded.plaintext);
    assert!(!format!("{loaded:?}").contains(SENTINEL));
    let tokens = loaded.xai_oauth.expect("oauth");
    assert_eq!(tokens.refresh_token, format!("refresh-{SENTINEL}"));
    assert_eq!(tokens.access_token, SENTINEL);
    assert!(!format!("{tokens:?}").contains(SENTINEL));
}

#[test]
fn missing_version_is_legacy_plaintext() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(&path, br#"{"xai_api_key":"legacy"}"#).expect("write");
    let loaded = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect("load");
    assert_eq!(loaded.version, 1);
    assert!(loaded.plaintext);
    assert_eq!(loaded.xai_api_key.as_deref(), Some("legacy"));
}

#[test]
fn version_99_is_unsupported() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(&path, br#"{"version":99}"#).expect("write");
    let error = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect_err("version");
    assert!(matches!(
        error,
        SecretStoreError::UnsupportedVersion { version: 99, .. }
    ));
}

#[test]
fn oversize_body_is_too_large() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    let body = vec![b' '; crate::secrets::MAX_SECRETS_BYTES + 1];
    std::fs::write(&path, body).expect("write");
    let error = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect_err("size");
    assert!(matches!(error, SecretStoreError::TooLarge { .. }));
}

#[test]
fn empty_path_is_rejected() {
    let error = FileSecretStore::new("").expect_err("empty");
    assert!(matches!(error, SecretStoreError::EmptyPath));
}

#[test]
fn pointer_with_secret_does_not_echo_the_value() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"version":2,"plaintext":false,"backend":"keyring","keyring_service":"softwake","keyring_user":"secret-bag","xai_api_key":"{SENTINEL}"}}"#
        ),
    )
    .expect("write");
    let error = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect_err("pointer");
    assert!(matches!(error, SecretStoreError::PointerHasSecrets { .. }));
    assert!(!error.to_string().contains(SENTINEL));
    assert!(!format!("{error:?}").contains(SENTINEL));
}

#[test]
fn bogus_backend_label_is_a_fixed_sentence() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(
        &path,
        format!(r#"{{"version":2,"backend":"{SENTINEL}","xai_api_key":"{SENTINEL}"}}"#),
    )
    .expect("write");
    let error = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect_err("backend");
    assert!(matches!(
        error,
        SecretStoreError::UnsupportedBackend {
            reason: UNSUPPORTED_BACKEND_MESSAGE
        }
    ));
    assert!(!error.to_string().contains(SENTINEL));
    assert!(!format!("{error:?}").contains(SENTINEL));
}

#[test]
fn bogus_keyring_user_is_a_fixed_sentence() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"version":2,"plaintext":false,"backend":"keyring","keyring_service":"softwake","keyring_user":"{SENTINEL}"}}"#
        ),
    )
    .expect("write");
    let error = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect_err("user");
    assert!(matches!(
        error,
        SecretStoreError::UnsupportedBackend {
            reason: POINTER_IDENTITY_MESSAGE
        }
    ));
    assert_eq!(error.to_string(), POINTER_IDENTITY_MESSAGE);
    assert!(!error.to_string().contains(SENTINEL));
}

#[test]
fn invalid_json_does_not_quote_the_body() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(
        &path,
        format!(r#"{{"version":2,"backend":"plaintext","xai_api_key":{SENTINEL}}}"#),
    )
    .expect("write");
    let error = FileSecretStore::new(&path)
        .expect("store")
        .load()
        .expect_err("json");
    assert!(matches!(error, SecretStoreError::Invalid { .. }));
    assert!(!error.to_string().contains(SENTINEL));
    assert!(!format!("{error:?}").contains(SENTINEL));
}

#[test]
fn plaintext_save_refuses_a_pointer() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    let pointer = br#"{"version":2,"plaintext":false,"backend":"keyring","keyring_service":"softwake","keyring_user":"secret-bag"}"#;
    std::fs::write(&path, pointer).expect("write");
    let store = FileSecretStore::new(&path).expect("store");
    let error = store.save(&SecretBag::empty()).expect_err("refuse");
    assert!(matches!(error, SecretStoreError::WrongBackend { .. }));
    assert_eq!(std::fs::read(&path).expect("unchanged"), pointer);
}

#[test]
fn save_does_not_replace_a_pointer_that_holds_a_secret() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    let original = format!(
        r#"{{"version":2,"plaintext":false,"backend":"keyring","keyring_service":"softwake","keyring_user":"secret-bag","xai_api_key":"{SENTINEL}"}}"#
    );
    std::fs::write(&path, &original).expect("write");
    let error = FileSecretStore::new(&path)
        .expect("store")
        .save(&SecretBag::empty())
        .expect_err("refuse");
    assert!(matches!(error, SecretStoreError::PointerHasSecrets { .. }));
    assert_eq!(std::fs::read_to_string(&path).expect("unchanged"), original);
    assert!(!error.to_string().contains(SENTINEL));
}
