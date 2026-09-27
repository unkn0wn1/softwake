//! Authenticated local HTTP webhook wake (ADR-0038).
//!
//! Binds `127.0.0.1` only. Requires `webhook_enabled` in softwake.json and a
//! non-empty `webhook_secret` in the secret bag. `POST /v1/wake` with Bearer
//! auth wakes from sleep; hibernate returns 409 without auto-resume.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;
use std::{env, thread};

use serde_json::{Value, json};
use softwake_ipc::Event as WireEvent;
use softwake_providers::{open_store, resolve_secrets_file};
use softwake_soul::{DEFAULT_WEBHOOK_PORT, load_app_config, resolve_config_dir};

use crate::serve::Shared;

/// Max Unicode scalars accepted in an optional webhook message body.
pub(crate) const MAX_MESSAGE_CHARS: usize = 2000;

/// Reject request bodies larger than this (headers + framing still bounded by read).
const MAX_BODY_BYTES: usize = 64 * 1024;

const PATH: &str = "/v1/wake";

/// Spawn the webhook accept loop. Safe to call once from serve.
pub(crate) fn spawn(shared: Arc<Shared>) {
    let _ = thread::Builder::new()
        .name("softwake-webhook".to_owned())
        .spawn(move || serve_loop(&shared));
}

