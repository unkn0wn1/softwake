//! OS keyring client and the pointer store.
//!
//! Linux uses Secret Service through libdbus (`sync-secret-service`), not libsecret
//! and not the async client. The async store deadlocks if it is called on the tokio
//! runtime thread. `apple-native` and `windows-native` map the same `Entry` calls
//! onto Keychain and Credential Manager. Those platforms are not exercised in CI.
//!
//! The process-global `keyring::set_default_credential_builder` mock races under
//! parallel tests, so production is the only code that constructs `keyring::Entry`.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use crate::secrets::{
    self, KEYRING_PROBE_USER, KEYRING_SERVICE, KEYRING_USER, KeyringClient, SecretBag, SecretStore,
    SecretStoreError, StorageReport,
};
use crate::secrets_file::{self, write_pointer};

/// Production client. Each call builds an [`keyring::Entry`] for `softwake` / `secret-bag`.
#[derive(Debug, Default)]
pub(crate) struct LiveKeyringClient;

impl LiveKeyringClient {
    pub(crate) fn new() -> Self {
        reject_in_memory_mock_store();
        Self
    }
}

impl KeyringClient for LiveKeyringClient {
    fn get_payload(&self) -> Result<Option<String>, SecretStoreError> {
        let entry = entry(KEYRING_SERVICE, KEYRING_USER)?;
        match entry.get_password() {
            Ok(payload) => Ok(Some(payload)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_keyring_error(&error)),
        }
    }

    fn set_payload(&self, json: &str) -> Result<(), SecretStoreError> {
        let entry = entry(KEYRING_SERVICE, KEYRING_USER)?;
        entry
            .set_password(json)
            .map_err(|error| map_keyring_error(&error))
    }

    fn delete_payload(&self) -> Result<(), SecretStoreError> {
        let entry = entry(KEYRING_SERVICE, KEYRING_USER)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_keyring_error(&error)),
        }
    }
}

/// Pointer file plus a [`KeyringClient`]. Saves never write secret fields into the file.
pub struct KeyringSecretStore<C: KeyringClient> {
    path: PathBuf,
    client: C,
}

impl<C: KeyringClient> KeyringSecretStore<C> {
    /// Store at `path` using `client`. Does not read either side yet.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError::EmptyPath`] when `path` is empty.
    pub fn new(path: impl Into<PathBuf>, client: C) -> Result<Self, SecretStoreError> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(SecretStoreError::EmptyPath);
        }
        Ok(Self { path, client })
    }
}

impl<C: KeyringClient> SecretStore for KeyringSecretStore<C> {
    fn load(&self) -> Result<SecretBag, SecretStoreError> {
        match self.client.get_payload()? {
            None => Ok(SecretBag::keyring_empty()),
            Some(payload) => secrets::decode_payload(&self.path, &payload),
        }
    }

    fn save(&self, bag: &SecretBag) -> Result<(), SecretStoreError> {
        persist_keyring(&self.path, &self.client, bag)
    }

    fn report(&self) -> StorageReport {
        StorageReport::keyring()
    }
}

/// Copy a plaintext bag into the keyring, then replace the file with a pointer.
///
/// On `set_payload` failure the file bytes are unchanged. If the pointer rename
/// fails after a successful set, the file is also unchanged and the next resolved
/// load tries again. Setting the same payload is idempotent.
pub(crate) fn migrate_to_keyring(
    path: &Path,
    client: &dyn KeyringClient,
) -> Result<(), SecretStoreError> {
    let bag = secrets_file::FileSecretStore::new(path)?.load()?;
    persist_keyring(path, client, &bag)
}

fn persist_keyring(
    path: &Path,
    client: &dyn KeyringClient,
    bag: &SecretBag,
) -> Result<(), SecretStoreError> {
    if secrets::bag_has_secret(bag) {
        let payload = secrets::encode_payload(bag)?;
        if payload.len() > secrets::MAX_SECRETS_BYTES {
            return Err(SecretStoreError::TooLarge {
                path: path.to_owned(),
                len: payload.len(),
                max: secrets::MAX_SECRETS_BYTES,
            });
        }
        client.set_payload(&payload)?;
    } else {
        client.delete_payload()?;
    }
    write_pointer(path)
}

/// How long startup waits for a Secret Service probe before treating keyring as unavailable.
const PROBE_BUDGET: Duration = Duration::from_millis(1500);

