//! Laptop-side Remote Agent client (presence, schedule/vault/soul/skills/tools mirror, outbox pull).
//!
//! Uses raw TCP HTTP so offline CI does not need `live-http` / ureq.

use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use softwake_providers::{OauthMirrorDocument, SecretBag, open_store, resolve_secrets_file};
use softwake_soul::resolve_config_dir;
use softwake_state::VoiceState;
use softwake_tools::{
    DEFAULT_NODE_PORT, FileToolsSettings, ScheduleRunOn, first_enabled_agent, list_profile_ids,
    load_messengers, load_remote_agents, load_schedules, node_base_url, resolve_messengers_file,
    resolve_remote_agents_file, resolve_schedules_file, resolve_tools_file,
};

use crate::hud_chat_write;

const HEARTBEAT_SECS: u64 = 30;
const OUTBOX_SYNC_SECS: u64 = 60;
const MIRROR_SECS: u64 = 45;
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct OutboxCursor {
    #[serde(default)]
    last_ts_ms: u64,
    #[serde(default)]
    last_id: String,
    #[serde(default)]
    agent_id: String,
}

#[derive(Debug, Deserialize)]
struct OutboxResponse {
    #[serde(default)]
    items: Vec<OutboxWire>,
}

#[derive(Debug, Deserialize)]
struct OutboxWire {
    id: String,
    profile_id: String,
    #[serde(default)]
    kind: String,
    ts_ms: u64,
    summary: String,
}

#[derive(Debug, Deserialize)]
struct PresenceResponse {
    state: String,
}

