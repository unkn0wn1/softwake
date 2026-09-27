//! Settings commands for MCP servers (ADR-0031).

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use serde::{Deserialize, Serialize};
use softwake_providers::{SecretBag, SecretStore, update_bag};
use softwake_tools::{
    McpFile, McpServerConfig, McpTransport, ToolPermission, load_mcp, parse_mcp_permission,
    resolve_mcp_file, sanitize_mcp_id, save_mcp,
};
use std::collections::BTreeMap;

/// One server chip in the MCP subnav.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRow {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

/// Snapshot for Settings → MCP.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSnapshot {
    pub servers: Vec<McpServerRow>,
    pub selected_id: String,
    pub storage_backend: String,
    pub storage_message: String,
    pub has_secret: bool,
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub transport: String,
    pub command: String,
    pub args_text: String,
    pub url: String,
    pub auth_header_name: String,
    pub permission: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSaveArgs {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub transport: String,
    pub command: String,
    pub args_text: String,
    pub url: String,
    pub auth_header_name: String,
    pub permission: String,
    /// Empty = leave secret unchanged.
    pub secret: Option<String>,
    pub clear_secret: bool,
}

fn open_secrets() -> Result<Box<dyn SecretStore + Send>, String> {
    let path = softwake_providers::resolve_secrets_file().map_err(|e| e.to_string())?;
    softwake_providers::open_store(&path).map_err(|e| e.to_string())
}

fn parse_transport(value: &str) -> Result<McpTransport, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "stdio" => Ok(McpTransport::Stdio),
        "url" => Ok(McpTransport::Url),
        other => Err(format!("unknown MCP transport: {other}")),
    }
}

fn parse_args_text(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .collect()
}

fn snapshot_for(
    file: &McpFile,
    selected_id: &str,
    bag: &SecretBag,
    backend: &str,
    message: &str,
) -> McpSnapshot {
    let servers: Vec<McpServerRow> = file
        .servers
        .iter()
        .map(|s| McpServerRow {
            id: s.id.clone(),
            label: s.display_label().to_owned(),
            enabled: s.enabled,
        })
        .collect();
    let selected = if file.servers.iter().any(|s| s.id == selected_id) {
        selected_id.to_owned()
    } else {
        file.servers
            .first()
            .map(|s| s.id.clone())
            .unwrap_or_default()
    };
    let server = file.servers.iter().find(|s| s.id == selected);
    let has_secret = server.is_some_and(|s| {
        bag.mcp_secrets
            .get(&s.id)
            .is_some_and(|v| !v.trim().is_empty())
    });
    McpSnapshot {
        servers,
        selected_id: selected.clone(),
        storage_backend: backend.to_owned(),
        storage_message: message.to_owned(),
        has_secret,
        id: server.map(|s| s.id.clone()).unwrap_or_default(),
        label: server.map(|s| s.label.clone()).unwrap_or_default(),
        enabled: server.is_some_and(|s| s.enabled),
        transport: server.map_or_else(|| "stdio".into(), |s| s.transport.as_str().to_owned()),
        command: server.and_then(|s| s.command.clone()).unwrap_or_default(),
        args_text: server.map(|s| s.args.join(" ")).unwrap_or_default(),
        url: server.and_then(|s| s.url.clone()).unwrap_or_default(),
        auth_header_name: server
            .and_then(|s| s.auth_header_name.clone())
            .unwrap_or_else(|| "Authorization".into()),
        permission: server.map_or_else(
            || ToolPermission::Ask.as_str().to_owned(),
            |s| s.permission.as_str().to_owned(),
        ),
    }
}

fn load_file() -> Result<(std::path::PathBuf, McpFile), String> {
    let path = resolve_mcp_file().map_err(|e| e.to_string())?;
    let file = load_mcp(&path).map_err(|e| e.to_string())?;
    Ok((path, file))
}

