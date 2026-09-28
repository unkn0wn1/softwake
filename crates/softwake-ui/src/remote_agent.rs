//! Settings commands for Remote Agent pairing (ADR-0039).

#![allow(
    clippy::needless_pass_by_value,
    reason = "Tauri deserializes command arguments as owned values"
)]

use serde::{Deserialize, Serialize};
use softwake_providers::{SecretBag, SecretStore, update_bag};
use softwake_tools::{
    RemoteAgentConfig, RemoteAgentRoles, RemoteAgentsFile, RemoteConflictPolicy, delete_agent,
    load_remote_agents, parse_conflict_policy, resolve_remote_agents_file,
    sanitize_remote_agent_id, save_remote_agents, upsert_agent, validate_agent,
};

/// One agent chip / list row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgentRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

/// Snapshot for Settings → Remote Agent.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Settings checkbox/status JSON"
)]
pub struct RemoteAgentSnapshot {
    pub agents: Vec<RemoteAgentRow>,
    pub selected_id: String,
    pub has_enabled_companion: bool,
    pub storage_backend: String,
    pub storage_message: String,
    pub has_secret: bool,
    pub id: String,
    pub name: String,
    pub tailscale_hostname: String,
    pub ssh_user: String,
    pub role_timers: bool,
    pub role_outbox: bool,
    pub role_webhook_wake: bool,
    pub role_telegram_owner: bool,
    pub conflict_policy: String,
    pub enabled: bool,
    pub test_status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools, reason = "Settings checkbox JSON")]
pub struct RemoteAgentSaveArgs {
    pub id: String,
    pub name: String,
    pub tailscale_hostname: String,
    pub ssh_user: String,
    pub role_timers: bool,
    pub role_outbox: bool,
    pub role_webhook_wake: bool,
    pub role_telegram_owner: bool,
    pub conflict_policy: String,
    pub enabled: bool,
    /// Empty = leave secret unchanged.
    pub secret: Option<String>,
    pub clear_secret: bool,
}

fn open_secrets() -> Result<Box<dyn SecretStore + Send>, String> {
    let path = softwake_providers::resolve_secrets_file().map_err(|e| e.to_string())?;
    softwake_providers::open_store(&path).map_err(|e| e.to_string())
}

fn load_file() -> Result<(std::path::PathBuf, RemoteAgentsFile), String> {
    let path = resolve_remote_agents_file().map_err(|e| e.to_string())?;
    let file = load_remote_agents(&path).map_err(|e| e.to_string())?;
    Ok((path, file))
}

fn snapshot_for(
    file: &RemoteAgentsFile,
    selected_id: &str,
    bag: &SecretBag,
    backend: &str,
    message: &str,
    test_status: &str,
) -> RemoteAgentSnapshot {
    let agents: Vec<RemoteAgentRow> = file
        .agents
        .iter()
        .map(|a| RemoteAgentRow {
            id: a.id.clone(),
            name: a.display_name().to_owned(),
            enabled: a.enabled,
        })
        .collect();
    let selected = if file.agents.iter().any(|a| a.id == selected_id) {
        selected_id.to_owned()
    } else {
        file.agents
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_default()
    };
    let row = file.agents.iter().find(|a| a.id == selected);
    let has_secret = !selected.is_empty()
        && bag
            .remote_agent_pairing_secrets
            .get(&selected)
            .is_some_and(|s| !s.is_empty());
    RemoteAgentSnapshot {
        agents,
        selected_id: selected.clone(),
        has_enabled_companion: file.has_enabled_companion(),
        storage_backend: backend.to_owned(),
        storage_message: message.to_owned(),
        has_secret,
        id: row.map_or_else(String::new, |a| a.id.clone()),
        name: row.map_or_else(String::new, |a| a.name.clone()),
        tailscale_hostname: row.map_or_else(String::new, |a| a.tailscale_hostname.clone()),
        ssh_user: row.map_or_else(|| "root".into(), |a| a.ssh_user.clone()),
        role_timers: row.is_none_or(|a| a.roles.timers),
        role_outbox: row.is_none_or(|a| a.roles.outbox),
        role_webhook_wake: row.is_some_and(|a| a.roles.webhook_wake),
        role_telegram_owner: row.is_some_and(|a| a.roles.telegram_owner),
        conflict_policy: row.map_or_else(
            || RemoteConflictPolicy::PreferLocal.as_str().to_owned(),
            |a| a.conflict_policy.as_str().to_owned(),
        ),
        enabled: row.is_some_and(|a| a.enabled),
        test_status: test_status.to_owned(),
    }
}

/// Load Remote Agent Settings.
#[tauri::command]
pub fn remote_agent_snapshot(selected_id: Option<String>) -> Result<RemoteAgentSnapshot, String> {
    let (_path, file) = load_file()?;
    let store = open_secrets()?;
    let bag = store.load().map_err(|e| e.to_string())?;
    let report = store.report();
    Ok(snapshot_for(
        &file,
        selected_id.as_deref().unwrap_or(""),
        &bag,
        report.backend.as_str(),
        &report.message,
        "",
    ))
}

