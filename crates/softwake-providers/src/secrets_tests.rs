use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::{
    BackendChoice, BackendPref, KEYRING_STATUS, KeyringClient, OnDiskKind,
    PLAINTEXT_OPT_IN_MESSAGE, PLAINTEXT_WARNING, SecretBackend, SecretStore, SecretStoreError,
    StorageReport, UNSUPPORTED_BACKEND_MESSAGE, UnavailableSecretStore, decode_payload,
    open_store_with, opt_in_plaintext, pref_from_str, resolve_backend, resolve_secrets_file_from,
};
use crate::oauth::OAuthTokenSet;
use crate::secrets::SecretBag;

const SENTINEL: &str = "sentinel-secret-7c1e";

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "softwake-providers-resolve-{}-{n}",
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

#[derive(Clone)]
struct FakeKeyring {
    inner: Arc<Mutex<FakeState>>,
}

struct FakeState {
    payload: Option<String>,
    fail_set: bool,
    fail_get: bool,
}

impl FakeKeyring {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeState {
                payload: None,
                fail_set: false,
                fail_get: false,
            })),
        }
    }

    fn fail_set(self) -> Self {
        self.inner.lock().expect("lock").fail_set = true;
        self
    }

    fn fail_get(self) -> Self {
        self.inner.lock().expect("lock").fail_get = true;
        self
    }

    fn payload(&self) -> Option<String> {
        self.inner.lock().expect("lock").payload.clone()
    }
}

impl KeyringClient for FakeKeyring {
    fn get_payload(&self) -> Result<Option<String>, SecretStoreError> {
        let state = self.inner.lock().expect("lock");
        if state.fail_get {
            return Err(SecretStoreError::KeyringUnavailable);
        }
        Ok(state.payload.clone())
    }

    fn set_payload(&self, json: &str) -> Result<(), SecretStoreError> {
        let mut state = self.inner.lock().expect("lock");
        if state.fail_set {
            return Err(SecretStoreError::Keyring);
        }
        state.payload = Some(json.to_owned());
        Ok(())
    }

    fn delete_payload(&self) -> Result<(), SecretStoreError> {
        self.inner.lock().expect("lock").payload = None;
        Ok(())
    }
}

fn assert_choice(on_disk: OnDiskKind, pref: BackendPref, probe_ok: bool, expected: BackendChoice) {
    assert_eq!(
        resolve_backend(on_disk, pref, probe_ok).expect("choice"),
        expected
    );
}

fn assert_unavailable_keyring(on_disk: OnDiskKind, pref: BackendPref, probe_ok: bool) {
    let error = resolve_backend(on_disk, pref, probe_ok).expect_err("forced");
    assert!(matches!(error, SecretStoreError::KeyringUnavailable));
}

#[test]
fn resolve_missing_rows() {
    assert_choice(
        OnDiskKind::Missing,
        BackendPref::Auto,
        true,
        BackendChoice::Keyring { migrate: false },
    );
    assert_eq!(
        resolve_backend(OnDiskKind::Missing, BackendPref::Auto, false).expect("choice"),
        BackendChoice::Unavailable
    );
    assert_choice(
        OnDiskKind::Missing,
        BackendPref::Plaintext,
        true,
        BackendChoice::Plaintext,
    );
    assert_choice(
        OnDiskKind::Missing,
        BackendPref::Plaintext,
        false,
        BackendChoice::Plaintext,
    );
    assert_unavailable_keyring(OnDiskKind::Missing, BackendPref::Keyring, false);
    assert_choice(
        OnDiskKind::Missing,
        BackendPref::Keyring,
        true,
        BackendChoice::Keyring { migrate: false },
    );
}

#[test]
fn resolve_legacy_rows() {
    assert_choice(
        OnDiskKind::LegacyV1,
        BackendPref::Auto,
        true,
        BackendChoice::Keyring { migrate: true },
    );
    assert_choice(
        OnDiskKind::LegacyV1,
        BackendPref::Auto,
        false,
        BackendChoice::Plaintext,
    );
    assert_choice(
        OnDiskKind::LegacyV1,
        BackendPref::Plaintext,
        true,
        BackendChoice::Plaintext,
    );
    assert_choice(
        OnDiskKind::LegacyV1,
        BackendPref::Plaintext,
        false,
        BackendChoice::Plaintext,
    );
    assert_unavailable_keyring(OnDiskKind::LegacyV1, BackendPref::Keyring, false);
    assert_choice(
        OnDiskKind::LegacyV1,
        BackendPref::Keyring,
        true,
        BackendChoice::Keyring { migrate: true },
    );
}