/// Load MCP Settings.
#[tauri::command]
pub fn mcp_snapshot(selected_id: Option<String>) -> Result<McpSnapshot, String> {
    let (_path, file) = load_file()?;
    let store = open_secrets()?;
    let bag = store.load().map_err(|e| e.to_string())?;
    let report = store.report();
    let selected = selected_id.unwrap_or_default();
    Ok(snapshot_for(
        &file,
        &selected,
        &bag,
        report.backend.as_str(),
        &report.message,
    ))
}

/// Create an empty server stub and select it.
#[tauri::command]
pub fn mcp_add_server() -> Result<McpSnapshot, String> {
    let (path, mut file) = load_file()?;
    let mut n = 1;
    let id = loop {
        let candidate = format!("server{n}");
        if !file.servers.iter().any(|s| s.id == candidate) {
            break candidate;
        }
        n += 1;
    };
    file.servers.push(McpServerConfig {
        id: id.clone(),
        label: format!("Server {n}"),
        enabled: false,
        transport: McpTransport::Stdio,
        command: None,
        args: Vec::new(),
        env: BTreeMap::new(),
        url: None,
        auth_header_name: Some("Authorization".into()),
        permission: ToolPermission::Ask,
    });
    save_mcp(&path, &file).map_err(|e| e.to_string())?;
    mcp_snapshot(Some(id))
}

/// Save one server row + optional secret.
#[tauri::command]
pub fn mcp_save(args: McpSaveArgs) -> Result<McpSnapshot, String> {
    let id = sanitize_mcp_id(&args.id);
    if id.is_empty() {
        return Err("server id required".into());
    }
    let transport = parse_transport(&args.transport)?;
    let permission = parse_mcp_permission(&args.permission)?;
    let (path, mut file) = load_file()?;
    let command = args.command.trim();
    let url = args.url.trim();
    let header = args.auth_header_name.trim();
    let row = McpServerConfig {
        id: id.clone(),
        label: args.label.trim().to_owned(),
        enabled: args.enabled,
        transport,
        command: if command.is_empty() {
            None
        } else {
            Some(command.to_owned())
        },
        args: parse_args_text(&args.args_text),
        env: BTreeMap::new(),
        url: if url.is_empty() {
            None
        } else {
            Some(url.to_owned())
        },
        auth_header_name: if header.is_empty() {
            None
        } else {
            Some(header.to_owned())
        },
        permission,
    };
    if let Some(existing) = file.servers.iter_mut().find(|s| s.id == id) {
        *existing = row;
    } else {
        file.servers.push(row);
    }
    save_mcp(&path, &file).map_err(|e| e.to_string())?;

    let store = open_secrets()?;
    if args.clear_secret {
        update_bag(store.as_ref(), |bag| {
            bag.mcp_secrets.remove(&id);
        })
        .map_err(|e| e.to_string())?;
    } else if let Some(secret) = args.secret {
        let trimmed = secret.trim().to_owned();
        if !trimmed.is_empty() {
            update_bag(store.as_ref(), |bag| {
                bag.mcp_secrets.insert(id.clone(), trimmed);
            })
            .map_err(|e| e.to_string())?;
        }
    }
    mcp_snapshot(Some(id))
}

/// Delete a server and its secret.
#[tauri::command]
pub fn mcp_delete(server_id: String) -> Result<McpSnapshot, String> {
    let id = sanitize_mcp_id(&server_id);
    let (path, mut file) = load_file()?;
    file.servers.retain(|s| s.id != id);
    save_mcp(&path, &file).map_err(|e| e.to_string())?;
    let store = open_secrets()?;
    update_bag(store.as_ref(), |bag| {
        bag.mcp_secrets.remove(&id);
    })
    .map_err(|e| e.to_string())?;
    mcp_snapshot(None)
}

/// Clear auth secret for one server.
#[tauri::command]
pub fn mcp_clear_secret(server_id: String) -> Result<McpSnapshot, String> {
    let id = sanitize_mcp_id(&server_id);
    let store = open_secrets()?;
    update_bag(store.as_ref(), |bag| {
        bag.mcp_secrets.remove(&id);
    })
    .map_err(|e| e.to_string())?;
    mcp_snapshot(Some(id))
}
