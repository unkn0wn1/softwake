//! Tailscale-only Remote Agent probe + SSH companion installer (ADR-0044).
//!
//! Prefer shipping a laptop-built `softwake-node` via scp. Never bind `0.0.0.0`
//! on a public NIC. Pairing secrets are passed in by the caller (secret bag).

use std::env;
use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::remote_agents::{
    DEFAULT_NODE_PORT, RemoteAgentConfig, assert_tailscale_host, is_tailscale_host, node_base_url,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const SSH_CONNECT_TIMEOUT: &str = "5";

/// Result of a Tailnet probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// Human-readable status lines (no secrets).
    pub lines: Vec<String>,
    /// True when host is Tailscale-valid and SSH or health succeeded.
    pub ok: bool,
}

impl ProbeReport {
    /// Single-line status for Settings / ctl.
    #[must_use]
    pub fn summary(&self) -> String {
        let joined = self.lines.join(" | ");
        if joined.len() > 800 {
            format!("{}…", &joined[..797])
        } else {
            joined
        }
    }
}

/// Result of an install attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    /// Human-readable status lines (no secrets).
    pub lines: Vec<String>,
    /// True when health check succeeded after install.
    pub ok: bool,
    /// How the binary was resolved.
    pub binary_source: String,
}

impl InstallReport {
    /// Single-line status for Settings / ctl.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut lines = self.lines.clone();
        if !self.binary_source.is_empty() {
            lines.insert(0, format!("bin={}", self.binary_source));
        }
        let joined = lines.join(" | ");
        if joined.len() > 800 {
            format!("{}…", &joined[..797])
        } else {
            joined
        }
    }
}

/// Render `/etc/softwake-node.env` contents (no trailing secret logging by callers).
#[must_use]
pub fn render_node_env(listen_host: &str, pairing_secret: &str) -> String {
    format!(
        "# Managed by Softwake Remote Agent installer (ADR-0044).\n\
         # Optional: SOFTWAKE_NODE_XAI_API_KEY=…  (needed for real agent_task / Telegram LLM)\n\
         # Optional: SOFTWAKE_NODE_MODEL=grok-4-fast-non-reasoning\n\
         # Optional, ADR-0045: publisher client for refresh while the laptop is away.\n\
         # SOFTWAKE_GOOGLE_CLIENT_ID=\n\
         # SOFTWAKE_GOOGLE_CLIENT_SECRET=\n\
         # SOFTWAKE_MICROSOFT_CLIENT_ID=\n\
         # Or oauth-clients.env under the softwake user's home\n\
         # (/var/lib/softwake-node/.config/softwake/oauth-clients.env).\n\
         SOFTWAKE_NODE_LISTEN={listen_host}:{port}\n\
         SOFTWAKE_NODE_PAIRING_SECRET={pairing_secret}\n\
         SOFTWAKE_NODE_DATA=/var/lib/softwake-node\n",
        listen_host = listen_host.trim(),
        port = DEFAULT_NODE_PORT,
        pairing_secret = pairing_secret,
    )
}

/// Keys from a previous `/etc/softwake-node.env` that reinstall must keep when the
/// new render does not already set them (ADR-0045). Exact names only.
#[allow(dead_code, reason = "documented allowlist; CT shell duplicates names")]
pub const PRESERVED_NODE_ENV_KEYS: &[&str] = &[
    "SOFTWAKE_NODE_XAI_API_KEY",
    "SOFTWAKE_NODE_MODEL",
    "SOFTWAKE_GOOGLE_CLIENT_ID",
    "SOFTWAKE_GOOGLE_CLIENT_SECRET",
    "SOFTWAKE_MICROSOFT_CLIENT_ID",
    "MEETREC_GOOGLE_CLIENT_ID",
    "MEETREC_GOOGLE_CLIENT_SECRET",
    "MEETREC_MICROSOFT_CLIENT_ID",
];

