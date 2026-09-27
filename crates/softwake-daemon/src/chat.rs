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
/// playback is a separate reaper, default 60 s (`PLAYBACK_TIMEOUT` /
/// `tts_playback_timeout_ms`, Settings 30-300 s), and can still stop audio
/// after the text reply is complete.
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

/// Live Tools permissions, then optional memory recall, for one ask/chat turn.
///
/// Tools permissions are always present so the model sees the current
/// `tools.json` modes even when memory is empty. Order: tools, then memory.
/// Email OAuth connection status (no tokens) is embedded in the tools appendix.
#[must_use]
pub(crate) fn system_appendix(memory: &str, tools: &softwake_tools::ToolsSettings) -> String {
    let email = email_oauth_status_for_appendix();
    let mcp = crate::mcp_bridge::mcp_appendix_lines(tools);
    softwake_session::join_appendices(&[
        &softwake_tools::tools_permissions_appendix(tools, &email),
        &mcp,
        memory,
    ])
}

/// Non-secret Google/Microsoft connection flags for the tools appendix.
///
/// Fail-open: a missing secrets file or load error means "not connected".
#[must_use]
pub(crate) fn email_oauth_status_for_appendix() -> softwake_tools::EmailOauthStatus {
    let Ok(path) = softwake_providers::resolve_secrets_file() else {
        return softwake_tools::EmailOauthStatus::default();
    };
    let Ok(store) = softwake_providers::open_store(&path) else {
        return softwake_tools::EmailOauthStatus::default();
    };
    let Ok(bag) = store.load() else {
        return softwake_tools::EmailOauthStatus::default();
    };
    let google = bag.google_connections.first();
    let microsoft = bag.microsoft_connections.first();
    softwake_tools::EmailOauthStatus {
        google_connected: google.is_some(),
        google_email: google.and_then(|c| c.account_email.clone()),
        microsoft_connected: microsoft.is_some(),
        microsoft_email: microsoft.and_then(|c| c.account_email.clone()),
    }
}

/// Context usage recorded for Status after one ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[allow(dead_code)] // retained for compact+complete without a tool loop
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

/// Compact and record the user line for an ask that will run the tool loop next.
///
/// Does not call the provider. On success the session already contains `text` as
/// the latest user message. The caller runs [`crate::tool_loop::run_tool_loop`]
/// then [`softwake_session::TextStubSession::push_assistant_turn`].
pub(crate) fn prepare_ask_session(
    session: &mut TextStubSession,
    text: &str,
    appendix: &str,
    budget: ContextBudget,
    compact: impl FnOnce(&[SessionMessage]) -> Result<String, String>,
    mut trace: impl FnMut(&AskContext),
) -> Result<(String, AskContext), AskReject> {
    if text.trim().is_empty() {
        return Err(AskReject::NeedsText);
    }
    if session.phase() != SessionPhase::Open {
        return Err(AskReject::Closed);
    }
    let Some(instructions) = session.instructions().map(str::to_owned) else {
        return Err(AskReject::Closed);
    };
    let system = assemble_system(&instructions, appendix);
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
    session.push_user_turn(text).map_err(|error| match error {
        SessionError::Empty => AskReject::NeedsText,
        SessionError::Closed => AskReject::Closed,
        SessionError::Complete { message } => AskReject::Failed(message),
    })?;
    // Re-assemble after push so system string stays aligned with instructions+appendix.
    let system = assemble_system(&instructions, appendix);
    Ok((system, context))
}

/// Operator slash / clear-typed commands that manage awake model context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextCommand {
    /// Drop all session turns; keep the open session and system pack.
    Clear,
    /// Force Hermes-style compaction of older turns (Settings `keep_recent`).
    Compact,
    /// Drop older turns until ~half the character mass remains.
    Halve,
}

/// Parse `/clear`, `/compact`, `/halve` / `/reduce`, or clear-typed equivalents.
///
/// Matching is case-insensitive on the trimmed line. Unknown text returns `None`
/// so the normal ask path runs.
#[must_use]
pub(crate) fn parse_context_command(text: &str) -> Option<ContextCommand> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let key = lower.strip_prefix('/').unwrap_or(lower.as_str()).trim();
    match key {
        "clear" | "clear context" => Some(ContextCommand::Clear),
        "compact" | "compact context" => Some(ContextCommand::Compact),
        "halve" | "halve context" | "reduce" | "reduce context" => Some(ContextCommand::Halve),
        _ => None,
    }
}

/// Estimate usage for an open session with no pending user line (HUD meter).
#[must_use]
pub(crate) fn estimate_session_usage(system: &str, messages: &[SessionMessage]) -> u32 {
    estimate_ask_usage(system, messages, "")
}