fn serve_loop(shared: &Arc<Shared>) {
    let mut bound_port: Option<u16> = None;
    let mut listener: Option<TcpListener> = None;
    loop {
        let Some((port, secret)) = ready_bind_config() else {
            listener = None;
            bound_port = None;
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        if bound_port != Some(port) {
            match TcpListener::bind(("127.0.0.1", port)) {
                Ok(next) => {
                    eprintln!("softwaked: webhook listening on 127.0.0.1:{port}");
                    listener = Some(next);
                    bound_port = Some(port);
                }
                Err(error) => {
                    eprintln!("softwaked: webhook bind 127.0.0.1:{port} failed: {error}");
                    listener = None;
                    bound_port = None;
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }
        // Re-check enable/secret each accept so ctl enable/disable/rotate applies
        // without restart (port change still rebinds above).
        let Some((_, secret_now)) = ready_bind_config() else {
            listener = None;
            bound_port = None;
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let _ = secret;
        let accepted = listener.as_ref().map(TcpListener::accept);
        match accepted {
            Some(Ok((stream, _))) => {
                let shared = Arc::clone(shared);
                let secret = secret_now;
                let _ = thread::Builder::new()
                    .name("softwake-webhook-conn".to_owned())
                    .spawn(move || handle_connection(stream, &shared, &secret));
            }
            Some(Err(error)) => {
                eprintln!("softwaked: webhook accept: {error}");
                thread::sleep(Duration::from_millis(200));
            }
            None => thread::sleep(Duration::from_secs(2)),
        }
    }
}

fn ready_bind_config() -> Option<(u16, String)> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let config_dir = resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()?;
    let app = load_app_config(&config_dir).unwrap_or_default();
    if !app.webhook_enabled {
        return None;
    }
    let port = resolve_webhook_port(app.webhook_port);
    let secret = load_webhook_secret()?;
    if secret.is_empty() {
        return None;
    }
    Some((port, secret))
}

/// Resolve bind port: env `SOFTWAKE_WEBHOOK_PORT` wins over softwake.json.
#[must_use]
pub(crate) fn resolve_webhook_port(file_port: u16) -> u16 {
    if let Ok(raw) = env::var("SOFTWAKE_WEBHOOK_PORT") {
        if let Ok(port) = raw.trim().parse::<u16>() {
            if port != 0 {
                return port;
            }
        }
    }
    if file_port == 0 {
        DEFAULT_WEBHOOK_PORT
    } else {
        file_port
    }
}

fn load_webhook_secret() -> Option<String> {
    let path = resolve_secrets_file().ok()?;
    let store = open_store(&path).ok()?;
    let bag = store.load().ok()?;
    let secret = bag.webhook_secret?;
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn handle_connection(mut stream: TcpStream, shared: &Shared, expected_secret: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Ok(request) = read_http_request(&mut stream) else {
        let _ = write_response(
            &mut stream,
            400,
            &json!({"ok":false,"detail":"bad request"}),
        );
        return;
    };
    if request.method != "POST" {
        let _ = write_response(
            &mut stream,
            405,
            &json!({"ok":false,"detail":"method not allowed"}),
        );
        return;
    }
    if request.path != PATH {
        let _ = write_response(&mut stream, 404, &json!({"ok":false,"detail":"not found"}));
        return;
    }
    let provided = extract_token(&request.headers);
    if !token_matches(provided.as_deref(), expected_secret) {
        let _ = write_response(
            &mut stream,
            401,
            &json!({"ok":false,"detail":"unauthorized"}),
        );
        return;
    }
    let message = match parse_wake_body(request.content_type.as_deref(), &request.body) {
        Ok(message) => message,
        Err(detail) => {
            let _ = write_response(&mut stream, 400, &json!({"ok":false,"detail":detail}));
            return;
        }
    };
    let (status, body, events) = {
        let mut runtime = shared
            .runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = runtime.webhook_wake(message.as_deref());
        (result.status, result.body, result.events)
    };
    for event in &events {
        shared.broadcast(event);
    }
    let _ = write_response(&mut stream, status, &body);
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    content_type: Option<String>,
    body: Vec<u8>,
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, ()> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let n = stream.read(&mut chunk).map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_BODY_BYTES + 8192 {
            return Err(());
        }
        if let Some(pos) = find_header_end(&buf) {
            header_end = pos;
            break;
        }
    }
    let header_text = std::str::from_utf8(&buf[..header_end]).map_err(|_| ())?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or(())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(())?.to_owned();
    let path = parts.next().ok_or(())?.to_owned();
    let mut headers = Vec::new();
    let mut content_length = 0_usize;
    let mut content_type = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if name == "content-length" {
            content_length = value.parse().map_err(|_| ())?;
            if content_length > MAX_BODY_BYTES {
                return Err(());
            }
        }
        if name == "content-type" {
            content_type = Some(value.clone());
        }
        headers.push((name, value));
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk).map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > MAX_BODY_BYTES {
            return Err(());
        }
    }
    body.truncate(content_length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        content_type,
        body,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn write_response(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        _ => "Error",
    };
    let payload = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()
}

/// Pull bearer token from Authorization or X-Softwake-Webhook-Token.
#[must_use]
pub(crate) fn extract_token(headers: &[(String, String)]) -> Option<String> {
    let mut bearer = None;
    let mut alias = None;
    for (name, value) in headers {
        if name == "authorization" {
            let trimmed = value.trim();
            if let Some(rest) = trimmed.strip_prefix("Bearer ") {
                bearer = Some(rest.trim().to_owned());
            } else if let Some(rest) = trimmed.strip_prefix("bearer ") {
                bearer = Some(rest.trim().to_owned());
            }
        }
        if name == "x-softwake-webhook-token" {
            alias = Some(value.trim().to_owned());
        }
    }
    bearer.or(alias).filter(|s| !s.is_empty())
}

/// Constant-time compare when lengths match.
#[must_use]
pub(crate) fn token_matches(provided: Option<&str>, expected: &str) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    let a = provided.as_bytes();
    let b = expected.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Parse optional wake message from the request body.
///
/// # Errors
///
/// Returns a short operator-facing detail string.
pub(crate) fn parse_wake_body(
    content_type: Option<&str>,
    body: &[u8],
) -> Result<Option<String>, String> {
    if body.is_empty() {
        return Ok(None);
    }
    let ct = content_type.unwrap_or("").to_ascii_lowercase();
    if ct.starts_with("application/json") || ct.is_empty() {
        let value: Value =
            serde_json::from_slice(body).map_err(|_| "invalid json body".to_owned())?;
        if value.is_null() {
            return Ok(None);
        }
        let obj = value
            .as_object()
            .ok_or_else(|| "json body must be an object".to_owned())?;
        let Some(message) = obj.get("message") else {
            return Ok(None);
        };
        let text = message
            .as_str()
            .ok_or_else(|| "message must be a string".to_owned())?;
        return normalize_message(text);
    }
    if ct.starts_with("text/plain") {
        let text = std::str::from_utf8(body).map_err(|_| "body is not utf-8".to_owned())?;
        return normalize_message(text);
    }
    Err("unsupported content-type".to_owned())
}

fn normalize_message(text: &str) -> Result<Option<String>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("message is empty".to_owned());
    }
    if trimmed.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!("message exceeds {MAX_MESSAGE_CHARS} characters"));
    }
    Ok(Some(trimmed.to_owned()))
}