/// Merge allowlisted keys from `previous` into `rendered` when missing.
///
/// Used by unit tests and documented for the CT-side install script. Does not
/// log values.
#[must_use]
#[allow(dead_code, reason = "unit-tested; CT install uses equivalent shell")]
pub fn merge_preserved_node_env(previous: &str, rendered: &str) -> String {
    let mut out = rendered.trim_end().to_owned();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    for key in PRESERVED_NODE_ENV_KEYS {
        let prefix = format!("{key}=");
        if out.lines().any(|line| line.starts_with(&prefix)) {
            continue;
        }
        if let Some(line) = previous.lines().find(|line| line.starts_with(&prefix)) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Render the systemd unit for softwake-node.
#[must_use]
pub fn render_node_unit() -> String {
    "[Unit]
Description=Softwake Remote Agent companion (softwake-node)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=softwake
Group=softwake
EnvironmentFile=/etc/softwake-node.env
ExecStart=/usr/local/bin/softwake-node
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/softwake-node
PrivateTmp=true

[Install]
WantedBy=multi-user.target
"
    .to_string()
}

/// Probe Tailnet reachability: optional `tailscale ping`, SSH `BatchMode`, GET `/health`.
#[must_use]
pub fn probe_tailnet(agent: &RemoteAgentConfig, pairing_secret: Option<&str>) -> ProbeReport {
    let mut lines = Vec::new();
    let host = agent.tailscale_hostname.trim().trim_end_matches('/');
    if let Err(err) = assert_tailscale_host(host) {
        lines.push(format!("host=reject ({err})"));
        return ProbeReport { lines, ok: false };
    }
    lines.push(format!("host=ok ({host})"));

    match run_capture(
        "tailscale",
        &["ping", "-c", "1", "--timeout", "3s", host],
        Duration::from_secs(8),
    ) {
        Ok((0, _out, _err)) => lines.push("tailscale_ping=ok".into()),
        Ok((_, out, err)) => lines.push(format!(
            "tailscale_ping=fail ({})",
            trim_cmd_out(&out, &err)
        )),
        Err(_) => lines.push("tailscale_ping=skipped (no CLI)".into()),
    }

    let ssh_ok = match ssh_batch(agent, &["true"]) {
        Ok((0, _out, _err)) => {
            lines.push("ssh=ok".into());
            true
        }
        Ok((_, out, err)) => {
            lines.push(format!("ssh=fail ({})", trim_cmd_out(&out, &err)));
            false
        }
        Err(e) => {
            lines.push(format!("ssh=fail ({e})"));
            false
        }
    };

    let base = node_base_url(agent, DEFAULT_NODE_PORT);
    let health_ok = match http_get(&base, "/health", None) {
        Ok(raw) if status_ok(&raw) && raw.contains("companion") => {
            lines.push("health=ok".into());
            true
        }
        Ok(raw) if status_ok(&raw) => {
            lines.push("health=ok (body unexpected)".into());
            true
        }
        Ok(raw) => {
            lines.push(format!("health=bad ({})", status_line(&raw)));
            false
        }
        Err(e) => {
            lines.push(format!("health=fail ({e})"));
            false
        }
    };

    if let Some(secret) = pairing_secret.map(str::trim).filter(|s| !s.is_empty()) {
        match http_get(&base, "/v1/presence", Some(secret)) {
            Ok(raw) if status_ok(&raw) => lines.push("presence=ok".into()),
            Ok(raw) => lines.push(format!("presence={}", status_line(&raw))),
            Err(e) => lines.push(format!("presence=fail ({e})")),
        }
    } else {
        lines.push("presence=skipped (no pairing secret)".into());
    }

    ProbeReport {
        ok: ssh_ok || health_ok,
        lines,
    }
}

/// Install / reinstall softwake-node on the companion over Tailscale SSH.
///
/// # Errors
///
/// Returned as `InstallReport.ok = false` with lines; this function always returns a report.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "install steps are intentionally linear"
)]
pub fn install_companion(agent: &RemoteAgentConfig, pairing_secret: &str) -> InstallReport {
    let mut lines = Vec::new();
    let secret = pairing_secret.trim();
    if secret.is_empty() {
        lines.push("install=blocked (pairing secret required in bag)".into());
        return InstallReport {
            lines,
            ok: false,
            binary_source: String::new(),
        };
    }
    let host = agent.tailscale_hostname.trim().trim_end_matches('/');
    if let Err(err) = assert_tailscale_host(host) {
        lines.push(format!("host=reject ({err})"));
        return InstallReport {
            lines,
            ok: false,
            binary_source: String::new(),
        };
    }

    let (bin, source) = match resolve_softwake_node_bin() {
        Ok(v) => v,
        Err(e) => {
            lines.push(format!("bin=fail ({e})"));
            return InstallReport {
                lines,
                ok: false,
                binary_source: String::new(),
            };
        }
    };
    lines.push(format!("bin={source}"));

    match ssh_batch(agent, &["true"]) {
        Ok((0, _, _)) => lines.push("ssh=ok".into()),
        Ok((_, out, err)) => {
            lines.push(format!(
                "ssh=fail ({}) — need BatchMode key auth",
                trim_cmd_out(&out, &err)
            ));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
        Err(e) => {
            lines.push(format!("ssh=fail ({e})"));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
    }

    let ts_ip = match ssh_batch(agent, &["tailscale", "ip", "-4"]) {
        Ok((0, out, _err)) => {
            let ip = out.lines().next().unwrap_or("").trim().to_owned();
            if !is_tailscale_host(&ip) {
                lines.push(format!("ts_ip=reject ({ip})"));
                return InstallReport {
                    lines,
                    ok: false,
                    binary_source: source,
                };
            }
            lines.push(format!("ts_ip={ip}"));
            ip
        }
        Ok((_, out, err)) => {
            lines.push(format!("ts_ip=fail ({})", trim_cmd_out(&out, &err)));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
        Err(e) => {
            lines.push(format!("ts_ip=fail ({e})"));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
    };

    // Bootstrap user + dirs (idempotent). Prefer root SSH; fall back to sudo -n.
    let bootstrap = r#"
set -e
if ! id -u softwake >/dev/null 2>&1; then
  if [ "$(id -u)" -eq 0 ]; then
    useradd --system --home /var/lib/softwake-node --shell /usr/sbin/nologin softwake || useradd --system --home /var/lib/softwake-node --shell /sbin/nologin softwake
  else
    sudo -n useradd --system --home /var/lib/softwake-node --shell /usr/sbin/nologin softwake
  fi
fi
if [ "$(id -u)" -eq 0 ]; then
  mkdir -p /var/lib/softwake-node
  chown softwake:softwake /var/lib/softwake-node
else
  sudo -n mkdir -p /var/lib/softwake-node
  sudo -n chown softwake:softwake /var/lib/softwake-node
fi
"#;
    match ssh_bash(agent, bootstrap) {
        Ok((0, _, _)) => lines.push("user=ok".into()),
        Ok((_, out, err)) => {
            lines.push(format!("user=fail ({})", trim_cmd_out(&out, &err)));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
        Err(e) => {
            lines.push(format!("user=fail ({e})"));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
    }

    // scp binary to /tmp then install into /usr/local/bin
    let remote_tmp = "/tmp/softwake-node.new";
    match scp_to(agent, &bin, remote_tmp) {
        Ok(()) => lines.push("scp=ok".into()),
        Err(e) => {
            lines.push(format!("scp=fail ({e})"));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
    }
    let install_bin = format!(
        r#"
set -e
if [ "$(id -u)" -eq 0 ]; then
  install -o root -g root -m 755 {remote_tmp} /usr/local/bin/softwake-node
  rm -f {remote_tmp}
else
  sudo -n install -o root -g root -m 755 {remote_tmp} /usr/local/bin/softwake-node
  rm -f {remote_tmp}
fi
"#
    );
    match ssh_bash(agent, &install_bin) {
        Ok((0, _, _)) => lines.push("install_bin=ok".into()),
        Ok((_, out, err)) => {
            lines.push(format!("install_bin=fail ({})", trim_cmd_out(&out, &err)));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
        Err(e) => {
            lines.push(format!("install_bin=fail ({e})"));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
    }

    let env_body = render_node_env(&ts_ip, secret);
    let unit_body = render_node_unit();
    // Write env + unit via base64 to avoid shell quoting of secrets in process list as much as practical.
    let env_b64 = base64_encode(env_body.as_bytes());
    let unit_b64 = base64_encode(unit_body.as_bytes());
    let write_files = format!(
        r#"
set -e
ENV_B64='{env_b64}'
UNIT_B64='{unit_b64}'
if [ "$(id -u)" -eq 0 ]; then
  PREV=""
  if [ -f /etc/softwake-node.env ]; then PREV=$(cat /etc/softwake-node.env); fi
  printf '%s' "$ENV_B64" | base64 -d > /tmp/softwake-node.env.new
  {{
    cat /tmp/softwake-node.env.new
    for key in SOFTWAKE_NODE_XAI_API_KEY SOFTWAKE_NODE_MODEL SOFTWAKE_GOOGLE_CLIENT_ID SOFTWAKE_GOOGLE_CLIENT_SECRET SOFTWAKE_MICROSOFT_CLIENT_ID MEETREC_GOOGLE_CLIENT_ID MEETREC_GOOGLE_CLIENT_SECRET MEETREC_MICROSOFT_CLIENT_ID; do
      if ! grep -q "^${{key}}=" /tmp/softwake-node.env.new 2>/dev/null; then
        echo "$PREV" | grep -E "^${{key}}=" || true
      fi
    done
  }} > /etc/softwake-node.env
  rm -f /tmp/softwake-node.env.new
  chown root:softwake /etc/softwake-node.env
  chmod 0640 /etc/softwake-node.env
  printf '%s' "$UNIT_B64" | base64 -d > /etc/systemd/system/softwake-node.service
  chmod 0644 /etc/systemd/system/softwake-node.service
  systemctl daemon-reload
  systemctl enable --now softwake-node
  systemctl restart softwake-node || true
else
  PREV=""
  if sudo -n test -f /etc/softwake-node.env; then PREV=$(sudo -n cat /etc/softwake-node.env); fi
  printf '%s' "$ENV_B64" | base64 -d > /tmp/softwake-node.env.new
  {{
    cat /tmp/softwake-node.env.new
    for key in SOFTWAKE_NODE_XAI_API_KEY SOFTWAKE_NODE_MODEL SOFTWAKE_GOOGLE_CLIENT_ID SOFTWAKE_GOOGLE_CLIENT_SECRET SOFTWAKE_MICROSOFT_CLIENT_ID MEETREC_GOOGLE_CLIENT_ID MEETREC_GOOGLE_CLIENT_SECRET MEETREC_MICROSOFT_CLIENT_ID; do
      if ! grep -q "^${{key}}=" /tmp/softwake-node.env.new 2>/dev/null; then
        echo "$PREV" | grep -E "^${{key}}=" || true
      fi
    done
  }} | sudo -n tee /etc/softwake-node.env >/dev/null
  rm -f /tmp/softwake-node.env.new
  sudo -n chown root:softwake /etc/softwake-node.env
  sudo -n chmod 0640 /etc/softwake-node.env
  printf '%s' "$UNIT_B64" | base64 -d | sudo -n tee /etc/systemd/system/softwake-node.service >/dev/null
  sudo -n chmod 0644 /etc/systemd/system/softwake-node.service
  sudo -n systemctl daemon-reload
  sudo -n systemctl enable --now softwake-node
  sudo -n systemctl restart softwake-node || true
fi
"#
    );
    match ssh_bash(agent, &write_files) {
        Ok((0, _, _)) => lines.push("systemd=ok".into()),
        Ok((_, out, err)) => {
            lines.push(format!("systemd=fail ({})", trim_cmd_out(&out, &err)));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
        Err(e) => {
            lines.push(format!("systemd=fail ({e})"));
            return InstallReport {
                lines,
                ok: false,
                binary_source: source,
            };
        }
    }

    std::thread::sleep(Duration::from_secs(1));
    let base = format!("http://{host}:{DEFAULT_NODE_PORT}");
    let health_ok = match http_get(&base, "/health", None) {
        Ok(raw) if status_ok(&raw) => {
            lines.push("health=ok".into());
            true
        }
        Ok(raw) => {
            lines.push(format!("health=bad ({})", status_line(&raw)));
            false
        }
        Err(e) => {
            lines.push(format!("health=fail ({e})"));
            false
        }
    };
    if health_ok {
        lines.push("install=ok (re-run Install companion to reinstall)".into());
    }

    InstallReport {
        lines,
        ok: health_ok,
        binary_source: source,
    }
}

/// Resolve a `softwake-node` binary to ship (env → PATH → build release).
///
/// # Errors
///
/// When no binary can be found or built.
pub fn resolve_softwake_node_bin() -> Result<(PathBuf, String), String> {
    if let Ok(p) = env::var("SOFTWAKE_NODE_BIN") {
        let path = PathBuf::from(p.trim());
        if path.is_file() {
            return Ok((path, "SOFTWAKE_NODE_BIN".into()));
        }
        return Err(format!("SOFTWAKE_NODE_BIN not a file: {}", path.display()));
    }
    if let Ok(path) = which("softwake-node") {
        return Ok((path, "PATH".into()));
    }
    let root = find_workspace_root()?;
    let release = root.join("target/release/softwake-node");
    if release.is_file() {
        return Ok((release, format!("workspace {}", root.display())));
    }
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "softwake-node",
            "--features",
            "live-http",
            "--locked",
        ])
        .current_dir(&root)
        .status()
        .map_err(|e| format!("cargo build: {e}"))?;
    if !status.success() {
        return Err(format!("cargo build -p softwake-node failed ({status})"));
    }
    if !release.is_file() {
        return Err(format!("built binary missing: {}", release.display()));
    }
    Ok((release, format!("cargo-build {}", root.display())))
}

fn find_workspace_root() -> Result<PathBuf, String> {
    if let Ok(p) = env::var("SOFTWAKE_REPO") {
        let root = PathBuf::from(p.trim());
        if root.join("crates/softwake-node/Cargo.toml").is_file() {
            return Ok(root);
        }
    }
    let mut dir = env::current_dir().map_err(|e| e.to_string())?;
    for _ in 0..12 {
        if dir.join("crates/softwake-node/Cargo.toml").is_file() && dir.join("Cargo.toml").is_file()
        {
            return Ok(dir);
        }
        if !dir.pop() {
            break;
        }
    }
    Err(
        "could not find Softwake workspace (set SOFTWAKE_NODE_BIN or SOFTWAKE_REPO, or run from repo)"
            .into(),
    )
}

fn which(name: &str) -> Result<PathBuf, ()> {
    let Ok(path_env) = env::var("PATH") else {
        return Err(());
    };
    for dir in env::split_paths(&path_env) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}

fn ssh_target(agent: &RemoteAgentConfig) -> String {
    format!(
        "{}@{}",
        agent.ssh_user.trim(),
        agent.tailscale_hostname.trim().trim_end_matches('/')
    )
}

fn ssh_batch(
    agent: &RemoteAgentConfig,
    remote_args: &[&str],
) -> Result<(i32, String, String), String> {
    let mut args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        format!("ConnectTimeout={SSH_CONNECT_TIMEOUT}"),
        ssh_target(agent),
    ];
    for a in remote_args {
        args.push((*a).to_owned());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_capture("ssh", &arg_refs, Duration::from_secs(20))
}

fn ssh_bash(agent: &RemoteAgentConfig, script: &str) -> Result<(i32, String, String), String> {
    let mut child = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            &format!("ConnectTimeout={SSH_CONNECT_TIMEOUT}"),
            &ssh_target(agent),
            "bash",
            "-s",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh spawn: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .map_err(|e| format!("ssh stdin: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("ssh wait: {e}"))?;
    Ok((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn scp_to(agent: &RemoteAgentConfig, local: &Path, remote_path: &str) -> Result<(), String> {
    let dest = format!("{}:{}", ssh_target(agent), remote_path);
    let (code, out, err) = run_capture(
        "scp",
        &[
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            &format!("ConnectTimeout={SSH_CONNECT_TIMEOUT}"),
            &local.display().to_string(),
            &dest,
        ],
        Duration::from_secs(120),
    )?;
    if code != 0 {
        return Err(trim_cmd_out(&out, &err));
    }
    Ok(())
}

fn run_capture(
    cmd: &str,
    args: &[&str],
    _timeout: Duration,
) -> Result<(i32, String, String), String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    Ok((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn trim_cmd_out(out: &str, err: &str) -> String {
    let s = if err.trim().is_empty() {
        out.trim()
    } else if out.trim().is_empty() {
        err.trim()
    } else {
        return format!("{} {}", out.trim(), err.trim())
            .chars()
            .take(160)
            .collect();
    };
    s.chars().take(160).collect()
}

fn http_get(base_url: &str, path: &str, secret: Option<&str>) -> Result<String, String> {
    let (host, port) = host_port(base_url).ok_or_else(|| "bad url".to_owned())?;
    let mut stream = TcpStream::connect((host.as_str(), port)).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(HTTP_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HTTP_TIMEOUT));
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(secret) = secret {
        let _ = write!(req, "Authorization: Bearer {secret}\r\n");
    }
    req.push_str("\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn host_port(base_url: &str) -> Option<(String, u16)> {
    let rest = base_url
        .strip_prefix("http://")
        .or_else(|| base_url.strip_prefix("https://"))?;
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h.to_owned(), p.parse().ok()?),
        None => (rest.to_owned(), DEFAULT_NODE_PORT),
    };
    Some((host, port))
}

fn status_ok(raw: &str) -> bool {
    raw.starts_with("HTTP/1.1 200") || raw.starts_with("HTTP/1.0 200")
}

fn status_line(raw: &str) -> String {
    raw.lines()
        .next()
        .unwrap_or("no-status")
        .chars()
        .take(80)
        .collect()
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n =
            (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8) | u32::from(bytes[i + 2]);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = u32::from(bytes[i]) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_agents::{RemoteAgentRoles, RemoteConflictPolicy};

    fn sample(host: &str) -> RemoteAgentConfig {
        RemoteAgentConfig {
            id: "ct".into(),
            name: "CT".into(),
            tailscale_hostname: host.into(),
            ssh_user: "root".into(),
            roles: RemoteAgentRoles::default(),
            conflict_policy: RemoteConflictPolicy::PreferLocal,
            enabled: true,
            oauth_mirror: false,
            created_ms: None,
            updated_ms: None,
        }
    }

    #[test]
    fn env_and_unit_render_safe() {
        let env = render_node_env("100.64.1.2", "sekrit");
        assert!(env.contains("SOFTWAKE_NODE_LISTEN=100.64.1.2:8790"));
        assert!(env.contains("SOFTWAKE_NODE_PAIRING_SECRET=sekrit"));
        assert!(!env.contains("0.0.0.0"));
        let unit = render_node_unit();
        assert!(unit.contains("User=softwake"));
        assert!(unit.contains("EnvironmentFile=/etc/softwake-node.env"));
        assert!(unit.contains("ExecStart=/usr/local/bin/softwake-node"));
    }

    #[test]
    fn install_refuses_public_host_before_ssh() {
        let report = install_companion(&sample("8.8.8.8"), "sekrit");
        assert!(!report.ok);
        assert!(report.lines.iter().any(|l| l.contains("host=reject")));
    }

    #[test]
    fn install_refuses_empty_secret() {
        let report = install_companion(&sample("100.64.1.2"), "  ");
        assert!(!report.ok);
        assert!(report.lines.iter().any(|l| l.contains("pairing secret")));
    }

    #[test]
    fn probe_rejects_non_ts() {
        let report = probe_tailnet(&sample("1.2.3.4"), None);
        assert!(!report.ok);
        assert!(report.summary().contains("host=reject"));
    }

    #[test]
    fn merge_preserved_node_env_keeps_allowlisted() {
        let rendered = render_node_env("100.64.1.2", "sekrit");
        assert!(!rendered.contains("sentinel-xai"));
        let previous = "SOFTWAKE_NODE_XAI_API_KEY=sentinel-xai
FOO=drop-me
SOFTWAKE_GOOGLE_CLIENT_ID=sentinel-gid
";
        let merged = merge_preserved_node_env(previous, &rendered);
        assert!(merged.contains("SOFTWAKE_NODE_XAI_API_KEY=sentinel-xai"));
        assert!(merged.contains("SOFTWAKE_GOOGLE_CLIENT_ID=sentinel-gid"));
        assert!(!merged.contains("FOO=drop-me"));
        assert!(merged.contains("SOFTWAKE_NODE_LISTEN=100.64.1.2:"));
    }
}
