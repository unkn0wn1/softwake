//! Non-secret MCP server Settings (`mcp.json`).
//!
//! Auth secrets stay in the provider secret bag (`mcp_secrets`). See ADR-0031.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::{ToolPermission, parse_tool_permission};

/// File name under the Softwake config directory.
pub const MCP_FILE_NAME: &str = "mcp.json";

/// Largest mcp.json this backend will read.
pub const MAX_MCP_BYTES: usize = 256 * 1024;

const DOCUMENT_VERSION: u32 = 1;

/// How Softwake reaches an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// Spawn `command` + `args` (JSON-RPC over stdio, Content-Length framing).
    #[default]
    Stdio,
    /// Remote URL (stored in v1; invoke path is best-effort / future).
    Url,
}

impl McpTransport {
    /// Stable spelling for Settings.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Url => "url",
        }
    }
}

impl std::fmt::Display for McpTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One configured MCP server (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Stable id used in tool names (`mcp_<id>_<tool>`) and secret bag keys.
    pub id: String,
    /// Operator-facing label.
    #[serde(default)]
    pub label: String,
    /// Master switch.
    #[serde(default)]
    pub enabled: bool,
    /// stdio or url.
    #[serde(default)]
    pub transport: McpTransport,
    /// Executable for stdio (ignored for url).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Args for stdio.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Extra env for the child (non-secret). Secrets use the bag.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Remote URL when transport is url.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Optional HTTP header name for bag secret (default Authorization).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header_name: Option<String>,
    /// Group default permission for this server's tools.
    #[serde(default = "default_server_permission")]
    pub permission: ToolPermission,
}

fn default_server_permission() -> ToolPermission {
    ToolPermission::Ask
}

impl McpServerConfig {
    /// Display label or id.
    #[must_use]
    pub fn display_label(&self) -> &str {
        let trimmed = self.label.trim();
        if trimmed.is_empty() {
            self.id.as_str()
        } else {
            trimmed
        }
    }
}

/// On-disk MCP document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpFile {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Configured servers.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

fn one() -> u32 {
    DOCUMENT_VERSION
}

impl Default for McpFile {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            servers: Vec::new(),
        }
    }
}

impl McpFile {
    /// Drop empty ids, clamp version, ensure unique ids (last wins).
    pub fn normalize(&mut self) {
        self.version = DOCUMENT_VERSION;
        let mut by_id = BTreeMap::new();
        for server in self.servers.drain(..) {
            let id = sanitize_mcp_id(&server.id);
            if id.is_empty() {
                continue;
            }
            let mut server = server;
            server.id = id;
            by_id.insert(server.id.clone(), server);
        }
        self.servers = by_id.into_values().collect();
    }
}

/// Errors loading or saving mcp.json.
#[derive(Debug, thiserror::Error)]
pub enum McpConfigError {
    /// Neither XDG config nor HOME is set.
    #[error("cannot resolve Softwake config directory: XDG_CONFIG_HOME and HOME are unset")]
    NoConfigDir,
    /// Path was empty.
    #[error("mcp Settings path is empty")]
    EmptyPath,
    /// File is larger than [`MAX_MCP_BYTES`].
    #[error("mcp Settings file is too large ({len} > {max}): {}", path.display())]
    TooLarge {
        /// Path that was rejected.
        path: PathBuf,
        /// Observed length.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// JSON did not parse.
    #[error("mcp Settings JSON is invalid: {message}")]
    InvalidJson {
        /// serde message.
        message: String,
    },
    /// IO failure.
    #[error("mcp Settings IO error: {message}")]
    Io {
        /// OS message.
        message: String,
    },
}

impl From<io::Error> for McpConfigError {
    fn from(error: io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }
}

/// Resolve `$XDG_CONFIG_HOME/softwake/mcp.json` (else `~/.config/softwake/mcp.json`).
///
/// # Errors
///
/// When config root cannot be resolved.
pub fn resolve_mcp_file() -> Result<PathBuf, McpConfigError> {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let root = softwake_soul::resolve_config_dir(xdg.as_deref(), home.as_deref())
        .map_err(|_| McpConfigError::NoConfigDir)?;
    Ok(root.join(MCP_FILE_NAME))
}

/// Load mcp.json. Missing file → default empty.
///
/// # Errors
///
/// IO / parse / size.
pub fn load_mcp(path: &Path) -> Result<McpFile, McpConfigError> {
    if path.as_os_str().is_empty() {
        return Err(McpConfigError::EmptyPath);
    }
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > MAX_MCP_BYTES {
                return Err(McpConfigError::TooLarge {
                    path: path.to_owned(),
                    len: bytes.len(),
                    max: MAX_MCP_BYTES,
                });
            }
            if bytes.iter().all(u8::is_ascii_whitespace) {
                return Ok(McpFile::default());
            }
            let mut file: McpFile =
                serde_json::from_slice(&bytes).map_err(|error| McpConfigError::InvalidJson {
                    message: error.to_string(),
                })?;
            file.normalize();
            Ok(file)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(McpFile::default()),
        Err(error) => Err(error.into()),
    }
}

