//! Model provider credentials, device-code OAuth, Test probes, chat completion, and secret bag.
//!
//! Default builds use [`MockTransport`] and do not open a network socket.
//! Enable `live-http` for real HTTPS via [`live::LiveTransport`].
//!
//! See [ADR 0012](../../docs/ADR-0012-model-providers.md),
//! [ADR 0013](../../docs/ADR-0013-session-provider.md), and
//! [ADR 0021](../../docs/ADR-0021-multi-turn-compact.md).

mod account_oauth;
mod chat;
mod constants;
mod context;
mod handle;
mod ids;
mod models;
mod oauth;
mod pkce;
mod probe;
mod registry;
mod secrets;
mod secrets_file;
mod secrets_keyring;
mod secrets_mock;
mod settings;
mod transport;
mod voice;

#[cfg(feature = "live-http")]
pub mod live;

pub use account_oauth::{
    AccountConnection, AccountProvider, GOOGLE_AUTHORIZE_URL, GOOGLE_EMAIL_SCOPES,
    GOOGLE_REVOKE_URL, GOOGLE_TOKEN_URL, GOOGLE_USERINFO_URL, MICROSOFT_AUTHORIZE_URL,
    MICROSOFT_EMAIL_SCOPES, MICROSOFT_PROFILE_URL, MICROSOFT_TOKEN_URL, OAUTH_CLIENT_MISSING,
    PkceStart, exchange_and_profile, google_authorize_url, microsoft_authorize_url,
    parse_google_profile, parse_microsoft_profile, parse_token_json, publisher_google_client_id,
    publisher_google_client_secret, publisher_microsoft_client_id, refresh_token_body,
    revoke_google_refresh, token_exchange_body,
};
pub use chat::{
    CHAT_MAX_TOKENS, COMPACT_SYSTEM, ChatError, ChatMessage, ChatRole, PrepareError, PreparedChat,
    complete_chat, complete_compact, extractive_summary, missing_credential_message, prepare_chat,
};
pub use constants::{
    OPENAI_API_BASE, OPENAI_CHAT_SEED, OPENAI_COMPATIBLE_CHAT_SEED, OPENAI_VOICE_SEED,
    OPENROUTER_API_BASE, OPENROUTER_CHAT_SEED, XAI_API_BASE, XAI_CHAT_SEED, XAI_OAUTH_CLIENT_ID,
    XAI_OAUTH_DEVICE_URL, XAI_OAUTH_GRANT_DEVICE, XAI_OAUTH_ISSUER, XAI_OAUTH_SCOPE,
    XAI_OAUTH_TOKEN_URL, XAI_REFRESH_SKEW_MS, XAI_TTS_VOICE_EVE, XAI_TTS_VOICES, XAI_VOICE_SEED,
};
pub use context::{
    DEFAULT_COMPACT_AT_PERCENT, DEFAULT_CONTEXT_LIMIT_TOKENS, DEFAULT_KEEP_RECENT_TURNS,
    builtin_context_limit, estimate_tokens, estimate_tokens_parts, resolve_compact_at_percent,
    resolve_context_limit, resolve_keep_recent_turns, should_compact, usage_percent,
};
pub use handle::{HandleError, ProviderHandle};
pub use ids::{ParseProviderIdError, ProviderId};
pub use models::{
    filter_chat_models, filter_voice_models, is_chat_model, is_voice_model, parse_model_ids,
};
pub use oauth::{
    DeviceCodeStart, DevicePoll, OAuthTokenSet, access_needs_refresh, device_code_body,
    merge_refresh, parse_device_poll, parse_device_start, parse_token_response, refresh_body,
    token_poll_body,
};
pub use pkce::{code_challenge, code_verifier, oauth_state};
pub use probe::{
    ProbeError, TestOutcome, apply_test_outcome, ensure_fresh_access, poll_device_code,
    resolve_bearer, run_test, start_device_code,
};
pub use registry::{
    ApiBaseError, CredentialKind, PROVIDER_REGISTRY, ProviderDefinition, ProviderFamily,
    normalize_compatible_base, provider_definition, resolve_api_base,
};
pub use secrets::{
    BackendChoice, BackendPref, KEYRING_PROBE_USER, KEYRING_SERVICE, KEYRING_STATUS, KEYRING_USER,
    KeyringClient, MAX_SECRETS_BYTES, OnDiskKind, PLAINTEXT_OPT_IN_MESSAGE, PLAINTEXT_WARNING,
    POINTER_IDENTITY_MESSAGE, SECRETS_FILE_NAME, SecretBackend, SecretBag, SecretStore,
    SecretStoreError, StorageReport, UNSUPPORTED_BACKEND_MESSAGE, UnavailableSecretStore,
    open_store, open_store_with, opt_in_plaintext, opt_in_plaintext_resolved, resolve_backend,
    resolve_secrets_file, resolve_secrets_file_from, update_bag,
};
pub use secrets_file::FileSecretStore;
pub use secrets_keyring::KeyringSecretStore;
pub use secrets_mock::MockSecretStore;
pub use settings::{
    FileProviderSettings, MAX_SETTINGS_BYTES, ModelCache, PROVIDERS_FILE_NAME, ProviderSettings,
    SettingsError, TestReport, resolve_providers_file, resolve_providers_file_from,
};
pub use transport::{
    HttpBytes, HttpResponse, MockTransport, MultipartField, RecordedBytePost, RecordedMultipart,
    Transport, TransportError,
};
pub use voice::{
    TTS_MAX_CHARS, VOICE_LANGUAGE, VoiceHttpError, family_speaks_xai, resolve_stt_model,
    resolve_tts_voice, stt_transcribe, tts_synthesize, tts_voice_roster, wav_from_pcm16,
};
