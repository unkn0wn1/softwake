//! MCP discovery cache, `OpenAI` advertise merge, and invoke (ADR-0031).

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde_json::{Value, json};
use softwake_providers::{open_store, resolve_secrets_file};
use softwake_tools::{
    McpFile, McpTransport, ToolPermission, ToolsSettings, is_mcp_tool_name, load_mcp,
    mcp_tool_name, resolve_mcp_file, split_mcp_tool_name,
};

use crate::mcp_client::{self, McpToolInfo};

#[derive(Debug, Clone)]
struct CachedTool {
    server_id: String,
    info: McpToolInfo,
    advertised_name: String,
    group_permission: ToolPermission,
}

#[derive(Debug, Default)]
struct McpCache {
    tools: Vec<CachedTool>,
    notes: Vec<String>,
}

fn cache() -> &'static Mutex<McpCache> {
    static CACHE: OnceLock<Mutex<McpCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(McpCache::default()))
}

/// Rediscover tools for every enabled stdio MCP server.
pub(crate) fn rediscover() -> String {
    let path = match resolve_mcp_file() {
        Ok(path) => path,
        Err(error) => {
            let msg = format!("mcp rediscover: {error}");
            if let Ok(mut guard) = cache().lock() {
                guard.tools.clear();
                guard.notes = vec![msg.clone()];
            }
            return msg;
        }
    };
    let file = load_mcp(&path).unwrap_or_default();
    let secrets = load_mcp_secrets();
    let mut tools = Vec::new();
    let mut notes = Vec::new();
    for server in &file.servers {
        if !server.enabled {
            notes.push(format!("mcp `{}`: disabled", server.id));
            continue;
        }
        if server.permission == ToolPermission::Deny {
            notes.push(format!("mcp `{}`: group deny", server.id));
            continue;
        }
        match server.transport {
            McpTransport::Stdio => {
                let Some(command) = server
                    .command
                    .as_deref()
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                else {
                    notes.push(format!("mcp `{}`: missing command", server.id));
                    continue;
                };
                let auth = secrets.get(&server.id).map(String::as_str);
                match mcp_client::list_tools_stdio(command, &server.args, &server.env, auth) {
                    Ok(listed) => {
                        notes.push(format!("mcp `{}`: {} tool(s)", server.id, listed.len()));
                        for info in listed {
                            let advertised_name = mcp_tool_name(&server.id, &info.name);
                            tools.push(CachedTool {
                                server_id: server.id.clone(),
                                info,
                                advertised_name,
                                group_permission: server.permission,
                            });
                        }
                    }
                    Err(error) => {
                        notes.push(format!("mcp `{}`: {error}", server.id));
                    }
                }
            }
            McpTransport::Url => {
                notes.push(format!(
                    "mcp `{}`: url transport stored; stdio invoke only in v1",
                    server.id
                ));
            }
        }
    }
    let summary = if notes.is_empty() {
        "mcp: no servers configured".to_owned()
    } else {
        notes.join("; ")
    };
    if let Ok(mut guard) = cache().lock() {
        guard.tools = tools;
        guard.notes = notes;
    }
    summary
}

fn load_mcp_secrets() -> BTreeMap<String, String> {
    let Ok(path) = resolve_secrets_file() else {
        return BTreeMap::new();
    };
    let Ok(store) = open_store(&path) else {
        return BTreeMap::new();
    };
    store.load().map(|bag| bag.mcp_secrets).unwrap_or_default()
}

fn load_mcp_file() -> McpFile {
    resolve_mcp_file()
        .ok()
        .and_then(|path| load_mcp(&path).ok())
        .unwrap_or_default()
}

/// Effective permission for an advertised MCP tool name.
#[must_use]
pub(crate) fn mcp_permission(settings: &ToolsSettings, advertised_name: &str) -> ToolPermission {
    if let Some(stored) = settings.permissions.get(advertised_name).copied() {
        return stored;
    }
    let Ok(guard) = cache().lock() else {
        return ToolPermission::Deny;
    };
    guard
        .tools
        .iter()
        .find(|tool| tool.advertised_name == advertised_name)
        .map_or(ToolPermission::Deny, |tool| tool.group_permission)
}

