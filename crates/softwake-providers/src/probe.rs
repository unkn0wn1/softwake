//! Credential Test: chat probe + model list.
//!
//! Network stays behind [`crate::transport::Transport`]. CI uses mocks.

use serde_json::{Value, json};

use crate::constants::{OPENAI_API_BASE, XAI_API_BASE};
use crate::constants::{XAI_OAUTH_DEVICE_URL, XAI_OAUTH_TOKEN_URL};
use crate::ids::ProviderId;
use crate::models::{filter_chat_models, parse_model_ids};
use crate::oauth::{
    DeviceCodeStart, DevicePoll, OAuthTokenSet, access_needs_refresh, device_code_body,
    merge_refresh, parse_device_poll, parse_device_start, parse_token_response, refresh_body,
    token_poll_body,
};
use crate::registry::{ProviderFamily, provider_definition};
use crate::secrets::SecretBag;
use crate::settings::{ProviderSettings, TestReport};
use crate::transport::{HttpResponse, Transport, TransportError};

/// Outcome of [`run_test`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    /// Whether the probe and catalog path succeeded enough to fill models.
    pub ok: bool,
    /// Safe display message.
    pub message: String,
    /// Chat model ids to store when `ok`. Empty when not ok.
    pub chat_models: Vec<String>,
}

/// Resolve a bearer token for `provider` from the bag and the environment.
///
/// A saved key or OAuth access token wins over the matching environment variable.
#[must_use]
pub fn resolve_bearer(
    provider: ProviderId,
    bag: &SecretBag,
    env_xai: Option<&str>,
    env_openai: Option<&str>,
) -> Option<String> {
    match provider {
        ProviderId::XaiOauth => bag
            .xai_oauth
            .as_ref()
            .map(|tokens| tokens.access_token.clone())
            .filter(|token| !token.trim().is_empty()),
        ProviderId::XaiKey => bag
            .xai_api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                env_xai
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(str::to_owned)
            }),
        ProviderId::Openai => bag
            .openai_api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                env_openai
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(str::to_owned)
            }),
    }
}

/// Start xAI device-code sign-in through `transport`.
///
/// # Errors
///
/// Transport failures or an unparseable response.
pub fn start_device_code<T: Transport>(
    transport: &T,
    now_ms: u64,
) -> Result<DeviceCodeStart, ProbeError> {
    let response = transport
        .post_form(XAI_OAUTH_DEVICE_URL, &device_code_body())
        .map_err(ProbeError::from)?;
    let payload = parse_json_body(&response.body);
    parse_device_start(&payload, now_ms).ok_or(ProbeError::OauthStart)
}

/// Poll the device-code token endpoint once.
///
/// # Errors
///
/// Transport failures.
pub fn poll_device_code<T: Transport>(
    transport: &T,
    device_code: &str,
    interval_sec: u64,
    now_ms: u64,
) -> Result<DevicePoll, ProbeError> {
    let response = transport
        .post_form(XAI_OAUTH_TOKEN_URL, &token_poll_body(device_code))
        .map_err(ProbeError::from)?;
    let payload = parse_json_body(&response.body);
    if (200..300).contains(&response.status) {
        return match parse_token_response(&payload, now_ms, true) {
            Some(tokens) => Ok(DevicePoll::Tokens(tokens)),
            None => Ok(DevicePoll::Denied {
                message: "xAI sign-in did not return tokens.".to_owned(),
            }),
        };
    }
    Ok(parse_device_poll(&payload, interval_sec))
}

/// Refresh an access token when it is near expiry.
///
/// # Errors
///
/// Transport or parse failures.
pub fn ensure_fresh_access<T: Transport>(
    transport: &T,
    tokens: &OAuthTokenSet,
    now_ms: u64,
) -> Result<OAuthTokenSet, ProbeError> {
    if !access_needs_refresh(tokens, now_ms) {
        return Ok(tokens.clone());
    }
    if tokens.refresh_token.trim().is_empty() {
        return Err(ProbeError::OauthRefresh);
    }
    let response = transport
        .post_form(XAI_OAUTH_TOKEN_URL, &refresh_body(&tokens.refresh_token))
        .map_err(ProbeError::from)?;
    let payload = parse_json_body(&response.body);
    let next = parse_token_response(&payload, now_ms, false).ok_or(ProbeError::OauthRefresh)?;
    if !(200..300).contains(&response.status) {
        return Err(ProbeError::OauthRefresh);
    }
    Ok(merge_refresh(tokens, next))
}

