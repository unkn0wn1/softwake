//! One-shot client for `softwaked ctl`.
//!
//! Each invocation connects, sends one command, prints the status or the
//! error, and exits. Events that arrive before the response are skipped.

use std::path::Path;

use softwake_ipc::{CallError, Client, Command, Status};

/// Subcommand of `softwaked ctl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Protocol command for this subcommand.
    #[must_use]
    pub(crate) const fn command(self) -> Command {
        match self {
            Self::Status => Command::GetStatus,
            Self::Hibernate => Command::Hibernate,
            Self::Resume => Command::WakeFromUi,
            Self::Sleep => Command::Sleep,
            Self::ReloadSoul => Command::ReloadSoul,
        }
    }
}

/// Connect, apply `action`, and return the text `ctl` prints.
///
/// # Errors
///
/// Returns [`CallError`] when the daemon cannot be reached or rejects the command.
pub(crate) fn run(path: &Path, action: CtlAction) -> Result<String, CallError> {
    Ok(format_status(&call(path, action.command())?))
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
    let mut text = format!(
        "state: {}\ncapture: {capture}\nsoul reload: {reload}\n",
        status.state
    );
    if let Some(message) = &status.message {
        text.push_str(message);
        text.push('\n');
    }
    text
}
