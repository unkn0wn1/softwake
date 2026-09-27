//! One-shot client for `softwaked ctl`.
//!
//! Each invocation connects, sends one command, prints the status or the
//! error, and exits. Events that arrive before the response are skipped.

use std::path::Path;

use std::fmt::Write;

use softwake_ipc::{CallError, Client, Command, IpcError, Status};

/// Subcommand of `softwaked ctl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CtlAction {
    /// `ctl status`
    Status,
    /// `ctl hibernate`
    Hibernate,
    /// `ctl resume` — leaves hibernate and lands in sleep.
    Resume,
    /// `ctl wake` — enters awake from sleep when the soul pack is valid.
    Wake,
    /// `ctl sleep`
    Sleep,
    /// `ctl reload-soul`
    ReloadSoul,
    /// `ctl reload-kws`
    ReloadKws,
    /// `ctl reload-utterance`
    ReloadUtterance,
    /// `ctl reload-playback`
    ReloadPlayback,
    /// `ctl tool <name> [args...]`
    Tool {
        /// Tool name. Safe tools run while awake. Confirm-gated tools wait.
        name: String,
        /// Arguments forwarded to the tool.
        args: Vec<String>,
    },
    /// `ctl confirm-tool <pending_id>`
    ConfirmTool {
        /// Id returned with the pending confirmation.
        pending_id: String,
    },
    /// `ctl cancel-tool <pending_id>`
    CancelTool {
        /// Id returned with the pending confirmation.
        pending_id: String,
    },
    /// `ctl ask <text…>`
    Ask {
        /// User line. Blank text is rejected before connect.
        text: String,
    },
    /// `ctl chat <text…>` — same socket message as [`Self::Ask`].
    Chat {
        /// User line. Blank text is rejected before connect.
        text: String,
    },
    /// `ctl voice-test` with no argument prints the flag. `on` / `off` sets it.
    VoiceTest {
        /// `None` reads status. `Some` sets the flag for this daemon process.
        enabled: Option<bool>,
    },
    /// `ctl webhook status|enable|disable|port`
    Webhook {
        /// Subcommand payload.
        action: WebhookCtl,
    },
    /// `ctl webhook-secret set|generate|clear`
    WebhookSecret {
        /// Subcommand payload.
        action: WebhookSecretCtl,
    },
}

/// Local-disk webhook config ctl (no IPC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WebhookCtl {
    /// Print enabled / secret configured / bind.
    Status,
    /// Set `webhook_enabled` true.
    Enable,
    /// Set `webhook_enabled` false.
    Disable,
    /// Set `webhook_port`.
    Port {
        /// 1..=65535
        port: u16,
    },
}

/// Local-disk webhook secret ctl (no IPC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WebhookSecretCtl {
    /// Store the provided token.
    Set {
        /// Shared secret.
        token: String,
    },
    /// Generate a random secret, store it, print once.
    Generate,
    /// Clear the secret.
    Clear,
}

impl CtlAction {
    /// Parse a ctl subcommand.
    #[must_use]
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "status" => Some(Self::Status),
            "hibernate" => Some(Self::Hibernate),
            "resume" => Some(Self::Resume),
            "wake" => Some(Self::Wake),
            "sleep" => Some(Self::Sleep),
            "reload-soul" => Some(Self::ReloadSoul),
            "reload-kws" => Some(Self::ReloadKws),
            "reload-utterance" => Some(Self::ReloadUtterance),
            "reload-playback" => Some(Self::ReloadPlayback),
            _ => None,
        }
    }
}