/// Map Softwake voice state → companion presence spelling.
#[must_use]
pub(crate) fn presence_from_voice(state: VoiceState) -> &'static str {
    match state {
        VoiceState::Awake => "present",
        VoiceState::Sleep => "sleeping",
        VoiceState::Hibernate => "hibernated",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

struct CompanionTarget {
    agent_id: String,
    base_url: String,
    secret: String,
    oauth_mirror: bool,
}

fn resolve_target() -> Option<CompanionTarget> {
    let path = resolve_remote_agents_file().ok()?;
    let file = load_remote_agents(&path).ok()?;
    let agent = first_enabled_agent(&file)?;
    let port = std::env::var("SOFTWAKE_NODE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_NODE_PORT);
    let base_url = node_base_url(agent, port);
    let secrets_path = resolve_secrets_file().ok()?;
    let store = open_store(&secrets_path).ok()?;
    let bag = store.load().ok()?;
    let secret = bag
        .remote_agent_pairing_secrets
        .get(&agent.id)
        .cloned()
        .unwrap_or_default();
    if secret.is_empty() {
        return None;
    }
    Some(CompanionTarget {
        agent_id: agent.id.clone(),
        base_url,
        secret,
        oauth_mirror: agent.oauth_mirror,
    })
}

fn host_port(base_url: &str) -> Option<(String, u16)> {
    let rest = base_url.strip_prefix("http://")?;
    let (host, port_s) = rest.split_once(':')?;
    let port: u16 = port_s.parse().ok()?;
    Some((host.to_owned(), port))
}

fn http_json(
    base_url: &str,
    method: &str,
    path: &str,
    body: &str,
    secret: &str,
    extra_headers: &[(&str, &str)],
) -> Result<String, String> {
    let (host, port) = host_port(base_url).ok_or_else(|| "bad companion url".to_owned())?;
    let mut stream = TcpStream::connect((host.as_str(), port)).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HTTP_TIMEOUT));
    let mut headers = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {secret}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        let _ = write!(headers, "{k}: {v}\r\n");
    }
    headers.push_str("\r\n");
    stream
        .write_all(headers.as_bytes())
        .map_err(|e| e.to_string())?;
    if !body.is_empty() {
        stream
            .write_all(body.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let mut buf = String::new();
    stream.read_to_string(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn response_body(raw: &str) -> &str {
    raw.split("\r\n\r\n").nth(1).unwrap_or("")
}

fn status_ok(raw: &str) -> bool {
    raw.starts_with("HTTP/1.1 200") || raw.starts_with("HTTP/1.0 200")
}

/// True when an enabled companion is configured (pairing may still be missing).
#[must_use]
pub(crate) fn companion_enabled() -> bool {
    let Ok(path) = resolve_remote_agents_file() else {
        return false;
    };
    let Ok(file) = load_remote_agents(&path) else {
        return false;
    };
    file.has_enabled_companion()
}

/// Post presence heartbeat for current voice state.
pub(crate) fn heartbeat(voice: VoiceState) {
    let Some(target) = resolve_target() else {
        return;
    };
    let body = serde_json::json!({
        "state": presence_from_voice(voice),
        "ts_ms": now_ms(),
    })
    .to_string();
    let _ = http_json(
        &target.base_url,
        "POST",
        "/v1/presence",
        &body,
        &target.secret,
        &[],
    );
}

/// Query companion effective presence (`present` / … / `offline`).
pub(crate) fn fetch_presence_state() -> Option<String> {
    let target = resolve_target()?;
    let raw = http_json(
        &target.base_url,
        "GET",
        "/v1/presence",
        "",
        &target.secret,
        &[],
    )
    .ok()?;
    if !status_ok(&raw) {
        return Some("offline".into());
    }
    let body = response_body(&raw);
    let parsed: PresenceResponse = serde_json::from_str(body).ok()?;
    Some(parsed.state)
}

/// True when companion reports laptop `present` (fallback: assume present if unreachable so local auto still fires).
pub(crate) fn companion_says_present() -> bool {
    match fetch_presence_state().as_deref() {
        // No companion / unreachable → prefer local for auto.
        Some("present") | None => true,
        Some(_) => false,
    }
}

/// Claim a fire lease on the companion. Ok(true)=won, Ok(false)=lost, Err=transport.
pub(crate) fn claim_lease(
    profile_id: &str,
    schedule_id: &str,
    fire_ms: u64,
    claimer: &str,
) -> Result<bool, String> {
    let target = resolve_target().ok_or_else(|| "no companion".to_owned())?;
    let body = serde_json::json!({
        "profile_id": profile_id,
        "schedule_id": schedule_id,
        "fire_ms": fire_ms,
        "claimer": claimer,
        "ttl_ms": 120_000_u64,
    })
    .to_string();
    let raw = http_json(
        &target.base_url,
        "POST",
        "/v1/leases",
        &body,
        &target.secret,
        &[],
    )?;
    if status_ok(&raw) {
        return Ok(true);
    }
    if raw.contains(" 409 ") {
        return Ok(false);
    }
    Err(format!("lease http: {}", raw.lines().next().unwrap_or("")))
}

fn oauth_mirror_document(bag: &SecretBag) -> OauthMirrorDocument {
    OauthMirrorDocument {
        google_connections: bag.google_connections.clone(),
        microsoft_connections: bag.microsoft_connections.clone(),
        active_google_connection_id: bag.active_google_connection_id.clone(),
        active_microsoft_connection_id: bag.active_microsoft_connection_id.clone(),
    }
}

/// Mirror Google/Microsoft OAuth connections to the node when opt-in (ADR-0045).
///
/// Default off. When disabled, PUT an empty document so the node clears the vault.
pub(crate) fn mirror_oauth_vault() {
    let Some(target) = resolve_target() else {
        return;
    };
    let body = if target.oauth_mirror {
        let Ok(secrets_path) = resolve_secrets_file() else {
            return;
        };
        let Ok(store) = open_store(&secrets_path) else {
            return;
        };
        let Ok(bag) = store.load() else {
            return;
        };
        let doc = oauth_mirror_document(&bag);
        serde_json::to_string(&doc).unwrap_or_else(|_| "{}".into())
    } else {
        serde_json::to_string(&OauthMirrorDocument::default()).unwrap_or_else(|_| "{}".into())
    };
    let _ = http_json(
        &target.base_url,
        "PUT",
        "/v1/vault/oauth",
        &body,
        &target.secret,
        &[],
    );
}

/// Mirror Telegram bot token, optional xAI key, and per-profile messengers to the node vault.
pub(crate) fn mirror_telegram_vault() {
    let Some(target) = resolve_target() else {
        return;
    };
    let Ok(secrets_path) = resolve_secrets_file() else {
        return;
    };
    let Ok(store) = open_store(&secrets_path) else {
        return;
    };
    let Ok(bag) = store.load() else {
        return;
    };
    let token = bag
        .telegram_bot_token
        .as_ref()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty());
    let body = match token {
        Some(t) => serde_json::json!({"telegram_bot_token": t}).to_string(),
        None => serde_json::json!({"telegram_bot_token": null}).to_string(),
    };
    let _ = http_json(
        &target.base_url,
        "PUT",
        "/v1/vault/telegram",
        &body,
        &target.secret,
        &[],
    );
    let xai = bag
        .xai_api_key
        .as_ref()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty());
    let llm_body = match xai {
        Some(k) => serde_json::json!({"xai_api_key": k}).to_string(),
        None => serde_json::json!({"xai_api_key": null}).to_string(),
    };
    let _ = http_json(
        &target.base_url,
        "PUT",
        "/v1/vault/llm",
        &llm_body,
        &target.secret,
        &[],
    );
    let Ok(profiles) = list_profile_ids() else {
        return;
    };
    for profile_id in profiles {
        let Ok(path) = resolve_messengers_file(&profile_id) else {
            continue;
        };
        let Ok(file) = load_messengers(&path) else {
            continue;
        };
        let Ok(body) = serde_json::to_string(&file) else {
            continue;
        };
        let api = format!("/v1/profiles/{profile_id}/messengers");
        let _ = http_json(&target.base_url, "PUT", &api, &body, &target.secret, &[]);
    }
    // Best-effort ownership claim when Awake so node releases promptly.
    // Presence heartbeat is the primary signal; this is a nudge.
}

/// Mirror companion/auto schedules for every profile to the node.
pub(crate) fn mirror_schedules() {
    let Some(target) = resolve_target() else {
        return;
    };
    let Ok(profiles) = list_profile_ids() else {
        return;
    };
    for profile_id in profiles {
        let Ok(path) = resolve_schedules_file(&profile_id) else {
            continue;
        };
        let Ok(mut file) = load_schedules(&path) else {
            continue;
        };
        file.entries.retain(|e| {
            e.enabled && matches!(e.run_on, ScheduleRunOn::Companion | ScheduleRunOn::Auto)
        });
        let body = serde_json::to_string(&file).unwrap_or_else(|_| "{}".into());
        let path = format!("/v1/schedules/{profile_id}");
        let _ = http_json(&target.base_url, "PUT", &path, &body, &target.secret, &[]);
    }
}

fn cursor_path() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).ok()?;
    Some(config.join("remote-outbox-cursor.json"))
}