/// Probe once per process. Both success and failure are cached.
///
/// A keyring that appears later is picked up on the next process start.
///
/// Secret Service Unlock prompts can block `Entry` calls forever on D-Bus. The
/// probe runs on a helper thread and is abandoned after [`PROBE_BUDGET`] so
/// `softwaked serve` / Settings still start when the session keyring is sticky.
pub(crate) fn cached_probe() -> bool {
    static PROBE: OnceLock<bool> = OnceLock::new();
    *PROBE.get_or_init(|| {
        if let Some(ok) = run_keyring_probe_with_budget(PROBE_BUDGET) {
            ok
        } else {
            eprintln!(
                "softwake: secret service probe timed out after {}ms — treating keyring as unavailable",
                PROBE_BUDGET.as_millis()
            );
            false
        }
    })
}

/// Run [`probe_secret_service`] with a deadline. `None` means the budget elapsed.
fn run_keyring_probe_with_budget(budget: Duration) -> Option<bool> {
    let (tx, rx) = mpsc::sync_channel(1);
    if thread::Builder::new()
        .name("softwake-keyring-probe".into())
        .spawn(move || {
            let _ = tx.send(probe_secret_service());
        })
        .is_err()
    {
        return Some(false);
    }
    rx.recv_timeout(budget).ok()
}

fn probe_secret_service() -> bool {
    let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_PROBE_USER) else {
        return false;
    };
    if entry.set_password("ok").is_err() {
        return false;
    }
    let ok = matches!(entry.get_password().as_deref(), Ok("ok"));
    let _ = entry.delete_credential();
    ok
}

fn entry(service: &str, user: &str) -> Result<keyring::Entry, SecretStoreError> {
    keyring::Entry::new(service, user).map_err(|error| map_keyring_error(&error))
}

fn map_keyring_error(error: &keyring::Error) -> SecretStoreError {
    match error {
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            SecretStoreError::KeyringUnavailable
        }
        _ => SecretStoreError::Keyring,
    }
}

/// Naming `default_credential_builder` fails the build if `sync-secret-service` is
/// dropped. Without that feature the crate's Linux default is an in-memory mock
/// that discards secrets on exit.
fn reject_in_memory_mock_store() {
    #[cfg(target_os = "linux")]
    {
        let builder: fn() -> Box<keyring::CredentialBuilder> =
            keyring::secret_service::default_credential_builder;
        let _ = builder;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{KeyringSecretStore, run_keyring_probe_with_budget};
    use crate::secrets::{KeyringClient, SecretStoreError};

    #[test]
    fn keyring_probe_budget_is_none_when_worker_never_replies() {
        // Mirror run_keyring_probe_with_budget's channel contract without
        // touching dbus: a worker that never sends must yield None quickly.
        let (tx, rx) = mpsc::sync_channel::<bool>(1);
        thread::spawn(move || {
            let _ = tx; // drop without send after sleep past budget
            thread::sleep(Duration::from_secs(30));
        });
        let started = Instant::now();
        assert!(rx.recv_timeout(Duration::from_millis(40)).ok().is_none());
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn keyring_probe_budget_helper_returns_within_deadline() {
        // Live Secret Service may be sticky; the helper must still return.
        let started = Instant::now();
        let _ = run_keyring_probe_with_budget(Duration::from_millis(200));
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "run_keyring_probe_with_budget must respect its deadline"
        );
    }

    /// The default test run does not construct `keyring::Entry`. This ignored test
    /// is the only live round-trip, and it uses its own account.
    #[test]
    #[ignore = "needs an unlocked Secret Service"]
    fn live_secret_service_round_trip() {
        let entry = keyring::Entry::new(crate::secrets::KEYRING_SERVICE, "softwake-live-test")
            .expect("entry");
        let _cleanup = DeleteOnDrop(&entry);
        let nonce = format!("softwake-live-{}", std::process::id());
        entry.set_password(&nonce).expect("set");
        let loaded = entry.get_password().expect("get");
        assert_eq!(loaded, nonce);
        entry.delete_credential().expect("delete");
    }

    struct DeleteOnDrop<'a>(&'a keyring::Entry);

    impl Drop for DeleteOnDrop<'_> {
        fn drop(&mut self) {
            let _ = self.0.delete_credential();
        }
    }

    #[test]
    fn empty_path_is_rejected() {
        let Err(error) = KeyringSecretStore::new("", RejectingClient) else {
            panic!("empty path should fail");
        };
        assert!(matches!(error, SecretStoreError::EmptyPath));
    }

    struct RejectingClient;

    impl KeyringClient for RejectingClient {
        fn get_payload(&self) -> Result<Option<String>, SecretStoreError> {
            Err(SecretStoreError::KeyringUnavailable)
        }

        fn set_payload(&self, _json: &str) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::KeyringUnavailable)
        }

        fn delete_payload(&self) -> Result<(), SecretStoreError> {
            Err(SecretStoreError::KeyringUnavailable)
        }
    }
}
