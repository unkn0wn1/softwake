//! Bounded companion `agent_task` LLM turn (ADR-0043).
//!
//! Same family as laptop `messenger_ask_oneshot` / ADR-0036: soul + tools
//! appendix + advertised tools, max 6 rounds. Ask → pending text (never silent
//! Always-allow). OAuth tools run from the opt-in mirror vault when present (ADR-0045). TTS is never used on the node.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use softwake_tools::{
    ConnectedAccount, ECHO_TOOL, EMAIL_GET_TOOL, EMAIL_LIST_TOOL, EMAIL_SEARCH_TOOL,
    EMAIL_SEND_TOOL, EmailOauthStatus, NOTIFY_TOOL, SCHEDULE_TOOL, SHELL_TOOL, SKILL_GET_TOOL,
    SKILL_LIST_TOOL, ScheduleEntry, ToolPermission, ToolsSettings, advertise_chat_tools,
    agent_task_user_prompt, fire_agent_notify_line, format_shell_output, run_shell,
    tool_args_from_json, tools_permissions_appendix,
};

use crate::state::NodeState;
use crate::telegram;

/// Max assistant→tools→continue rounds (matches daemon `MAX_TOOL_ROUNDS`).
/// Final round omits tools and soft-finalizes (ADR-0047).
pub const MAX_TOOL_ROUNDS: usize = 6;

const DEFAULT_MODEL: &str = "grok-4-fast-non-reasoning";
const API_URL: &str = "https://api.x.ai/v1/chat/completions";

/// Run one scheduled agent task for `profile_id`; returns delivery summary.
pub fn run_schedule_agent_task(
    state: &NodeState,
    profile_id: &str,
    entry: &ScheduleEntry,
) -> String {
    let prompt = agent_task_user_prompt(entry);
    let summary = match run_agent_turn(state, profile_id, &prompt) {
        Ok(text) => text,
        Err(err) => format!("agent task failed: {err}"),
    };
    let notify = fire_agent_notify_line(&entry.title, &summary);
    telegram::maybe_fanout_timer(state, profile_id, &summary);
    notify
}

/// Core turn used by schedules (and tests).
pub fn run_agent_turn(
    state: &NodeState,
    profile_id: &str,
    user_text: &str,
) -> Result<String, String> {
    let Some(api_key) = state.xai_api_key() else {
        return Ok(
            "Softwake companion: no LLM key (set SOFTWAKE_NODE_XAI_API_KEY or mirror xAI via vault/llm)."
                .to_owned(),
        );
    };

    let soul_dir = state.profile_soul_dir(profile_id);
    let pack = softwake_soul::try_load(&soul_dir)
        .map_err(|e| format!("soul pack not mirrored for profile `{profile_id}` ({e})"))?;
    let system_soul = pack.render_instructions();

    let tools_settings = state.load_tools_settings();
    let oauth_doc = state.load_oauth_document();
    let email_status = email_oauth_status_from_doc(&oauth_doc);
    let tools_appendix = tools_permissions_appendix(&tools_settings, &email_status);
    let skills_appendix = skills_catalog_appendix(&state.skills_dir());
    let companion_note = if oauth_doc.has_usable_connection() {
        "You are Softwake's companion node running a scheduled agent task while the laptop Softwake is away. Ask-mode tools cannot be approved here — if you need one, say so and stop. Mirrored OAuth accounts are available for email/calendar/Drive; pass account= to pick a mailbox. Be concise."
    } else {
        "You are Softwake's companion node running a scheduled agent task while the laptop Softwake is away. Ask-mode tools cannot be approved here — if you need one, say so and stop. OAuth (email/calendar/drive) is unavailable until Mirror OAuth tokens is enabled in Settings → Remote Agent. Be concise."
    };

    let system = format!(
        "{system_soul}\n\n{tools_appendix}\n\n{skills_appendix}\n\n# Companion\n\n{companion_note}"
    );

    let tools = advertise_chat_tools(&tools_settings);
    let mut messages: Vec<Value> = vec![json!({"role": "user", "content": user_text})];

    for round in 0..MAX_TOOL_ROUNDS {
        let last = round + 1 == MAX_TOOL_ROUNDS;
        let round_tools: &[Value] = if last { &[] } else { &tools };
        let turn = chat_turn(&api_key, &system, &messages, round_tools)?;
        match turn {
            Turn::Message(text) => return Ok(text),
            Turn::ToolCalls { content, calls } => {
                if last {
                    return Ok(soft_finalize_messages(&messages, content.as_deref()));
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": content,
                    "tool_calls": calls.iter().map(|c| json!({
                        "id": c.id,
                        "type": "function",
                        "function": {"name": c.name, "arguments": c.arguments}
                    })).collect::<Vec<_>>(),
                }));
                for call in &calls {
                    let perm = tools_settings.permission(&call.name);
                    if perm == ToolPermission::Ask {
                        let pending = format!(
                            "Pending confirmation on laptop Softwake: tool `{}` needs Ask approval (companion never silently Always-allows).",
                            call.name
                        );
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call.id,
                            "content": pending.clone(),
                        }));
                        return Ok(pending);
                    }
                    if perm == ToolPermission::Deny {
                        messages.push(json!({
                            "role": "tool",
                            "tool_call_id": call.id,
                            "content": format!("tool `{}` is denied", call.name),
                        }));
                        continue;
                    }
                    let args = parse_args(&call.name, &call.arguments);
                    let result =
                        invoke_companion(state, profile_id, &call.name, &args, &tools_settings);
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": result,
                    }));
                }
            }
        }
    }
    Ok(soft_finalize_messages(&messages, None))
}

