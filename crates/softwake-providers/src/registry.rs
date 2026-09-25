//! Static provider table.

use crate::constants::{OPENAI_API_BASE, OPENAI_CHAT_SEED, XAI_API_BASE, XAI_CHAT_SEED};
use crate::ids::ProviderId;

/// How Settings collects credentials for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// Device-code OAuth.
    XaiOauth,
    /// Paste an xAI API key.
    XaiKey,
    /// Paste an `OpenAI` API key.
    OpenaiKey,
}

/// API family used for Test probes, model listing, and chat completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    /// `api.x.ai`.
    Xai,
    /// `api.openai.com`.
    Openai,
}

impl ProviderFamily {
    /// API origin for this family. No trailing slash.
    #[must_use]
    pub const fn api_base(self) -> &'static str {
        match self {
            Self::Xai => XAI_API_BASE,
            Self::Openai => OPENAI_API_BASE,
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

/// The three v1 providers, in display order.
pub const PROVIDER_REGISTRY: [ProviderDefinition; 3] = [
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
    use super::{PROVIDER_REGISTRY, ProviderFamily, provider_definition};
    use crate::constants::{OPENAI_API_BASE, XAI_API_BASE};
    use crate::ids::ProviderId;

    #[test]
    fn registry_covers_every_id() {
        for id in ProviderId::all() {
            assert_eq!(provider_definition(id).id, id);
        }
        assert_eq!(PROVIDER_REGISTRY.len(), ProviderId::all().len());
    }

    #[test]
    fn api_base_matches_the_family_constants() {
        assert_eq!(ProviderFamily::Xai.api_base(), XAI_API_BASE);
        assert_eq!(ProviderFamily::Openai.api_base(), OPENAI_API_BASE);
    }
}
