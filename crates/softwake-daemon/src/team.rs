//! Agent team helpers: peer DM wake + `goal_run` outer loop (ADR-0052).

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use softwake_soul::{
    ProfileMeta, ensure_profile_home, list_profiles, load_profile_meta, profile_pack_dir,
    resolve_config_dir, try_load_effective,
};
use softwake_tools::{
    AGENT_MESSAGE_TOOL, AgentMessageArgs, GOAL_RUN_TOOL, GoalBackend, GoalProgressEvent,
    GoalRunArgs, GoalRunResult, GoalStopReason, ROOM_COOLDOWN_MS, RoomLogKind, RoomLogLine,
    append_room_log, format_goal_result, mark_room_turn, resolve_rooms_dir,
    resolve_rooms_state_dir, room_cooldown_elapsed, select_goal_backend, softwake_plan_prompt,
    verify_acceptance,
};

use crate::runtime::Runtime;

/// Resolve profile meta by id or case-insensitive name.
pub(crate) fn resolve_profile_meta(to: &str) -> Result<ProfileMeta, String> {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())?;
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    let needle = to.trim();
    if let Some(meta) = profiles.iter().find(|p| p.id == needle) {
        return Ok(meta.clone());
    }
    let lower = needle.to_ascii_lowercase();
    profiles
        .into_iter()
        .find(|p| p.name.to_ascii_lowercase() == lower)
        .ok_or_else(|| format!("unknown profile `{needle}`"))
}

/// Load rendered instructions for a profile pack (global flags applied).
pub(crate) fn load_profile_instructions(profile_id: &str) -> Result<(ProfileMeta, String), String> {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())?;
    let dir = profile_pack_dir(&config, profile_id);
    if !dir.is_dir() {
        return Err(format!("unknown profile `{profile_id}`"));
    }
    let meta = load_profile_meta(&dir);
    let pack = try_load_effective(&config, profile_id).map_err(|e| e.to_string())?;
    let name = if meta.name.trim().is_empty() {
        softwake_soul::DEFAULT_AGENT_NAME.to_owned()
    } else {
        meta.name.clone()
    };
    Ok((meta, pack.render_instructions_as(&name)))
}

fn rooms_dirs() -> Result<(PathBuf, PathBuf), String> {
    let xdg_c = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let xdg_s = env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let rooms = resolve_rooms_dir(xdg_c.as_deref(), home.as_deref())?;
    let state = resolve_rooms_state_dir(xdg_s.as_deref(), home.as_deref())?;
    Ok((rooms, state))
}

fn home_env(profile_id: &str) -> Result<(PathBuf, Vec<(String, String)>), String> {
    let home = ensure_profile_home(profile_id).map_err(|e| e.to_string())?;
    let home_s = home.to_string_lossy().into_owned();
    let env_pairs = vec![
        ("HOME".to_owned(), home_s.clone()),
        ("SOFTWAKE_AGENT_HOME".to_owned(), home_s.clone()),
        ("SOFTWAKE_PROFILE_ID".to_owned(), profile_id.to_owned()),
    ];
    Ok((home, env_pairs))
}

fn log_room(
    room_id: Option<&str>,
    profile_id: &str,
    name: &str,
    kind: RoomLogKind,
    text: &str,
    iteration: Option<u32>,
    phase: Option<&str>,
) {
    let Some(room_id) = room_id else {
        return;
    };
    let Ok((rooms_dir, state_dir)) = rooms_dirs() else {
        return;
    };
    let _ = append_room_log(
        &state_dir,
        room_id,
        &RoomLogLine {
            ts_ms: softwake_tools::now_ms(),
            profile_id: profile_id.to_owned(),
            name: name.to_owned(),
            kind,
            text: text.to_owned(),
            iteration,
            phase: phase.map(str::to_owned),
        },
    );
    let _ = mark_room_turn(&rooms_dir, room_id, profile_id);
}

