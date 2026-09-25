//! Read-only Settings and secret bag.
//!
//! Typed demo chat holds a [`ProviderHandle`] and asks for a bearer token
//! without this type opening a socket. See [ADR 0013](../../docs/ADR-0013-session-provider.md).

use std::path::Path;

use crate::ids::ProviderId;
use crate::secrets::{FileSecretStore, SecretBag, SecretStoreError};
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
    /// # Errors
    ///
    /// Path resolution or file errors.
    pub fn load_default() -> Result<Self, HandleError> {
        let settings =
            FileProviderSettings::new(crate::settings::resolve_providers_file()?)?.load()?;
        let bag = FileSecretStore::new(crate::secrets::resolve_secrets_file()?)?.load()?;
        Ok(Self { settings, bag })
    }

    /// Load Settings and the secret bag from explicit paths.
    ///
    /// The `bool` is `settings_file_present`: false when `providers.json` was
    /// not on disk. A missing secrets file is an empty bag and still [`Ok`].
    /// This does not create either file.
    ///
    /// # Errors
    ///
    /// Path, I/O, or JSON errors. A missing Settings file is [`Ok`] with the
    /// present flag false. A missing secrets file is an empty bag.
    pub fn load_paths(settings: &Path, secrets: &Path) -> Result<(Self, bool), HandleError> {
        let settings_file_present = settings.is_file();
        let loaded = FileProviderSettings::new(settings)?.load()?;
        let bag = FileSecretStore::new(secrets)?.load()?;
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
    pub fn bearer_token(&self, env_xai: Option<&str>, env_openai: Option<&str>) -> Option<String> {
        crate::probe::resolve_bearer(
            self.settings.selected_provider,
            &self.bag,
            env_xai,
            env_openai,
        )
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