/// Result of an authenticated webhook wake attempt.
#[derive(Debug)]
pub(crate) struct WebhookHttpResult {
    pub(crate) status: u16,
    pub(crate) body: Value,
    pub(crate) events: Vec<WireEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use softwake_ipc::Command;

    use crate::runtime::Runtime;
    use crate::soul::TestSoulDir;

    #[test]
    fn token_matches_accepts_equal_and_rejects_wrong() {
        assert!(token_matches(Some("abc"), "abc"));
        assert!(!token_matches(Some("abd"), "abc"));
        assert!(!token_matches(Some("ab"), "abc"));
        assert!(!token_matches(None, "abc"));
    }

    #[test]
    fn extract_token_reads_bearer_or_alias() {
        let headers = vec![
            ("authorization".into(), "Bearer s3cret".into()),
            ("x-softwake-webhook-token".into(), "other".into()),
        ];
        assert_eq!(extract_token(&headers).as_deref(), Some("s3cret"));
        let alias_only = vec![("x-softwake-webhook-token".into(), "tok".into())];
        assert_eq!(extract_token(&alias_only).as_deref(), Some("tok"));
    }

    #[test]
    fn parse_wake_body_empty_json_and_cap() {
        assert_eq!(parse_wake_body(None, b"").unwrap(), None);
        assert_eq!(
            parse_wake_body(Some("application/json"), br#"{"message":"hi"}"#).unwrap(),
            Some("hi".into())
        );
        assert!(parse_wake_body(Some("application/json"), br#"{"message":""}"#).is_err());
        let long = "x".repeat(MAX_MESSAGE_CHARS + 1);
        let body = serde_json::to_vec(&json!({"message": long})).unwrap();
        assert!(parse_wake_body(Some("application/json"), &body).is_err());
    }

    #[test]
    fn webhook_wake_sleep_awake_and_hibernate_409() {
        let dir = TestSoulDir::valid();
        let mut runtime = Runtime::new(dir.soul_dir());
        let woke = runtime.webhook_wake(None);
        assert_eq!(woke.status, 200);
        assert_eq!(woke.body["state"], "awake");
        assert_eq!(woke.body["ok"], true);

        let again = runtime.webhook_wake(None);
        assert_eq!(again.status, 200);
        assert_eq!(again.body["state"], "awake");

        let hibernated = runtime.handle(Command::Hibernate);
        assert!(hibernated.body.status().is_some());
        let refused = runtime.webhook_wake(Some("hello"));
        assert_eq!(refused.status, 409);
        assert_eq!(refused.body["state"], "hibernate");
        assert_eq!(runtime.webhook_wake(None).body["state"], "hibernate");
    }

    #[test]
    fn resolve_webhook_port_zero_falls_back_to_default() {
        assert_eq!(resolve_webhook_port(0), DEFAULT_WEBHOOK_PORT);
        assert_eq!(resolve_webhook_port(9090), 9090);
    }
}
