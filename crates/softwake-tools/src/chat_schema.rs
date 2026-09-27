//! OpenAI-compatible / xAI chat-completions tool schemas for Softwake tools.
//!
//! Deny is omitted. Ask and Always allow are advertised. Argument JSON maps onto
//! the positional `Vec<String>` Hands already uses. See ADR-0025.

use serde_json::{Value, json};

use crate::settings::{ToolPermission, ToolsSettings};
use crate::{
    CALENDAR_CREATE_TOOL, CALENDAR_DELETE_TOOL, CALENDAR_GET_TOOL, CALENDAR_LIST_TOOL,
    CALENDAR_UPDATE_TOOL, DRIVE_GET_TOOL, DRIVE_LIST_TOOL, DRIVE_SEARCH_TOOL, ECHO_TOOL,
    EMAIL_GET_TOOL, EMAIL_LIST_TOOL, EMAIL_SEARCH_TOOL, EMAIL_SEND_TOOL, FORGET_TOOL, NOTIFY_TOOL,
    REMEMBER_TOOL, SCHEDULE_TOOL, SHELL_TOOL, SKILL_GET_TOOL, SKILL_LIST_TOOL, SKILL_SAVE_TOOL,
    SOFTWAKE_HIBERNATE_TOOL, SOFTWAKE_LIST_MODELS_TOOL, SOFTWAKE_LIST_PROFILES_TOOL,
    SOFTWAKE_LIST_REASONING_TOOL, SOFTWAKE_LIST_VOICES_TOOL, SOFTWAKE_NEW_SESSION_TOOL,
    SOFTWAKE_REFRESH_TOOL, SOFTWAKE_RESUME_TOOL, SOFTWAKE_SET_MODEL_TOOL,
    SOFTWAKE_SET_PROFILE_TOOL, SOFTWAKE_SET_REASONING_TOOL, SOFTWAKE_SET_VOICE_TOOL,
    SOFTWAKE_SLEEP_TOOL, SOFTWAKE_STATUS_TOOL, ToolRegistry,
};

/// Build the `tools` array for one chat/completions request.
///
/// Omits every registered tool whose operator permission is [`ToolPermission::Deny`].
/// Empty when every tool is deny (caller should omit the `tools` field).
#[must_use]
pub fn advertise_chat_tools(settings: &ToolsSettings) -> Vec<Value> {
    ToolRegistry::phase2()
        .entries()
        .iter()
        .filter(|meta| settings.permission(meta.name) != ToolPermission::Deny)
        .map(|meta| function_tool(meta.name, meta.description, &parameters_for(meta.name)))
        .collect()
}

fn function_tool(name: &str, description: &str, parameters: &Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}

