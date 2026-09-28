//! Softwake companion node (Remote Agent / ADR-0039 + ADR-0040).
//!
//! Slice 2: presence heartbeats, fire leases, durable outbox, per-profile
//! schedule mirror + tick. Bind via `SOFTWAKE_NODE_LISTEN` (default
//! `127.0.0.1:8790`). Production binds the node's Tailscale IP only.
//! Auth: `SOFTWAKE_NODE_PAIRING_SECRET` + Bearer / X-Softwake-Remote-Token.

mod auth;
mod http;
mod state;

use std::env;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::json;

use crate::auth::{PAIRING_SECRET_ENV, authorized};
use crate::http::{HttpRequest, read_request, write_response};
use crate::state::{LEASE_TTL_MS, NodeState, PRESENCE_GRACE_MS, PresenceState};

const DEFAULT_LISTEN: &str = "127.0.0.1:8790";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let listen = env::var("SOFTWAKE_NODE_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.to_owned());
    let secret = env::var(PAIRING_SECRET_ENV).unwrap_or_default();
    if secret.is_empty() {
        eprintln!("softwake-node: warning: {PAIRING_SECRET_ENV} unset — auth endpoints will 401");
    }
    let data_dir = data_dir();
    let state = Arc::new(NodeState::open(data_dir.clone()));
    {
        let tick_state = Arc::clone(&state);
        let _ = thread::Builder::new()
            .name("softwake-node-tick".into())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(15));
                    let n = tick_state.tick_schedules();
                    if n > 0 {
                        eprintln!("softwake-node: fired {n} mirrored schedule(s)");
                    }
                }
            });
    }
    let listener = TcpListener::bind(&listen).unwrap_or_else(|error| {
        eprintln!("softwake-node: bind {listen} failed: {error}");
        std::process::exit(1);
    });
    let addr = listener.local_addr().expect("local addr");
    eprintln!(
        "softwake-node: companion listening on {addr} data={} (role=companion)",
        data_dir.display()
    );
    for conn in listener.incoming() {
        match conn {
            Ok(mut stream) => {
                let state = Arc::clone(&state);
                let secret = secret.clone();
                let _ = thread::spawn(move || {
                    if let Err(error) = handle_client(&mut stream, &state, &secret) {
                        eprintln!("softwake-node: connection error: {error}");
                    }
                });
            }
            Err(error) => eprintln!("softwake-node: accept: {error}"),
        }
    }
}

fn data_dir() -> PathBuf {
    if let Ok(p) = env::var("SOFTWAKE_NODE_DATA") {
        return PathBuf::from(p);
    }
    if let Ok(xdg) = env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("softwake-node");
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".local/share/softwake-node");
    }
    PathBuf::from("/tmp/softwake-node-data")
}

fn handle_client(
    stream: &mut std::net::TcpStream,
    state: &NodeState,
    secret: &str,
) -> std::io::Result<()> {
    let req = read_request(stream)?;
    route(stream, state, secret, &req)
}

