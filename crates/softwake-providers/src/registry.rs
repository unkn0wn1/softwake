//! Static provider table.

use crate::constants::{
    OPENAI_API_BASE, OPENAI_CHAT_SEED, OPENAI_COMPATIBLE_CHAT_SEED, OPENROUTER_API_BASE,
    OPENROUTER_CHAT_SEED, XAI_API_BASE, XAI_CHAT_SEED,
};
use crate::ids::ProviderId;
use crate::settings::ProviderSettings;

/// How Settings collects credentials for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// Device-code OAuth.
    XaiOauth,
    /// Paste an xAI API key.
    XaiKey,
    /// Paste an `OpenAI` API key.
    OpenaiKey,
    /// Paste an `OpenRouter` API key.
    OpenrouterKey,
    /// Paste an OpenAI-compatible API key (base URL is non-secret Settings).
    OpenaiCompatibleKey,
}

/// API family used for Test probes, model listing, and chat completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    /// `api.x.ai`.
    Xai,
    /// `api.openai.com`.
    Openai,
    /// `openrouter.ai`.
    Openrouter,
    /// Operator-configured OpenAI-compatible base URL.
    OpenaiCompatible,
}

impl ProviderFamily {
    /// Fixed API origin for this family when it does not need Settings.
    ///
    /// [`ProviderFamily::OpenaiCompatible`] returns [`None`]; resolve that base
    /// with [`resolve_api_base`].
    #[must_use]
    pub const fn fixed_api_base(self) -> Option<&'static str> {
        match self {
            Self::Xai => Some(XAI_API_BASE),
            Self::Openai => Some(OPENAI_API_BASE),
            Self::Openrouter => Some(OPENROUTER_API_BASE),
            Self::OpenaiCompatible => None,
        }
    }
}

/// One row in the Settings provider list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderDefinition {
    /// Stable id.
    pub id: ProviderId,
    /// Short label for the picker.
    pub label: &'static str,
    /// HTTP family.
    pub family: ProviderFamily,
    /// Credential form.
    pub credential: CredentialKind,
    /// Seed chat model id.
    pub chat_seed: &'static str,
}

/// Failure resolving an API base URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiBaseError {
    /// OpenAI-compatible base URL is missing or blank.
    #[error("No OpenAI-compatible base URL is configured. Set one in Settings.")]
    Missing,
    /// OpenAI-compatible base URL is not an http(s) URL.
    #[error("OpenAI-compatible base URL must start with http:// or https://.")]
    Invalid,
}

/// Normalize and validate a configured OpenAI-compatible base URL.
///
/// Trims whitespace and trailing `/` characters. Does not append `/v1`.
///
/// # Errors
///
/// [`ApiBaseError::Missing`] when blank. [`ApiBaseError::Invalid`] when the
/// scheme is not `http` or `https`.
pub fn normalize_compatible_base(raw: &str) -> Result<String, ApiBaseError> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(ApiBaseError::Missing);
    }
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(ApiBaseError::Invalid);
    }
    Ok(trimmed.to_owned())
}

/// Resolve the API base used for Test and chat for `provider`.
///
/// # Errors
///
/// [`ApiBaseError`] when the OpenAI-compatible provider has no usable base URL.
pub fn resolve_api_base(
    provider: ProviderId,
    settings: &ProviderSettings,
) -> Result<String, ApiBaseError> {
    let def = provider_definition(provider);
    match def.family.fixed_api_base() {
        Some(base) => Ok(base.to_owned()),
        None => normalize_compatible_base(&settings.openai_compatible_base_url),
    }
}

/// The five v1 providers, in display order.
pub const PROVIDER_REGISTRY: [ProviderDefinition; 5] = [
    ProviderDefinition {
        id: ProviderId::XaiOauth,
        label: "xAI sign-in",
        family: ProviderFamily::Xai,
        credential: CredentialKind::XaiOauth,
        chat_seed: XAI_CHAT_SEED,
    },
    ProviderDefinition {
        id: ProviderId::XaiKey,
        label: "xAI API key",
        family: ProviderFamily::Xai,
        credential: CredentialKind::XaiKey,
        chat_seed: XAI_CHAT_SEED,
    },
    ProviderDefinition {
        id: ProviderId::Openai,
        label: "OpenAI",
        family: ProviderFamily::Openai,
        credential: CredentialKind::OpenaiKey,
        chat_seed: OPENAI_CHAT_SEED,
    },
    ProviderDefinition {
        id: ProviderId::Openrouter,
        label: "OpenRouter",
        family: ProviderFamily::Openrouter,
        credential: CredentialKind::OpenrouterKey,
        chat_seed: OPENROUTER_CHAT_SEED,
    },
    ProviderDefinition {
        id: ProviderId::OpenaiCompatible,
        label: "OpenAI-compatible",
        family: ProviderFamily::OpenaiCompatible,
        credential: CredentialKind::OpenaiCompatibleKey,
        chat_seed: OPENAI_COMPATIBLE_CHAT_SEED,
    },
];

/// Look up a registry row.
///
/// # Panics
///
/// Panics only if a new [`ProviderId`] variant is added without a registry row.
#[must_use]
pub fn provider_definition(id: ProviderId) -> ProviderDefinition {
    PROVIDER_REGISTRY
        .iter()
        .copied()
        .find(|row| row.id == id)
        .expect("ProviderId variants match PROVIDER_REGISTRY")
}

#[cfg(test)]
mod tests {
    use super::{
        PROVIDER_REGISTRY, ProviderFamily, normalize_compatible_base, provider_definition,
        resolve_api_base,
    };
    use crate::constants::{OPENAI_API_BASE, OPENROUTER_API_BASE, XAI_API_BASE};
    use crate::ids::ProviderId;
    use crate::settings::ProviderSettings;

    #[test]
    fn registry_covers_every_id() {
        for id in ProviderId::all() {
            assert_eq!(provider_definition(id).id, id);
        }
        assert_eq!(PROVIDER_REGISTRY.len(), ProviderId::all().len());
    }

    #[test]
    fn fixed_api_base_matches_the_family_constants() {
        assert_eq!(ProviderFamily::Xai.fixed_api_base(), Some(XAI_API_BASE));
        assert_eq!(
            ProviderFamily::Openai.fixed_api_base(),
            Some(OPENAI_API_BASE)
        );
        assert_eq!(
            ProviderFamily::Openrouter.fixed_api_base(),
            Some(OPENROUTER_API_BASE)
        );
        assert_eq!(ProviderFamily::OpenaiCompatible.fixed_api_base(), None);
    }

    #[test]
    fn resolve_uses_settings_for_compatible() {
        let settings = ProviderSettings {
            openai_compatible_base_url: " https://llm.example/v1/ ".to_owned(),
            ..ProviderSettings::default()
        };
        assert_eq!(
            resolve_api_base(ProviderId::OpenaiCompatible, &settings).expect("base"),
            "https://llm.example/v1"
        );
        assert_eq!(
            resolve_api_base(ProviderId::Openrouter, &settings).expect("or"),
            OPENROUTER_API_BASE
        );
        let blank = ProviderSettings::default();
        assert!(resolve_api_base(ProviderId::OpenaiCompatible, &blank).is_err());
        assert!(normalize_compatible_base("ftp://x").is_err());
    }
}
