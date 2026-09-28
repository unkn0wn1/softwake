//! Non-secret Remote Agent pairing Settings (`remote-agents.json`).
//!
//! Pairing secrets stay in the provider secret bag (`remote_agent_pairing_secrets`).
//! See ADR-0039.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// File name under the Softwake config directory.
pub const REMOTE_AGENTS_FILE_NAME: &str = "remote-agents.json";

/// Largest remote-agents.json this backend will read.
pub const MAX_REMOTE_AGENTS_BYTES: usize = 64 * 1024;

/// Cap on configured companions (v1 is typically one).
pub const MAX_REMOTE_AGENTS: usize = 4;

const DOCUMENT_VERSION: u32 = 1;

/// Roles a companion may advertise (enforcement mostly slice 2+).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "role flags map 1:1 to Settings checkboxes"
)]
pub struct RemoteAgentRoles {
    /// May host / lease timer fires.
    #[serde(default = "default_true")]
    pub timers: bool,
    /// Holds shared outbox (fire leases + away results).
    #[serde(default = "default_true")]
    pub outbox: bool,
    /// May accept Tailnet webhook wake (future bind on TS IP).
    #[serde(default)]
    pub webhook_wake: bool,
    /// Telegram sticky owner candidate (slice 2; UI disabled in slice 1).
    #[serde(default)]
    pub telegram_owner: bool,
}

fn default_true() -> bool {
    true
}

impl Default for RemoteAgentRoles {
    fn default() -> Self {
        Self {
            timers: true,
            outbox: true,
            webhook_wake: false,
            telegram_owner: false,
        }
    }
}

/// Stub conflict policy (no runtime resolver in slice 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RemoteConflictPolicy {
    /// Prefer laptop when both present (default).
    #[default]
    PreferLocal,
    /// Prefer companion (stub).
    PreferCompanion,
    /// Operator decides (stub).
    Manual,
}

impl RemoteConflictPolicy {
    /// Stable spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreferLocal => "prefer_local",
            Self::PreferCompanion => "prefer_companion",
            Self::Manual => "manual",
        }
    }
}

impl std::fmt::Display for RemoteConflictPolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Parse conflict policy spelling.
///
/// # Errors
///
/// Unknown spelling.
pub fn parse_conflict_policy(raw: &str) -> Result<RemoteConflictPolicy, RemoteAgentsError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "prefer_local" | "local" => Ok(RemoteConflictPolicy::PreferLocal),
        "prefer_companion" | "companion" => Ok(RemoteConflictPolicy::PreferCompanion),
        "manual" => Ok(RemoteConflictPolicy::Manual),
        other => Err(RemoteAgentsError::Invalid(format!(
            "unknown conflict_policy: {other} (want prefer_local|prefer_companion|manual)"
        ))),
    }
}

/// One paired companion node (non-secret fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAgentConfig {
    /// Stable node id / slug.
    pub id: String,
    /// Operator-facing name.
    #[serde(default)]
    pub name: String,
    /// Tailscale `MagicDNS` hostname or `100.x` address.
    pub tailscale_hostname: String,
    /// SSH user for Tailnet install / probe.
    pub ssh_user: String,
    /// Role flags.
    #[serde(default)]
    pub roles: RemoteAgentRoles,
    /// Stub conflict policy.
    #[serde(default)]
    pub conflict_policy: RemoteConflictPolicy,
    /// Master switch.
    #[serde(default)]
    pub enabled: bool,
    /// Created unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_ms: Option<u64>,
    /// Updated unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_ms: Option<u64>,
}

impl RemoteAgentConfig {
    /// Display label or id.
    #[must_use]
    pub fn display_name(&self) -> &str {
        let trimmed = self.name.trim();
        if trimmed.is_empty() {
            self.id.as_str()
        } else {
            trimmed
        }
    }
}

/// On-disk remote agents document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAgentsFile {
    /// Document version.
    #[serde(default = "one")]
    pub version: u32,
    /// Configured agents.
    #[serde(default)]
    pub agents: Vec<RemoteAgentConfig>,
}

fn one() -> u32 {
    DOCUMENT_VERSION
}

impl Default for RemoteAgentsFile {
    fn default() -> Self {
        Self {
            version: DOCUMENT_VERSION,
            agents: Vec::new(),
        }
    }
}

impl RemoteAgentsFile {
    /// Drop empty ids, clamp version, ensure unique ids (last wins), cap length.
    pub fn normalize(&mut self) {
        self.version = DOCUMENT_VERSION;
        let mut by_id = BTreeMap::new();
        for agent in self.agents.drain(..) {
            let id = sanitize_remote_agent_id(&agent.id);
            if id.is_empty() {
                continue;
            }
            let mut agent = agent;
            agent.id = id;
            by_id.insert(agent.id.clone(), agent);
        }
        self.agents = by_id.into_values().collect();
        if self.agents.len() > MAX_REMOTE_AGENTS {
            self.agents.truncate(MAX_REMOTE_AGENTS);
        }
    }

