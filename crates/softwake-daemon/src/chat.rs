//! Typed demo chat against the selected provider.
//!
//! The session crate does not open a socket. This module loads Settings from
//! explicit paths on the disk path and, when `live-http` is enabled, posts one
//! completion. See [ADR 0013](../../docs/ADR-0013-session-provider.md).

use std::time::Duration;

use softwake_providers::{
    PreparedChat, ProviderHandle, prepare_chat, resolve_providers_file, resolve_secrets_file,
};
use softwake_session::{SessionError, SessionPhase, TextStubSession};

/// Connect, read, and overall timeout for one live chat call.
#[cfg_attr(not(feature = "live-http"), allow(dead_code))]
pub(crate) const CHAT_TIMEOUT: Duration = Duration::from_secs(30);

/// Shown when readiness passed and this binary was built without `live-http`.
#[cfg_attr(feature = "live-http", allow(dead_code))]
pub(crate) const LIVE_HTTP_DISABLED: &str = "Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.";

/// Build the memory appendix for one ask/chat turn.
///
/// Opens [`softwake_memory::FileMemory`] at the resolved Softwake state path
/// only when that file already exists. Missing file, resolve failure, open
/// failure, and recall failure are all an empty appendix (fail-open). Does
/// not create `memory.json`. See [ADR 0009](../../docs/ADR-0009-long-term-memory.md).
#[cfg_attr(not(any(test, feature = "live-http")), allow(dead_code))]
pub(crate) fn disk_memory_appendix(query: &str) -> String {
    let Ok(path) = softwake_memory::resolve_memory_file() else {
        return String::new();
    };
    memory_appendix_at_path(&path, query)
}

/// Fail-open recall against an explicit `memory.json` path.
#[cfg_attr(not(any(test, feature = "live-http")), allow(dead_code))]
pub(crate) fn memory_appendix_at_path(path: &std::path::Path, query: &str) -> String {
    if !path.is_file() {
        return String::new();
    }
    let Ok(memory) = softwake_memory::FileMemory::open_enabled(path) else {
        return String::new();
    };
    softwake_memory::recall_for_prompt(&memory, query)
}

/// Why one ask did not return assistant text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskReject {
    /// `text` is empty or only whitespace. The completer was not called.
    NeedsText,
    /// The session is closed. The completer was not called.
    Closed,
    /// The completer failed. The user line stays recorded.
    Failed(String),
}

impl AskReject {
    /// Operator sentence. No `rejected:` prefix and no bearer.
    #[must_use]
    pub(crate) fn sentence(&self) -> &str {
        match self {
            Self::NeedsText => "ask needs text",
            Self::Closed => "session is closed",
            Self::Failed(message) => message,
        }
    }
}

/// Budgeted appendix from `fixture` when present, otherwise the disk file.
///
/// A missing file is an empty appendix. This does not create `memory.json`.
#[must_use]
pub(crate) fn appendix_for_ask(
    query: &str,
    fixture: Option<&impl softwake_memory::Memory>,
) -> String {
    match fixture {
        Some(memory) => softwake_memory::recall_for_prompt(memory, query),
        None => disk_memory_appendix(query),
    }
}

/// Record `text` and call `complete`, unless the line is blank or the session is closed.
///
/// Blank text and a closed session do not call `complete`.
pub(crate) fn perform_ask(
    session: &mut TextStubSession,
    text: &str,
    appendix: &str,
    complete: impl FnOnce(&str, &str) -> Result<String, String>,
) -> Result<String, AskReject> {
    if text.trim().is_empty() {
        return Err(AskReject::NeedsText);
    }
    if session.phase() != SessionPhase::Open {
        return Err(AskReject::Closed);
    }
    session
        .ask(text, appendix, complete)
        .map_err(|error| match error {
            SessionError::Empty => AskReject::NeedsText,
            SessionError::Closed => AskReject::Closed,
            SessionError::Complete { message } => AskReject::Failed(message),
        })
}