fn load_cursor(path: &Path) -> OutboxCursor {
    let Ok(bytes) = std::fs::read(path) else {
        return OutboxCursor::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn save_cursor(path: &Path, cursor: &OutboxCursor) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(cursor) {
        let _ = std::fs::write(path, format!("{json}\n"));
    }
}

/// Pull new outbox items and append while-away notices per profile.
pub(crate) fn pull_outbox_into_hud() {
    let Some(target) = resolve_target() else {
        return;
    };
    let Some(path) = cursor_path() else {
        return;
    };
    let mut cursor = load_cursor(&path);
    let raw = http_json(
        &target.base_url,
        "GET",
        "/v1/outbox",
        "",
        &target.secret,
        &[
            ("X-Softwake-Since-Ts", &cursor.last_ts_ms.to_string()),
            ("X-Softwake-Since-Id", &cursor.last_id),
        ],
    );
    let Ok(raw) = raw else {
        return;
    };
    if !status_ok(&raw) {
        return;
    }
    let body = response_body(&raw);
    let Ok(parsed) = serde_json::from_str::<OutboxResponse>(body) else {
        return;
    };
    for item in parsed.items {
        let text = if item.summary.starts_with("While you were away:") {
            item.summary.clone()
        } else {
            format!("While you were away: {}", item.summary)
        };
        hud_chat_write::append_assistant_notice(&item.profile_id, &text);
        if item.ts_ms > cursor.last_ts_ms
            || (item.ts_ms == cursor.last_ts_ms && item.id > cursor.last_id)
        {
            cursor.last_ts_ms = item.ts_ms;
            cursor.last_id = item.id;
        }
        cursor.agent_id.clone_from(&target.agent_id);
        let _ = item.kind;
    }
    save_cursor(&path, &cursor);
}

/// Mirror per-profile soul packs, global tools.json, and authored skills to the node.
///
/// Soul/skills/tools mirror does not include OAuth; OAuth is `mirror_oauth_vault` (ADR-0045), default off.
pub(crate) fn mirror_soul_skills_tools() {
    let Some(target) = resolve_target() else {
        return;
    };
    // tools.json (global)
    if let Ok(tools_path) = resolve_tools_file() {
        if let Ok(store) = FileToolsSettings::new(&tools_path) {
            if let Ok(settings) = store.load() {
                if let Ok(body) = serde_json::to_string(&settings) {
                    let _ = http_json(
                        &target.base_url,
                        "PUT",
                        "/v1/tools",
                        &body,
                        &target.secret,
                        &[],
                    );
                }
            }
        }
    }
    // skills catalog
    if let Ok(skills_dir) = softwake_skills::resolve_skills_dir() {
        if let Ok(list) = softwake_skills::list_skills(&skills_dir) {
            let mut skills = Vec::new();
            for skill in list.into_iter().take(softwake_skills::MAX_CATALOG_ENTRIES) {
                skills.push(serde_json::json!({
                    "id": skill.id,
                    "title": skill.title,
                    "source": skill.source.as_str(),
                    "procedure": skill.procedure,
                    "pitfalls": skill.pitfalls,
                    "verify": skill.verify,
                }));
            }
            let body = serde_json::json!({ "skills": skills }).to_string();
            let _ = http_json(
                &target.base_url,
                "PUT",
                "/v1/skills",
                &body,
                &target.secret,
                &[],
            );
        }
    }
    // per-profile soul packs
    let Ok(profiles) = list_profile_ids() else {
        return;
    };
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Ok(config) = resolve_config_dir(xdg.as_deref(), home.as_deref()) else {
        return;
    };
    for profile_id in profiles {
        let pack = softwake_soul::profile_pack_dir(&config, &profile_id);
        let read =
            |name: &str| -> String { std::fs::read_to_string(pack.join(name)).unwrap_or_default() };
        let soul_md = read("soul.md");
        let user_md = read("user.md");
        let rules_md = read("rules.md");
        let glossary_md = read("glossary.md");
        if soul_md.trim().is_empty() {
            continue;
        }
        let body = serde_json::json!({
            "soul_md": soul_md,
            "user_md": user_md,
            "rules_md": rules_md,
            "glossary_md": glossary_md,
        })
        .to_string();
        let api = format!("/v1/profiles/{profile_id}/soul");
        let _ = http_json(&target.base_url, "PUT", &api, &body, &target.secret, &[]);
    }
}

/// Background loops: heartbeat, schedule mirror, outbox pull.
pub(crate) fn spawn_background(shared: &std::sync::Arc<crate::serve::Shared>) {
    let shared_hb = std::sync::Arc::clone(shared);
    let _ = std::thread::Builder::new()
        .name("softwake-ra-heartbeat".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
                let Ok(runtime) = shared_hb.runtime.try_lock() else {
                    continue;
                };
                let voice = runtime.voice_state_for_remote();
                drop(runtime);
                heartbeat(voice);
            }
        });

    let _ = std::thread::Builder::new()
        .name("softwake-ra-mirror".into())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(MIRROR_SECS));
                mirror_schedules();
                mirror_telegram_vault();
                mirror_oauth_vault();
                mirror_soul_skills_tools();
            }
        });

    let _ = std::thread::Builder::new()
        .name("softwake-ra-outbox".into())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(OUTBOX_SYNC_SECS));
                pull_outbox_into_hud();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_mapping() {
        assert_eq!(presence_from_voice(VoiceState::Awake), "present");
        assert_eq!(presence_from_voice(VoiceState::Sleep), "sleeping");
        assert_eq!(presence_from_voice(VoiceState::Hibernate), "hibernated");
    }
}

