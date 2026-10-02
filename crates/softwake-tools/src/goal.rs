//! First-class goal-oriented outer loop (ADR-0052).
//!
//! Any profile / room may invoke `goal_run`: define goal + measurable acceptance,
//! then plan → (human gate) → execute → verify until happy, cancelled, failed,
//! or the iteration cap. Softwake owns caps, gates, and progress logs. Backends
//! are `softwake` (default) or `grok_cli` (coding profiles when `grok` is on PATH).

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::shell::{DEFAULT_OUTPUT_CAP, ShellOutput, run_shell_in};
use crate::software_install::grok_cli_available;

/// Tool bus name.
pub const GOAL_RUN_TOOL: &str = "goal_run";

/// Default iteration cap.
pub const GOAL_DEFAULT_MAX_ITERATIONS: u32 = 8;

/// Hard ceiling for `max_iterations`.
pub const GOAL_HARD_MAX_ITERATIONS: u32 = 32;

/// Goal loop backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalBackend {
    /// Softwake oneshot plan + Hands tools + shell verify.
    Softwake,
    /// Local `grok` CLI plan/execute inside Softwake's outer loop.
    GrokCli,
}

impl GoalBackend {
    /// Stable spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Softwake => "softwake",
            Self::GrokCli => "grok_cli",
        }
    }
}

/// How the loop stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStopReason {
    /// Acceptance checks all passed.
    Success,
    /// Hit max iterations without success.
    Exhausted,
    /// Operator cancelled / denied a gate.
    Cancelled,
    /// Unrecoverable failure (empty acceptance, backend missing, …).
    Failed,
}

impl GoalStopReason {
    /// Stable spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Exhausted => "exhausted",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// Parsed `goal_run` arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalRunArgs {
    /// Prose goal.
    pub goal: String,
    /// Measurable acceptance (newline-separated shell checks in v1).
    pub acceptance: String,
    /// Iteration cap (clamped).
    pub max_iterations: u32,
    /// Backend.
    pub backend: GoalBackend,
    /// Optional room for progress.
    pub room_id: Option<String>,
    /// Owning profile id (optional; daemon fills from context).
    pub profile_id: Option<String>,
}

/// One progress event from the loop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalProgressEvent {
    /// 1-based iteration index (0 = setup).
    pub iteration: u32,
    /// Phase name.
    pub phase: String,
    /// Short summary.
    pub summary: String,
}

/// Loop result summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalRunResult {
    /// Stop reason.
    pub reason: GoalStopReason,
    /// Iterations completed (verify attempts).
    pub iterations: u32,
    /// Backend used.
    pub backend: String,
    /// Progress events.
    pub events: Vec<GoalProgressEvent>,
    /// Last plan text (if any).
    #[serde(default)]
    pub last_plan: String,
    /// Last verify detail.
    #[serde(default)]
    pub last_verify: String,
    /// Final operator-facing detail line.
    pub detail: String,
}

/// Clamp iterations into `1..=GOAL_HARD_MAX_ITERATIONS`.
#[must_use]
pub fn clamp_goal_iterations(raw: u32) -> u32 {
    raw.clamp(1, GOAL_HARD_MAX_ITERATIONS)
}

