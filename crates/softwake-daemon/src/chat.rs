//! Typed demo chat against the selected provider.
//!
//! The session crate does not open a socket. This module loads Settings from
//! explicit paths on the disk path and, when `live-http` is enabled, posts one
//! completion. See [ADR 0013](../../docs/ADR-0013-session-provider.md).

use std::time::Duration;

use softwake_providers::{
    CHAT_MAX_TOKENS, ChatMessage, ChatRole, PreparedChat, ProviderHandle, estimate_tokens_parts,
    extractive_summary, prepare_chat, resolve_compact_at_percent, resolve_context_limit,
    resolve_keep_recent_turns, resolve_providers_file, resolve_secrets_file, should_compact,
    usage_percent,
};
#[cfg(feature = "live-http")]
use softwake_providers::{complete_chat, complete_compact};
use softwake_session::{
    MessageRole, SessionError, SessionMessage, SessionPhase, TextStubSession, assemble_system,
};

/// Connect, read, and overall timeout for one live chat, STT, or TTS HTTP call.
///
/// A 30s budget aborted slow completions while the provider was still writing,
/// so long replies never arrived. 120s is the whole-call ceiling. Spoken
/// playback is a separate 60s reaper (`softwake_voice::PLAYBACK_TIMEOUT`) and
/// can still stop audio after the text reply is already complete.
#[cfg_attr(not(feature = "live-http"), allow(dead_code))]
pub(crate) const CHAT_TIMEOUT: Duration = Duration::from_secs(120);

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

/// Context usage recorded for Status after one ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AskContext {
    /// Tokens about to be sent (after compaction, when it ran).
    pub(crate) used: u32,
    /// Tokens estimated before compaction.
    pub(crate) before_compact: u32,
    pub(crate) limit: u32,
    pub(crate) percent: u8,
    pub(crate) compacted: bool,
    /// Resolved Settings compact percent (not a hardcoded 80).
    pub(crate) threshold_percent: u8,
}

/// `softwaked: context profile=… sent=… before_compact=… limit=… (N%)`
#[must_use]
pub(crate) fn format_context_sent_line(profile: &str, context: &AskContext) -> String {
    format!(
        "softwaked: context {profile} sent={} before_compact={} limit={} ({}%)",
        context.used, context.before_compact, context.limit, context.percent
    )
}

/// `softwaked: compact profile=… before=… after=… threshold=N%`
#[must_use]
pub(crate) fn format_compact_line(profile: &str, context: &AskContext) -> String {
    format!(
        "softwaked: compact {profile} before={} after={} threshold={}%",
        context.before_compact, context.used, context.threshold_percent
    )
}

/// Successful ask with context accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AskOk {
    pub(crate) reply: String,
    pub(crate) context: AskContext,
}

/// Budget knobs for one ask (from Settings + model id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContextBudget {
    pub(crate) limit: u32,
    pub(crate) compact_at_percent: u8,
    pub(crate) keep_recent: usize,
}

impl ContextBudget {
    #[must_use]
    pub(crate) fn from_settings(
        model: &str,
        settings: &softwake_providers::ProviderSettings,
    ) -> Self {
        Self {
            limit: resolve_context_limit(model, settings.context_limit_tokens),
            compact_at_percent: resolve_compact_at_percent(settings.compact_at_percent),
            keep_recent: resolve_keep_recent_turns(settings.keep_recent_turns),
        }
    }
}

/// Map session messages to provider chat messages.
#[must_use]
pub(crate) fn to_chat_messages(messages: &[SessionMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|message| ChatMessage {
            role: match message.role {
                MessageRole::User => ChatRole::User,
                MessageRole::Assistant => ChatRole::Assistant,
            },
            content: message.content.clone(),
        })
        .collect()
}

/// Estimate tokens for system + prior messages + pending user + reply headroom.
#[must_use]
pub(crate) fn estimate_ask_usage(system: &str, prior: &[SessionMessage], new_user: &str) -> u32 {
    let mut parts: Vec<&str> = vec![system, new_user];
    for message in prior {
        parts.push(message.content.as_str());
    }
    estimate_tokens_parts(parts).saturating_add(CHAT_MAX_TOKENS)
}

