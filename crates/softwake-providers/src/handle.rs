//! Session hook stub.
//!
//! The daemon does not call this yet. A later awake path can hold a
//! [`ProviderHandle`] and ask for a bearer token without owning HTTP.

use crate::ids::ProviderId;
use crate::secrets::{FileSecretStore, SecretBag, SecretStoreError};
use crate::settings::{FileProviderSettings, ProviderSettings, SettingsError};

/// Read-only view a later session can use to pick a model and token.
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
