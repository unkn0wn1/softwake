//! Confirm-gated software install proposals (ADR-0052).
//!
//! Softwake asks before package installs. `allow_all` does **not** auto-approve
//! this tool unless the operator sets it to Always allow.

use std::process::{Command, Stdio};
use std::time::Duration;

use crate::shell::{
    DEFAULT_OUTPUT_CAP, ShellError, ShellOutput, format_shell_output, run_shell_in,
};

/// Tool bus name.
pub const SOFTWARE_INSTALL_TOOL: &str = "software_install";

/// True when `command` looks like a host package / global toolchain install.
#[must_use]
pub fn looks_like_software_install(command: &str) -> bool {
    let line = command.trim().to_ascii_lowercase();
    if line.is_empty() {
        return false;
    }
    // Strip simple leading env assignments: FOO=bar apt install …
    let stripped = strip_leading_assignments(&line);
    let tokens: Vec<&str> = stripped.split_whitespace().collect();
    if tokens.is_empty() {
        return false;
    }
    // sudo apt-get install …
    let start = usize::from(tokens[0] == "sudo");
    if start >= tokens.len() {
        return false;
    }
    let head = tokens[start];
    let rest = &tokens[start + 1..];
    match head {
        "apt" | "apt-get" | "dnf" | "yum" | "zypper" | "pip" | "pip3" => rest.contains(&"install"),
        "pacman" => rest
            .iter()
            .any(|t| *t == "-S" || *t == "-Sy" || *t == "-Syu"),
        "brew" => rest.first().is_some_and(|t| *t == "install"),
        "cargo" => {
            rest.first().is_some_and(|t| *t == "install")
                && !rest
                    .iter()
                    .any(|t| *t == "--path" || t.starts_with("--path="))
        }
        "npm" | "pnpm" | "yarn" => {
            // global installs only
            rest.iter().any(|t| *t == "-g" || *t == "--global")
                && rest
                    .iter()
                    .any(|t| *t == "install" || *t == "i" || *t == "add")
        }
        "flatpak" => rest.first().is_some_and(|t| *t == "install"),
        "snap" => rest.first().is_some_and(|t| *t == "install"),
        _ => false,
    }
}

fn strip_leading_assignments(line: &str) -> &str {
    let mut s = line;
    loop {
        let trimmed = s.trim_start();
        let Some((key, rest)) = trimmed.split_once('=') else {
            return trimmed;
        };
        if key.is_empty() || key.contains(' ') || key.contains('/') {
            return trimmed;
        }
        // value until whitespace
        let rest = rest.trim_start();
        if let Some(idx) = rest.find(char::is_whitespace) {
            s = &rest[idx..];
        } else {
            return "";
        }
    }
}

/// Parse positional args: optional manager hint + command/packages summary.
///
/// Forms:
/// - `["apt install ripgrep"]`
/// - `["apt", "install ripgrep"]`
#[must_use]
pub fn parse_software_install_args(args: &[String]) -> String {
    args.join(" ").trim().to_owned()
}

/// Run an already-confirmed install command in `cwd` with env (same as shell home pin).
///
/// # Errors
///
/// [`ShellError`].
pub fn run_software_install(
    command: &str,
    cwd: Option<&std::path::Path>,
    extra_env: &[(&str, &str)],
) -> Result<ShellOutput, ShellError> {
    // Longer timeout than default shell — installs can be slow.
    let timeout = Duration::from_secs(600);
    run_shell_in(command, cwd, extra_env, timeout, DEFAULT_OUTPUT_CAP)
}

/// Format install output for Hands detail.
#[must_use]
pub fn format_install_output(output: &ShellOutput) -> String {
    format!("software_install: {}", format_shell_output(output))
}

/// Whether `grok` appears to be on PATH (for coding backend selection).
#[must_use]
pub fn grok_cli_available() -> bool {
    Command::new("grok")
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_installers() {
        assert!(looks_like_software_install("apt install ripgrep"));
        assert!(looks_like_software_install("sudo apt-get install -y foo"));
        assert!(looks_like_software_install("brew install jq"));
        assert!(looks_like_software_install("cargo install ripgrep"));
        assert!(looks_like_software_install("npm install -g typescript"));
        assert!(looks_like_software_install("FOO=1 apt install bar"));
        assert!(!looks_like_software_install("cargo install --path ."));
        assert!(!looks_like_software_install("npm install lodash"));
        assert!(!looks_like_software_install("echo hello"));
        assert!(!looks_like_software_install(""));
    }
}