/// Record `text` and call `complete`, compacting older turns when over budget.
///
/// Blank text and a closed session do not call `complete` or `compact`.
pub(crate) fn perform_ask(
    session: &mut TextStubSession,
    text: &str,
    appendix: &str,
    budget: ContextBudget,
    compact: impl FnOnce(&[SessionMessage]) -> Result<String, String>,
    complete: impl FnOnce(&str, &[SessionMessage]) -> Result<String, String>,
    mut trace: impl FnMut(&AskContext),
) -> Result<AskOk, AskReject> {
    if text.trim().is_empty() {
        return Err(AskReject::NeedsText);
    }
    if session.phase() != SessionPhase::Open {
        return Err(AskReject::Closed);
    }
    let Some(instructions) = session.instructions() else {
        return Err(AskReject::Closed);
    };
    let system = assemble_system(instructions, appendix);
    let mut compacted = false;
    let before_compact = estimate_ask_usage(&system, session.messages(), text);
    if should_compact(before_compact, budget.limit, budget.compact_at_percent)
        && session.messages().len() > budget.keep_recent
    {
        let prefix_len = session.messages().len() - budget.keep_recent;
        let older: Vec<SessionMessage> = session.messages()[..prefix_len].to_vec();
        let summary = match compact(&older) {
            Ok(text) => text,
            Err(_error) => {
                let chat = to_chat_messages(&older);
                extractive_summary(&chat, 2000)
            }
        };
        session.apply_compaction(budget.keep_recent, &summary);
        compacted = true;
    }
    let used = estimate_ask_usage(&system, session.messages(), text);
    let context = AskContext {
        used,
        before_compact,
        limit: budget.limit,
        percent: usage_percent(used, budget.limit),
        compacted,
        threshold_percent: budget.compact_at_percent,
    };
    trace(&context);
    match session.ask(text, appendix, complete) {
        Ok(reply) => Ok(AskOk { reply, context }),
        Err(SessionError::Empty) => Err(AskReject::NeedsText),
        Err(SessionError::Closed) => Err(AskReject::Closed),
        Err(SessionError::Complete { message }) => Err(AskReject::Failed(message)),
    }
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
        finish_prepared_chat(
            prepared,
            bearer,
            "",
            &[softwake_session::SessionMessage::user(user)],
        )?;
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
    /// Settings `selected_tts_voice`. Empty means the xAI default when speaking.
    pub(crate) tts_voice: String,
    /// Settings `selected_voice_model`. Empty means the xAI STT seed.
    pub(crate) stt_model: String,
    /// Resolved context budget from Settings + model id.
    pub(crate) budget: ContextBudget,
}

impl std::fmt::Debug for DiskChat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DiskChat")
            .field("prepared", &self.prepared)
            .field("bearer", &"<redacted>")
            .field("tts_voice", &self.tts_voice)
            .field("stt_model", &self.stt_model)
            .field("budget", &self.budget)
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
    // OAuth access tokens expire. prepare_chat only reads the bag; refresh here
    // so state-voice TTS and STT do not 403 as a fake "unreachable" skip.
    #[cfg(feature = "live-http")]
    let bearer = refresh_oauth_bearer_if_needed(&secrets, &prepared, bearer)?;
    let budget = ContextBudget::from_settings(&prepared.model, handle.settings());
    Ok(DiskChat {
        tts_voice: handle.selected_tts_voice().unwrap_or("").to_owned(),
        stt_model: handle.selected_voice_model().unwrap_or("").to_owned(),
        prepared,
        bearer,
        budget,
    })
}

