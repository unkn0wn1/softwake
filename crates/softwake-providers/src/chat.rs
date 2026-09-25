//! One chat completion against the selected provider.
//!
//! Readiness is local. The HTTP call goes through [`crate::Transport`].
//! CI uses [`crate::MockTransport`]. See [ADR 0013](../../docs/ADR-0013-session-provider.md).

use serde_json::Value;

use crate::handle::ProviderHandle;
use crate::ids::ProviderId;
use crate::registry::{ProviderFamily, provider_definition};
use crate::transport::{Transport, TransportError};

/// `max_tokens` sent on every chat completion in this slice.
pub const CHAT_MAX_TOKENS: u32 = 1024;

/// Chat target that passed readiness.
///
/// The bearer token is not a field. [`Debug`](std::fmt::Debug) cannot print it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedChat {
    /// Selected provider.
    pub provider: ProviderId,
    /// API family (model filtering / diagnostics).
    pub family: ProviderFamily,
    /// Resolved API base used for chat completions (no trailing slash).
    pub api_base: String,
    /// Settings model id. Not a registry seed substitute.
    pub model: String,
}

/// First failed readiness check. No I/O and no HTTP.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrepareError {
    /// `providers.json` was not on disk.
    #[error("No provider is configured. Choose a provider in Settings and run Test.")]
    NoProvider,
    /// The last Test report is missing or not ok.
    #[error("Test has not succeeded for {provider}. Run Test in Settings.")]
    TestNotSucceeded {
        /// Selected provider id.
        provider: ProviderId,
    },
    /// No selected model, or that id is not in the selected provider's cache.
    #[error("No chat model is selected. Choose one in Settings after Test.")]
    NoModel,
    /// No saved credential and no env fallback for the selected provider.
    #[error("{}", missing_credential_message(*provider))]
    NoCredential {
        /// Selected provider id.
        provider: ProviderId,
    },
    /// OpenAI-compatible base URL missing or invalid.
    #[error("{message}")]
    NoApiBase {
        /// Operator-facing sentence.
        message: String,
    },
}

/// Chat completion failure. Display text omits the body, the bearer, and the URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    /// The transport could not complete the request.
    #[error("Could not reach the provider.")]
    Unreachable,
    /// HTTP 401 or 403.
    #[error("Provider rejected the credentials.")]
    Rejected,
    /// Any other non-2xx status.
    #[error("Chat completion failed ({status}).")]
    Failed {
        /// HTTP status code.
        status: u16,
    },
    /// 2xx content was empty after trim.
    #[error("The provider returned an empty reply.")]
    Empty,
    /// 2xx body was not a JSON string at `choices[0].message.content`.
    #[error("The provider reply could not be read.")]
    Unparseable,
}

/// Operator sentence when the selected provider has no bearer.
#[must_use]
pub fn missing_credential_message(provider: ProviderId) -> String {
    match provider {
        ProviderId::XaiOauth => "No xAI sign-in is configured.".to_owned(),
        ProviderId::XaiKey => "No xAI API key is configured.".to_owned(),
        ProviderId::Openai => "No OpenAI API key is configured.".to_owned(),
        ProviderId::Openrouter => "No OpenRouter API key is configured.".to_owned(),
        ProviderId::OpenaiCompatible => "No OpenAI-compatible API key is configured.".to_owned(),
    }
}

/// Check readiness and return the chat target plus the bearer token.
///
/// The bearer is the second [`Ok`](Result::Ok) value. Do not log it.
/// `settings_file_present == false` returns [`PrepareError::NoProvider`]
/// even when `handle` holds a model and an env key is set.
///
/// # Errors
///
/// The first failed check: Settings file absent, Test missing or not ok,
/// model missing from the cache, or no bearer.
pub fn prepare_chat(
    handle: &ProviderHandle,
    settings_file_present: bool,
    env_xai: Option<&str>,
    env_openai: Option<&str>,
    env_openrouter: Option<&str>,
    env_openai_compatible: Option<&str>,
) -> Result<(PreparedChat, String), PrepareError> {
    if !settings_file_present {
        return Err(PrepareError::NoProvider);
    }
    let provider = handle.selected_provider();
    match handle.test_report() {
        Some(report) if report.ok => {}
        _ => return Err(PrepareError::TestNotSucceeded { provider }),
    }
    let Some(model) = handle.selected_model() else {
        return Err(PrepareError::NoModel);
    };
    if !handle.cached_models().iter().any(|cached| cached == model) {
        return Err(PrepareError::NoModel);
    }
    let Some(bearer) =
        handle.bearer_token(env_xai, env_openai, env_openrouter, env_openai_compatible)
    else {
        return Err(PrepareError::NoCredential { provider });
    };
    let api_base =
        crate::registry::resolve_api_base(provider, handle.settings()).map_err(|error| {
            PrepareError::NoApiBase {
                message: error.to_string(),
            }
        })?;
    Ok((
        PreparedChat {
            provider,
            family: provider_definition(provider).family,
            api_base,
            model: model.to_owned(),
        },
        bearer,
    ))
}