/// Add an empty draft agent id (operator fills fields then Save).
#[tauri::command]
pub fn remote_agent_add() -> Result<RemoteAgentSnapshot, String> {
    let (path, mut file) = load_file()?;
    let base = "companion";
    let mut id = base.to_owned();
    let mut n = 2_u32;
    while file.agents.iter().any(|a| a.id == id) {
        id = format!("{base}_{n}");
        n += 1;
    }
    let agent = RemoteAgentConfig {
        id: id.clone(),
        name: "Companion".into(),
        tailscale_hostname: String::new(),
        ssh_user: "root".into(),
        roles: RemoteAgentRoles::default(),
        conflict_policy: RemoteConflictPolicy::PreferLocal,
        enabled: false,
        created_ms: None,
        updated_ms: None,
    };
    // Skip full validate (hostname empty) — stash draft only after sanitize id.
    let id = sanitize_remote_agent_id(&agent.id);
    if id.is_empty() {
        return Err("could not allocate agent id".into());
    }
    if file.agents.len() >= softwake_tools::MAX_REMOTE_AGENTS {
        return Err(format!(
            "remote agent cap reached ({})",
            softwake_tools::MAX_REMOTE_AGENTS
        ));
    }
    let mut agent = agent;
    agent.id.clone_from(&id);
    file.agents.push(agent);
    save_remote_agents(&path, &file).map_err(|e| e.to_string())?;
    remote_agent_snapshot(Some(id))
}

/// Save one agent row and optional pairing secret.
#[tauri::command]
pub fn remote_agent_save(args: RemoteAgentSaveArgs) -> Result<RemoteAgentSnapshot, String> {
    let id = sanitize_remote_agent_id(&args.id);
    if id.is_empty() {
        return Err("id is required".into());
    }
    let conflict_policy =
        parse_conflict_policy(&args.conflict_policy).map_err(|e| e.to_string())?;
    let agent = RemoteAgentConfig {
        id: id.clone(),
        name: args.name.trim().to_owned(),
        tailscale_hostname: args.tailscale_hostname.trim().to_owned(),
        ssh_user: args.ssh_user.trim().to_owned(),
        roles: RemoteAgentRoles {
            timers: args.role_timers,
            outbox: args.role_outbox,
            webhook_wake: args.role_webhook_wake,
            // Slice 1: ignore UI attempts to enable telegram_owner.
            telegram_owner: {
                let _ = args.role_telegram_owner;
                false
            },
        },
        conflict_policy,
        enabled: args.enabled,
        created_ms: None,
        updated_ms: None,
    };
    validate_agent(&agent).map_err(|e| e.to_string())?;
    let (path, mut file) = load_file()?;
    upsert_agent(&mut file, agent).map_err(|e| e.to_string())?;
    save_remote_agents(&path, &file).map_err(|e| e.to_string())?;

    let store = open_secrets()?;
    if args.clear_secret {
        update_bag(store.as_ref(), |bag| {
            bag.remote_agent_pairing_secrets.remove(&id);
        })
        .map_err(|e| e.to_string())?;
    } else if let Some(secret) = args.secret {
        let trimmed = secret.trim().to_owned();
        if !trimmed.is_empty() {
            update_bag(store.as_ref(), |bag| {
                bag.remote_agent_pairing_secrets.insert(id.clone(), trimmed);
            })
            .map_err(|e| e.to_string())?;
        }
    }
    remote_agent_snapshot(Some(id))
}

/// Delete an agent and its pairing secret.
#[tauri::command]
pub fn remote_agent_delete(agent_id: String) -> Result<RemoteAgentSnapshot, String> {
    let id = sanitize_remote_agent_id(&agent_id);
    let (path, mut file) = load_file()?;
    delete_agent(&mut file, &id);
    save_remote_agents(&path, &file).map_err(|e| e.to_string())?;
    let store = open_secrets()?;
    update_bag(store.as_ref(), |bag| {
        bag.remote_agent_pairing_secrets.remove(&id);
    })
    .map_err(|e| e.to_string())?;
    remote_agent_snapshot(None)
}

/// Clear pairing secret for one agent.
#[tauri::command]
pub fn remote_agent_clear_secret(agent_id: String) -> Result<RemoteAgentSnapshot, String> {
    let id = sanitize_remote_agent_id(&agent_id);
    let store = open_secrets()?;
    update_bag(store.as_ref(), |bag| {
        bag.remote_agent_pairing_secrets.remove(&id);
    })
    .map_err(|e| e.to_string())?;
    remote_agent_snapshot(Some(id))
}

/// Honest Tailnet probe stub (slice 1).
#[tauri::command]
pub fn remote_agent_test(agent_id: Option<String>) -> Result<RemoteAgentSnapshot, String> {
    let (_path, file) = load_file()?;
    let store = open_secrets()?;
    let bag = store.load().map_err(|e| e.to_string())?;
    let report = store.report();
    let id = agent_id
        .as_deref()
        .map(sanitize_remote_agent_id)
        .filter(|s| !s.is_empty())
        .or_else(|| file.agents.first().map(|a| a.id.clone()))
        .unwrap_or_default();
    let status = if let Some(agent) = file.agents.iter().find(|a| a.id == id) {
        if agent.tailscale_hostname.trim().is_empty() {
            "Test blocked: set Tailscale hostname (MagicDNS or 100.x) first.".to_owned()
        } else {
            format!(
                "Tailnet probe not implemented in slice 1 (would: `tailscale ping {}` / SSH BatchMode as {}). Config looks structurally valid.",
                agent.tailscale_hostname.trim(),
                agent.ssh_user.trim()
            )
        }
    } else {
        "No remote agent selected.".to_owned()
    };
    Ok(snapshot_for(
        &file,
        &id,
        &bag,
        report.backend.as_str(),
        &report.message,
        &status,
    ))
}