/// Connect, apply `action`, and return the text `ctl` prints.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or rejects the command.
pub(crate) fn run(path: &Path, action: &CtlAction) -> Result<String, CallError> {
    match action {
        CtlAction::Webhook { action } => run_webhook_ctl(action)
            .map_err(|message| CallError::Rejected(IpcError::protocol(message))),
        CtlAction::WebhookSecret { action } => run_webhook_secret_ctl(action)
            .map_err(|message| CallError::Rejected(IpcError::protocol(message))),
        other => {
            let status = match other {
                CtlAction::Status => call(path, Command::GetStatus)?,
                CtlAction::Hibernate => call(path, Command::Hibernate)?,
                CtlAction::Resume => call(path, Command::WakeFromUi)?,
                CtlAction::Wake => call_wake(path)?,
                CtlAction::Sleep => call(path, Command::Sleep)?,
                CtlAction::ReloadSoul => call(path, Command::ReloadSoul)?,
                CtlAction::ReloadKws => call_reload_kws(path)?,
                CtlAction::ReloadUtterance => call_reload_utterance(path)?,
                CtlAction::ReloadPlayback => call_reload_playback(path)?,
                CtlAction::Tool { name, args } => call_tool(path, name, args)?,
                CtlAction::ConfirmTool { pending_id } => call_confirm(path, pending_id)?,
                CtlAction::CancelTool { pending_id } => call_cancel(path, pending_id)?,
                CtlAction::Ask { text } | CtlAction::Chat { text } => call_ask(path, text)?,
                CtlAction::VoiceTest { enabled } => match enabled {
                    None => call(path, Command::GetStatus)?,
                    Some(enabled) => call_voice_test(path, *enabled)?,
                },
                CtlAction::Webhook { .. } | CtlAction::WebhookSecret { .. } => unreachable!(),
            };
            Ok(format_status(&status))
        }
    }
}

/// Connect and apply one protocol command.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or rejects the command.
pub(crate) fn call(path: &Path, command: Command) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.call(command)
}

/// Connect and run one tool.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or refuses the tool.
pub(crate) fn call_tool(path: &Path, name: &str, args: &[String]) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.call_tool(name, args)
}

/// Connect and set voice test mode.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or rejects the update.
pub(crate) fn call_voice_test(path: &Path, enabled: bool) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.set_voice_test(enabled)
}

/// Connect and rebuild KWS thresholds from disk / env.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or rejects the reload.
pub(crate) fn call_reload_kws(path: &Path) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.reload_kws()
}

/// Connect and apply free-speech end silence from disk / env.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or rejects the reload.
pub(crate) fn call_reload_utterance(path: &Path) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.reload_utterance()
}

/// Connect and re-read the TTS playback reaper deadline from disk / env.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or rejects the reload.
pub(crate) fn call_reload_playback(path: &Path) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.reload_playback()
}

/// Connect and send one ask.
///
/// `ask` and `chat` both use this. The daemon must already be awake.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or refuses the turn.
pub(crate) fn call_ask(path: &Path, text: &str) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.call_ask(text)
}

/// Connect and send one wake.
///
/// The daemon enters awake only from sleep, and only when the loaded pack is valid.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or refuses awake.
pub(crate) fn call_wake(path: &Path) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.call_wake()
}

/// Connect and confirm one pending tool.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or refuses the confirm.
pub(crate) fn call_confirm(path: &Path, pending_id: &str) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.confirm_tool(pending_id)
}

/// Connect and cancel one pending tool.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or the id is unknown.
pub(crate) fn call_cancel(path: &Path, pending_id: &str) -> Result<Status, CallError> {
    let mut client = Client::connect(path)?;
    client.cancel_tool(pending_id)
}

/// Human-readable status. The string ends with a newline.
pub(crate) fn run_webhook_ctl(action: &WebhookCtl) -> Result<String, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let config_dir = softwake_soul::resolve_config_dir(xdg.as_deref(), home.as_deref())
        .map_err(|e| e.to_string())?;
    match action {
        WebhookCtl::Status => {
            let app = softwake_soul::load_app_config(&config_dir).unwrap_or_default();
            let port = crate::webhook::resolve_webhook_port(app.webhook_port);
            let secret_configured = webhook_secret_configured();
            Ok(format!(
                "webhook enabled: {}\nwebhook secret: {}\nwebhook bind: 127.0.0.1:{port}\nwebhook path: POST /v1/wake\n",
                if app.webhook_enabled { "yes" } else { "no" },
                if secret_configured {
                    "configured"
                } else {
                    "missing"
                },
            ))
        }
        WebhookCtl::Enable => {
            softwake_soul::set_webhook_enabled(&config_dir, true).map_err(|e| e.to_string())?;
            Ok("webhook enabled: yes\n".to_owned())
        }
        WebhookCtl::Disable => {
            softwake_soul::set_webhook_enabled(&config_dir, false).map_err(|e| e.to_string())?;
            Ok("webhook enabled: no\n".to_owned())
        }
        WebhookCtl::Port { port } => {
            softwake_soul::set_webhook_port(&config_dir, *port).map_err(|e| e.to_string())?;
            Ok(format!("webhook port: {port}\n"))
        }
    }
}

