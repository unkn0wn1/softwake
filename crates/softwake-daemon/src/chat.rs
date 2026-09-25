//! Typed demo chat against the selected provider.
//!
//! The session crate does not open a socket. This module loads Settings from
//! explicit paths on the disk path and, when `live-http` is enabled, posts one
//! completion. See [ADR 0013](../../docs/ADR-0013-session-provider.md).

use std::time::Duration;

use softwake_providers::{
    PreparedChat, ProviderHandle, prepare_chat, resolve_providers_file, resolve_secrets_file,
};

/// Connect, read, and overall timeout for one live chat call.
#[cfg_attr(not(feature = "live-http"), allow(dead_code))]
pub(crate) const CHAT_TIMEOUT: Duration = Duration::from_secs(30);

/// Shown when readiness passed and this binary was built without `live-http`.
#[cfg_attr(feature = "live-http", allow(dead_code))]
pub(crate) const LIVE_HTTP_DISABLED: &str = "Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.";

/// Readiness result for the disk path. The bearer is not logged.
pub(crate) struct DiskChat {
    pub(crate) prepared: PreparedChat,
    pub(crate) bearer: String,
}

impl std::fmt::Debug for DiskChat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiskChat")
            .field("prepared", &self.prepared)
            .field("bearer", &"<redacted>")
            .finish()
    }
}

/// Load Settings and check readiness. Does not open a socket.
///
/// # Errors
///
/// Path, file, or readiness failures. The string is operator display text.
pub(crate) fn load_disk_chat() -> Result<DiskChat, String> {
    let settings = resolve_providers_file().map_err(|error| error.to_string())?;
    let secrets = resolve_secrets_file().map_err(|error| error.to_string())?;
    let (handle, settings_file_present) =
        ProviderHandle::load_paths(&settings, &secrets).map_err(|error| error.to_string())?;
    let env_xai = std::env::var("XAI_API_KEY").ok();
    let env_openai = std::env::var("OPENAI_API_KEY").ok();
    let (prepared, bearer) = prepare_chat(
        &handle,
        settings_file_present,
        env_xai.as_deref(),
        env_openai.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    Ok(DiskChat { prepared, bearer })
}

/// Finish a prepared disk chat.
///
/// Without `live-http` this returns [`LIVE_HTTP_DISABLED`] and does not construct
/// a live client. With the feature it posts with [`CHAT_TIMEOUT`] on connect,
/// read, and the overall call.
///
/// # Errors
///
/// The live-HTTP sentence, or a chat-completion display string. The text does
/// not include the bearer.
pub(crate) fn finish_prepared_chat(
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    finish_prepared_chat_inner(prepared, bearer, system, user)
}

#[cfg(not(feature = "live-http"))]
fn finish_prepared_chat_inner(
    _prepared: &PreparedChat,
    _bearer: &str,
    _system: &str,
    _user: &str,
) -> Result<String, String> {
    Err(LIVE_HTTP_DISABLED.to_owned())
}

#[cfg(feature = "live-http")]
fn finish_prepared_chat_inner(
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let transport = softwake_providers::live::LiveTransport::bounded(CHAT_TIMEOUT);
    softwake_providers::complete_chat(&transport, prepared, bearer, system, user)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod fixture {
    use std::cell::RefCell;
    use std::rc::Rc;

    use softwake_providers::{HttpResponse, ProviderHandle, Transport, TransportError};

    /// One recorded JSON POST. The bearer is not stored.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RecordedPost {
        pub(crate) url: String,
        pub(crate) body: String,
    }

    pub(crate) struct ScriptedTransport {
        response: HttpResponse,
        posts: RefCell<Vec<RecordedPost>>,
    }

    impl ScriptedTransport {
        fn new(response: HttpResponse) -> Rc<Self> {
            Rc::new(Self {
                response,
                posts: RefCell::new(Vec::new()),
            })
        }

        pub(crate) fn posts(&self) -> Vec<RecordedPost> {
            self.posts.borrow().clone()
        }
    }

    impl Transport for ScriptedTransport {
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
            _bearer: &str,
            body: &str,
        ) -> Result<HttpResponse, TransportError> {
            self.posts.borrow_mut().push(RecordedPost {
                url: url.to_owned(),
                body: body.to_owned(),
            });
            Ok(self.response.clone())
        }
    }

    /// In-test chat target. Does not read process environment.
    pub(crate) struct ChatFixture {
        pub(crate) handle: ProviderHandle,
        pub(crate) settings_file_present: bool,
        pub(crate) env_xai: Option<String>,
        pub(crate) env_openai: Option<String>,
        pub(crate) transport: Rc<ScriptedTransport>,
    }

    impl std::fmt::Debug for ChatFixture {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("ChatFixture")
                .field("handle", &self.handle.selected_provider())
                .field("settings_file_present", &self.settings_file_present)
                .field("env_xai", &self.env_xai.as_ref().map(|_| "<redacted>"))
                .field(
                    "env_openai",
                    &self.env_openai.as_ref().map(|_| "<redacted>"),
                )
                .field("transport", &self.transport.posts().len())
                .finish()
        }
    }

    impl ChatFixture {
        pub(crate) fn new(
            handle: ProviderHandle,
            settings_file_present: bool,
            response: HttpResponse,
        ) -> Self {
            Self {
                handle,
                settings_file_present,
                env_xai: None,
                env_openai: None,
                transport: ScriptedTransport::new(response),
            }
        }
    }
}