    /// True when at least one enabled companion exists.
    #[must_use]
    pub fn has_enabled_companion(&self) -> bool {
        self.agents.iter().any(|a| a.enabled)
    }
}

/// Errors loading or saving remote-agents.json.
#[derive(Debug, thiserror::Error)]
pub enum RemoteAgentsError {
    /// Neither XDG config nor HOME is set.
    #[error("cannot resolve Softwake config directory: XDG_CONFIG_HOME and HOME are unset")]
    NoConfigDir,
    /// Path was empty.
    #[error("remote agents Settings path is empty")]
    EmptyPath,
    /// File is larger than [`MAX_REMOTE_AGENTS_BYTES`].
    #[error("remote agents Settings file is too large ({len} > {max}): {}", path.display())]
    TooLarge {
        /// Path that was rejected.
        path: PathBuf,
        /// Observed length.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// JSON did not parse.
    #[error("remote agents Settings JSON is invalid: {message}")]
    InvalidJson {
        /// serde message.
        message: String,
    },
    /// IO failure.
    #[error("remote agents Settings IO error: {message}")]
    Io {
        /// OS message.
        message: String,
    },
    /// Validation failure.
    #[error("{0}")]
    Invalid(String),
    /// Cap reached.
    #[error("remote agent cap reached ({MAX_REMOTE_AGENTS})")]
    CapReached,
}

impl From<io::Error> for RemoteAgentsError {
    fn from(error: io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }
}

/// Resolve `$XDG_CONFIG_HOME/softwake/remote-agents.json`.
///
/// # Errors
///
/// When config root cannot be resolved.
pub fn resolve_remote_agents_file() -> Result<PathBuf, RemoteAgentsError> {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let root = softwake_soul::resolve_config_dir(xdg.as_deref(), home.as_deref())
        .map_err(|_| RemoteAgentsError::NoConfigDir)?;
    Ok(root.join(REMOTE_AGENTS_FILE_NAME))
}

/// Load remote-agents.json. Missing file → default empty.
///
/// # Errors
///
/// IO / parse / size.
pub fn load_remote_agents(path: &Path) -> Result<RemoteAgentsFile, RemoteAgentsError> {
    if path.as_os_str().is_empty() {
        return Err(RemoteAgentsError::EmptyPath);
    }
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > MAX_REMOTE_AGENTS_BYTES {
                return Err(RemoteAgentsError::TooLarge {
                    path: path.to_owned(),
                    len: bytes.len(),
                    max: MAX_REMOTE_AGENTS_BYTES,
                });
            }
            if bytes.iter().all(u8::is_ascii_whitespace) {
                return Ok(RemoteAgentsFile::default());
            }
            let mut file: RemoteAgentsFile =
                serde_json::from_slice(&bytes).map_err(|error| RemoteAgentsError::InvalidJson {
                    message: error.to_string(),
                })?;
            file.normalize();
            Ok(file)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(RemoteAgentsFile::default()),
        Err(error) => Err(error.into()),
    }
}