#[cfg(test)]
mod oauth_mirror_tests {
    use super::oauth_mirror_document;
    use softwake_providers::{AccountConnection, OauthMirrorDocument, SecretBag};

    fn conn(id: &str, email: &str, access: &str, refresh: &str) -> AccountConnection {
        AccountConnection {
            id: id.into(),
            access_token: access.into(),
            refresh_token: refresh.into(),
            expires_at_ms: 9_999_999_999_999,
            token_type: "Bearer".into(),
            scope: "s".into(),
            account_email: Some(email.into()),
        }
    }

    #[test]
    fn oauth_mirror_document_omits_other_secrets() {
        let mut bag = SecretBag::empty();
        bag.xai_api_key = Some("sentinel-xai".into());
        bag.telegram_bot_token = Some("sentinel-tg".into());
        bag.email_smtp_password = Some("sentinel-smtp".into());
        bag.mcp_secrets.insert("mcp1".into(), "sentinel-mcp".into());
        bag.remote_agent_pairing_secrets
            .insert("c".into(), "sentinel-pair".into());
        bag.google_connections = vec![conn(
            "g1",
            "ada@example.com",
            "sentinel-access",
            "sentinel-refresh",
        )];
        let doc = oauth_mirror_document(&bag);
        let json = serde_json::to_string(&doc).expect("ser");
        assert!(json.contains("ada@example.com"));
        assert!(json.contains("sentinel-access"));
        assert!(!json.contains("sentinel-xai"));
        assert!(!json.contains("sentinel-tg"));
        assert!(!json.contains("sentinel-smtp"));
        assert!(!json.contains("sentinel-mcp"));
        assert!(!json.contains("sentinel-pair"));
        let dbg = format!("{doc:?}");
        assert!(!dbg.contains("sentinel-access"));
        assert!(!dbg.contains("sentinel-refresh"));
    }

    #[test]
    fn clear_document_has_empty_arrays() {
        let json = serde_json::to_string(&OauthMirrorDocument::default()).expect("ser");
        assert!(json.contains("google_connections"));
        assert!(!json.contains("sentinel-access"));
        let doc: OauthMirrorDocument =
            serde_json::from_str(r#"{"google_connections":null}"#).expect("null");
        assert!(doc.is_empty());
    }
}
