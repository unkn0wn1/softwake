//! In-memory secret store for tests. It does not open a file and does not reference `keyring`.

use std::sync::Mutex;

use crate::secrets::{SecretBag, SecretStore, SecretStoreError, StorageReport};

/// Owned bag plus the report Settings would show.
pub struct MockSecretStore {
    bag: Mutex<SecretBag>,
    report: StorageReport,
}

impl MockSecretStore {
    /// Store `bag` in memory and return `report` from [`SecretStore::report`].
    #[must_use]
    pub fn new(bag: SecretBag, report: StorageReport) -> Self {
        Self {
            bag: Mutex::new(bag),
            report,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SecretBag> {
        self.bag
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl SecretStore for MockSecretStore {
    fn load(&self) -> Result<SecretBag, SecretStoreError> {
        Ok(self.lock().clone())
    }

    fn save(&self, bag: &SecretBag) -> Result<(), SecretStoreError> {
        *self.lock() = bag.clone();
        Ok(())
    }

    fn report(&self) -> StorageReport {
        self.report.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::MockSecretStore;
    use crate::secrets::{
        KEYRING_STATUS, SecretBackend, SecretBag, SecretStore, StorageReport, update_bag,
    };

    #[test]
    fn update_bag_round_trip_stays_in_memory() {
        let store = MockSecretStore::new(
            SecretBag::empty(),
            StorageReport {
                backend: SecretBackend::Keyring,
                message: KEYRING_STATUS.to_owned(),
            },
        );
        let saved = update_bag(&store, |bag| {
            bag.xai_api_key = Some("mock-key".to_owned());
        })
        .expect("update");
        assert_eq!(saved.xai_api_key.as_deref(), Some("mock-key"));
        assert_eq!(
            store.load().expect("load").xai_api_key.as_deref(),
            Some("mock-key")
        );
        assert_eq!(store.report().backend, SecretBackend::Keyring);
        assert_eq!(store.report().message, KEYRING_STATUS);
    }
}