/// Peer DM: wake target profile with a oneshot ask (does not steal `active_profile`).
pub(crate) fn run_agent_message(
    runtime: &mut Runtime,
    args: &AgentMessageArgs,
) -> Result<String, String> {
    let target = resolve_profile_meta(&args.to)?;
    let (meta, instructions) = load_profile_instructions(&target.id)?;
    let _ = ensure_profile_home(&target.id);
    // Cool-down when room-scoped
    if let Some(room_id) = args.room_id.as_deref() {
        let (rooms_dir, _) = rooms_dirs()?;
        let path = softwake_tools::room_file_path(&rooms_dir, room_id);
        if let Ok(room) = softwake_tools::load_room(&path) {
            if !room.members.iter().any(|m| m == &target.id) {
                return Err(format!(
                    "profile `{}` is not a member of room `{room_id}`",
                    target.id
                ));
            }
            let now = softwake_tools::now_ms();
            if !room_cooldown_elapsed(room.last_turn_ms, now, ROOM_COOLDOWN_MS) {
                return Err(format!(
                    "room `{room_id}` cool-down active ({ROOM_COOLDOWN_MS} ms); try again shortly"
                ));
            }
        }
    }
    let reply =
        runtime.oneshot_as_profile(&target.id, &instructions, meta.allow_all, &args.text)?;
    let name = if meta.name.trim().is_empty() {
        softwake_soul::DEFAULT_AGENT_NAME
    } else {
        meta.name.as_str()
    };
    log_room(
        args.room_id.as_deref(),
        &target.id,
        name,
        RoomLogKind::Dm,
        &format!("DM in: {}\nDM out: {reply}", args.text),
        None,
        None,
    );
    // Best-effort HUD for target
    let () = crate::hud_chat_write::append_exchange(&target.id, "peer", &args.text, name, &reply);
    Ok(format!(
        "{AGENT_MESSAGE_TOOL}: {} ({}) replied:\n{reply}",
        name, target.id
    ))
}

/// Goal loop owned by Softwake (caps, verify, progress). Backends: softwake | `grok_cli`.
#[allow(
    clippy::too_many_lines,
    reason = "goal loop phases stay in one function for ADR-0052 readability"
)]
pub(crate) fn run_goal(runtime: &mut Runtime, args: &GoalRunArgs) -> Result<String, String> {
    let profile_id = args
        .profile_id
        .clone()
        .or_else(crate::hud_chat_write::active_profile_id)
        .unwrap_or_else(|| "default".into());
    let (meta, instructions) = load_profile_instructions(&profile_id)?;
    let (home, env_owned) = home_env(&profile_id)?;
    let env_refs: Vec<(&str, &str)> = env_owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let backend = select_goal_backend(args.backend, meta.is_coding());
    let name = if meta.name.trim().is_empty() {
        softwake_soul::DEFAULT_AGENT_NAME.to_owned()
    } else {
        meta.name.clone()
    };

    let mut events = Vec::new();
    let mut last_plan = String::new();
    let mut last_verify = String::new();
    let mut prior: Option<String> = None;

    events.push(GoalProgressEvent {
        iteration: 0,
        phase: "start".into(),
        summary: format!(
            "goal_run backend={} max_iterations={} profile={}",
            backend.as_str(),
            args.max_iterations,
            profile_id
        ),
    });
    log_room(
        args.room_id.as_deref(),
        &profile_id,
        &name,
        RoomLogKind::GoalProgress,
        &events[0].summary,
        Some(0),
        Some("start"),
    );

    for iter in 1..=args.max_iterations {
        // Plan
        let plan = match backend {
            GoalBackend::Softwake => {
                let prompt = softwake_plan_prompt(&args.goal, &args.acceptance, prior.as_deref());
                runtime.oneshot_as_profile(&profile_id, &instructions, meta.allow_all, &prompt)?
            }
            GoalBackend::GrokCli => {
                grok_cli_plan(&args.goal, &args.acceptance, prior.as_deref(), &home)?
            }
        };
        last_plan.clone_from(&plan);
        let plan_summary: String = plan.chars().take(240).collect();
        events.push(GoalProgressEvent {
            iteration: iter,
            phase: "plan".into(),
            summary: plan_summary.clone(),
        });
        log_room(
            args.room_id.as_deref(),
            &profile_id,
            &name,
            RoomLogKind::GoalProgress,
            &plan_summary,
            Some(iter),
            Some("plan"),
        );

        // Execute: softwake asks model to run shell steps extracted; grok_cli execute prompt
        let exec_detail = match backend {
            GoalBackend::Softwake => {
                let exec_prompt = format!(
                    "Execute this plan in your agent home using tools/shell as needed. Do not invent secrets.\nGoal:\n{}\n\nPlan:\n{plan}\n\nWhen done, reply DONE.",
                    args.goal
                );
                runtime.oneshot_as_profile(
                    &profile_id,
                    &instructions,
                    meta.allow_all,
                    &exec_prompt,
                )?
            }
            GoalBackend::GrokCli => grok_cli_execute(&args.goal, &plan, &home)?,
        };
        events.push(GoalProgressEvent {
            iteration: iter,
            phase: "execute".into(),
            summary: exec_detail.chars().take(240).collect(),
        });
        log_room(
            args.room_id.as_deref(),
            &profile_id,
            &name,
            RoomLogKind::GoalProgress,
            &events.last().unwrap().summary,
            Some(iter),
            Some("execute"),
        );

        // Verify
        match verify_acceptance(&args.acceptance, Some(home.as_path()), &env_refs) {
            Ok(ok) => {
                last_verify.clone_from(&ok);
                events.push(GoalProgressEvent {
                    iteration: iter,
                    phase: "verify".into(),
                    summary: "acceptance passed".into(),
                });
                log_room(
                    args.room_id.as_deref(),
                    &profile_id,
                    &name,
                    RoomLogKind::GoalProgress,
                    "acceptance passed",
                    Some(iter),
                    Some("verify"),
                );
                let result = GoalRunResult {
                    reason: GoalStopReason::Success,
                    iterations: iter,
                    backend: backend.as_str().to_owned(),
                    events,
                    last_plan,
                    last_verify,
                    detail: format!("{GOAL_RUN_TOOL}: success after {iter} iteration(s)"),
                };
                return Ok(format_goal_result(&result));
            }
            Err(err) => {
                last_verify.clone_from(&err);
                prior = Some(format!(
                    "verify failed:\n{err}\nexecute said:\n{exec_detail}"
                ));
                events.push(GoalProgressEvent {
                    iteration: iter,
                    phase: "verify".into(),
                    summary: err.chars().take(240).collect(),
                });
                log_room(
                    args.room_id.as_deref(),
                    &profile_id,
                    &name,
                    RoomLogKind::GoalProgress,
                    &events.last().unwrap().summary,
                    Some(iter),
                    Some("verify"),
                );
            }
        }
    }

    let result = GoalRunResult {
        reason: GoalStopReason::Exhausted,
        iterations: args.max_iterations,
        backend: backend.as_str().to_owned(),
        events,
        last_plan,
        last_verify,
        detail: format!(
            "{GOAL_RUN_TOOL}: exhausted after {} iteration(s)",
            args.max_iterations
        ),
    };
    Ok(format_goal_result(&result))
}

