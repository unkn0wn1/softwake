//! OpenAI-compatible / xAI chat-completions tool schemas for Softwake tools.
//!
//! Deny is omitted. Ask and Always allow are advertised. Argument JSON maps onto
//! the positional `Vec<String>` Hands already uses. See ADR-0025.

use serde_json::{Value, json};

use crate::settings::{ToolPermission, ToolsSettings};
use crate::{
    CALENDAR_GET_TOOL, CALENDAR_LIST_TOOL, DRIVE_GET_TOOL, DRIVE_LIST_TOOL, DRIVE_SEARCH_TOOL,
    ECHO_TOOL, EMAIL_GET_TOOL, EMAIL_LIST_TOOL, EMAIL_SEARCH_TOOL, EMAIL_SEND_TOOL, NOTIFY_TOOL,
    SCHEDULE_TOOL, SHELL_TOOL, SKILL_GET_TOOL, SKILL_LIST_TOOL, SKILL_SAVE_TOOL, ToolRegistry,
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
                "body": { "type": "string", "description": "Body text." }
            },
            "required": ["to", "subject", "body"]
        }),
        EMAIL_LIST_TOOL => json!({
            "type": "object",
            "properties": {
                "max_results": {
                    "type": "integer",
                    "description": "Max messages to return (default 10, max 50)."
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
                }
            },
            "required": ["query"]
        }),
        EMAIL_GET_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Message id from email_list or email_search." }
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
                }
            }
        }),
        CALENDAR_GET_TOOL => json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Event id from calendar_list." }
            },
            "required": ["id"]
        }),
        DRIVE_LIST_TOOL => json!({
            "type": "object",
            "properties": {
                "max_results": {
                    "type": "integer",
                    "description": "Max files (default 20, max 50). Scope-limited to drive.file / AppFolder."
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
                "when": {
                    "type": "string",
                    "description": "When expression (time, datetime, or cron)."
                },
                "text": {
                    "type": "string",
                    "description": "Reminder message text."
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
            Ok(vec![to, subject, body])
        }
        EMAIL_LIST_TOOL | DRIVE_LIST_TOOL => {
            let mut args = Vec::new();
            if let Some(max) = int_field(obj, "max_results") {
                args.push(max);
            }
            Ok(args)
        }
        EMAIL_SEARCH_TOOL => {
            let query =
                string_field(obj, "query").ok_or_else(|| "email_search needs query".to_owned())?;
            let mut args = vec![query];
            if let Some(max) = int_field(obj, "max_results") {
                args.push(max);
            }
            Ok(args)
        }
        EMAIL_GET_TOOL => {
            let id = string_field(obj, "id").ok_or_else(|| "email_get needs id".to_owned())?;
            Ok(vec![id])
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
            Ok(args)
        }
        CALENDAR_GET_TOOL => {
            let id = string_field(obj, "id").ok_or_else(|| "calendar_get needs id".to_owned())?;
            Ok(vec![id])
        }
        DRIVE_SEARCH_TOOL => {
            let query =
                string_field(obj, "query").ok_or_else(|| "drive_search needs query".to_owned())?;
            let mut args = vec![query];
            if let Some(max) = int_field(obj, "max_results") {
                args.push(max);
            }
            Ok(args)
        }
        DRIVE_GET_TOOL => {
            let id = string_field(obj, "id").ok_or_else(|| "drive_get needs id".to_owned())?;
            let mut args = vec![id];
            if obj.get("read_text").and_then(Value::as_bool) == Some(true) {
                args.push("text".to_owned());
            }
            Ok(args)
        }
        SKILL_LIST_TOOL => Ok(Vec::new()),
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
        SCHEDULE_TOOL => schedule_args_from_object(obj),
        other => Err(format!("unknown tool for API args: {other}")),
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
        ECHO_TOOL, EMAIL_SEND_TOOL, NOTIFY_TOOL, SCHEDULE_TOOL, SHELL_TOOL, SKILL_SAVE_TOOL,
        ToolRegistry,
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
}