/// Append non-deny MCP tools to an `OpenAI` tools array.
pub(crate) fn append_mcp_chat_tools(settings: &ToolsSettings, tools: &mut Vec<Value>) {
    let Ok(guard) = cache().lock() else {
        return;
    };
    for tool in &guard.tools {
        if mcp_permission(settings, &tool.advertised_name) == ToolPermission::Deny {
            continue;
        }
        let description = if tool.info.description.trim().is_empty() {
            format!("MCP tool {} on server {}", tool.info.name, tool.server_id)
        } else {
            tool.info.description.clone()
        };
        tools.push(json!({
            "type": "function",
            "function": {
                "name": tool.advertised_name,
                "description": description,
                "parameters": tool.info.input_schema,
            }
        }));
    }
}

/// Extra appendix lines for MCP.
#[must_use]
pub(crate) fn mcp_appendix_lines(settings: &ToolsSettings) -> String {
    let Ok(guard) = cache().lock() else {
        return String::new();
    };
    if guard.tools.is_empty() && guard.notes.is_empty() {
        return "MCP: no discovered tools (Settings → MCP, then /refresh).".to_owned();
    }
    let mut out = String::from("MCP tools (group or per-tool Tools Settings):");
    for tool in &guard.tools {
        let mode = mcp_permission(settings, &tool.advertised_name);
        out.push('\n');
        out.push_str("- ");
        out.push_str(&tool.advertised_name);
        out.push_str(": ");
        out.push_str(mode.as_str());
        out.push_str(" (server ");
        out.push_str(&tool.server_id);
        out.push(')');
    }
    if !guard.notes.is_empty() {
        out.push('\n');
        out.push_str("MCP rediscover: ");
        out.push_str(&guard.notes.join("; "));
    }
    out
}

/// Whether this advertised name is in the discovery cache.
#[must_use]
pub(crate) fn is_known_mcp_tool(name: &str) -> bool {
    if !is_mcp_tool_name(name) {
        return false;
    }
    let Ok(guard) = cache().lock() else {
        return false;
    };
    guard.tools.iter().any(|tool| tool.advertised_name == name)
}

/// Invoke a discovered MCP tool by advertised name.
pub(crate) fn invoke_mcp_tool(name: &str, args_json: &str) -> Result<String, String> {
    let file = load_mcp_file();
    let ids: Vec<String> = file.servers.iter().map(|s| s.id.clone()).collect();
    let (server_id, tool_name) =
        split_mcp_tool_name(name, &ids).ok_or_else(|| format!("unknown MCP tool name: {name}"))?;
    let server = file
        .servers
        .iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP server `{server_id}` not configured"))?;
    if !server.enabled {
        return Err(format!("MCP server `{server_id}` is disabled"));
    }
    if server.transport != McpTransport::Stdio {
        return Err(format!(
            "MCP server `{server_id}` is not stdio (url invoke not in v1)"
        ));
    }
    let command = server
        .command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| format!("MCP server `{server_id}` missing command"))?;
    let arguments: Value = if args_json.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args_json)
            .map_err(|_| format!("{name} arguments were not valid JSON"))?
    };
    let arguments = match arguments {
        Value::Object(_) => arguments,
        other => json!({ "value": other }),
    };
    let secrets = load_mcp_secrets();
    let auth = secrets.get(&server_id).map(String::as_str);
    mcp_client::call_tool_stdio(
        command,
        &server.args,
        &server.env,
        auth,
        &tool_name,
        &arguments,
    )
    .map_err(|error| error.to_string())
}

/// Map Hands positional args back to JSON object for MCP call.
#[must_use]
pub(crate) fn mcp_args_json_from_positional(args: &[String]) -> String {
    if args.len() == 1 {
        let trimmed = args[0].trim();
        if trimmed.starts_with('{') {
            return trimmed.to_owned();
        }
    }
    if args.is_empty() {
        return "{}".to_owned();
    }
    json!({ "args": args }).to_string()
}