/// Save remote-agents.json (0600 on Unix).
///
/// # Errors
///
/// IO.
pub fn save_remote_agents(path: &Path, file: &RemoteAgentsFile) -> Result<(), RemoteAgentsError> {
    if path.as_os_str().is_empty() {
        return Err(RemoteAgentsError::EmptyPath);
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
    let json = serde_json::to_vec_pretty(&normalized).map_err(|error| RemoteAgentsError::Io {
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

/// Sanitize a remote agent id (slug).
#[must_use]
pub fn sanitize_remote_agent_id(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch.is_whitespace() {
            out.push('_');
        }
    }
    if out.len() > 32 {
        out.truncate(32);
    }
    out
}

/// Validate hostname / ssh user fields on an agent.
///
/// # Errors
///
/// Empty or clearly non-Tailnet public URL schemes.
pub fn validate_agent(agent: &RemoteAgentConfig) -> Result<(), RemoteAgentsError> {
    if agent.id.is_empty() {
        return Err(RemoteAgentsError::Invalid(
            "remote agent id is required".to_owned(),
        ));
    }
    let host = agent.tailscale_hostname.trim();
    if host.is_empty() {
        return Err(RemoteAgentsError::Invalid(
            "tailscale_hostname is required (MagicDNS or 100.x)".to_owned(),
        ));
    }
    let lower = host.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.contains("://") {
        return Err(RemoteAgentsError::Invalid(
            "tailscale_hostname must be MagicDNS or 100.x (no URL scheme)".to_owned(),
        ));
    }
    if agent.ssh_user.trim().is_empty() || agent.ssh_user.chars().any(char::is_whitespace) {
        return Err(RemoteAgentsError::Invalid(
            "ssh_user is required and must not contain whitespace".to_owned(),
        ));
    }
    Ok(())
}

/// Upsert an agent (validate + cap).
///
/// # Errors
///
/// Validation or cap.
pub fn upsert_agent(
    file: &mut RemoteAgentsFile,
    mut agent: RemoteAgentConfig,
) -> Result<(), RemoteAgentsError> {
    agent.id = sanitize_remote_agent_id(&agent.id);
    validate_agent(&agent)?;
    let now = now_ms();
    if let Some(existing) = file.agents.iter_mut().find(|a| a.id == agent.id) {
        agent.created_ms = existing.created_ms.or(Some(now));
        agent.updated_ms = Some(now);
        *existing = agent;
    } else {
        if file.agents.len() >= MAX_REMOTE_AGENTS {
            return Err(RemoteAgentsError::CapReached);
        }
        agent.created_ms = Some(now);
        agent.updated_ms = Some(now);
        file.agents.push(agent);
    }
    file.normalize();
    Ok(())
}

/// Delete by id.
pub fn delete_agent(file: &mut RemoteAgentsFile, id: &str) {
    let id = sanitize_remote_agent_id(id);
    file.agents.retain(|a| a.id != id);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Default softwake-node HTTP port.
pub const DEFAULT_NODE_PORT: u16 = 8790;

/// Build `http://{hostname}:{port}` for a companion (no trailing slash).
#[must_use]
pub fn node_base_url(agent: &RemoteAgentConfig, port: u16) -> String {
    let host = agent.tailscale_hostname.trim().trim_end_matches('/');
    format!("http://{host}:{port}")
}

/// First enabled agent, if any.
#[must_use]
pub fn first_enabled_agent(file: &RemoteAgentsFile) -> Option<&RemoteAgentConfig> {
    file.agents.iter().find(|a| a.enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn default_empty_and_has_enabled_false() {
        let file = RemoteAgentsFile::default();
        assert!(file.agents.is_empty());
        assert!(!file.has_enabled_companion());
    }

    #[test]
    fn sanitize_and_round_trip() {
        let dir = tempfile_dir();
        let path = dir.join(REMOTE_AGENTS_FILE_NAME);
        let mut file = RemoteAgentsFile::default();
        upsert_agent(
            &mut file,
            RemoteAgentConfig {
                id: "Softwake-CT".into(),
                name: "Home CT".into(),
                tailscale_hostname: "softwake-ct".into(),
                ssh_user: "root".into(),
                roles: RemoteAgentRoles::default(),
                conflict_policy: RemoteConflictPolicy::PreferLocal,
                enabled: true,
                created_ms: None,
                updated_ms: None,
            },
        )
        .expect("upsert");
        assert_eq!(file.agents[0].id, "softwake_ct");
        assert!(file.has_enabled_companion());
        save_remote_agents(&path, &file).expect("save");
        let loaded = load_remote_agents(&path).expect("load");
        assert_eq!(loaded.agents.len(), 1);
        assert_eq!(loaded.agents[0].tailscale_hostname, "softwake-ct");
        assert_eq!(
            loaded.agents[0].conflict_policy,
            RemoteConflictPolicy::PreferLocal
        );
    }

    #[test]
    fn reject_url_scheme_hostname() {
        let agent = RemoteAgentConfig {
            id: "n1".into(),
            name: String::new(),
            tailscale_hostname: "https://example.com".into(),
            ssh_user: "root".into(),
            roles: RemoteAgentRoles::default(),
            conflict_policy: RemoteConflictPolicy::PreferLocal,
            enabled: true,
            created_ms: None,
            updated_ms: None,
        };
        assert!(validate_agent(&agent).is_err());
    }

    #[test]
    fn reject_bad_id_empty_after_sanitize() {
        let mut file = RemoteAgentsFile::default();
        let err = upsert_agent(
            &mut file,
            RemoteAgentConfig {
                id: "!!!".into(),
                name: String::new(),
                tailscale_hostname: "softwake-ct".into(),
                ssh_user: "root".into(),
                roles: RemoteAgentRoles::default(),
                conflict_policy: RemoteConflictPolicy::PreferLocal,
                enabled: false,
                created_ms: None,
                updated_ms: None,
            },
        );
        assert!(err.is_err());
    }

    fn tempfile_dir() -> PathBuf {
        let mut path = env::temp_dir();
        path.push(format!(
            "softwake-remote-agents-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("tmpdir");
        path
    }
}