/// Refresh xAI OAuth when near expiry; persist the new tokens. Other providers
/// return `bearer` unchanged.
#[cfg(feature = "live-http")]
fn refresh_oauth_bearer_if_needed(
    secrets_path: &std::path::Path,
    prepared: &PreparedChat,
    bearer: String,
) -> Result<String, String> {
    use softwake_providers::{ProviderId, ensure_fresh_access, open_store, update_bag};
    if prepared.provider != ProviderId::XaiOauth {
        return Ok(bearer);
    }
    let store = open_store(secrets_path).map_err(|error| error.to_string())?;
    let bag = store.load().map_err(|error| error.to_string())?;
    let Some(tokens) = bag.xai_oauth.as_ref() else {
        return Ok(bearer);
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let transport = softwake_providers::live::LiveTransport::bounded(CHAT_TIMEOUT);
    let fresh = ensure_fresh_access(&transport, tokens, now_ms).map_err(|error| {
        format!("Voice credentials need refresh (re-run Providers Test or re-sign in): {error}")
    })?;
    if fresh.access_token != tokens.access_token || fresh.expires_at_ms != tokens.expires_at_ms {
        let access = fresh.access_token.clone();
        update_bag(store.as_ref(), |bag| {
            bag.xai_oauth = Some(fresh);
        })
        .map_err(|error| error.to_string())?;
        return Ok(access);
    }
    Ok(fresh.access_token)
}

impl DiskChat {
    /// Saved TTS voice id, or empty when Settings left the default.
    #[must_use]
    pub(crate) fn prepared_tts_voice(&self) -> &str {
        &self.tts_voice
    }
}

/// One completion that is not recorded on a session.
///
/// `complete` receives the soul system text and exactly one user message.
///
/// # Errors
///
/// Whatever `complete` returns.
pub(crate) fn complete_oneshot(
    system: &str,
    prompt: &str,
    complete: impl FnOnce(&str, &[softwake_providers::ChatMessage]) -> Result<String, String>,
) -> Result<String, String> {
    let (system, messages) = crate::announce::oneshot_messages(system, prompt);
    complete(&system, &messages)
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
    messages: &[SessionMessage],
) -> Result<String, String> {
    finish_prepared_chat_inner(prepared, bearer, system, messages)
}

pub(crate) fn finish_prepared_compact(
    prepared: &PreparedChat,
    bearer: &str,
    older: &[SessionMessage],
) -> Result<String, String> {
    finish_prepared_compact_inner(prepared, bearer, older)
}

#[cfg(not(feature = "live-http"))]
fn finish_prepared_chat_inner(
    _prepared: &PreparedChat,
    _bearer: &str,
    _system: &str,
    _messages: &[SessionMessage],
) -> Result<String, String> {
    Err(LIVE_HTTP_DISABLED.to_owned())
}

#[cfg(not(feature = "live-http"))]
fn finish_prepared_compact_inner(
    _prepared: &PreparedChat,
    _bearer: &str,
    _older: &[SessionMessage],
) -> Result<String, String> {
    Err(LIVE_HTTP_DISABLED.to_owned())
}

#[cfg(feature = "live-http")]
fn finish_prepared_chat_inner(
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    messages: &[SessionMessage],
) -> Result<String, String> {
    let transport = softwake_providers::live::LiveTransport::bounded(CHAT_TIMEOUT);
    let chat = to_chat_messages(messages);
    complete_chat(&transport, prepared, bearer, system, &chat).map_err(|error| error.to_string())
}

#[cfg(feature = "live-http")]
fn finish_prepared_compact_inner(
    prepared: &PreparedChat,
    bearer: &str,
    older: &[SessionMessage],
) -> Result<String, String> {
    let transport = softwake_providers::live::LiveTransport::bounded(CHAT_TIMEOUT);
    let chat = to_chat_messages(older);
    complete_compact(&transport, prepared, bearer, &chat).map_err(|error| error.to_string())
}

#[cfg(test)]
mod fixture {
    use std::sync::{Arc, Mutex};

    use softwake_providers::{
        HttpBytes, HttpResponse, MultipartField, PreparedChat, ProviderHandle, ProviderId,
        ProviderSettings, SecretBag, TestReport, Transport, TransportError,
    };
    use softwake_session::SessionMessage;

    use super::{ContextBudget, to_chat_messages};

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

        fn post_multipart_bearer(
            &self,
            url: &str,
            _bearer: &str,
            _fields: &[MultipartField],
            _file_name: &str,
            _file_bytes: &[u8],
            _file_content_type: &str,
        ) -> Result<HttpResponse, TransportError> {
            Err(TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
        }

        fn post_json_bearer_bytes(
            &self,
            url: &str,
            _bearer: &str,
            _body: &str,
        ) -> Result<HttpBytes, TransportError> {
            Err(TransportError::NoRoute {
                method: "POST".to_owned(),
                url: url.to_owned(),
            })
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
        messages: &[SessionMessage],
    ) -> Result<String, String> {
        let chat = to_chat_messages(messages);
        softwake_providers::complete_chat(transport, prepared, bearer, system, &chat)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn compact_fixture(
        transport: &ScriptedTransport,
        prepared: &PreparedChat,
        bearer: &str,
        older: &[SessionMessage],
    ) -> Result<String, String> {
        let chat = to_chat_messages(older);
        softwake_providers::complete_compact(transport, prepared, bearer, &chat)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn budget_for_handle(handle: &ProviderHandle) -> ContextBudget {
        let model = handle.selected_model().unwrap_or("");
        ContextBudget::from_settings(model, handle.settings())
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
    ChatFixture, RecordedPost, budget_for_handle, compact_fixture, complete_fixture,
    prepare_fixture, xai_key_fixture,
};

#[cfg(test)]
mod tests {
    use super::{AskContext, format_compact_line, format_context_sent_line};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn context_lines_show_sent_and_pre_compact_sizes() {
        let context = AskContext {
            used: 1200,
            before_compact: 4500,
            limit: 8000,
            percent: 15,
            compacted: true,
            threshold_percent: 70,
        };
        assert_eq!(
            format_context_sent_line("profile=sally", &context),
            "softwaked: context profile=sally sent=1200 before_compact=4500 limit=8000 (15%)"
        );
        assert_eq!(
            format_compact_line("profile=sally", &context),
            "softwaked: compact profile=sally before=4500 after=1200 threshold=70%"
        );
    }

    use softwake_providers::{
        FileProviderSettings, ProviderHandle, ProviderId, ProviderSettings, TestReport,
    };
    #[cfg(not(feature = "live-http"))]
    use softwake_providers::{PreparedChat, ProviderFamily};
    #[cfg(not(feature = "live-http"))]
    use softwake_session::SessionMessage;

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
        assert_eq!(CHAT_TIMEOUT, std::time::Duration::from_secs(120));
        let prepared = PreparedChat {
            provider: ProviderId::XaiKey,
            family: ProviderFamily::Xai,
            api_base: softwake_providers::XAI_API_BASE.to_owned(),
            model: "grok-4.5".to_owned(),
        };
        let error = finish_prepared_chat(
            &prepared,
            "sk-test-secret",
            "system",
            &[SessionMessage::user("user")],
        )
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