/// Parse positional / joined args for `goal_run`.
///
/// Preferred single JSON object string, or key lines:
/// `goal=…`, `acceptance=…`, optional `max_iterations=`, `backend=`, `room_id=`, `profile_id=`.
///
/// # Errors
///
/// Missing goal/acceptance or bad JSON.
pub fn parse_goal_run_args(args: &[String]) -> Result<GoalRunArgs, String> {
    let joined = args.join(" ").trim().to_owned();
    if joined.is_empty() {
        return Err("goal_run needs goal and acceptance".into());
    }
    if joined.starts_with('{') {
        return parse_goal_json(&joined);
    }
    let mut goal = String::new();
    let mut acceptance = String::new();
    let mut max_iterations = GOAL_DEFAULT_MAX_ITERATIONS;
    let mut backend = GoalBackend::Softwake;
    let mut room_id = None;
    let mut profile_id = None;
    for part in split_keyed(&joined) {
        if let Some((k, v)) = part.split_once('=') {
            match k.trim().to_ascii_lowercase().as_str() {
                "goal" => v.clone_into(&mut goal),
                "acceptance" => v.clone_into(&mut acceptance),
                "max_iterations" | "max" => {
                    max_iterations = v
                        .trim()
                        .parse::<u32>()
                        .unwrap_or(GOAL_DEFAULT_MAX_ITERATIONS);
                }
                "backend" => backend = parse_backend(v),
                "room_id" | "room" => room_id = Some(v.trim().to_owned()),
                "profile_id" | "profile" => profile_id = Some(v.trim().to_owned()),
                _ => {}
            }
        }
    }
    // Fallback: first arg goal, second acceptance when two plain args.
    if goal.is_empty() && args.len() >= 2 && !args[0].contains('=') {
        goal.clone_from(&args[0]);
        acceptance.clone_from(&args[1]);
        if let Some(m) = args.get(2) {
            if let Ok(n) = m.parse::<u32>() {
                max_iterations = n;
            }
        }
    }
    if goal.trim().is_empty() || acceptance.trim().is_empty() {
        return Err("goal_run needs non-empty goal and acceptance".into());
    }
    Ok(GoalRunArgs {
        goal: goal.trim().to_owned(),
        acceptance: acceptance.trim().to_owned(),
        max_iterations: clamp_goal_iterations(max_iterations),
        backend,
        room_id: room_id.filter(|s| !s.is_empty()),
        profile_id: profile_id.filter(|s| !s.is_empty()),
    })
}

fn parse_goal_json(raw: &str) -> Result<GoalRunArgs, String> {
    #[derive(Deserialize)]
    struct Raw {
        goal: String,
        acceptance: String,
        #[serde(default)]
        max_iterations: Option<u32>,
        #[serde(default)]
        backend: Option<String>,
        #[serde(default)]
        room_id: Option<String>,
        #[serde(default)]
        profile_id: Option<String>,
    }
    let raw: Raw = serde_json::from_str(raw).map_err(|e| format!("goal_run JSON: {e}"))?;
    if raw.goal.trim().is_empty() || raw.acceptance.trim().is_empty() {
        return Err("goal_run needs non-empty goal and acceptance".into());
    }
    Ok(GoalRunArgs {
        goal: raw.goal.trim().to_owned(),
        acceptance: raw.acceptance.trim().to_owned(),
        max_iterations: clamp_goal_iterations(
            raw.max_iterations.unwrap_or(GOAL_DEFAULT_MAX_ITERATIONS),
        ),
        backend: raw
            .backend
            .as_deref()
            .map_or(GoalBackend::Softwake, parse_backend),
        room_id: raw.room_id.filter(|s| !s.is_empty()),
        profile_id: raw.profile_id.filter(|s| !s.is_empty()),
    })
}

fn parse_backend(value: &str) -> GoalBackend {
    match value.trim().to_ascii_lowercase().as_str() {
        "grok" | "grok_cli" | "grok-cli" => GoalBackend::GrokCli,
        _ => GoalBackend::Softwake,
    }
}

fn split_keyed(joined: &str) -> Vec<&str> {
    // Split on `%;%` or newlines for multi-field; else whole string as one blob handled above.
    if joined.contains('\n') {
        joined
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    } else if joined.contains("%;%") {
        joined
            .split("%;%")
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        // single line with goal=… acceptance=… — split on ` acceptance=` / ` max_iterations=` etc.
        let keys = [
            " acceptance=",
            " max_iterations=",
            " max=",
            " backend=",
            " room_id=",
            " room=",
            " profile_id=",
            " profile=",
        ];
        let mut indices: Vec<usize> = Vec::new();
        let lower = joined.to_ascii_lowercase();
        for key in keys {
            if let Some(i) = lower.find(&key.to_ascii_lowercase()) {
                indices.push(i);
            }
        }
        if indices.is_empty() {
            return vec![joined];
        }
        indices.sort_unstable();
        let mut parts = Vec::new();
        let mut start = 0;
        for i in indices {
            if i > start {
                parts.push(joined[start..i].trim());
            }
            // index points at the space before the next key
            start = i + 1;
        }
        parts.push(joined[start..].trim());
        parts.into_iter().filter(|p| !p.is_empty()).collect()
    }
}