/// Best-effort companion reply when the last round has no clean Message (ADR-0047).
fn soft_finalize_messages(messages: &[Value], last_content: Option<&str>) -> String {
    if let Some(text) = last_content.map(str::trim).filter(|s| !s.is_empty()) {
        return text.to_owned();
    }
    for message in messages.iter().rev() {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(text) = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return text.to_owned();
        }
    }
    let mut snippets = Vec::new();
    for message in messages.iter().rev() {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if let Some(text) = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let clipped: String = text.chars().take(240).collect();
            snippets.push(clipped);
            if snippets.len() >= 3 {
                break;
            }
        }
    }
    snippets.reverse();
    if snippets.is_empty() {
        return "Tool budget reached before a final reply.".to_owned();
    }
    format!(
        "Tool budget reached before a full reply. From tool results:
{}",
        snippets.join(
            "
"
        )
    )
}

struct ToolCall {
    id: String,
    name: String,
    arguments: String,
}

enum Turn {
    Message(String),
    ToolCalls {
        content: Option<String>,
        calls: Vec<ToolCall>,
    },
}

fn chat_turn(
    api_key: &str,
    system: &str,
    messages: &[Value],
    tools: &[Value],
) -> Result<Turn, String> {
    let model = std::env::var("SOFTWAKE_NODE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned());
    let mut body = json!({
        "model": model,
        "messages": {
            // placeholder replaced below
        },
        "max_tokens": 1200,
        "temperature": 0.4,
    });
    let mut wire = vec![json!({"role": "system", "content": system})];
    wire.extend(messages.iter().cloned());
    body["messages"] = Value::Array(wire);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_read(Duration::from_secs(120))
        .timeout_connect(Duration::from_secs(15))
        .build();
    let response = agent
        .post(API_URL)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| e.to_string())?;
    let raw = response.into_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let message = v
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| "empty model choices".to_owned())?;

    if let Some(calls) = message.get("tool_calls").and_then(|c| c.as_array()) {
        if !calls.is_empty() {
            let content = message
                .get("content")
                .and_then(|c| c.as_str())
                .map(str::to_owned)
                .filter(|s| !s.trim().is_empty());
            let mut parsed = Vec::new();
            for call in calls {
                let id = call
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("call")
                    .to_owned();
                let name = call
                    .pointer("/function/name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_owned();
                let arguments = call
                    .pointer("/function/arguments")
                    .and_then(|x| x.as_str())
                    .unwrap_or("{}")
                    .to_owned();
                if name.is_empty() {
                    continue;
                }
                parsed.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            if !parsed.is_empty() {
                return Ok(Turn::ToolCalls {
                    content,
                    calls: parsed,
                });
            }
        }
    }

    let text = message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err("empty model content".into());
    }
    Ok(Turn::Message(text))
}