#[test]
fn resolve_plaintext_v2_rows() {
    assert_choice(
        OnDiskKind::PlaintextV2,
        BackendPref::Auto,
        true,
        BackendChoice::Keyring { migrate: true },
    );
    assert_choice(
        OnDiskKind::PlaintextV2,
        BackendPref::Auto,
        false,
        BackendChoice::Plaintext,
    );
    assert_choice(
        OnDiskKind::PlaintextV2,
        BackendPref::Plaintext,
        true,
        BackendChoice::Plaintext,
    );
    assert_choice(
        OnDiskKind::PlaintextV2,
        BackendPref::Plaintext,
        false,
        BackendChoice::Plaintext,
    );
    assert_choice(
        OnDiskKind::PlaintextV2,
        BackendPref::Keyring,
        true,
        BackendChoice::Keyring { migrate: true },
    );
    assert_unavailable_keyring(OnDiskKind::PlaintextV2, BackendPref::Keyring, false);
}

#[test]
fn plaintext_pref_does_not_downgrade_a_pointer() {
    for probe_ok in [true, false] {
        assert_choice(
            OnDiskKind::KeyringPointer,
            BackendPref::Plaintext,
            probe_ok,
            BackendChoice::Keyring { migrate: false },
        );
        assert_choice(
            OnDiskKind::KeyringPointer,
            BackendPref::Auto,
            probe_ok,
            BackendChoice::Keyring { migrate: false },
        );
        assert_choice(
            OnDiskKind::KeyringPointer,
            BackendPref::Keyring,
            probe_ok,
            BackendChoice::Keyring { migrate: false },
        );
    }
}

#[test]
fn pref_strings_do_not_echo_the_raw_value() {
    assert_eq!(pref_from_str("").expect("blank"), BackendPref::Auto);
    assert_eq!(
        pref_from_str("plaintext").expect("plain"),
        BackendPref::Plaintext
    );
    assert_eq!(
        pref_from_str("keyring").expect("keyring"),
        BackendPref::Keyring
    );
    let error = pref_from_str(SENTINEL).expect_err("bad");
    assert_eq!(error.to_string(), UNSUPPORTED_BACKEND_MESSAGE);
    assert!(!error.to_string().contains(SENTINEL));
    assert!(!format!("{error:?}").contains(SENTINEL));
}

#[test]
fn resolve_prefers_xdg_state_and_rejects_blank_bases() {
    let path = resolve_secrets_file_from(Some("/state"), Some("/home")).expect("path");
    assert_eq!(path, PathBuf::from("/state/softwake/secrets.json"));
    let home = resolve_secrets_file_from(Some("  "), Some("/home")).expect("home");
    assert_eq!(
        home,
        PathBuf::from("/home/.local/state/softwake/secrets.json")
    );
    assert!(matches!(
        resolve_secrets_file_from(None::<&str>, None::<&str>),
        Err(SecretStoreError::NoStateDir)
    ));
    assert!(matches!(
        resolve_secrets_file_from(Some(" "), Some("")),
        Err(SecretStoreError::NoStateDir)
    ));
}

#[test]
fn debug_redacts_secret_strings_and_omits_lengths() {
    let bag = SecretBag {
        xai_api_key: Some(SENTINEL.to_owned()),
        openai_api_key: Some(SENTINEL.to_owned()),
        openrouter_api_key: Some(SENTINEL.to_owned()),
        openai_compatible_api_key: Some(SENTINEL.to_owned()),
        email_smtp_password: Some(SENTINEL.to_owned()),
        xai_oauth: Some(OAuthTokenSet {
            access_token: SENTINEL.to_owned(),
            refresh_token: SENTINEL.to_owned(),
            expires_at_ms: 44,
            token_type: "Bearer".to_owned(),
        }),
        ..SecretBag::empty()
    };
    let text = format!("{bag:?}");
    let secret_len = SENTINEL.len().to_string();
    assert!(text.contains("<redacted>"));
    assert!(!text.contains(SENTINEL));
    assert!(!text.contains(&secret_len));
    let tokens = bag.xai_oauth.expect("oauth");
    let oauth = format!("{tokens:?}");
    assert!(oauth.contains("<redacted>"));
    assert!(!oauth.contains(SENTINEL));
    assert!(!oauth.contains(&secret_len));
    assert_eq!(tokens.access_token, SENTINEL);
}