#[allow(clippy::too_many_lines)]
fn parameters_for(name: &str) -> Value {
    match name {
        SHELL_TOOL => json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run. Glossary aliases (for example a host name) expand before spawn."
                }
            },
            "required": ["command"]
        }),
        ECHO_TOOL => json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Text to echo. Omit or empty for the pong probe."
                }
            }
        }),
        NOTIFY_TOOL => json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Notification line to append to the in-memory sink."
                }
            },
            "required": ["message"]
        }),
        EMAIL_SEND_TOOL => json!({
            "type": "object",
            "properties": {
                "to": { "type": "string", "description": "Recipient." },
                "subject": { "type": "string", "description": "Subject line." },
                "body": { "type": "string", "description": "Body text." },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["to", "subject", "body"]
        }),
        EMAIL_LIST_TOOL => json!({
            "type": "object",
            "properties": {
                "max_results": {
                    "type": "integer",
                    "description": "Max messages to return (default 10, max 50)."
                },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            }
        }),
        EMAIL_SEARCH_TOOL => json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Gmail search query (e.g. from:ada newer_than:7d) or Graph search text."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max messages (default 10, max 50)."
                },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["query"]
        }),
        EMAIL_GET_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Message id from email_list or email_search." },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["id"]
        }),
        CALENDAR_LIST_TOOL => json!({
            "type": "object",
            "properties": {
                "days": {
                    "type": "integer",
                    "description": "Upcoming window in days (default 7, max 90)."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max events (default 20, max 50)."
                },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            }
        }),
        CALENDAR_GET_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Event id from calendar_list." },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["id"]
        }),
        CALENDAR_CREATE_TOOL => json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Event title." },
                "start": { "type": "string", "description": "Start time, RFC3339 (for example 2026-09-28T09:00:00Z)." },
                "end": { "type": "string", "description": "End time, RFC3339." },
                "location": { "type": "string", "description": "Optional location." },
                "description": { "type": "string", "description": "Optional description." },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["title", "start", "end"]
        }),
        CALENDAR_UPDATE_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Event id from calendar_list or calendar_get." },
                "title": { "type": "string", "description": "Replacement title. Omit to leave unchanged." },
                "start": { "type": "string", "description": "Replacement start, RFC3339. Omit to leave unchanged." },
                "end": { "type": "string", "description": "Replacement end, RFC3339. Omit to leave unchanged." },
                "location": { "type": "string", "description": "Replacement location. Empty string clears it." },
                "description": { "type": "string", "description": "Replacement description. Empty string clears it." },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["id"]
        }),
        CALENDAR_DELETE_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Event id to delete." },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["id"]
        }),
        DRIVE_LIST_TOOL => json!({
            "type": "object",
            "properties": {
                "max_results": {
                    "type": "integer",
                    "description": "Max files (default 20, max 50). Visible under drive.readonly / Files.Read."
                },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            }
        }),
        DRIVE_SEARCH_TOOL => json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "File name or Drive query fragment." },
                "max_results": {
                    "type": "integer",
                    "description": "Max files (default 20, max 50)."
                },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["query"]
        }),
        DRIVE_GET_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "File id from drive_list or drive_search." },
                "read_text": {
                    "type": "boolean",
                    "description": "When true, include cheap text body for text/* or Google Docs."
                },
                "account": {
                    "type": "string",
                    "description": "Optional connected account: connection id or email substring. Omit to use the only account, or the active Google account when several are connected."
                }
            },
            "required": ["id"]
        }),
        SKILL_LIST_TOOL => json!({
            "type": "object",
            "properties": {
                "_": {
                    "type": "string",
                    "description": "Unused. skill_list takes no arguments."
                }
            }
        }),
        SKILL_GET_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Skill id from skill_list." }
            },
            "required": ["id"]
        }),
        SKILL_SAVE_TOOL => json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Skill title." },
                "procedure": { "type": "string", "description": "Procedure section." },
                "pitfalls": { "type": "string", "description": "Pitfalls section." },
                "verify": { "type": "string", "description": "Verify section." }
            },
            "required": ["title", "procedure", "pitfalls", "verify"]
        }),
        REMEMBER_TOOL => json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Fact or snippet to store in long-term memory (unchanged)."
                }
            },
            "required": ["text"]
        }),
        FORGET_TOOL => json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Snippet id from a prior remember (decimal). Omit when all is true."
                },
                "all": {
                    "type": "boolean",
                    "description": "When true, forget every snippet. Ask-gated; prefer over wiping by hand."
                }
            }
        }),
        SCHEDULE_TOOL => json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "One of create, edit, delete, list."
                },
                "kind": {
                    "type": "string",
                    "description": "For create/edit: once, daily, or cron."
                },
                "fire": {
                    "type": "string",
                    "description": "notify (default fixed reminder) or agent_task (run prompt on fire)."
                },
                "when": {
                    "type": "string",
                    "description": "When expression (time, datetime, or cron)."
                },
                "text": {
                    "type": "string",
                    "description": "Reminder message text, or agent prompt when fire=agent_task."
                },
                "id": {
                    "type": "string",
                    "description": "Schedule id for edit/delete."
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Optional enabled flag for edit."
                }
            },
            "required": ["action"]
        }),
        SOFTWAKE_SET_MODEL_TOOL => json!({
            "type": "object",
            "properties": {
                "which": {
                    "type": "string",
                    "description": "ai for chat model, or voice for STT/voice model."
                },
                "id": { "type": "string", "description": "Model id from softwake_list_models / Test catalog." }
            },
            "required": ["which", "id"]
        }),
        SOFTWAKE_SET_VOICE_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "TTS voice id from softwake_list_voices." }
            },
            "required": ["id"]
        }),
        SOFTWAKE_SET_PROFILE_TOOL => json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Profile name or id." }
            },
            "required": ["name"]
        }),
        SOFTWAKE_SET_REASONING_TOOL => json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "description": "Reasoning effort: low, medium, high, xhigh, or default (omit / provider default)."
                }
            },
            "required": ["mode"]
        }),
        _ => json!({"type": "object", "properties": {}}),
    }
}

