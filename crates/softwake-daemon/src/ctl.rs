//! One-shot client for `softwaked ctl`.
//!
//! Each invocation connects, sends one command, prints the status or the
//! error, and exits. Events that arrive before the response are skipped.

use std::path::Path;

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
    /// `ctl sleep`
    Sleep,
    /// `ctl reload-soul`
    ReloadSoul,
    /// `ctl tool <name> [args...]`
    Tool {
        /// Tool name. The phase-1 allowlist accepts `echo`.
        name: String,
        /// Arguments forwarded to the tool.
        args: Vec<String>,
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
        CtlAction::Sleep => call(path, Command::Sleep)?,
        CtlAction::ReloadSoul => call(path, Command::ReloadSoul)?,
        CtlAction::Tool { name, args } => call_tool(path, name, args)?,
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
    let mut text = format!(
        "state: {}\ncapture: {capture}\nsoul: {soul}\nsoul reload: {reload}\n",
        status.state
    );
    if let Some(message) = &status.message {
        text.push_str(message);
        text.push('\n');
    }
    text
}