#[test]
fn fake_migration_moves_the_sentinel_out_of_the_file() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(
        &path,
        format!(r#"{{"version":1,"plaintext":true,"xai_api_key":"{SENTINEL}"}}"#),
    )
    .expect("write");
    let fake = FakeKeyring::new();
    let store = open_store_with(&path, BackendPref::Auto, true, fake.clone()).expect("open");
    let loaded = store.load().expect("load");
    assert_eq!(loaded.xai_api_key.as_deref(), Some(SENTINEL));
    assert!(!loaded.plaintext);
    assert_eq!(loaded.version, 2);
    let file = std::fs::read_to_string(&path).expect("file");
    assert!(!file.contains(SENTINEL));
    let value: serde_json::Value = serde_json::from_str(&file).expect("json");
    assert_eq!(value["backend"], "keyring");
    assert_eq!(value["plaintext"], false);
    assert!(value.get("xai_api_key").is_none());
    let payload = fake.payload().expect("payload");
    assert!(payload.contains(SENTINEL));
    assert_eq!(store.report().backend, SecretBackend::Keyring);
    assert_eq!(store.report().message, KEYRING_STATUS);
}

#[test]
fn failed_migration_leaves_version_1_bytes_unchanged() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    let original = format!(r#"{{"version":1,"plaintext":true,"xai_api_key":"{SENTINEL}"}}"#);
    std::fs::write(&path, &original).expect("write");
    let error = match open_store_with(
        &path,
        BackendPref::Auto,
        true,
        FakeKeyring::new().fail_set(),
    ) {
        Err(error) => error,
        Ok(_store) => panic!("migration should fail before the store is returned"),
    };
    assert!(matches!(error, SecretStoreError::Keyring));
    assert!(!error.to_string().contains(SENTINEL));
    assert!(!format!("{error:?}").contains(SENTINEL));
    assert_eq!(std::fs::read_to_string(&path).expect("file"), original);
}

#[test]
fn empty_legacy_bag_writes_a_pointer_and_deletes_the_item() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    std::fs::write(&path, br#"{"version":1,"plaintext":true}"#).expect("write");
    let fake = FakeKeyring::new();
    fake.inner.lock().expect("lock").payload = Some(SENTINEL.to_owned());
    let _store = open_store_with(&path, BackendPref::Auto, true, fake.clone()).expect("open");
    assert!(fake.payload().is_none());
    let file = std::fs::read_to_string(&path).expect("file");
    assert!(!file.contains(SENTINEL));
    assert!(file.contains("keyring"));
}

#[test]
fn pointer_without_an_item_is_an_empty_keyring_bag() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    crate::secrets_file::write_pointer(&path).expect("pointer");
    let store =
        open_store_with(&path, BackendPref::Plaintext, false, FakeKeyring::new()).expect("open");
    let loaded = store.load().expect("load");
    assert!(loaded.xai_api_key.is_none());
    assert!(!loaded.plaintext);
    assert_eq!(loaded.version, 2);
    assert_eq!(store.report().backend.as_str(), "keyring");
}

#[test]
fn pointer_service_error_is_not_an_empty_bag() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    crate::secrets_file::write_pointer(&path).expect("pointer");
    let store = open_store_with(
        &path,
        BackendPref::Auto,
        false,
        FakeKeyring::new().fail_get(),
    )
    .expect("open");
    let error = store.load().expect_err("service");
    assert!(matches!(error, SecretStoreError::KeyringUnavailable));
}