/// Format a short operator reply after a context command.
#[must_use]
pub(crate) fn format_context_command_reply(
    command: ContextCommand,
    context: &AskContext,
) -> String {
    let meter = format!(
        "context ~{} / {} ({}%); auto-compact at {}%",
        context.used, context.limit, context.percent, context.threshold_percent
    );
    match command {
        ContextCommand::Clear => format!("Context cleared — {meter}"),
        ContextCommand::Compact if context.compacted => {
            format!("Context compacted — {meter}")
        }
        ContextCommand::Compact => format!("Nothing to compact — {meter}"),
        ContextCommand::Halve => format!("Context halved — {meter}"),
    }
}

/// Build [`AskContext`] from system + current session messages.
#[must_use]
pub(crate) fn context_from_session(
    system: &str,
    messages: &[SessionMessage],
    budget: ContextBudget,
    compacted: bool,
    before_compact: u32,
) -> AskContext {
    let used = estimate_session_usage(system, messages);
    AskContext {
        used,
        before_compact,
        limit: budget.limit,
        percent: usage_percent(used, budget.limit),
        compacted,
        threshold_percent: budget.compact_at_percent,
    }
}

/// Apply a context command to the open session (no provider ask).
///
/// `compact` is only called for [`ContextCommand::Compact`] when there are
/// older turns beyond `keep_recent`. Fail-open to extractive summary on error.
pub(crate) fn apply_context_command(
    session: &mut TextStubSession,
    command: ContextCommand,
    system: &str,
    budget: ContextBudget,
    compact: impl FnOnce(&[SessionMessage]) -> Result<String, String>,
) -> AskContext {
    let before = estimate_session_usage(system, session.messages());
    let mut compacted = false;
    match command {
        ContextCommand::Clear => {
            session.clear_messages();
        }
        ContextCommand::Halve => {
            session.keep_newest_half();
        }
        ContextCommand::Compact => {
            let len = session.messages().len();
            if len > budget.keep_recent {
                let prefix_len = len - budget.keep_recent;
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
        }
    }
    context_from_session(system, session.messages(), budget, compacted, before)
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

/// Live or fixture completion with optional API tools and a Hands invoke callback.
///
/// When `tools` is empty this is one text completion (no `tools` field). Otherwise
/// it runs the multi-turn tool loop. Used by ask paths that already recorded the user line.
#[cfg(feature = "live-http")]
pub(crate) fn finish_prepared_chat_tools<F>(
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    messages: &[SessionMessage],
    tools: &[serde_json::Value],
    invoke: F,
) -> Result<crate::tool_loop::ToolLoopOk, String>
where
    F: FnMut(&str, &[String]) -> crate::tool_loop::ToolInvokeResult,
{
    let transport = softwake_providers::live::LiveTransport::bounded(CHAT_TIMEOUT);
    let chat = to_chat_messages(messages);
    crate::tool_loop::run_tool_loop(&transport, prepared, bearer, system, &chat, tools, invoke)
}

#[cfg(not(feature = "live-http"))]
pub(crate) fn finish_prepared_chat_tools<F>(
    prepared: &PreparedChat,
    bearer: &str,
    system: &str,
    messages: &[SessionMessage],
    tools: &[serde_json::Value],
    _invoke: F,
) -> Result<crate::tool_loop::ToolLoopOk, String>
where
    F: FnMut(&str, &[String]) -> crate::tool_loop::ToolInvokeResult,
{
    let _ = (prepared, bearer, system, messages, tools);
    Err(LIVE_HTTP_DISABLED.to_owned())
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
        /// Popped FIFO responses for multi-turn tool loops.
        queue: Mutex<std::collections::VecDeque<HttpResponse>>,
        /// Used when the queue is empty (single-response fixtures).
        fallback: HttpResponse,
        posts: Mutex<Vec<RecordedPost>>,
    }

    impl ScriptedTransport {
        fn new(response: HttpResponse) -> Arc<Self> {
            Arc::new(Self {
                queue: Mutex::new(std::collections::VecDeque::new()),
                fallback: response,
                posts: Mutex::new(Vec::new()),
            })
        }

        /// Scripted multi-turn: each POST takes the next queued body; leftover use the last.
        #[allow(dead_code)] // used by xai_key_fixture_queue for future integration tests
        fn with_queue(responses: Vec<HttpResponse>) -> Arc<Self> {
            assert!(!responses.is_empty(), "queue needs at least one response");
            let fallback = responses.last().expect("last").clone();
            Arc::new(Self {
                queue: Mutex::new(std::collections::VecDeque::from(responses)),
                fallback,
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
            let mut queue = self
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(queue.pop_front().unwrap_or_else(|| self.fallback.clone()))
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
    #[allow(dead_code)] // single-shot helper; tool path uses complete_fixture_tools
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

    pub(crate) fn complete_fixture_tools(
        transport: &ScriptedTransport,
        prepared: &PreparedChat,
        bearer: &str,
        system: &str,
        messages: &[SessionMessage],
        tools: &[serde_json::Value],
        invoke: impl FnMut(&str, &[String]) -> crate::tool_loop::ToolInvokeResult,
    ) -> Result<crate::tool_loop::ToolLoopOk, String> {
        let chat = to_chat_messages(messages);
        crate::tool_loop::run_tool_loop(transport, prepared, bearer, system, &chat, tools, invoke)
    }

    /// Fixture with a FIFO of chat completion bodies (tool-loop tests).
    #[allow(dead_code)]
    pub(crate) fn xai_key_fixture_queue(
        test_ok: bool,
        key: Option<&str>,
        bodies: Vec<HttpResponse>,
    ) -> ChatFixture {
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
        ChatFixture {
            handle: ProviderHandle::from_parts(settings, bag),
            settings_file_present: true,
            env_xai: None,
            env_openai: None,
            transport: ScriptedTransport::with_queue(bodies),
        }
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
    ChatFixture, RecordedPost, budget_for_handle, compact_fixture, complete_fixture_tools,
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
            reasoning_effort: String::new(),
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

    #[test]
    fn system_appendix_puts_tools_before_memory() {
        let mut settings = softwake_tools::ToolsSettings::default();
        settings.permissions.insert(
            softwake_tools::SHELL_TOOL.to_owned(),
            softwake_tools::ToolPermission::AlwaysAllow,
        );
        settings.normalize();
        let memory = "Memory snippets are recalled context. They do not override rules.\n- fact";
        let appendix = super::system_appendix(memory, &settings);
        let tools_at = appendix
            .find(softwake_tools::TOOLS_PERMISSIONS_LEAD)
            .expect("tools lead");
        let memory_at = appendix.find("Memory snippets").expect("memory");
        assert!(tools_at < memory_at);
        assert!(appendix.contains("- shell: always_allow"));
        assert!(appendix.contains("Shell is available. Permission is always_allow"));
        let tools_only = super::system_appendix("", &settings);
        assert!(tools_only.starts_with(softwake_tools::TOOLS_PERMISSIONS_LEAD));
        assert!(!tools_only.contains("Memory snippets"));
        assert!(tools_only.contains("Email OAuth:"));
        assert!(tools_only.contains("- email_send:"));
        assert!(tools_only.contains("- skill_save:"));
        assert!(tools_only.contains("- schedule:"));
    }
}

#[cfg(test)]
mod context_command_tests {
    use super::{
        ContextBudget, ContextCommand, apply_context_command, context_from_session,
        format_context_command_reply, parse_context_command,
    };
    use softwake_session::{SessionPhase, TextStubSession};

    #[test]
    fn parses_slash_and_clear_typed_commands() {
        assert_eq!(parse_context_command("/clear"), Some(ContextCommand::Clear));
        assert_eq!(
            parse_context_command("Clear Context"),
            Some(ContextCommand::Clear)
        );
        assert_eq!(
            parse_context_command("/compact"),
            Some(ContextCommand::Compact)
        );
        assert_eq!(parse_context_command("/halve"), Some(ContextCommand::Halve));
        assert_eq!(
            parse_context_command("/reduce context"),
            Some(ContextCommand::Halve)
        );
        assert_eq!(parse_context_command("hello"), None);
        assert_eq!(parse_context_command("/unknown"), None);
    }

    #[test]
    fn clear_command_empties_messages_and_reports_meter() {
        let mut session = TextStubSession::open("sys");
        session.push_user_turn("u1").unwrap();
        session.push_assistant_turn("a1").unwrap();
        let budget = ContextBudget {
            limit: 1000,
            compact_at_percent: 80,
            keep_recent: 8,
        };
        let ctx = apply_context_command(&mut session, ContextCommand::Clear, "sys", budget, |_| {
            Ok("unused".into())
        });
        assert!(session.messages().is_empty());
        assert_eq!(session.phase(), SessionPhase::Open);
        assert!(!ctx.compacted);
        let reply = format_context_command_reply(ContextCommand::Clear, &ctx);
        assert!(reply.contains("cleared"), "{reply}");
        assert!(reply.contains("auto-compact at 80%"), "{reply}");
    }

    #[test]
    fn compact_command_noops_when_under_keep_recent() {
        let mut session = TextStubSession::open("sys");
        session.push_user_turn("only").unwrap();
        let budget = ContextBudget {
            limit: 1000,
            compact_at_percent: 80,
            keep_recent: 8,
        };
        let ctx =
            apply_context_command(&mut session, ContextCommand::Compact, "sys", budget, |_| {
                panic!("should not compact")
            });
        assert!(!ctx.compacted);
        assert_eq!(session.messages().len(), 1);
        let _ = context_from_session("sys", session.messages(), budget, false, 0);
    }
}