#[cfg(test)]
pub(crate) use fixture::{ChatFixture, RecordedPost};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use softwake_providers::{
        FileProviderSettings, ProviderHandle, ProviderId, ProviderSettings, TestReport,
    };
    #[cfg(not(feature = "live-http"))]
    use softwake_providers::{PreparedChat, ProviderFamily};

    use super::CHAT_TIMEOUT;
    #[cfg(not(feature = "live-http"))]
    use super::{LIVE_HTTP_DISABLED, finish_prepared_chat};

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("softwake-chat-load-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn load_paths_flags_a_missing_settings_file_and_an_empty_bag() {
        let dir = TempDir::new();
        let settings = dir.path.join("providers.json");
        let secrets = dir.path.join("secrets.json");
        let (handle, present) = ProviderHandle::load_paths(&settings, &secrets).expect("missing");
        assert!(!present);
        assert!(!secrets.exists());
        assert!(handle.bearer_token(None, None).is_none());
        assert!(handle.test_report().is_none());

        let mut document = ProviderSettings {
            selected_provider: ProviderId::XaiKey,
            selected_model: "grok-4.5".to_owned(),
            ..ProviderSettings::default()
        };
        document.store_test(
            ProviderId::XaiKey,
            TestReport {
                ok: true,
                message: "Chat check passed.".to_owned(),
            },
        );
        document.store_models(ProviderId::XaiKey, vec!["grok-4.5".to_owned()], 1);
        FileProviderSettings::new(&settings)
            .expect("store")
            .save(&document)
            .expect("save");
        let (handle, present) = ProviderHandle::load_paths(&settings, &secrets).expect("load");
        assert!(present);
        assert!(!secrets.exists());
        assert!(handle.bearer_token(None, None).is_none());
        assert_eq!(handle.selected_model(), Some("grok-4.5"));
        assert_eq!(handle.selected_provider(), ProviderId::XaiKey);
        let report = handle.test_report().expect("report");
        assert!(report.ok);
    }

    #[cfg(not(feature = "live-http"))]
    #[test]
    fn disk_finish_without_live_http_returns_the_sentence() {
        assert_eq!(CHAT_TIMEOUT, std::time::Duration::from_secs(30));
        let prepared = PreparedChat {
            provider: ProviderId::XaiKey,
            family: ProviderFamily::Xai,
            model: "grok-4.5".to_owned(),
        };
        let error = finish_prepared_chat(&prepared, "sk-test-secret", "system", "user")
            .expect_err("disabled");
        assert_eq!(error, LIVE_HTTP_DISABLED);
        assert!(!error.contains("sk-test-secret"));
        assert!(!error.contains("LiveTransport"));
    }

    #[cfg(feature = "live-http")]
    #[test]
    #[ignore = "live network; builds the bounded agent only"]
    fn bounded_agent_builds_without_a_request() {
        let _transport = softwake_providers::live::LiveTransport::bounded(CHAT_TIMEOUT);
    }
}
