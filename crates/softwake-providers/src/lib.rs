//! Model provider credentials, device-code OAuth, Test probes, and secret bag.
//!
//! Default builds use [`MockTransport`] and do not open a network socket.
//! Enable `live-http` for real HTTPS via [`live::LiveTransport`].
//!
//! See [ADR 0012](../../docs/ADR-0012-model-providers.md).

mod constants;
mod handle;
mod ids;
mod models;
mod oauth;
mod probe;
mod registry;
mod secrets;
mod settings;
mod transport;

#[cfg(feature = "live-http")]
pub mod live;

pub use constants::{
    OPENAI_API_BASE, OPENAI_CHAT_SEED, XAI_API_BASE, XAI_CHAT_SEED, XAI_OAUTH_CLIENT_ID,
    XAI_OAUTH_DEVICE_URL, XAI_OAUTH_GRANT_DEVICE, XAI_OAUTH_ISSUER, XAI_OAUTH_SCOPE,
    XAI_OAUTH_TOKEN_URL, XAI_REFRESH_SKEW_MS,
};
pub use handle::{HandleError, ProviderHandle};
pub use ids::{ParseProviderIdError, ProviderId};
pub use models::{filter_chat_models, is_chat_model, parse_model_ids};
pub use oauth::{
    DeviceCodeStart, DevicePoll, OAuthTokenSet, access_needs_refresh, device_code_body,
    merge_refresh, parse_device_poll, parse_device_start, parse_token_response, refresh_body,
    token_poll_body,
};
pub use probe::{
    ProbeError, TestOutcome, apply_test_outcome, ensure_fresh_access, poll_device_code,
    resolve_bearer, run_test, start_device_code,
};
pub use registry::{
    CredentialKind, PROVIDER_REGISTRY, ProviderDefinition, ProviderFamily, provider_definition,
};
pub use secrets::{
    FileSecretStore, MAX_SECRETS_BYTES, PLAINTEXT_WARNING, SECRETS_FILE_NAME, SecretBag,
    SecretStoreError, resolve_secrets_file, resolve_secrets_file_from,
};
pub use settings::{
    FileProviderSettings, MAX_SETTINGS_BYTES, ModelCache, PROVIDERS_FILE_NAME, ProviderSettings,
    SettingsError, TestReport, resolve_providers_file, resolve_providers_file_from,
};
pub use transport::{HttpResponse, MockTransport, Transport, TransportError};