/// Save mcp.json (0600 on Unix).
///
/// # Errors
///
/// IO.
pub fn save_mcp(path: &Path, file: &McpFile) -> Result<(), McpConfigError> {
    if path.as_os_str().is_empty() {
        return Err(McpConfigError::EmptyPath);
    }
    let mut normalized = file.clone();
    normalized.normalize();
    if let Some(parent) = path.parent() {
        #[cfg(unix)]
        {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        #[cfg(not(unix))]
        {
            fs::create_dir_all(parent)?;
        }
    }
    let json = serde_json::to_vec_pretty(&normalized).map_err(|error| McpConfigError::Io {
        message: error.to_string(),
    })?;
    let mut opts = OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut out = opts.open(path)?;
    out.write_all(&json)?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Sanitize a server or tool fragment for `mcp_<id>_<tool>` names.
#[must_use]
pub fn sanitize_mcp_id(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch.is_whitespace() {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_owned()
}

/// Build the `OpenAI` function name for one MCP tool.
#[must_use]
pub fn mcp_tool_name(server_id: &str, tool_name: &str) -> String {
    format!(
        "mcp_{}_{}",
        sanitize_mcp_id(server_id),
        sanitize_mcp_id(tool_name)
    )
}

/// Parse `mcp_<server>_<tool…>` into (`server_id`, `tool_name`). Tool may contain `_`.
///
/// Server id is matched against known servers (longest id first).
#[must_use]
pub fn split_mcp_tool_name(name: &str, server_ids: &[String]) -> Option<(String, String)> {
    let rest = name.strip_prefix("mcp_")?;
    let mut ids: Vec<&String> = server_ids.iter().collect();
    ids.sort_by_key(|id| std::cmp::Reverse(id.len()));
    for id in ids {
        let prefix = format!("{id}_");
        if let Some(tool) = rest.strip_prefix(&prefix) {
            if !tool.is_empty() {
                return Some((id.clone(), tool.to_owned()));
            }
        }
    }
    None
}

/// Whether `name` looks like an MCP-bridged tool (`mcp_…`).
#[must_use]
pub fn is_mcp_tool_name(name: &str) -> bool {
    name.starts_with("mcp_") && name.len() > 4
}

/// Parse a permission spelling for MCP server group defaults.
///
/// # Errors
///
/// Unknown spelling.
pub fn parse_mcp_permission(value: &str) -> Result<ToolPermission, String> {
    parse_tool_permission(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_and_name_round_trip_fragments() {
        assert_eq!(sanitize_mcp_id("File System"), "file_system");
        assert_eq!(
            mcp_tool_name("filesystem", "read_file"),
            "mcp_filesystem_read_file"
        );
    }

    #[test]
    fn split_prefers_longest_server_id() {
        let ids = vec!["fs".into(), "fs_extra".into()];
        assert_eq!(
            split_mcp_tool_name("mcp_fs_extra_list", &ids),
            Some(("fs_extra".into(), "list".into()))
        );
        assert_eq!(
            split_mcp_tool_name("mcp_fs_list", &ids),
            Some(("fs".into(), "list".into()))
        );
    }

    #[test]
    fn load_missing_is_default() {
        let dir = std::env::temp_dir().join(format!(
            "softwake-mcp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let path = dir.join("mcp.json");
        let file = load_mcp(&path).expect("missing");
        assert!(file.servers.is_empty());
        let mut file = McpFile::default();
        file.servers.push(McpServerConfig {
            id: "Demo!".into(),
            label: "Demo".into(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: Some("true".into()),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            auth_header_name: None,
            permission: ToolPermission::Ask,
        });
        save_mcp(&path, &file).expect("save");
        let loaded = load_mcp(&path).expect("load");
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers[0].id, "demo");
        let _ = fs::remove_dir_all(dir);
    }
}