/// Choose backend given explicit arg, profile coding flag, and PATH.
#[must_use]
pub fn select_goal_backend(explicit: GoalBackend, profile_is_coding: bool) -> GoalBackend {
    match explicit {
        GoalBackend::GrokCli => {
            if grok_cli_available() {
                GoalBackend::GrokCli
            } else {
                GoalBackend::Softwake
            }
        }
        GoalBackend::Softwake => {
            if profile_is_coding && grok_cli_available() {
                GoalBackend::GrokCli
            } else {
                GoalBackend::Softwake
            }
        }
    }
}

/// Split acceptance into individual shell check lines (ignore blank / `#` comments).
#[must_use]
pub fn acceptance_checks(acceptance: &str) -> Vec<String> {
    acceptance
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(ToOwned::to_owned)
        .collect()
}

/// Run acceptance checks in `cwd` with env. All must exit 0.
///
/// # Errors
///
/// Returns Err with combined detail when any check fails or spawn fails.
pub fn verify_acceptance(
    acceptance: &str,
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
) -> Result<String, String> {
    let checks = acceptance_checks(acceptance);
    if checks.is_empty() {
        return Err("acceptance has no runnable checks".into());
    }
    let mut ok_lines = Vec::new();
    for (idx, cmd) in checks.iter().enumerate() {
        let output = run_shell_in(
            cmd,
            cwd,
            extra_env,
            Duration::from_secs(120),
            DEFAULT_OUTPUT_CAP,
        )
        .map_err(|e| format!("check {}: {e}", idx + 1))?;
        if output.timed_out {
            return Err(format!("check {} timed out: {cmd}", idx + 1));
        }
        if output.exit_code != Some(0) {
            return Err(format_verify_failure(idx + 1, cmd, &output));
        }
        ok_lines.push(format!("OK {}: {cmd}", idx + 1));
    }
    Ok(ok_lines.join("\n"))
}

fn format_verify_failure(idx: usize, cmd: &str, output: &ShellOutput) -> String {
    let code = output
        .exit_code
        .map_or_else(|| "none".into(), |c| c.to_string());
    format!(
        "check {idx} failed (exit {code}): {cmd}\nstdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    )
}

/// Build a concise plan prompt for the softwake backend.
#[must_use]
pub fn softwake_plan_prompt(goal: &str, acceptance: &str, prior: Option<&str>) -> String {
    let mut s = format!(
        "You are planning a Softwake goal-loop iteration.\nGoal:\n{goal}\n\nAcceptance checks (each must exit 0 in the agent home):\n{acceptance}\n\nReply with a short numbered plan (max 8 steps) to satisfy acceptance. Prefer shell/tools inside the agent home. Do not include secrets.\n"
    );
    if let Some(prior) = prior {
        s.push_str("\nPrevious attempt evidence:\n");
        s.push_str(prior);
        s.push('\n');
    }
    s
}

/// Format a `GoalRunResult` as tool detail text.
#[must_use]
pub fn format_goal_result(result: &GoalRunResult) -> String {
    let mut lines = vec![format!(
        "goal_run {}: iterations={} backend={}",
        result.reason.as_str(),
        result.iterations,
        result.backend
    )];
    for ev in &result.events {
        lines.push(format!("  [{}:{}] {}", ev.iteration, ev.phase, ev.summary));
    }
    if !result.detail.is_empty() {
        lines.push(result.detail.clone());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_and_clamp() {
        let args = parse_goal_run_args(&[
            r#"{"goal":"ship","acceptance":"true","max_iterations":99,"backend":"grok_cli"}"#
                .into(),
        ])
        .unwrap();
        assert_eq!(args.goal, "ship");
        assert_eq!(args.max_iterations, GOAL_HARD_MAX_ITERATIONS);
        assert_eq!(args.backend, GoalBackend::GrokCli);
        assert_eq!(clamp_goal_iterations(0), 1);
    }

    #[test]
    fn verify_acceptance_runs_checks() {
        let dir = std::env::temp_dir().join(format!(
            "softwake-goal-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ok.txt"), b"hi").unwrap();
        let ok =
            verify_acceptance("test -f ok.txt\n# comment\ntrue", Some(dir.as_path()), &[]).unwrap();
        assert!(ok.contains("OK"));
        let err = verify_acceptance("test -f missing.txt", Some(dir.as_path()), &[]).unwrap_err();
        assert!(err.contains("failed"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_plain_args() {
        let args = parse_goal_run_args(&["build it".into(), "true".into()]).unwrap();
        assert_eq!(args.goal, "build it");
        assert_eq!(args.acceptance, "true");
    }
}
