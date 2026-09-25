//! Read-only Settings and secret bag.
//!
//! Typed demo chat holds a [`ProviderHandle`] and asks for a bearer token
//! without this type opening a socket. See [ADR 0013](../../docs/ADR-0013-session-provider.md).

use std::path::Path;

use crate::ids::ProviderId;
use crate::secrets::{SecretBag, SecretStoreError};
use crate::settings::{FileProviderSettings, ProviderSettings, SettingsError, TestReport};

/// Selected provider, model, and bearer lookup for one acting session.
///
/// Bearer lookup does not refresh OAuth and does not open a socket.
#[derive(Debug, Clone)]
pub struct ProviderHandle {
    settings: ProviderSettings,
    bag: SecretBag,
}

impl ProviderHandle {
    /// Build from already-loaded documents.
    #[must_use]
    pub fn from_parts(settings: ProviderSettings, bag: SecretBag) -> Self {
        Self { settings, bag }
    }

    /// Load from the default XDG paths.
    ///
    /// This probes the OS keyring at most once per process and may migrate a
    /// plaintext bag. Unit tests use [`Self::load_paths`] instead.
    ///
    /// # Errors
    ///
    /// Path resolution, file, or keyring errors.
    pub fn load_default() -> Result<Self, HandleError> {
        let settings =
            FileProviderSettings::new(crate::settings::resolve_providers_file()?)?.load()?;
        let bag = crate::secrets::open_store(&crate::secrets::resolve_secrets_file()?)?.load()?;
        Ok(Self { settings, bag })
    }

    /// Load Settings and the secret bag from explicit paths without probing.
    ///
    /// The `bool` is `settings_file_present`: false when `providers.json` was
    /// not on disk. A missing secrets file is an empty bag and still [`Ok`].
    /// This does not create either file, does not migrate, and does not talk to
    /// the keyring unless `secrets` is already a version-2 pointer. Daemon unit
    /// tests call this so a missing file stays dbus-free. Production ask uses
    /// [`Self::load_resolved`].
    ///
    /// # Errors
    ///
    /// Path, I/O, or JSON errors. A missing Settings file is [`Ok`] with the
    /// present flag false. A missing secrets file is an empty bag.
    pub fn load_paths(settings: &Path, secrets: &Path) -> Result<(Self, bool), HandleError> {
        let settings_file_present = settings.is_file();
        let loaded = FileProviderSettings::new(settings)?.load()?;
        let bag = crate::secrets::load_unmigrated(secrets)?;
        Ok((Self::from_parts(loaded, bag), settings_file_present))
    }

    /// Load Settings and secrets the way a running ask does.
    ///
    /// The `bool` is `settings_file_present`, same as [`Self::load_paths`].
    /// Secrets go through [`crate::secrets::open_store`], which may probe and
    /// may migrate a plaintext bag into the keyring.
    ///
    /// # Errors
    ///
    /// Path, file, preference, or keyring errors.
    pub fn load_resolved(settings: &Path, secrets: &Path) -> Result<(Self, bool), HandleError> {
        let settings_file_present = settings.is_file();
        let loaded = FileProviderSettings::new(settings)?.load()?;
        let bag = crate::secrets::open_store(secrets)?.load()?;
        Ok((Self::from_parts(loaded, bag), settings_file_present))
    }

    /// Selected provider id.
    #[must_use]
    pub fn selected_provider(&self) -> ProviderId {
        self.settings.selected_provider
    }

    /// Selected model id, if the operator picked one after Test.
    #[must_use]
    pub fn selected_model(&self) -> Option<&str> {
        let model = self.settings.selected_model.trim();
        if model.is_empty() { None } else { Some(model) }
    }

    /// Cached chat models for the selected provider (empty until Test).
    #[must_use]
    pub fn cached_models(&self) -> &[String] {
        self.settings.models_for(self.settings.selected_provider)
    }

    /// Selected voice / STT model id, if the operator picked one after Test.
    #[must_use]
    pub fn selected_voice_model(&self) -> Option<&str> {
        let model = self.settings.selected_voice_model.trim();
        if model.is_empty() { None } else { Some(model) }
    }

    /// Cached voice / STT models for the selected provider (empty until Test).
    #[must_use]
    pub fn cached_voice_models(&self) -> &[String] {
        self.settings
            .voice_models_for(self.settings.selected_provider)
    }

    /// Last Test report for the selected provider.
    ///
    /// `None` when Settings has no report for that id. One report is stored
    /// per provider, not a history bit.
    #[must_use]
    pub fn test_report(&self) -> Option<&TestReport> {
        self.settings
            .last_test
            .get(self.selected_provider().as_str())
    }

    /// Bearer token for the selected provider, if configured.
    ///
    /// Does not refresh OAuth. Does not open a network socket.
    #[must_use]
    pub fn bearer_token(
        &self,
        env_xai: Option<&str>,
        env_openai: Option<&str>,
        env_openrouter: Option<&str>,
        env_openai_compatible: Option<&str>,
    ) -> Option<String> {
        crate::probe::resolve_bearer(
            self.settings.selected_provider,
            &self.bag,
            env_xai,
            env_openai,
            env_openrouter,
            env_openai_compatible,
        )
    }

    /// Borrow the non-secret Settings document.
    #[must_use]
    pub fn settings(&self) -> &crate::settings::ProviderSettings {
        &self.settings
    }
}

/// Failure loading a [`ProviderHandle`].
#[derive(Debug, thiserror::Error)]
pub enum HandleError {
    /// Settings path or file.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// Secrets path or file.
    #[error(transparent)]
    Secrets(#[from] SecretStoreError),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::ProviderHandle;

    const SENTINEL: &str = "sentinel-secret-7c1e";

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "softwake-providers-handle-{}-{n}",
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

    #[test]
    fn load_paths_missing_file_is_empty_and_does_not_create_it() {
        let dir = TempDir::new();
        let settings = dir.path.join("providers.json");
        let secrets = dir.path.join("secrets.json");
        let (handle, present) = ProviderHandle::load_paths(&settings, &secrets).expect("missing");
        assert!(!present);
        assert!(!secrets.exists());
        assert!(handle.bag.xai_api_key.is_none());
        assert!(handle.bearer_token(None, None, None, None).is_none());
    }

    #[test]
    fn load_paths_reads_version_1_without_a_keyring() {
        let dir = TempDir::new();
        let settings = dir.path.join("providers.json");
        let secrets = dir.path.join("secrets.json");
        let original = format!(r#"{{"version":1,"plaintext":true,"xai_api_key":"{SENTINEL}"}}"#);
        std::fs::write(&secrets, &original).expect("write");
        let (handle, present) = ProviderHandle::load_paths(&settings, &secrets).expect("load");
        assert!(!present);
        assert_eq!(handle.bag.version, 1);
        assert!(handle.bag.plaintext);
        assert_eq!(handle.bag.xai_api_key.as_deref(), Some(SENTINEL));
        assert_eq!(std::fs::read_to_string(&secrets).expect("file"), original);
        assert!(!format!("{:?}", handle.bag).contains(SENTINEL));
    }
}
