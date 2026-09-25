//! Public xAI OAuth and API endpoints.
//!
//! The device-code client id has no secret and is safe to commit.

/// xAI OAuth issuer.
pub const XAI_OAUTH_ISSUER: &str = "https://auth.x.ai";

/// Device authorization endpoint.
pub const XAI_OAUTH_DEVICE_URL: &str = "https://auth.x.ai/oauth2/device/code";

/// Token endpoint (device poll and refresh).
pub const XAI_OAUTH_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";

/// Public device-code client id (Hermes / Grok Build family). No client secret.
pub const XAI_OAUTH_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// OAuth scope string.
pub const XAI_OAUTH_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

/// Device-code grant type.
pub const XAI_OAUTH_GRANT_DEVICE: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// xAI OpenAI-compatible API base (`api.x.ai`).
pub const XAI_API_BASE: &str = "https://api.x.ai/v1";

/// `OpenAI` API base.
pub const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

/// Refresh this many milliseconds before access-token expiry.
pub const XAI_REFRESH_SKEW_MS: u64 = 60_000;

/// Registry seed for xAI chat when Test's catalog omits chat ids after a pass.
pub const XAI_CHAT_SEED: &str = "grok-4.5";

/// Registry seed for `OpenAI` chat when Test's catalog omits chat ids after a pass.
pub const OPENAI_CHAT_SEED: &str = "gpt-4.1-mini";

/// `OpenRouter` OpenAI-compatible API base.
pub const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

/// Registry seed for `OpenRouter` chat when Test's catalog omits chat ids after a pass.
pub const OPENROUTER_CHAT_SEED: &str = "openai/gpt-4.1-mini";

/// Registry seed for OpenAI-compatible chat when Test's catalog omits chat ids after a pass.
pub const OPENAI_COMPATIBLE_CHAT_SEED: &str = "gpt-4.1-mini";

/// Registry seed for xAI voice/STT when Test's catalog omits STT ids after a pass.
pub const XAI_VOICE_SEED: &str = "grok-voice-transcribe-2.0";

/// Default xAI text-to-speech voice id (`POST /v1/tts`).
///
/// Documented built-in on the xAI Voice API. Used only for the xAI family.
pub const XAI_TTS_VOICE_EVE: &str = "eve";

/// Built-in xAI TTS voice ids from the Voice API docs.
///
/// `eve` is the API default. The rest are documented built-ins (case-insensitive
/// on the wire). This list is not a live `GET /v1/tts/voices` catalog.
pub const XAI_TTS_VOICES: &[&str] = &[
    "eve", "ara", "leo", "rex", "sal", "carina", "zagan", "helix", "orion", "luna", "iris",
    "altair",
];

/// Registry seed for `OpenAI` voice/STT when Test's catalog omits STT ids after a pass.
pub const OPENAI_VOICE_SEED: &str = "gpt-4o-transcribe-diarize";