/// Map one tool-call `arguments` JSON object onto Hands argv.
///
/// # Errors
///
/// Returns a short operator sentence when JSON is not an object or required
/// fields are missing. Does not run the tool.
#[allow(clippy::too_many_lines)]
pub fn tool_args_from_json(name: &str, arguments: &str) -> Result<Vec<String>, String> {
    let trimmed = arguments.trim();
    let value: Value = if trimmed.is_empty() {
        json!({})
    } else {
        serde_json::from_str(trimmed)
            .map_err(|_| format!("{name} tool arguments were not valid JSON"))?
    };
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{name} tool arguments must be a JSON object"))?;

    match name {
        SHELL_TOOL => {
            let command =
                string_field(obj, "command").ok_or_else(|| "shell needs command".to_owned())?;
            if command.trim().is_empty() {
                return Err("shell needs command".to_owned());
            }
            Ok(vec![command])
        }
        ECHO_TOOL => {
            if let Some(text) = string_field(obj, "text") {
                if text.is_empty() {
                    Ok(Vec::new())
                } else {
                    Ok(vec![text])
                }
            } else {
                Ok(Vec::new())
            }
        }
        NOTIFY_TOOL => {
            let message =
                string_field(obj, "message").ok_or_else(|| "notify needs message".to_owned())?;
            Ok(vec![message])
        }
        EMAIL_SEND_TOOL => {
            let to = string_field(obj, "to").ok_or_else(|| "email_send needs to".to_owned())?;
            let subject = string_field(obj, "subject")
                .ok_or_else(|| "email_send needs subject".to_owned())?;
            let body =
                string_field(obj, "body").ok_or_else(|| "email_send needs body".to_owned())?;
            let mut args = vec![to, subject, body];
            push_account(&mut args, obj);
            Ok(args)
        }
        EMAIL_LIST_TOOL | DRIVE_LIST_TOOL => {
            let mut args = Vec::new();
            if let Some(max) = int_field(obj, "max_results") {
                args.push(max);
            }
            push_account(&mut args, obj);
            Ok(args)
        }
        EMAIL_SEARCH_TOOL => {
            let query =
                string_field(obj, "query").ok_or_else(|| "email_search needs query".to_owned())?;
            let mut args = vec![query];
            if let Some(max) = int_field(obj, "max_results") {
                args.push(max);
            }
            push_account(&mut args, obj);
            Ok(args)
        }
        EMAIL_GET_TOOL => {
            let id = string_field(obj, "id").ok_or_else(|| "email_get needs id".to_owned())?;
            let mut args = vec![id];
            push_account(&mut args, obj);
            Ok(args)
        }
        CALENDAR_LIST_TOOL => {
            let mut args = Vec::new();
            if let Some(days) = int_field(obj, "days") {
                args.push(days);
                if let Some(max) = int_field(obj, "max_results") {
                    args.push(max);
                }
            } else if let Some(max) = int_field(obj, "max_results") {
                args.push("7".to_owned());
                args.push(max);
            }
            push_account(&mut args, obj);
            Ok(args)
        }
        CALENDAR_GET_TOOL => {
            let id = string_field(obj, "id").ok_or_else(|| "calendar_get needs id".to_owned())?;
            let mut args = vec![id];
            push_account(&mut args, obj);
            Ok(args)
        }
        CALENDAR_CREATE_TOOL => {
            let title = string_field(obj, "title")
                .ok_or_else(|| "calendar_create needs title".to_owned())?;
            let start = string_field(obj, "start")
                .ok_or_else(|| "calendar_create needs start".to_owned())?;
            let end =
                string_field(obj, "end").ok_or_else(|| "calendar_create needs end".to_owned())?;
            if title.trim().is_empty() || start.trim().is_empty() || end.trim().is_empty() {
                return Err("calendar_create needs title, start, and end".to_owned());
            }
            let mut args = vec![title, start, end];
            if let Some(location) = string_field(obj, "location") {
                args.push(format!("location={location}"));
            }
            if let Some(description) = string_field(obj, "description") {
                args.push(format!("description={description}"));
            }
            push_account(&mut args, obj);
            Ok(args)
        }
        CALENDAR_UPDATE_TOOL => {
            let id =
                string_field(obj, "id").ok_or_else(|| "calendar_update needs id".to_owned())?;
            if id.trim().is_empty() {
                return Err("calendar_update needs id".to_owned());
            }
            let mut args = vec![id];
            let mut any = false;
            for (key, label) in [
                ("title", "title"),
                ("start", "start"),
                ("end", "end"),
                ("location", "location"),
                ("description", "description"),
            ] {
                if obj.contains_key(key) {
                    let Some(value) = string_field(obj, key) else {
                        return Err(format!("calendar_update {label} must be a string"));
                    };
                    args.push(format!("{label}={value}"));
                    any = true;
                }
            }
            if !any {
                return Err(
                    "calendar_update needs at least one of title, start, end, location, or description"
                        .to_owned(),
                );
            }
            push_account(&mut args, obj);
            Ok(args)
        }
        CALENDAR_DELETE_TOOL => {
            let id =
                string_field(obj, "id").ok_or_else(|| "calendar_delete needs id".to_owned())?;
            if id.trim().is_empty() {
                return Err("calendar_delete needs id".to_owned());
            }
            let mut args = vec![id];
            push_account(&mut args, obj);
            Ok(args)
        }
        DRIVE_SEARCH_TOOL => {
            let query =
                string_field(obj, "query").ok_or_else(|| "drive_search needs query".to_owned())?;
            let mut args = vec![query];
            if let Some(max) = int_field(obj, "max_results") {
                args.push(max);
            }
            push_account(&mut args, obj);
            Ok(args)
        }
        DRIVE_GET_TOOL => {
            let id = string_field(obj, "id").ok_or_else(|| "drive_get needs id".to_owned())?;
            let mut args = vec![id];
            if obj.get("read_text").and_then(Value::as_bool) == Some(true) {
                args.push("text".to_owned());
            }
            push_account(&mut args, obj);
            Ok(args)
        }
        SKILL_LIST_TOOL
        | SOFTWAKE_STATUS_TOOL
        | SOFTWAKE_LIST_MODELS_TOOL
        | SOFTWAKE_LIST_VOICES_TOOL
        | SOFTWAKE_LIST_PROFILES_TOOL
        | SOFTWAKE_LIST_REASONING_TOOL
        | SOFTWAKE_SLEEP_TOOL
        | SOFTWAKE_HIBERNATE_TOOL
        | SOFTWAKE_RESUME_TOOL
        | SOFTWAKE_NEW_SESSION_TOOL
        | SOFTWAKE_REFRESH_TOOL => Ok(Vec::new()),
        SKILL_GET_TOOL => {
            let id = string_field(obj, "id").ok_or_else(|| "skill_get needs id".to_owned())?;
            Ok(vec![id])
        }
        SKILL_SAVE_TOOL => {
            let title =
                string_field(obj, "title").ok_or_else(|| "skill_save needs title".to_owned())?;
            let procedure = string_field(obj, "procedure").unwrap_or_default();
            let pitfalls = string_field(obj, "pitfalls").unwrap_or_default();
            let verify = string_field(obj, "verify").unwrap_or_default();
            if title.trim().is_empty() {
                return Err("skill_save needs title".to_owned());
            }
            Ok(vec![title, procedure, pitfalls, verify])
        }
        REMEMBER_TOOL => {
            let text = string_field(obj, "text").ok_or_else(|| "remember needs text".to_owned())?;
            if text.trim().is_empty() {
                return Err("remember needs text".to_owned());
            }
            Ok(vec![text])
        }
        FORGET_TOOL => {
            let all = obj
                .get("all")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if all {
                return Ok(vec!["all".to_owned()]);
            }
            let id = string_field(obj, "id").ok_or_else(|| "forget needs id or all".to_owned())?;
            if id.trim().is_empty() || id.trim() == "0" {
                return Err("forget needs id or all".to_owned());
            }
            // allow literal "all" via id field too
            Ok(vec![id.trim().to_owned()])
        }
        SCHEDULE_TOOL => schedule_args_from_object(obj),
        SOFTWAKE_SET_MODEL_TOOL => {
            let which = string_field(obj, "which")
                .ok_or_else(|| "softwake_set_model needs which (ai|voice)".to_owned())?
                .to_ascii_lowercase();
            if which != "ai" && which != "voice" {
                return Err("softwake_set_model which must be ai or voice".to_owned());
            }
            let id =
                string_field(obj, "id").ok_or_else(|| "softwake_set_model needs id".to_owned())?;
            if id.trim().is_empty() {
                return Err("softwake_set_model needs id".to_owned());
            }
            Ok(vec![which, id])
        }
        SOFTWAKE_SET_VOICE_TOOL => {
            let id =
                string_field(obj, "id").ok_or_else(|| "softwake_set_voice needs id".to_owned())?;
            if id.trim().is_empty() {
                return Err("softwake_set_voice needs id".to_owned());
            }
            Ok(vec![id])
        }
        SOFTWAKE_SET_PROFILE_TOOL => {
            let name = string_field(obj, "name")
                .ok_or_else(|| "softwake_set_profile needs name".to_owned())?;
            if name.trim().is_empty() {
                return Err("softwake_set_profile needs name".to_owned());
            }
            Ok(vec![name])
        }
        SOFTWAKE_SET_REASONING_TOOL => {
            let mode = string_field(obj, "mode")
                .ok_or_else(|| "softwake_set_reasoning needs mode".to_owned())?;
            if mode.trim().is_empty() {
                return Err("softwake_set_reasoning needs mode".to_owned());
            }
            Ok(vec![mode])
        }
        other => Err(format!("unknown tool for API args: {other}")),
    }
}