/// Run Test for `provider`: chat probe, then model list.
///
/// On success, returns chat model ids (seed fallback when the catalog has none).
/// On failure, `chat_models` is empty so callers leave any previous catalog alone.
pub fn run_test<T: Transport>(
    transport: &T,
    provider: ProviderId,
    bearer: &str,
    now_ms: u64,
) -> TestOutcome {
    let _ = now_ms;
    let def = provider_definition(provider);
    let token = bearer.trim();
    if token.is_empty() {
        return TestOutcome {
            ok: false,
            message: missing_message(provider),
            chat_models: Vec::new(),
        };
    }

    let chat_url = format!("{}/chat/completions", api_base(def.family));
    let models_url = format!("{}/models", api_base(def.family));
    let seed = def.chat_seed;
    let body = json!({
        "model": seed,
        "temperature": 0,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "ping"}]
    })
    .to_string();

    let probe = match transport.post_json_bearer(&chat_url, token, &body) {
        Ok(response) => classify_chat_probe(&response),
        Err(_) => {
            return TestOutcome {
                ok: false,
                message: "Could not reach the provider to run Test.".to_owned(),
                chat_models: Vec::new(),
            };
        }
    };
    if !probe.ok {
        return TestOutcome {
            ok: false,
            message: probe.message,
            chat_models: Vec::new(),
        };
    }

    let catalog = match transport.get_bearer(&models_url, token) {
        Ok(response) if (200..300).contains(&response.status) => {
            let payload = parse_json_body(&response.body);
            filter_chat_models(def.family, &parse_model_ids(&payload))
        }
        Ok(_) | Err(_) => {
            // Probe passed but catalog failed: keep seed so the picker is usable.
            return TestOutcome {
                ok: true,
                message: format!(
                    "{}. Model list failed; using the registry seed.",
                    probe.message
                ),
                chat_models: vec![seed.to_owned()],
            };
        }
    };

    let chat_models = if catalog.is_empty() {
        vec![seed.to_owned()]
    } else {
        catalog
    };

    TestOutcome {
        ok: true,
        message: format!("{}. Loaded {} model(s).", probe.message, chat_models.len()),
        chat_models,
    }
}

/// Apply a Test outcome onto Settings. Failed tests do not clear catalogs.
pub fn apply_test_outcome(
    settings: &mut ProviderSettings,
    provider: ProviderId,
    outcome: &TestOutcome,
    now_ms: u64,
) {
    settings.store_test(
        provider,
        TestReport {
            ok: outcome.ok,
            message: outcome.message.clone(),
        },
    );
    if outcome.ok {
        settings.store_models(provider, outcome.chat_models.clone(), now_ms);
        if settings.selected_provider == provider {
            let current = settings.selected_model.clone();
            if current.is_empty() || !outcome.chat_models.iter().any(|id| id == &current) {
                settings.selected_model = outcome.chat_models.first().cloned().unwrap_or_default();
            }
        }
    }
}

/// Probe / OAuth failure.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// HTTP seam failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Device-code start failed.
    #[error("could not start xAI sign-in")]
    OauthStart,
    /// Refresh failed.
    #[error("xAI sign-in expired; sign in again in Settings")]
    OauthRefresh,
}

fn api_base(family: ProviderFamily) -> &'static str {
    match family {
        ProviderFamily::Xai => XAI_API_BASE,
        ProviderFamily::Openai => OPENAI_API_BASE,
    }
}

fn missing_message(provider: ProviderId) -> String {
    match provider {
        ProviderId::XaiOauth => "No xAI sign-in is configured.".to_owned(),
        ProviderId::XaiKey => "No xAI API key is configured.".to_owned(),
        ProviderId::Openai => "No OpenAI API key is configured.".to_owned(),
    }
}

struct ProbeResult {
    ok: bool,
    message: String,
}

fn classify_chat_probe(response: &HttpResponse) -> ProbeResult {
    if (200..300).contains(&response.status) {
        return ProbeResult {
            ok: true,
            message: "Chat check passed.".to_owned(),
        };
    }
    if response.status == 401 || response.status == 403 {
        return ProbeResult {
            ok: false,
            message: "Provider rejected the credentials.".to_owned(),
        };
    }
    ProbeResult {
        ok: false,
        message: format!("Chat check failed ({}).", response.status),
    }
}