fn grok_cli_plan(
    goal: &str,
    acceptance: &str,
    prior: Option<&str>,
    cwd: &std::path::Path,
) -> Result<String, String> {
    let mut prompt = softwake_plan_prompt(goal, acceptance, prior);
    prompt.push_str("\nWrite the plan only.");
    run_grok_prompt(&prompt, cwd, true)
}

fn grok_cli_execute(goal: &str, plan: &str, cwd: &std::path::Path) -> Result<String, String> {
    let prompt = format!(
        "Execute this Softwake goal plan in the current working directory (agent home).\nGoal:\n{goal}\n\nPlan:\n{plan}\n\nImplement until the plan steps are done. Prefer local files; ask before destructive host installs."
    );
    run_grok_prompt(&prompt, cwd, false)
}

fn run_grok_prompt(prompt: &str, cwd: &std::path::Path, plan_mode: bool) -> Result<String, String> {
    let mut cmd = Command::new("grok");
    cmd.arg("-m")
        .arg("grok-4.7")
        .arg("--effort")
        .arg("xhigh")
        .arg("-p")
        .arg(prompt)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if plan_mode {
        cmd.arg("--permission-mode").arg("plan");
    } else {
        cmd.arg("--always-approve");
    }
    let output = cmd
        .output()
        .map_err(|e| format!("grok_cli spawn failed: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !output.status.success() && stdout.is_empty() {
        return Err(format!(
            "grok_cli failed (status {:?}): {stderr}",
            output.status.code()
        ));
    }
    if stdout.is_empty() {
        Ok(stderr)
    } else {
        Ok(stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_path_compiles_helpers() {
        let _ = ROOM_COOLDOWN_MS;
    }
}
