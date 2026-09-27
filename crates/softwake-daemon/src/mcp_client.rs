//! Minimal MCP stdio client (JSON-RPC + Content-Length framing).
//!
//! Used to `initialize`, `tools/list`, and `tools/call`. See ADR-0031.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

/// One discovered MCP tool.
#[derive(Debug, Clone)]
pub(crate) struct McpToolInfo {
    /// MCP tool name (server-local).
    pub name: String,
    /// Description for `OpenAI` advertise.
    pub description: String,
    /// JSON Schema object for arguments.
    pub input_schema: Value,
}

/// Failures talking to an MCP server.
#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum McpClientError {
    #[error("{0}")]
    Message(String),
}

impl McpClientError {
    fn msg(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}

/// Spawn a stdio MCP server, list tools, shut it down.
pub(crate) fn list_tools_stdio(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    auth: Option<&str>,
) -> Result<Vec<McpToolInfo>, McpClientError> {
    let mut session = McpSession::spawn(command, args, env, auth)?;
    let tools = session.list_tools()?;
    session.shutdown();
    Ok(tools)
}

/// Spawn, call one tool, shut down.
pub(crate) fn call_tool_stdio(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    auth: Option<&str>,
    tool_name: &str,
    arguments: &Value,
) -> Result<String, McpClientError> {
    let mut session = McpSession::spawn(command, args, env, auth)?;
    let detail = session.call_tool(tool_name, arguments)?;
    session.shutdown();
    Ok(detail)
}

struct McpSession {
    child: Child,
    next_id: u64,
}

impl McpSession {
    fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        auth: Option<&str>,
    ) -> Result<Self, McpClientError> {
        if command.trim().is_empty() {
            return Err(McpClientError::msg("MCP stdio command is empty"));
        }
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in env {
            cmd.env(key, value);
        }
        if let Some(secret) = auth {
            // Common convention; servers that need it read Authorization.
            cmd.env("MCP_AUTH", secret);
            cmd.env("AUTHORIZATION", secret);
        }
        let child = cmd
            .spawn()
            .map_err(|error| McpClientError::msg(format!("spawn MCP `{command}`: {error}")))?;
        let mut session = Self { child, next_id: 1 };
        session.initialize()?;
        Ok(session)
    }

    fn initialize(&mut self) -> Result<(), McpClientError> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "softwake", "version": "0.1.0" }
        });
        let _ = self.request("initialize", &params)?;
        self.notify("notifications/initialized", &json!({}))?;
        Ok(())
    }

    fn list_tools(&mut self) -> Result<Vec<McpToolInfo>, McpClientError> {
        let result = self.request("tools/list", &json!({}))?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for tool in tools {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned();
            if name.is_empty() {
                continue;
            }
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let input_schema = tool
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            out.push(McpToolInfo {
                name,
                description,
                input_schema,
            });
        }
        Ok(out)
    }

    fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<String, McpClientError> {
        let result = self.request(
            "tools/call",
            &json!({
                "name": name,
                "arguments": arguments,
            }),
        )?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(McpClientError::msg(format_mcp_content(&result)));
        }
        Ok(format_mcp_content(&result))
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, McpClientError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&message)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if std::time::Instant::now() > deadline {
                return Err(McpClientError::msg(format!(
                    "MCP `{method}` timed out waiting for response"
                )));
            }
            let value = self.read_message()?;
            let matches_id = match value.get("id") {
                Some(Value::Number(n)) => {
                    n.as_u64() == Some(id) || n.as_i64() == i64::try_from(id).ok()
                }
                _ => false,
            };
            if matches_id {
                if let Some(error) = value.get("error") {
                    return Err(McpClientError::msg(format!(
                        "MCP `{method}` error: {error}"
                    )));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
            // Skip notifications / unmatched ids.
        }
    }

    fn notify(&mut self, method: &str, params: &Value) -> Result<(), McpClientError> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&message)
    }

    fn write_message(&mut self, value: &Value) -> Result<(), McpClientError> {
        let body = serde_json::to_vec(value)
            .map_err(|error| McpClientError::msg(format!("encode MCP: {error}")))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| McpClientError::msg("MCP child stdin closed"))?;
        stdin
            .write_all(header.as_bytes())
            .and_then(|()| stdin.write_all(&body))
            .and_then(|()| stdin.flush())
            .map_err(|error| McpClientError::msg(format!("write MCP: {error}")))
    }

    fn read_message(&mut self) -> Result<Value, McpClientError> {
        let stdout = self
            .child
            .stdout
            .as_mut()
            .ok_or_else(|| McpClientError::msg("MCP child stdout closed"))?;
        let mut headers = Vec::new();
        let mut line = Vec::new();
        loop {
            line.clear();
            read_line(stdout, &mut line)?;
            if line.is_empty() || line == b"\r" {
                break;
            }
            headers.extend_from_slice(&line);
            headers.push(b'\n');
        }
        let header_text = String::from_utf8_lossy(&headers);
        let mut content_length = None;
        for raw in header_text.lines() {
            let line = raw.trim().trim_end_matches('\r');
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                content_length = rest.trim().parse::<usize>().ok();
            }
        }
        let len = content_length
            .ok_or_else(|| McpClientError::msg("MCP response missing Content-Length"))?;
        if len > 4 * 1024 * 1024 {
            return Err(McpClientError::msg("MCP response too large"));
        }
        let mut body = vec![0_u8; len];
        stdout
            .read_exact(&mut body)
            .map_err(|error| McpClientError::msg(format!("read MCP body: {error}")))?;
        serde_json::from_slice(&body)
            .map_err(|error| McpClientError::msg(format!("decode MCP JSON: {error}")))
    }

    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for McpSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn read_line(reader: &mut impl Read, buf: &mut Vec<u8>) -> Result<(), McpClientError> {
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => {
                return Err(McpClientError::msg("MCP stdout EOF"));
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    return Ok(());
                }
                buf.push(byte[0]);
                if buf.len() > 16 * 1024 {
                    return Err(McpClientError::msg("MCP header line too long"));
                }
            }
            Err(error) => {
                return Err(McpClientError::msg(format!("read MCP header: {error}")));
            }
        }
    }
}

fn format_mcp_content(result: &Value) -> String {
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        let mut parts = Vec::new();
        for item in content {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                parts.push(text.to_owned());
            } else {
                parts.push(item.to_string());
            }
        }
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    result.to_string()
}

#[cfg(test)]
mod tests {
    use super::format_mcp_content;
    use serde_json::json;

    #[test]
    fn formats_text_content_blocks() {
        let result = json!({
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "text", "text": "world" }
            ]
        });
        assert_eq!(format_mcp_content(&result), "hello\nworld");
    }
}