#[test]
fn plaintext_pref_does_not_copy_a_pointer_payload_into_the_file() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    crate::secrets_file::write_pointer(&path).expect("pointer");
    let before = std::fs::read(&path).expect("before");
    let fake = FakeKeyring::new();
    fake.inner.lock().expect("lock").payload = Some(format!(r#"{{"xai_api_key":"{SENTINEL}"}}"#));
    let store = open_store_with(&path, BackendPref::Plaintext, false, fake).expect("open");
    let loaded = store.load().expect("load");
    assert_eq!(loaded.xai_api_key.as_deref(), Some(SENTINEL));
    assert!(!loaded.plaintext);
    let mut bag = loaded;
    bag.openai_api_key = Some(SENTINEL.to_owned());
    store.save(&bag).expect("save");
    let file = std::fs::read_to_string(&path).expect("file");
    assert!(!file.contains(SENTINEL));
    assert_eq!(std::fs::read(&path).expect("still pointer"), before);
}

#[test]
fn unavailable_store_load_is_empty_and_save_needs_opt_in() {
    let store = UnavailableSecretStore;
    assert!(store.load().expect("load").xai_api_key.is_none());
    let error = store.save(&SecretBag::empty()).expect_err("save");
    assert!(matches!(error, SecretStoreError::PlaintextOptInRequired));
    assert_eq!(error.to_string(), PLAINTEXT_OPT_IN_MESSAGE);
    assert_eq!(store.report().backend.as_str(), "unavailable");
    assert_eq!(store.report().message, PLAINTEXT_OPT_IN_MESSAGE);
}

#[test]
fn plaintext_report_uses_the_warning_constant() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    let store =
        open_store_with(&path, BackendPref::Plaintext, false, FakeKeyring::new()).expect("open");
    assert_eq!(store.report().backend.as_str(), "plaintext");
    assert_eq!(store.report().message, PLAINTEXT_WARNING);
    assert!(!path.exists());
}

#[test]
fn opt_in_writes_a_marker_without_secret_keys() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    let report = opt_in_plaintext(&path, BackendPref::Auto, false).expect("opt in");
    assert_eq!(report.backend, SecretBackend::Plaintext);
    assert_eq!(report.message, PLAINTEXT_WARNING);
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("file")).expect("json");
    assert_eq!(value["version"], 2);
    assert_eq!(value["plaintext"], true);
    assert_eq!(value["backend"], "plaintext");
    assert_eq!(value["plaintext_opt_in"], true);
    assert!(value.get("xai_api_key").is_none());
    assert!(value.get("openai_api_key").is_none());
    assert!(value.get("openrouter_api_key").is_none());
    assert!(value.get("openai_compatible_api_key").is_none());
    assert!(value.get("xai_oauth").is_none());
    let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn opt_in_when_keyring_is_chosen_does_not_change_a_pointer() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    crate::secrets_file::write_pointer(&path).expect("pointer");
    let before = std::fs::read(&path).expect("before");
    let error = opt_in_plaintext(&path, BackendPref::Auto, true).expect_err("keyring");
    assert!(matches!(error, SecretStoreError::KeyringAvailable));
    assert_eq!(std::fs::read(&path).expect("after"), before);

    let missing = dir.path.join("missing.json");
    let error = opt_in_plaintext(&missing, BackendPref::Auto, true).expect_err("available");
    assert!(matches!(error, SecretStoreError::KeyringAvailable));
    assert!(!missing.exists());
}

#[test]
fn payload_round_trip_keeps_plaintext_false() {
    let dir = TempDir::new();
    let path = dir.path.join("secrets.json");
    let mut bag = SecretBag::keyring_empty();
    bag.xai_api_key = Some(SENTINEL.to_owned());
    let json = super::encode_payload(&bag).expect("encode");
    let loaded = decode_payload(&path, &json).expect("decode");
    assert_eq!(loaded.xai_api_key.as_deref(), Some(SENTINEL));
    assert!(!loaded.plaintext);
    let error = decode_payload(&path, SENTINEL).expect_err("bad payload");
    assert!(matches!(error, SecretStoreError::Keyring));
    assert!(!error.to_string().contains(SENTINEL));
}

#[test]
fn storage_backend_names_match_the_three_reports() {
    assert_eq!(SecretBackend::Keyring.as_str(), "keyring");
    assert_eq!(SecretBackend::Plaintext.as_str(), "plaintext");
    assert_eq!(SecretBackend::Unavailable.as_str(), "unavailable");
    assert_eq!(StorageReport::keyring().message, KEYRING_STATUS);
    assert_eq!(StorageReport::plaintext().message, PLAINTEXT_WARNING);
    assert_eq!(
        StorageReport::unavailable().message,
        PLAINTEXT_OPT_IN_MESSAGE
    );
}