fn email_oauth_status_from_doc(doc: &softwake_providers::OauthMirrorDocument) -> EmailOauthStatus {
    let google_active = doc
        .active_google_connection_id
        .as_ref()
        .and_then(|id| doc.google_connections.iter().find(|c| &c.id == id))
        .or_else(|| doc.google_connections.first());
    let microsoft_active = doc
        .active_microsoft_connection_id
        .as_ref()
        .and_then(|id| doc.microsoft_connections.iter().find(|c| &c.id == id))
        .or_else(|| doc.microsoft_connections.first());
    EmailOauthStatus {
        google_connected: !doc.google_connections.is_empty(),
        google_email: google_active.and_then(|c| c.account_email.clone()),
        google_accounts: doc
            .google_connections
            .iter()
            .map(|row| ConnectedAccount {
                id: row.id.clone(),
                email: row.account_email.clone(),
                active: google_active.is_some_and(|c| c.id == row.id),
            })
            .collect(),
        microsoft_connected: !doc.microsoft_connections.is_empty(),
        microsoft_email: microsoft_active.and_then(|c| c.account_email.clone()),
        microsoft_accounts: doc
            .microsoft_connections
            .iter()
            .map(|row| ConnectedAccount {
                id: row.id.clone(),
                email: row.account_email.clone(),
                active: microsoft_active.is_some_and(|c| c.id == row.id),
            })
            .collect(),
    }
}

fn parse_args(name: &str, arguments: &str) -> Vec<String> {
    tool_args_from_json(name, arguments).unwrap_or_default()
}

#[allow(clippy::too_many_lines)]
fn invoke_companion(
    state: &NodeState,
    profile_id: &str,
    name: &str,
    args: &[String],
    _settings: &ToolsSettings,
) -> String {
    // OAuth family — opt-in mirror vault (ADR-0045).
    if matches!(
        name,
        EMAIL_SEND_TOOL
            | EMAIL_LIST_TOOL
            | EMAIL_SEARCH_TOOL
            | EMAIL_GET_TOOL
            | "calendar_list"
            | "calendar_get"
            | "calendar_create"
            | "calendar_update"
            | "calendar_delete"
            | "drive_list"
            | "drive_search"
            | "drive_get"
    ) {
        let doc = state.load_oauth_document();
        if !doc.has_usable_connection() {
            return crate::oauth_tools::oauth_enable_hint(name);
        }
        #[cfg(not(feature = "live-http"))]
        {
            let _ = (state, args);
            return crate::oauth_tools::LIVE_REQUIRED.to_owned();
        }
        #[cfg(feature = "live-http")]
        {
            let transport =
                softwake_providers::live::LiveTransport::bounded(Duration::from_secs(30));
            return crate::oauth_tools::run_oauth_tool(state, name, args, &transport);
        }
    }

    match name {
        ECHO_TOOL => {
            if args.is_empty() || args[0].is_empty() {
                "pong".to_owned()
            } else {
                args.join(" ")
            }
        }
        NOTIFY_TOOL => {
            let message = args.first().map_or("", String::as_str).trim();
            if message.is_empty() {
                return "notify needs a message".into();
            }
            state.push_outbox(crate::state::OutboxItem {
                id: format!("ob-notify-{}", NodeState::now_ms()),
                profile_id: profile_id.to_owned(),
                kind: "notify".into(),
                schedule_id: None,
                ts_ms: NodeState::now_ms(),
                summary: format!("notify: {message}"),
                lease_id: None,
            });
            format!("notified: {message}")
        }
        SHELL_TOOL => {
            let command = args.first().map_or("", String::as_str).trim();
            if command.is_empty() {
                return "shell needs a command".into();
            }
            match run_shell(command) {
                Ok(out) => format_shell_output(&out),
                Err(err) => format!("shell failed: {err}"),
            }
        }
        SKILL_LIST_TOOL => {
            let dir = state.skills_dir();
            match softwake_skills::list_skills(&dir) {
                Ok(list) if list.is_empty() => "no skills mirrored".into(),
                Ok(list) => list
                    .iter()
                    .map(|s| format!("{} — {}", s.id, s.title))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(err) => format!("skill_list failed: {err}"),
            }
        }
        SKILL_GET_TOOL => {
            let id = args.first().map_or("", String::as_str).trim();
            if id.is_empty() {
                return "skill_get needs an id".into();
            }
            match softwake_skills::load_skill(&state.skills_dir(), id) {
                Ok(skill) => format!(
                    "# {}\n\n## Procedure\n\n{}\n\n## Pitfalls\n\n{}\n\n## Verify\n\n{}",
                    skill.title, skill.procedure, skill.pitfalls, skill.verify
                ),
                Err(err) => format!("skill_get failed: {err}"),
            }
        }
        SCHEDULE_TOOL => {
            if !args.is_empty() && args[0] != "list" {
                return "companion schedule tool is list-only while laptop is away".into();
            }
            state.format_schedule_list(profile_id)
        }
        "softwake_status" => {
            let now = NodeState::now_ms();
            let presence = state.effective_presence(now);
            format!(
                "companion softwake-node; laptop presence={}; profile={profile_id}",
                presence.state.as_str()
            )
        }
        "skill_save"
        | "remember"
        | "forget"
        | "softwake_sleep"
        | "softwake_hibernate"
        | "softwake_resume"
        | "softwake_new_session"
        | "softwake_refresh"
        | "softwake_set_model"
        | "softwake_set_voice"
        | "softwake_set_profile"
        | "softwake_set_reasoning"
        | "softwake_list_models"
        | "softwake_list_voices"
        | "softwake_list_profiles"
        | "softwake_list_reasoning" => {
            format!("tool `{name}` is not available on the companion node")
        }
        other if other.starts_with("mcp_") => {
            format!("MCP tool `{other}` is not available on the companion node")
        }
        other => format!("tool `{other}` is not executable on the companion node"),
    }
}

