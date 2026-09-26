//! One-shot client for `softwaked ctl`.
//!
//! Each invocation connects, sends one command, prints the status or the
//! error, and exits. Events that arrive before the response are skipped.

use std::path::Path;

use std::fmt::Write;

use softwake_ipc::{CallError, Client, Command, Status};

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
    let status = match action {
        CtlAction::Status => call(path, Command::GetStatus)?,
        CtlAction::Hibernate => call(path, Command::Hibernate)?,
        CtlAction::Resume => call(path, Command::WakeFromUi)?,
        CtlAction::Wake => call_wake(path)?,
        CtlAction::Sleep => call(path, Command::Sleep)?,
        CtlAction::ReloadSoul => call(path, Command::ReloadSoul)?,
        CtlAction::Tool { name, args } => call_tool(path, name, args)?,
        CtlAction::ConfirmTool { pending_id } => call_confirm(path, pending_id)?,
        CtlAction::CancelTool { pending_id } => call_cancel(path, pending_id)?,
        CtlAction::Ask { text } | CtlAction::Chat { text } => call_ask(path, text)?,
        CtlAction::VoiceTest { enabled } => match enabled {
            None => call(path, Command::GetStatus)?,
            Some(enabled) => call_voice_test(path, *enabled)?,
        },
    };
    Ok(format_status(&status))
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
#[must_use]
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
