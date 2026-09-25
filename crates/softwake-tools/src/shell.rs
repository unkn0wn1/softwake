//! Gated shell runner used after confirm.
//!
//! Spawns `/bin/sh -c` with a wall-clock timeout and stdout/stderr byte caps.
//! Does not log the command line (callers must not write secrets either).

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Default wall-clock timeout for one shell invocation.
pub const DEFAULT_SHELL_TIMEOUT: Duration = Duration::from_secs(30);

/// Max bytes kept from stdout (and separately from stderr).
pub const DEFAULT_OUTPUT_CAP: usize = 32 * 1024;

/// Result of one gated shell run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    /// Captured stdout, truncated to the cap.
    pub stdout: String,
    /// Captured stderr, truncated to the cap.
    pub stderr: String,
    /// Process exit code when it exited; `None` when killed on timeout.
    pub exit_code: Option<i32>,
    /// True when the process was killed after the timeout.
    pub timed_out: bool,
    /// True when stdout or stderr was truncated.
    pub truncated: bool,
}

/// Why a shell run did not start or finish cleanly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShellError {
    /// Empty command after trim.
    #[error("shell needs a command")]
    EmptyCommand,
    /// `sh` could not be spawned.
    #[error("shell spawn failed: {message}")]
    Spawn {
        /// Operator-safe OS error text (no command echo).
        message: String,
    },
}

/// Run `command` via `/bin/sh -c` with timeout and output caps.
///
/// # Errors
///
/// [`ShellError::EmptyCommand`] or [`ShellError::Spawn`].
pub fn run_shell(command: &str) -> Result<ShellOutput, ShellError> {
    run_shell_with(command, DEFAULT_SHELL_TIMEOUT, DEFAULT_OUTPUT_CAP)
}

/// Run with explicit timeout and per-stream cap.
///
/// # Errors
///
/// [`ShellError::EmptyCommand`] or [`ShellError::Spawn`].
pub fn run_shell_with(
    command: &str,
    timeout: Duration,
    output_cap: usize,
) -> Result<ShellOutput, ShellError> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(ShellError::EmptyCommand);
    }
    let mut command = Command::new("/bin/sh");
    let mut child = command
        .arg("-c")
        .arg(trimmed)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| ShellError::Spawn {
            message: error.to_string(),
        })?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (tx_out, rx_out) = mpsc::channel();
    let (tx_err, rx_err) = mpsc::channel();
    if let Some(mut pipe) = stdout_pipe {
        let cap = output_cap;
        thread::spawn(move || {
            let mut buf = Vec::new();
            let mut tmp = [0_u8; 4096];
            let mut truncated = false;
            loop {
                match pipe.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let room = cap.saturating_sub(buf.len());
                        if room == 0 {
                            truncated = true;
                            break;
                        }
                        let take = n.min(room);
                        buf.extend_from_slice(&tmp[..take]);
                        if take < n {
                            truncated = true;
                            break;
                        }
                    }
                }
            }
            let _ = tx_out.send((buf, truncated));
        });
    } else {
        let _ = tx_out.send((Vec::new(), false));
    }
    if let Some(mut pipe) = stderr_pipe {
        let cap = output_cap;
        thread::spawn(move || {
            let mut buf = Vec::new();
            let mut tmp = [0_u8; 4096];
            let mut truncated = false;
            loop {
                match pipe.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let room = cap.saturating_sub(buf.len());
                        if room == 0 {
                            truncated = true;
                            break;
                        }
                        let take = n.min(room);
                        buf.extend_from_slice(&tmp[..take]);
                        if take < n {
                            truncated = true;
                            break;
                        }
                    }
                }
            }
            let _ = tx_err.send((buf, truncated));
        });
    } else {
        let _ = tx_err.send((Vec::new(), false));
    }

    let (timed_out, exit_code) = wait_with_timeout(&mut child, timeout);
    // Bound pipe drains so a stuck reader cannot hang the daemon (or CI) forever.
    let drain_budget = Duration::from_secs(2);
    let (stdout_bytes, out_trunc) = rx_out.recv_timeout(drain_budget).unwrap_or_default();
    let (stderr_bytes, err_trunc) = rx_err.recv_timeout(drain_budget).unwrap_or_default();
    Ok(ShellOutput {
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        exit_code,
        timed_out,
        truncated: out_trunc || err_trunc,
    })
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> (bool, Option<i32>) {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (false, status.code()),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    kill_shell_tree(child);
                    return (true, None);
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                kill_shell_tree(child);
                return (true, None);
            }
        }
    }
}

fn kill_shell_tree(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Format a shell result for status / TTS. Does not include the command line.
#[must_use]
pub fn format_shell_output(output: &ShellOutput) -> String {
    let mut parts = Vec::new();
    if output.timed_out {
        parts.push("shell timed out".to_owned());
    } else if let Some(code) = output.exit_code {
        parts.push(format!("exit {code}"));
    }
    let stdout = output.stdout.trim();
    let stderr = output.stderr.trim();
    if !stdout.is_empty() {
        parts.push(stdout.to_owned());
    }
    if !stderr.is_empty() {
        parts.push(format!("stderr: {stderr}"));
    }
    if output.truncated {
        parts.push("(output truncated)".to_owned());
    }
    if parts.is_empty() {
        "shell finished (no output)".to_owned()
    } else {
        parts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellError, ShellOutput, format_shell_output, run_shell};

    #[test]
    fn empty_command_is_rejected() {
        assert_eq!(
            run_shell("  ").expect_err("empty"),
            ShellError::EmptyCommand
        );
    }

    #[test]
    fn echo_runs_and_captures_stdout() {
        let output = run_shell("echo hello-softwake").expect("run");
        assert!(!output.timed_out);
        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.contains("hello-softwake"));
        let text = format_shell_output(&output);
        assert!(text.contains("hello-softwake"));
        assert!(text.contains("exit 0"));
    }

    #[test]
    fn format_marks_timed_out_without_spawning() {
        let output = ShellOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timed_out: true,
            truncated: false,
        };
        assert!(format_shell_output(&output).contains("timed out"));
    }
}