/// Refuse a disk ask before the session records the line when live HTTP is off.
///
/// With `live-http` this does nothing, so the caller can record and then complete.
///
/// # Errors
///
/// [`LIVE_HTTP_DISABLED`] when this binary was built without `live-http`.
pub(crate) fn gate_live_http(
    prepared: &PreparedChat,
    bearer: &str,
    user: &str,
) -> Result<(), String> {
    #[cfg(not(feature = "live-http"))]
    {
        finish_prepared_chat(prepared, bearer, "", user)?;
        Ok(())
    }
    #[cfg(feature = "live-http")]
    {
        let _ = (prepared, bearer, user);
        Ok(())
    }
}

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
        ProviderHandle::load_resolved(&settings, &secrets).map_err(|error| error.to_string())?;
    let env_xai = std::env::var("XAI_API_KEY").ok();
    let env_openai = std::env::var("OPENAI_API_KEY").ok();
    let env_openrouter = std::env::var("OPENROUTER_API_KEY").ok();
    let env_openai_compatible = std::env::var("OPENAI_COMPATIBLE_API_KEY").ok();
    let (prepared, bearer) = prepare_chat(
        &handle,
        settings_file_present,
        env_xai.as_deref(),
        env_openai.as_deref(),
        env_openrouter.as_deref(),
        env_openai_compatible.as_deref(),
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
    use std::sync::{Arc, Mutex};

    use softwake_providers::{
        HttpResponse, PreparedChat, ProviderHandle, ProviderId, ProviderSettings, SecretBag,
        TestReport, Transport, TransportError,
    };

    /// One recorded JSON POST. The bearer is not stored.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) struct RecordedPost {
        pub(crate) url: String,
        pub(crate) body: String,
    }

    pub(crate) struct ScriptedTransport {
        response: HttpResponse,
        posts: Mutex<Vec<RecordedPost>>,
    }

    impl ScriptedTransport {
        fn new(response: HttpResponse) -> Arc<Self> {
            Arc::new(Self {
                response,
                posts: Mutex::new(Vec::new()),
            })
        }

        pub(crate) fn posts(&self) -> Vec<RecordedPost> {
            self.posts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
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
            self.posts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(RecordedPost {
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
        pub(crate) transport: Arc<ScriptedTransport>,
    }

    /// Readiness for one in-test ask. Env fallbacks stay unset.
    ///
    /// # Errors
    ///
    /// The provider sentence. No post is made.
    pub(crate) fn prepare_fixture(fixture: &ChatFixture) -> Result<(PreparedChat, String), String> {
        softwake_providers::prepare_chat(
            &fixture.handle,
            fixture.settings_file_present,
            fixture.env_xai.as_deref(),
            fixture.env_openai.as_deref(),
            None,
            None,
        )
        .map_err(|error| error.to_string())
    }

    /// One completion on the scripted transport.
    ///
    /// # Errors
    ///
    /// The chat-completion sentence. The text does not include the bearer.
    pub(crate) fn complete_fixture(
        transport: &ScriptedTransport,
        prepared: &PreparedChat,
        bearer: &str,
        system: &str,
        user: &str,
    ) -> Result<String, String> {
        softwake_providers::complete_chat(transport, prepared, bearer, system, user)
            .map_err(|error| error.to_string())
    }

    /// xAI API-key fixture with a scripted assistant `content` body.
    ///
    /// `key` is a test bearer. It is not a live credential.
    pub(crate) fn xai_key_fixture(test_ok: bool, key: Option<&str>, content: &str) -> ChatFixture {
        let mut settings = ProviderSettings {
            selected_provider: ProviderId::XaiKey,
            selected_model: "grok-4.5".to_owned(),
            ..ProviderSettings::default()
        };
        settings.store_models(
            ProviderId::XaiKey,
            vec!["grok-4.5".to_owned()],
            Vec::new(),
            1,
        );
        settings.store_test(
            ProviderId::XaiKey,
            TestReport {
                ok: test_ok,
                message: "recorded".to_owned(),
            },
        );
        let mut bag = SecretBag::empty();
        bag.xai_api_key = key.map(str::to_owned);
        let response = HttpResponse {
            status: 200,
            body: serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": content}}]
            })
            .to_string(),
        };
        ChatFixture::new(ProviderHandle::from_parts(settings, bag), true, response)
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
pub(crate) use fixture::{
    ChatFixture, RecordedPost, complete_fixture, prepare_fixture, xai_key_fixture,
};

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
        assert!(handle.bearer_token(None, None, None, None).is_none());
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
        document.store_models(
            ProviderId::XaiKey,
            vec!["grok-4.5".to_owned()],
            Vec::new(),
            1,
        );
        FileProviderSettings::new(&settings)
            .expect("store")
            .save(&document)
            .expect("save");
        let (handle, present) = ProviderHandle::load_paths(&settings, &secrets).expect("load");
        assert!(present);
        assert!(!secrets.exists());
        assert!(handle.bearer_token(None, None, None, None).is_none());
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
            api_base: softwake_providers::XAI_API_BASE.to_owned(),
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

#[cfg(test)]
mod memory_tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use softwake_memory::{FileMemory, MEMORY_FILE_NAME, MockMemory};

    use super::{disk_memory_appendix, memory_appendix_at_path};

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(1);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("softwake-chat-memory-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn memory_appendix_at_path_is_empty_when_file_missing() {
        let dir = TempDir::new();
        assert!(memory_appendix_at_path(&dir.path.join(MEMORY_FILE_NAME), "q").is_empty());
        assert!(memory_appendix_at_path(&dir.path.join("nope.json"), "q").is_empty());
    }

    #[test]
    fn memory_appendix_at_path_budgets_file_memory_hits() {
        let dir = TempDir::new();
        let path = dir.path.join(MEMORY_FILE_NAME);
        let mut memory = FileMemory::open_enabled(&path).expect("open");
        memory.remember("alpha one").expect("a");
        memory.remember("beta").expect("b");
        memory.remember("alpha two").expect("c");
        drop(memory);
        let appendix = memory_appendix_at_path(&path, "alpha");
        assert!(appendix.contains("alpha one"));
        assert!(appendix.contains("alpha two"));
        assert!(!appendix.contains("beta"));
        assert!(memory_appendix_at_path(&path, "").is_empty());
    }

    #[test]
    fn disk_memory_appendix_fail_opens_without_state_dir() {
        // Unset state dirs so resolve fails; fail-open returns empty.
        // Do not assert on a developer's real XDG state file.
        let _ = disk_memory_appendix;
        let disabled = MockMemory::default();
        assert!(softwake_memory::recall_for_prompt(&disabled, "x").is_empty());
    }
}