fn parse_json_body(body: &str) -> Value {
    if body.trim().is_empty() {
        return Value::Null;
    }
    serde_json::from_str(body).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{apply_test_outcome, resolve_bearer, run_test, start_device_code};
    use crate::constants::{XAI_API_BASE, XAI_CHAT_SEED, XAI_OAUTH_DEVICE_URL};
    use crate::ids::ProviderId;
    use crate::oauth::OAuthTokenSet;
    use crate::secrets::SecretBag;
    use crate::settings::ProviderSettings;
    use crate::transport::{HttpResponse, MockTransport};

    #[test]
    fn saved_key_wins_over_env() {
        let mut bag = SecretBag::empty();
        bag.xai_api_key = Some("saved".to_owned());
        assert_eq!(
            resolve_bearer(ProviderId::XaiKey, &bag, Some("env"), None).as_deref(),
            Some("saved")
        );
        bag.xai_api_key = None;
        assert_eq!(
            resolve_bearer(ProviderId::XaiKey, &bag, Some("env"), None).as_deref(),
            Some("env")
        );
    }

    #[test]
    fn test_fills_models_only_on_success() {
        let transport = MockTransport::new()
            .with_post_json(
                format!("{XAI_API_BASE}/chat/completions"),
                HttpResponse {
                    status: 200,
                    body: "{}".to_owned(),
                },
            )
            .with_get(
                format!("{XAI_API_BASE}/models"),
                HttpResponse {
                    status: 200,
                    body: json!({
                        "data": [
                            {"id": "grok-4.5"},
                            {"id": "grok-voice-transcribe-2.0"}
                        ]
                    })
                    .to_string(),
                },
            );
        let outcome = run_test(&transport, ProviderId::XaiKey, "key", 0);
        assert!(outcome.ok);
        assert_eq!(outcome.chat_models, vec!["grok-4.5".to_owned()]);

        let mut settings = ProviderSettings::default();
        apply_test_outcome(&mut settings, ProviderId::XaiKey, &outcome, 10);
        assert_eq!(
            settings.models_for(ProviderId::XaiKey),
            &["grok-4.5".to_owned()]
        );

        let fail = run_test(
            &MockTransport::new().with_post_json(
                format!("{XAI_API_BASE}/chat/completions"),
                HttpResponse {
                    status: 401,
                    body: "nope".to_owned(),
                },
            ),
            ProviderId::XaiKey,
            "bad",
            0,
        );
        assert!(!fail.ok);
        apply_test_outcome(&mut settings, ProviderId::XaiKey, &fail, 11);
        assert_eq!(
            settings.models_for(ProviderId::XaiKey),
            &["grok-4.5".to_owned()],
            "failed Test must not clear catalog"
        );
        assert!(!settings.last_test["xai-key"].ok);
    }

    #[test]
    fn seed_fallback_when_catalog_empty() {
        let transport = MockTransport::new()
            .with_post_json(
                format!("{XAI_API_BASE}/chat/completions"),
                HttpResponse {
                    status: 200,
                    body: "{}".to_owned(),
                },
            )
            .with_get(
                format!("{XAI_API_BASE}/models"),
                HttpResponse {
                    status: 200,
                    body: json!({"data": [{"id": "grok-voice-transcribe-2.0"}]}).to_string(),
                },
            );
        let outcome = run_test(&transport, ProviderId::XaiOauth, "tok", 0);
        assert!(outcome.ok);
        assert_eq!(outcome.chat_models, vec![XAI_CHAT_SEED.to_owned()]);
    }

    #[test]
    fn device_start_uses_mock_transport() {
        let transport = MockTransport::new().with_form(
            XAI_OAUTH_DEVICE_URL,
            HttpResponse {
                status: 200,
                body: json!({
                    "device_code": "dc",
                    "user_code": "WDJB-MJHT",
                    "verification_uri": "https://auth.x.ai/activate",
                    "interval": 5,
                    "expires_in": 600
                })
                .to_string(),
            },
        );
        let start = start_device_code(&transport, 1_000).expect("start");
        assert_eq!(start.user_code, "WDJB-MJHT");
        assert_eq!(start.device_code, "dc");
    }

    #[test]
    fn oauth_bearer_from_bag() {
        let mut bag = SecretBag::empty();
        bag.xai_oauth = Some(OAuthTokenSet {
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            expires_at_ms: 9,
            token_type: "Bearer".to_owned(),
        });
        assert_eq!(
            resolve_bearer(ProviderId::XaiOauth, &bag, None, None).as_deref(),
            Some("access")
        );
    }
}