#[allow(clippy::too_many_lines)]
fn route(
    stream: &mut std::net::TcpStream,
    state: &NodeState,
    secret: &str,
    req: &HttpRequest,
) -> std::io::Result<()> {
    let path = req.path.trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };

    if path == "/health" {
        let body = json!({
            "ok": true,
            "role": "companion",
            "version": VERSION,
            "presence_grace_ms": PRESENCE_GRACE_MS,
        })
        .to_string();
        return write_response(stream, 200, "application/json", &body);
    }

    // Everything below requires pairing secret.
    if !authorized(req, secret) {
        return write_response(
            stream,
            401,
            "application/json",
            "{\"ok\":false,\"error\":\"unauthorized\"}",
        );
    }

    match (req.method.as_str(), path) {
        ("GET", "/v1/presence") => {
            let now = NodeState::now_ms();
            let rec = state.effective_presence(now);
            let body = json!({
                "state": rec.state.as_str(),
                "last_heartbeat_ms": rec.last_heartbeat_ms,
                "grace_ms": PRESENCE_GRACE_MS,
            })
            .to_string();
            write_response(stream, 200, "application/json", &body)
        }
        ("POST", "/v1/presence") => {
            let v: serde_json::Value = serde_json::from_str(&req.body).unwrap_or(json!({}));
            let state_str = v.get("state").and_then(|x| x.as_str()).unwrap_or("");
            let ts = v
                .get("ts_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_else(NodeState::now_ms);
            let parsed = match state_str {
                "present" => PresenceState::Present,
                "sleeping" => PresenceState::Sleeping,
                "hibernated" => PresenceState::Hibernated,
                "offline" => PresenceState::Offline,
                _ => {
                    return write_response(
                        stream,
                        400,
                        "application/json",
                        "{\"ok\":false,\"error\":\"state must be present|sleeping|hibernated|offline\"}",
                    );
                }
            };
            state.set_presence(parsed, ts);
            write_response(stream, 200, "application/json", "{\"ok\":true}")
        }
        ("POST", "/v1/leases") => {
            let v: serde_json::Value = serde_json::from_str(&req.body).unwrap_or(json!({}));
            let profile_id = v
                .get("profile_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();
            let schedule_id = v
                .get("schedule_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();
            let fire_ms = v
                .get("fire_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let claimer = v
                .get("claimer")
                .and_then(|x| x.as_str())
                .unwrap_or("laptop")
                .to_owned();
            let ttl = v
                .get("ttl_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(LEASE_TTL_MS);
            if profile_id.is_empty() || schedule_id.is_empty() || fire_ms == 0 {
                return write_response(
                    stream,
                    400,
                    "application/json",
                    "{\"ok\":false,\"error\":\"profile_id, schedule_id, fire_ms required\"}",
                );
            }
            match state.claim_lease(&profile_id, &schedule_id, fire_ms, &claimer, ttl) {
                Ok(lease_id) => {
                    let body = json!({"ok": true, "lease_id": lease_id}).to_string();
                    write_response(stream, 200, "application/json", &body)
                }
                Err(holder) => {
                    let body = json!({"ok": false, "holder": holder}).to_string();
                    write_response(stream, 409, "application/json", &body)
                }
            }
        }
        ("GET", "/v1/outbox") => {
            // path may not include query — check raw first line already stripped.
            // Re-parse from body empty; support since_ts via header X-Since-Ts or query in original — our parser drops query.
            // Use header X-Softwake-Since-Ts / X-Softwake-Since-Id for simplicity + JSON default.
            let since_ts = header_u64(req, "x-softwake-since-ts").unwrap_or(0);
            let since_id = header_str(req, "x-softwake-since-id");
            let items = state.outbox_since(since_ts, since_id.as_deref());
            let body = json!({"items": items}).to_string();
            write_response(stream, 200, "application/json", &body)
        }
        ("PUT", path) if path.starts_with("/v1/schedules/") => {
            let profile_id = path.trim_start_matches("/v1/schedules/");
            if profile_id.is_empty() || profile_id.contains('/') {
                return write_response(
                    stream,
                    400,
                    "application/json",
                    "{\"ok\":false,\"error\":\"bad profile_id\"}",
                );
            }
            match serde_json::from_str::<softwake_tools::SchedulesFile>(&req.body) {
                Ok(file) => {
                    state.put_schedules(profile_id, file);
                    write_response(stream, 200, "application/json", "{\"ok\":true}")
                }
                Err(error) => {
                    let body = json!({"ok": false, "error": error.to_string()}).to_string();
                    write_response(stream, 400, "application/json", &body)
                }
            }
        }
        ("POST", "/v1/outbox") => {
            // Optional explicit push (laptop-side ack summaries)
            let v: serde_json::Value = serde_json::from_str(&req.body).unwrap_or(json!({}));
            let profile_id = v
                .get("profile_id")
                .and_then(|x| x.as_str())
                .unwrap_or("default")
                .to_owned();
            let summary = v
                .get("summary")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_owned();
            let kind = v
                .get("kind")
                .and_then(|x| x.as_str())
                .unwrap_or("fire_ack")
                .to_owned();
            if summary.is_empty() {
                return write_response(
                    stream,
                    400,
                    "application/json",
                    "{\"ok\":false,\"error\":\"summary required\"}",
                );
            }
            let id = format!("ob-{}", NodeState::now_ms());
            state.push_outbox(crate::state::OutboxItem {
                id: id.clone(),
                profile_id,
                kind,
                schedule_id: v
                    .get("schedule_id")
                    .and_then(|x| x.as_str())
                    .map(str::to_owned),
                ts_ms: NodeState::now_ms(),
                summary,
                lease_id: v
                    .get("lease_id")
                    .and_then(|x| x.as_str())
                    .map(str::to_owned),
            });
            let body = json!({"ok": true, "id": id}).to_string();
            write_response(stream, 200, "application/json", &body)
        }
        _ => {
            if matches!(req.method.as_str(), "GET" | "POST" | "PUT") {
                write_response(stream, 404, "text/plain", "not found\n")
            } else {
                write_response(stream, 405, "text/plain", "method not allowed\n")
            }
        }
    }
}

fn header_u64(req: &HttpRequest, name: &str) -> Option<u64> {
    header_str(req, name)?.parse().ok()
}

fn header_str(req: &HttpRequest, name: &str) -> Option<String> {
    req.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};

    fn spawn_node(secret: &str) -> (SocketAddr, PathBuf, thread::JoinHandle<()>) {
        let dir = std::env::temp_dir().join(format!(
            "sw-node-itest-{}-{}",
            std::process::id(),
            NodeState::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = Arc::new(NodeState::open(dir.clone()));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let secret = secret.to_owned();
        let handle = thread::spawn(move || {
            for _ in 0..16 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let _ = handle_client(&mut stream, &state, &secret);
                }
            }
        });
        thread::sleep(Duration::from_millis(30));
        (addr, dir, handle)
    }

    fn http(
        addr: SocketAddr,
        method: &str,
        path: &str,
        body: &str,
        token: Option<&str>,
        extra_headers: &[(&str, &str)],
    ) -> String {
        use std::fmt::Write as _;
        let mut stream = TcpStream::connect(addr).expect("connect");
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut headers = String::new();
        if let Some(tok) = token {
            let _ = write!(headers, "Authorization: Bearer {tok}\r\n");
        }
        for (k, v) in extra_headers {
            let _ = write!(headers, "{k}: {v}\r\n");
        }
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        );
        stream.write_all(req.as_bytes()).expect("write");
        let mut buf = String::new();
        stream.read_to_string(&mut buf).expect("read");
        buf
    }

    #[test]
    fn health_open_presence_needs_auth() {
        let (addr, dir, _h) = spawn_node("sekrit");
        let health = http(addr, "GET", "/health", "", None, &[]);
        assert!(health.contains("200"), "{health}");
        assert!(health.contains("\"role\":\"companion\""), "{health}");
        let denied = http(addr, "GET", "/v1/presence", "", None, &[]);
        assert!(denied.contains("401"), "{denied}");
        let ok = http(
            addr,
            "POST",
            "/v1/presence",
            r#"{"state":"present","ts_ms":1}"#,
            Some("sekrit"),
            &[],
        );
        assert!(ok.contains("200"), "{ok}");
        let got = http(addr, "GET", "/v1/presence", "", Some("sekrit"), &[]);
        assert!(got.contains("present") || got.contains("offline"), "{got}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn lease_conflict_and_outbox() {
        let (addr, dir, _h) = spawn_node("tok");
        let body = r#"{"profile_id":"default","schedule_id":"s1","fire_ms":42,"claimer":"laptop"}"#;
        let a = http(addr, "POST", "/v1/leases", body, Some("tok"), &[]);
        assert!(a.contains("200") && a.contains("lease_id"), "{a}");
        let body2 =
            r#"{"profile_id":"default","schedule_id":"s1","fire_ms":42,"claimer":"companion"}"#;
        let b = http(addr, "POST", "/v1/leases", body2, Some("tok"), &[]);
        assert!(b.contains("409"), "{b}");
        let push = http(
            addr,
            "POST",
            "/v1/outbox",
            r#"{"profile_id":"default","kind":"fire_ack","summary":"timer: hi"}"#,
            Some("tok"),
            &[],
        );
        assert!(push.contains("200"), "{push}");
        let out = http(addr, "GET", "/v1/outbox", "", Some("tok"), &[]);
        assert!(out.contains("timer: hi"), "{out}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