fn run_webhook_secret_ctl(action: &WebhookSecretCtl) -> Result<String, String> {
    let path = softwake_providers::resolve_secrets_file().map_err(|e| e.to_string())?;
    let store = softwake_providers::open_store(&path).map_err(|e| e.to_string())?;
    match action {
        WebhookSecretCtl::Set { token } => {
            let token = token.trim();
            if token.is_empty() {
                return Err("webhook secret must be non-empty".into());
            }
            softwake_providers::update_bag(store.as_ref(), |bag| {
                bag.webhook_secret = Some(token.to_owned());
            })
            .map_err(|e| e.to_string())?;
            Ok("webhook secret: set\n".to_owned())
        }
        WebhookSecretCtl::Generate => {
            let token = generate_webhook_secret()?;
            softwake_providers::update_bag(store.as_ref(), |bag| {
                bag.webhook_secret = Some(token.clone());
            })
            .map_err(|e| e.to_string())?;
            Ok(format!(
                "webhook secret: generated (store this; it will not be shown again)\n{token}\n"
            ))
        }
        WebhookSecretCtl::Clear => {
            softwake_providers::update_bag(store.as_ref(), |bag| {
                bag.webhook_secret = None;
            })
            .map_err(|e| e.to_string())?;
            Ok("webhook secret: cleared\n".to_owned())
        }
    }
}

fn webhook_secret_configured() -> bool {
    let Ok(path) = softwake_providers::resolve_secrets_file() else {
        return false;
    };
    let Ok(store) = softwake_providers::open_store(&path) else {
        return false;
    };
    let Ok(bag) = store.load() else {
        return false;
    };
    bag.webhook_secret
        .as_ref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn generate_webhook_secret() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    fill_random(&mut bytes)?;
    Ok(base64url_nopad(&bytes))
}

fn fill_random(bytes: &mut [u8]) -> Result<(), String> {
    use std::fs::File;
    use std::io::Read;
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .map_err(|error| format!("urandom: {error}"))
}

fn base64url_nopad(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
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
    } else if rem == 2 {
        let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
    }
    out
}

pub(crate) fn format_status(status: &Status) -> String {
    let capture = if status.capture_running {
        "running"
    } else {
        "stopped"
    };
    let reload = if status.soul_reload_pending {
        "pending"
    } else {
        "not pending"
    };
    let soul = match &status.soul {
        Some(report) if report.ok => "ok".to_owned(),
        Some(report) => match &report.reason {
            Some(reason) => format!("missing — {reason}"),
            None => "missing".to_owned(),
        },
        None => "unknown".to_owned(),
    };
    let voice_test = if status.voice_test { "on" } else { "off" };
    let mut text = format!(
        "state: {}\ncapture: {capture}\nsoul: {soul}\nsoul reload: {reload}\nvoice test: {voice_test}\n",
        status.state
    );
    if let Some(level) = status.capture_level {
        let _ = writeln!(text, "capture level: {level:.3}");
    }
    if let Some(pending) = &status.pending_tool {
        text.push_str("pending: ");
        text.push_str(&pending.pending_id);
        text.push(' ');
        text.push_str(&pending.name);
        if !pending.args.is_empty() {
            text.push(' ');
            text.push_str(&pending.args.join(" "));
        }
        text.push('\n');
    }
    if let Some(last) = &status.last_tool {
        text.push_str("last tool: ");
        text.push_str(last);
        text.push('\n');
    }
    if let Some(message) = &status.message {
        text.push_str(message);
        text.push('\n');
    }
    text
}