/// POST one chat completion. No `temperature`, tools, or stream.
///
/// Parses `choices[0].message.content` only when it is a JSON string.
///
/// # Errors
///
/// [`ChatError`] when the transport fails, the status is not success, or the
/// content is empty or not a string. The error text does not include the
/// response body, the bearer, or the request URL.
pub fn complete_chat<T: Transport>(
    transport: &T,
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    user: &str,
) -> Result<String, ChatError> {
    let url = format!("{}/chat/completions", prepared.api_base);
    let body = serde_json::json!({
        "model": prepared.model,
        "max_tokens": CHAT_MAX_TOKENS,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    })
    .to_string();
    let response = transport
        .post_json_bearer(&url, bearer, &body)
        .map_err(|_error: TransportError| ChatError::Unreachable)?;
    if response.status == 401 || response.status == 403 {
        return Err(ChatError::Rejected);
    }
    if !(200..300).contains(&response.status) {
        return Err(ChatError::Failed {
            status: response.status,
        });
    }
    let Ok(payload) = serde_json::from_str::<Value>(&response.body) else {
        return Err(ChatError::Unparseable);
    };
    let Some(text) = payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    else {
        return Err(ChatError::Unparseable);
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(ChatError::Empty);
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use serde_json::{Value, json};

    use super::{
        ChatError, PrepareError, PreparedChat, complete_chat, missing_credential_message,
        prepare_chat,
    };
    use crate::handle::ProviderHandle;
    use crate::ids::ProviderId;
    use crate::registry::ProviderFamily;
    use crate::secrets::SecretBag;
    use crate::settings::{ProviderSettings, TestReport};
    use crate::transport::{HttpResponse, MockTransport, Transport, TransportError};

    struct RecordingTransport {
        posts: RefCell<Vec<(String, String, String)>>,
        response: HttpResponse,
    }

    impl RecordingTransport {
        fn new(response: HttpResponse) -> Self {
            Self {
                posts: RefCell::new(Vec::new()),
                response,
            }
        }
    }

    impl Transport for RecordingTransport {
        fn post_form(&self, url: &str, _body: &str) -> Result<HttpResponse, TransportError> {
            Err(TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
        }

        fn get_bearer(&self, url: &str, _bearer: &str) -> Result<HttpResponse, TransportError> {
            Err(TransportError::NoRoute {
                method: "GET".to_owned(),
                url: url.to_owned(),
            })
        }

        fn post_json_bearer(
            &self,
            url: &str,
            bearer: &str,
            body: &str,
        ) -> Result<HttpResponse, TransportError> {
            self.posts
                .borrow_mut()
                .push((url.to_owned(), bearer.to_owned(), body.to_owned()));
            Ok(self.response.clone())
        }
    }

    fn report(ok: bool) -> TestReport {
        TestReport {
            ok,
            message: "recorded".to_owned(),
        }
    }

    fn handle_with(
        provider: ProviderId,
        model: &str,
        models: &[&str],
        test_ok: Option<bool>,
        key: Option<&str>,
    ) -> ProviderHandle {
        let mut settings = ProviderSettings {
            selected_provider: provider,
            selected_model: model.to_owned(),
            ..ProviderSettings::default()
        };
        if !models.is_empty() {
            settings.store_models(
                provider,
                models.iter().map(|id| (*id).to_owned()).collect(),
                1,
            );
        }
        if let Some(ok) = test_ok {
            settings.store_test(provider, report(ok));
        }
        let mut bag = SecretBag::empty();
        match provider {
            ProviderId::XaiKey => bag.xai_api_key = key.map(str::to_owned),
            ProviderId::Openai => bag.openai_api_key = key.map(str::to_owned),
            ProviderId::Openrouter => bag.openrouter_api_key = key.map(str::to_owned),
            ProviderId::OpenaiCompatible => bag.openai_compatible_api_key = key.map(str::to_owned),
            ProviderId::XaiOauth => {}
        }
        ProviderHandle::from_parts(settings, bag)
    }

    fn ready(provider: ProviderId, model: &str, key: Option<&str>) -> ProviderHandle {
        handle_with(provider, model, &[model], Some(true), key)
    }

    #[test]
    fn missing_settings_file_is_no_provider_even_with_a_key() {
        let handle = ready(ProviderId::XaiKey, "grok-4.5", Some("saved"));
        let error =
            prepare_chat(&handle, false, Some("env-key"), None, None, None).expect_err("absent");
        assert_eq!(error, PrepareError::NoProvider);
        assert_eq!(
            error.to_string(),
            "No provider is configured. Choose a provider in Settings and run Test."
        );
        assert!(!error.to_string().contains("env-key"));
        assert!(!error.to_string().contains("saved"));
    }

    #[test]
    fn missing_or_failed_test_blocks_chat() {
        let missing = handle_with(
            ProviderId::XaiKey,
            "grok-4.5",
            &["grok-4.5"],
            None,
            Some("k"),
        );
        let error = prepare_chat(&missing, true, None, None, None, None).expect_err("no report");
        assert_eq!(
            error,
            PrepareError::TestNotSucceeded {
                provider: ProviderId::XaiKey
            }
        );
        assert_eq!(
            error.to_string(),
            "Test has not succeeded for xai-key. Run Test in Settings."
        );

        let failed = handle_with(
            ProviderId::XaiKey,
            "grok-4.5",
            &["grok-4.5"],
            Some(false),
            Some("k"),
        );
        let error = prepare_chat(&failed, true, None, None, None, None).expect_err("failed test");
        assert_eq!(
            error,
            PrepareError::TestNotSucceeded {
                provider: ProviderId::XaiKey
            }
        );
        assert_eq!(failed.cached_models(), &["grok-4.5".to_owned()]);
    }

    #[test]
    fn blank_or_uncached_model_is_no_model() {
        let blank = handle_with(
            ProviderId::XaiKey,
            "  ",
            &["grok-4.5"],
            Some(true),
            Some("k"),
        );
        assert_eq!(
            prepare_chat(&blank, true, None, None, None, None).expect_err("blank"),
            PrepareError::NoModel
        );
        let other = handle_with(
            ProviderId::XaiKey,
            "not-cached",
            &["grok-4.5"],
            Some(true),
            Some("k"),
        );
        let error = prepare_chat(&other, true, None, None, None, None).expect_err("uncached");
        assert_eq!(error, PrepareError::NoModel);
        assert_eq!(
            error.to_string(),
            "No chat model is selected. Choose one in Settings after Test."
        );
    }

    #[test]
    fn missing_credential_uses_the_provider_sentence() {
        let cases = [
            (
                ProviderId::XaiOauth,
                "grok-4.5",
                "No xAI sign-in is configured.",
            ),
            (
                ProviderId::XaiKey,
                "grok-4.5",
                "No xAI API key is configured.",
            ),
            (
                ProviderId::Openai,
                "gpt-4.1-mini",
                "No OpenAI API key is configured.",
            ),
            (
                ProviderId::Openrouter,
                "openai/gpt-4.1-mini",
                "No OpenRouter API key is configured.",
            ),
            (
                ProviderId::OpenaiCompatible,
                "gpt-4.1-mini",
                "No OpenAI-compatible API key is configured.",
            ),
        ];
        for (provider, model, sentence) in cases {
            let handle = ready(provider, model, None);
            let error = prepare_chat(&handle, true, None, None, None, None).expect_err("no key");
            assert_eq!(error, PrepareError::NoCredential { provider }, "{sentence}");
            assert_eq!(error.to_string(), sentence);
            assert_eq!(missing_credential_message(provider), sentence);
        }
        let oauth = ready(ProviderId::XaiOauth, "grok-4.5", None);
        let error = prepare_chat(
            &oauth,
            true,
            Some("env-xai"),
            Some("env-openai"),
            None,
            None,
        )
        .expect_err("oauth env");
        assert_eq!(
            error,
            PrepareError::NoCredential {
                provider: ProviderId::XaiOauth
            }
        );
    }

    #[test]
    fn saved_key_beats_env_and_keeps_the_selected_model() {
        let handle = ready(ProviderId::XaiKey, "grok-custom", Some("saved-key"));
        let (prepared, bearer) =
            prepare_chat(&handle, true, Some("env-key"), None, None, None).expect("ready");
        assert_eq!(bearer, "saved-key");
        assert_eq!(prepared.model, "grok-custom");
        assert_eq!(prepared.provider, ProviderId::XaiKey);
        assert_eq!(prepared.family, ProviderFamily::Xai);
        assert!(!format!("{prepared:?}").contains("saved-key"));
        assert!(!format!("{prepared:?}").contains("env-key"));
    }

    fn prepared(family: ProviderFamily, model: &str) -> PreparedChat {
        let provider = match family {
            ProviderFamily::Xai => ProviderId::XaiKey,
            ProviderFamily::Openai => ProviderId::Openai,
            ProviderFamily::Openrouter => ProviderId::Openrouter,
            ProviderFamily::OpenaiCompatible => ProviderId::OpenaiCompatible,
        };
        let api_base = family
            .fixed_api_base()
            .unwrap_or("http://127.0.0.1:9/v1")
            .to_owned();
        PreparedChat {
            provider,
            family,
            api_base,
            model: model.to_owned(),
        }
    }

    fn content_response(status: u16, content: &Value) -> HttpResponse {
        HttpResponse {
            status,
            body: json!({
                "choices": [{"message": {"role": "assistant", "content": content}}]
            })
            .to_string(),
        }
    }

    #[test]
    fn complete_chat_posts_system_then_user_without_temperature() {
        let cases = [
            (
                ProviderFamily::Xai,
                "grok-custom",
                "https://api.x.ai/v1/chat/completions",
            ),
            (
                ProviderFamily::Openai,
                "gpt-custom",
                "https://api.openai.com/v1/chat/completions",
            ),
        ];
        for (family, model, url) in cases {
            let transport = RecordingTransport::new(content_response(200, &json!("pong")));
            let reply = complete_chat(
                &transport,
                &prepared(family, model),
                "sk-test-secret",
                "be brief",
                "hello",
            )
            .expect("pong");
            assert_eq!(reply, "pong");
            let posts = transport.posts.borrow();
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0].0, url);
            assert_eq!(posts[0].1, "sk-test-secret");
            let body: Value = serde_json::from_str(&posts[0].2).expect("json");
            assert_eq!(body["model"], model);
            assert_eq!(body["max_tokens"], 1024);
            assert_eq!(body["messages"][0]["role"], "system");
            assert_eq!(body["messages"][0]["content"], "be brief");
            assert_eq!(body["messages"][1]["role"], "user");
            assert_eq!(body["messages"][1]["content"], "hello");
            assert!(body.get("temperature").is_none());
            assert!(body.get("stream").is_none());
            assert!(body.get("tools").is_none());
        }
    }

    #[test]
    fn complete_chat_trims_and_rejects_empty_or_unreadable_content() {
        let prepared = prepared(ProviderFamily::Xai, "grok-4.5");
        let trimmed = RecordingTransport::new(content_response(200, &json!("  pong \n")));
        assert_eq!(
            complete_chat(&trimmed, &prepared, "token", "sys", "user").expect("trim"),
            "pong"
        );

        let empty = RecordingTransport::new(content_response(200, &json!("  \n\t")));
        assert_eq!(
            complete_chat(&empty, &prepared, "token", "sys", "user").expect_err("empty"),
            ChatError::Empty
        );

        let parts = RecordingTransport::new(content_response(
            200,
            &json!([{"type": "text", "text": "hi"}]),
        ));
        assert_eq!(
            complete_chat(&parts, &prepared, "token", "sys", "user").expect_err("parts"),
            ChatError::Unparseable
        );

        let none = RecordingTransport::new(HttpResponse {
            status: 200,
            body: json!({"choices": []}).to_string(),
        });
        assert_eq!(
            complete_chat(&none, &prepared, "token", "sys", "user").expect_err("choices"),
            ChatError::Unparseable
        );
    }

    #[test]
    fn complete_chat_hides_secrets_on_rejection_and_failure() {
        let prepared = prepared(ProviderFamily::Xai, "grok-4.5");
        let rejected = RecordingTransport::new(HttpResponse {
            status: 401,
            body: "sk-test-secret".to_owned(),
        });
        let error =
            complete_chat(&rejected, &prepared, "sk-test-secret", "sys", "user").expect_err("401");
        assert_eq!(error, ChatError::Rejected);
        assert_eq!(error.to_string(), "Provider rejected the credentials.");
        assert!(!error.to_string().contains("sk-test-secret"));

        let forbidden = RecordingTransport::new(HttpResponse {
            status: 403,
            body: "sk-test-secret".to_owned(),
        });
        assert_eq!(
            complete_chat(&forbidden, &prepared, "sk-test-secret", "sys", "user").expect_err("403"),
            ChatError::Rejected
        );

        let failed = RecordingTransport::new(HttpResponse {
            status: 500,
            body: "sk-test-secret".to_owned(),
        });
        let error =
            complete_chat(&failed, &prepared, "sk-test-secret", "sys", "user").expect_err("500");
        assert_eq!(error, ChatError::Failed { status: 500 });
        assert_eq!(error.to_string(), "Chat completion failed (500).");
        assert!(!error.to_string().contains("sk-test-secret"));
    }

    #[test]
    fn unregistered_route_is_unreachable_without_the_url() {
        let error = complete_chat(
            &MockTransport::new(),
            &prepared(ProviderFamily::Xai, "grok-4.5"),
            "token",
            "sys",
            "user",
        )
        .expect_err("no route");
        assert_eq!(error, ChatError::Unreachable);
        assert_eq!(error.to_string(), "Could not reach the provider.");
        assert!(!error.to_string().contains("https://"));
    }
}