fn skills_catalog_appendix(dir: &Path) -> String {
    let Ok(list) = softwake_skills::list_skills(dir) else {
        return String::new();
    };
    if list.is_empty() {
        return String::new();
    }
    let mut out = String::from("# Skills catalog (mirrored)\n\n");
    for skill in list.iter().take(softwake_skills::MAX_CATALOG_ENTRIES) {
        out.push_str("- ");
        out.push_str(&skill.id);
        out.push_str(": ");
        out.push_str(&skill.title);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use softwake_tools::{ECHO_TOOL, EMAIL_LIST_TOOL, ToolsSettings};

    struct Tmp {
        path: std::path::PathBuf,
        state: NodeState,
    }
    impl Tmp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "sw-node-agent-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let _ = std::fs::create_dir_all(&path);
            let state = NodeState::open(path.clone());
            Self { path, state }
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn oauth_tools_refuse_when_vault_empty() {
        let t = Tmp::new();
        let msg = invoke_companion(
            &t.state,
            "default",
            EMAIL_LIST_TOOL,
            &[],
            &ToolsSettings::default(),
        );
        assert!(msg.contains("OAuth"), "{msg}");
        assert!(
            msg.contains("Mirror OAuth") || msg.contains("mirrored"),
            "{msg}"
        );
    }

    #[test]
    fn echo_always_allow_runs() {
        let t = Tmp::new();
        let msg = invoke_companion(
            &t.state,
            "default",
            ECHO_TOOL,
            &["hello".into()],
            &ToolsSettings::default(),
        );
        assert_eq!(msg, "hello");
    }

    #[test]
    fn no_key_without_soul_is_honest() {
        if std::env::var("SOFTWAKE_NODE_XAI_API_KEY")
            .ok()
            .as_ref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            return;
        }
        let t = Tmp::new();
        let err = run_agent_turn(&t.state, "default", "hi").expect("ok stub or err");
        // Without key: Ok(stub). With key but no soul: would be Err — env key absent → stub.
        assert!(err.contains("no LLM key") || err.contains("soul"), "{err}");
    }
}