fn push_account(args: &mut Vec<String>, obj: &serde_json::Map<String, Value>) {
    let Some(account) = string_field(obj, "account") else {
        return;
    };
    let trimmed = account.trim();
    if !trimmed.is_empty() {
        args.push(format!("account={trimmed}"));
    }
}

fn string_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

fn int_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(|value| match value {
        Value::Number(number) => number.as_u64().map(|n| n.to_string()),
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.parse::<u32>().is_ok() {
                Some(trimmed.to_owned())
            } else {
                None
            }
        }
        _ => None,
    })
}

fn schedule_args_from_object(obj: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
    let action = string_field(obj, "action")
        .ok_or_else(|| "schedule needs action".to_owned())?
        .to_ascii_lowercase();
    let mut args = vec![action.clone()];
    match action.as_str() {
        "list" => Ok(args),
        "delete" => {
            let id =
                string_field(obj, "id").ok_or_else(|| "schedule delete needs id".to_owned())?;
            args.push(id);
            Ok(args)
        }
        "create" => {
            let kind =
                string_field(obj, "kind").ok_or_else(|| "schedule create needs kind".to_owned())?;
            let when =
                string_field(obj, "when").ok_or_else(|| "schedule create needs when".to_owned())?;
            let text =
                string_field(obj, "text").ok_or_else(|| "schedule create needs text".to_owned())?;
            if let Some(fire) = string_field(obj, "fire") {
                let fire = fire.to_ascii_lowercase();
                if fire == "agent_task" || fire == "agent" || fire == "task" {
                    args.push("agent_task".to_owned());
                } else if fire != "notify" && fire != "reminder" {
                    return Err(format!(
                        "unknown schedule fire: {fire} (want notify|agent_task)"
                    ));
                }
            }
            args.push(kind);
            args.push(when);
            args.push(text);
            Ok(args)
        }
        "edit" => {
            let id = string_field(obj, "id").ok_or_else(|| "schedule edit needs id".to_owned())?;
            args.push(id);
            if let Some(kind) = string_field(obj, "kind") {
                args.push("kind".to_owned());
                args.push(kind);
            }
            if let Some(fire) = string_field(obj, "fire") {
                args.push("action".to_owned());
                args.push(fire);
            }
            if let Some(when) = string_field(obj, "when") {
                args.push("when".to_owned());
                args.push(when);
            }
            if let Some(enabled) = obj.get("enabled") {
                args.push("enabled".to_owned());
                match enabled {
                    Value::Bool(flag) => args.push(flag.to_string()),
                    Value::String(text) => args.push(text.clone()),
                    _ => {
                        return Err("schedule edit enabled must be bool or string".to_owned());
                    }
                }
            }
            if let Some(text) = string_field(obj, "text") {
                args.push("message".to_owned());
                args.push(text);
            }
            Ok(args)
        }
        other => Err(format!(
            "unknown schedule action: {other} (want create|edit|delete|list)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{advertise_chat_tools, tool_args_from_json};
    use crate::settings::{ToolPermission, ToolsSettings};
    use crate::{
        CALENDAR_CREATE_TOOL, CALENDAR_DELETE_TOOL, CALENDAR_LIST_TOOL, CALENDAR_UPDATE_TOOL,
        DRIVE_GET_TOOL, ECHO_TOOL, EMAIL_LIST_TOOL, EMAIL_SEND_TOOL, FORGET_TOOL, NOTIFY_TOOL,
        REMEMBER_TOOL, SCHEDULE_TOOL, SHELL_TOOL, SKILL_SAVE_TOOL, ToolRegistry,
    };

    #[test]
    fn advertise_omits_deny_and_includes_ask_and_always_allow() {
        let mut settings = ToolsSettings::default();
        assert_eq!(settings.permission(SHELL_TOOL), ToolPermission::Deny);
        let denied = advertise_chat_tools(&settings);
        assert!(
            denied
                .iter()
                .all(|tool| tool["function"]["name"] != SHELL_TOOL)
        );
        assert!(
            denied
                .iter()
                .any(|tool| tool["function"]["name"] == ECHO_TOOL)
        );

        settings
            .permissions
            .insert(SHELL_TOOL.to_owned(), ToolPermission::AlwaysAllow);
        settings
            .permissions
            .insert(NOTIFY_TOOL.to_owned(), ToolPermission::Ask);
        settings
            .permissions
            .insert(EMAIL_SEND_TOOL.to_owned(), ToolPermission::Deny);
        settings.normalize();

        let tools = advertise_chat_tools(&settings);
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&SHELL_TOOL));
        assert!(names.contains(&NOTIFY_TOOL));
        assert!(names.contains(&ECHO_TOOL));
        assert!(!names.contains(&EMAIL_SEND_TOOL));

        let shell = tools
            .iter()
            .find(|tool| tool["function"]["name"] == SHELL_TOOL)
            .expect("shell");
        assert_eq!(shell["type"], "function");
        assert_eq!(
            shell["function"]["parameters"]["required"],
            serde_json::json!(["command"])
        );
    }

    #[test]
    fn advertise_covers_every_non_deny_registry_row() {
        let mut settings = ToolsSettings::default();
        for meta in ToolRegistry::phase2().entries() {
            settings
                .permissions
                .insert(meta.name.to_owned(), ToolPermission::Ask);
        }
        settings.normalize();
        let tools = advertise_chat_tools(&settings);
        assert_eq!(tools.len(), ToolRegistry::phase2().entries().len());
    }

    #[test]
    fn tool_args_from_json_maps_registered_tools() {
        assert_eq!(
            tool_args_from_json(SHELL_TOOL, r#"{"command":"free -h"}"#).expect("shell"),
            vec!["free -h".to_owned()]
        );
        assert_eq!(
            tool_args_from_json(ECHO_TOOL, r#"{"text":"hi"}"#).expect("echo"),
            vec!["hi".to_owned()]
        );
        assert!(
            tool_args_from_json(ECHO_TOOL, "{}")
                .expect("pong")
                .is_empty()
        );
        assert_eq!(
            tool_args_from_json(NOTIFY_TOOL, r#"{"message":"ping"}"#).expect("notify"),
            vec!["ping".to_owned()]
        );

        assert_eq!(
            tool_args_from_json(
                EMAIL_SEND_TOOL,
                r#"{"to":"a@b.c","subject":"s","body":"hello world"}"#
            )
            .expect("email"),
            vec!["a@b.c".to_owned(), "s".to_owned(), "hello world".to_owned()]
        );
        assert_eq!(
            tool_args_from_json(
                EMAIL_SEND_TOOL,
                r#"{"to":"a@b.c","subject":"s","body":"hello world","account":"ada@example.com"}"#
            )
            .expect("email account"),
            vec![
                "a@b.c".to_owned(),
                "s".to_owned(),
                "hello world".to_owned(),
                "account=ada@example.com".to_owned()
            ]
        );
        assert_eq!(
            tool_args_from_json(EMAIL_LIST_TOOL, r#"{"account":"ada@example.com"}"#)
                .expect("list account"),
            vec!["account=ada@example.com".to_owned()]
        );
        assert_eq!(
            tool_args_from_json(CALENDAR_LIST_TOOL, r#"{"days":3,"account":"id-9"}"#)
                .expect("calendar account"),
            vec!["3".to_owned(), "account=id-9".to_owned()]
        );
        assert_eq!(
            tool_args_from_json(
                DRIVE_GET_TOOL,
                r#"{"id":"f1","read_text":true,"account":"bob@example.com"}"#
            )
            .expect("drive account"),
            vec![
                "f1".to_owned(),
                "text".to_owned(),
                "account=bob@example.com".to_owned()
            ]
        );
        assert_eq!(
            tool_args_from_json(
                SKILL_SAVE_TOOL,
                r#"{"title":"T","procedure":"P","pitfalls":"X","verify":"V"}"#
            )
            .expect("skill"),
            vec![
                "T".to_owned(),
                "P".to_owned(),
                "X".to_owned(),
                "V".to_owned()
            ]
        );
        assert_eq!(
            tool_args_from_json(
                SCHEDULE_TOOL,
                r#"{"action":"create","kind":"once","when":"2026-09-27 15:00","text":"tea"}"#
            )
            .expect("schedule"),
            vec![
                "create".to_owned(),
                "once".to_owned(),
                "2026-09-27 15:00".to_owned(),
                "tea".to_owned()
            ]
        );
        assert!(tool_args_from_json(SHELL_TOOL, r#"{"command":""}"#).is_err());
        assert!(tool_args_from_json(SHELL_TOOL, "not-json").is_err());
    }

    #[test]
    fn calendar_write_tool_args_include_account() {
        assert_eq!(
            tool_args_from_json(
                CALENDAR_CREATE_TOOL,
                r#"{"title":"Stand-up","start":"2026-09-28T09:00:00Z","end":"2026-09-28T09:15:00Z","location":"Zoom","description":"daily","account":"ada@example.com"}"#
            )
            .expect("create"),
            vec![
                "Stand-up".to_owned(),
                "2026-09-28T09:00:00Z".to_owned(),
                "2026-09-28T09:15:00Z".to_owned(),
                "location=Zoom".to_owned(),
                "description=daily".to_owned(),
                "account=ada@example.com".to_owned(),
            ]
        );
        assert_eq!(
            tool_args_from_json(
                CALENDAR_UPDATE_TOOL,
                r#"{"id":"evt-1","title":"Moved","account":"id-9"}"#
            )
            .expect("update"),
            vec![
                "evt-1".to_owned(),
                "title=Moved".to_owned(),
                "account=id-9".to_owned(),
            ]
        );
        assert_eq!(
            tool_args_from_json(
                CALENDAR_DELETE_TOOL,
                r#"{"id":"evt-1","account":"ada@example.com"}"#
            )
            .expect("delete"),
            vec!["evt-1".to_owned(), "account=ada@example.com".to_owned()]
        );
        assert!(tool_args_from_json(CALENDAR_UPDATE_TOOL, r#"{"id":"evt-1"}"#).is_err());
    }
    #[test]
    fn tool_args_from_json_maps_remember_and_forget() {
        assert_eq!(
            tool_args_from_json(REMEMBER_TOOL, r#"{"text":"garage code"}"#).expect("remember"),
            vec!["garage code".to_owned()]
        );
        assert_eq!(
            tool_args_from_json(FORGET_TOOL, r#"{"id":"3"}"#).expect("forget id"),
            vec!["3".to_owned()]
        );
        assert_eq!(
            tool_args_from_json(FORGET_TOOL, r#"{"all":true}"#).expect("forget all"),
            vec!["all".to_owned()]
        );
        assert!(tool_args_from_json(REMEMBER_TOOL, r#"{"text":"  "}"#).is_err());
        assert!(tool_args_from_json(FORGET_TOOL, "{}").is_err());
    }
}
