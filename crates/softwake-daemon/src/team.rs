//! Agent team helpers: peer DM wake + `goal_run` outer loop (ADR-0052).

use std::env;
use std::fmt::Write;
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

/// Match a member token to a profile: exact id, case-insensitive id, then display name.
#[must_use]
pub(crate) fn find_profile_meta<'a>(
    profiles: &'a [ProfileMeta],
    needle: &str,
) -> Option<&'a ProfileMeta> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    if let Some(meta) = profiles.iter().find(|p| p.id == needle) {
        return Some(meta);
    }
    if let Some(meta) = profiles.iter().find(|p| p.id.eq_ignore_ascii_case(needle)) {
        return Some(meta);
    }
    let lower = needle.to_ascii_lowercase();
    profiles
        .iter()
        .find(|p| p.name.trim().to_ascii_lowercase() == lower)
}

/// Resolve profile meta by id or display name (case-insensitive).
pub(crate) fn resolve_profile_meta(to: &str) -> Result<ProfileMeta, String> {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())?;
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    let needle = to.trim();
    find_profile_meta(&profiles, needle)
        .cloned()
        .ok_or_else(|| format!("unknown profile `{needle}`"))
}

/// Wake order for a room post.
///
/// Every distinct member is included. Tokens may be profile ids or display names.
/// Resolution does not stop after the first match.
#[must_use]
pub(crate) fn resolve_fanout_members(
    raw_members: &[String],
    profiles: &[ProfileMeta],
) -> (Vec<ProfileMeta>, Vec<String>) {
    let mut resolved = Vec::new();
    let mut unknown = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in raw_members {
        let needle = raw.trim();
        if needle.is_empty() {
            continue;
        }
        match find_profile_meta(profiles, needle) {
            Some(meta) => {
                if seen.insert(meta.id.clone()) {
                    resolved.push(meta.clone());
                }
            }
            None => unknown.push(needle.to_owned()),
        }
    }
    (resolved, unknown)
}

fn room_lists_member(members: &[String], target: &ProfileMeta) -> bool {
    members.iter().any(|token| {
        let token = token.trim();
        !token.is_empty()
            && (token == target.id
                || token.eq_ignore_ascii_case(&target.id)
                || token.eq_ignore_ascii_case(target.name.trim()))
    })
}

fn is_no_reply(text: &str) -> bool {
    let lower = text
        .trim()
        .trim_matches(|c: char| c == '.' || c == '!' || c == '"')
        .to_ascii_lowercase();
    lower.is_empty() || lower == "no_reply" || lower == "no reply"
}

/// One in-flight room fan-out. Later posts wait so members do not interleave.
fn room_fanout_single_flight() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    let lock = LOCK.get_or_init(|| std::sync::Mutex::new(()));
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn wait_room_cooldown(path: &std::path::Path) {
    let now = softwake_tools::now_ms();
    if let Ok(room_now) = softwake_tools::load_room(path) {
        if !room_cooldown_elapsed(room_now.last_turn_ms, now, ROOM_COOLDOWN_MS) {
            let wait = ROOM_COOLDOWN_MS.saturating_sub(now.saturating_sub(room_now.last_turn_ms));
            if wait > 0 {
                std::thread::sleep(std::time::Duration::from_millis(wait));
            }
        }
    }
}

fn pause_between_members() {
    std::thread::sleep(std::time::Duration::from_millis(ROOM_COOLDOWN_MS));
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
    // Always mirror goal progress into the owning profile HUD (ADR-0052).
    if matches!(kind, RoomLogKind::GoalProgress) {
        let label = match (iteration, phase) {
            (Some(i), Some(p)) => format!("goal_run #{i} {p}: {text}"),
            (Some(i), None) => format!("goal_run #{i}: {text}"),
            (None, Some(p)) => format!("goal_run {p}: {text}"),
            (None, None) => format!("goal_run: {text}"),
        };
        crate::hud_chat_write::append_assistant_notice(profile_id, &label);
    }
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
            if !room_lists_member(&room.members, &target) {
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
        let plan_phase_label = if prior.is_some() { "revise" } else { "plan" };
        events.push(GoalProgressEvent {
            iteration: iter,
            phase: plan_phase_label.into(),
            summary: plan_summary.clone(),
        });
        log_room(
            args.room_id.as_deref(),
            &profile_id,
            &name,
            RoomLogKind::GoalProgress,
            &plan_summary,
            Some(iter),
            Some(plan_phase_label),
        );
        // Human gate: goal_run is Confirm-risk; starting the tool is the operator gate.
        // Log an explicit gate phase so room/HUD show the handoff before execute.
        log_room(
            args.room_id.as_deref(),
            &profile_id,
            &name,
            RoomLogKind::GoalProgress,
            "human gate cleared (goal_run confirmed); executing plan",
            Some(iter),
            Some("gate"),
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

/// Prepared member fan-out after the operator line is on disk.
///
/// [`commit_operator_room_post`] returns this so the IPC path can reply, then
/// [`run_room_fanout`] can run on a background thread.
#[derive(Debug, Clone)]
pub(crate) struct RoomFanoutJob {
    pub room_id: String,
    pub operator_text: String,
    pub title: String,
    pub members: Vec<ProfileMeta>,
    pub unknown: Vec<String>,
    pub recent_ctx: String,
    pub rooms_dir: PathBuf,
    pub state_dir: PathBuf,
    pub room_path: PathBuf,
}

/// Write the operator line and return immediately with an optional fan-out job.
///
/// Does **not** call the LLM. Member oneshots belong in [`run_room_fanout`].
pub(crate) fn commit_operator_room_post(
    room_id: &str,
    operator_text: &str,
) -> Result<(String, Option<RoomFanoutJob>), String> {
    let text = operator_text.trim();
    if text.is_empty() {
        return Err("room post needs text".into());
    }
    let (rooms_dir, state_dir) = rooms_dirs()?;
    let path = softwake_tools::room_file_path(&rooms_dir, room_id);
    let room = softwake_tools::load_room(&path)?;
    let recent = softwake_tools::tail_room_log(&state_dir, room_id, 40).unwrap_or_default();
    let recent_ctx: String = recent
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| {
            let who = if line.name.is_empty() {
                line.profile_id.as_str()
            } else {
                line.name.as_str()
            };
            format!("[{}] {who}: {}", line.kind.as_str(), line.text)
        })
        .collect::<Vec<_>>()
        .join("\n");

    append_room_log(
        &state_dir,
        room_id,
        &RoomLogLine {
            ts_ms: softwake_tools::now_ms(),
            profile_id: "operator".into(),
            name: "Operator".into(),
            kind: RoomLogKind::Say,
            text: text.to_owned(),
            iteration: None,
            phase: None,
        },
    )?;
    let _ = mark_room_turn(&rooms_dir, room_id, "operator");

    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let config = resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())?;
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    let (members, unknown) = resolve_fanout_members(&room.members, &profiles);
    let title = if room.title.is_empty() {
        room_id.to_owned()
    } else {
        room.title.clone()
    };
    let wake_n = members.len();
    let message =
        format!("room `{room_id}`: operator posted; waking {wake_n} member(s) in background");
    if members.is_empty() && unknown.is_empty() {
        return Ok((message, None));
    }
    let job = RoomFanoutJob {
        room_id: room_id.to_owned(),
        operator_text: text.to_owned(),
        title,
        members,
        unknown,
        recent_ctx,
        rooms_dir,
        state_dir,
        room_path: path,
    };
    Ok((message, Some(job)))
}

/// Sequential member oneshots with cool-down + single-flight.
///
/// `oneshot` must lock the runtime only for the duration of one LLM turn so
/// the UI/IPC path stays free. Replies append to the room log as they finish.
#[allow(
    clippy::too_many_lines,
    reason = "room fan-out + cool-down stay in one ADR-0052 flow"
)]
pub(crate) fn run_room_fanout<F>(job: &RoomFanoutJob, mut oneshot: F)
where
    F: FnMut(&str, &str, bool, &str) -> Result<String, String>,
{
    let _fanout_flight = room_fanout_single_flight();
    let room_id = job.room_id.as_str();
    let text = job.operator_text.as_str();
    let title = job.title.as_str();
    let mut replied: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for raw in &job.unknown {
        skipped.push(format!("{raw} (unknown profile)"));
    }
    let mut thread_notes = String::new();

    for (index, member) in job.members.iter().enumerate() {
        if index > 0 {
            pause_between_members();
        }
        wait_room_cooldown(&job.room_path);

        let member_id = member.id.as_str();
        let Ok((meta, instructions)) = load_profile_instructions(member_id) else {
            skipped.push(format!("{member_id} (unknown profile)"));
            continue;
        };
        let _ = softwake_soul::ensure_profile_home(member_id);
        let display = if meta.name.trim().is_empty() {
            softwake_soul::DEFAULT_AGENT_NAME.to_owned()
        } else {
            meta.name.clone()
        };
        let prior = if thread_notes.is_empty() {
            job.recent_ctx.clone()
        } else if job.recent_ctx.is_empty() {
            thread_notes.clone()
        } else {
            format!("{}\n{thread_notes}", job.recent_ctx)
        };
        let stimulus = format!(
            "You are in Softwake room `{room_id}` (title: {title}). You are {display} (profile id `{member_id}`).\n\nRecent room context:\n{prior}\n\nOperator message:\n{text}\n\nSoftwake wakes every room member, one at a time, with a cool-down between turns. Another member already speaking does not cancel your turn.\nDecide for yourself:\n- If you should reply, write your room message only (no preamble).\n- If you should stay quiet, reply with exactly: NO_REPLY"
        );
        let reply = match oneshot(member_id, &instructions, meta.allow_all, &stimulus) {
            Ok(r) => r,
            Err(error) => {
                skipped.push(format!("{member_id} (error: {error})"));
                continue;
            }
        };
        if is_no_reply(&reply) {
            skipped.push(format!("{member_id} (NO_REPLY)"));
            continue;
        }
        let trimmed = reply.trim();
        if let Err(error) = append_room_log(
            &job.state_dir,
            room_id,
            &RoomLogLine {
                ts_ms: softwake_tools::now_ms(),
                profile_id: member_id.to_owned(),
                name: display.clone(),
                kind: RoomLogKind::Say,
                text: trimmed.to_owned(),
                iteration: None,
                phase: None,
            },
        ) {
            skipped.push(format!("{member_id} (log error: {error})"));
            continue;
        }
        let _ = mark_room_turn(&job.rooms_dir, room_id, member_id);
        if !thread_notes.is_empty() {
            thread_notes.push('\n');
        }
        let _ = write!(thread_notes, "[say] {display}: {trimmed}");
        replied.push(format!("{display} ({member_id})"));
    }

    let replied_s = if replied.is_empty() {
        "(none)".to_owned()
    } else {
        replied.join(", ")
    };
    let skipped_s = if skipped.is_empty() {
        "(none)".to_owned()
    } else {
        skipped.join(", ")
    };
    eprintln!(
        "softwaked: room `{room_id}` fan-out done; woke {}; replied: {replied_s}; quiet: {skipped_s}",
        job.members.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_path_compiles_helpers() {
        let _ = ROOM_COOLDOWN_MS;
    }

    #[test]
    fn fanout_resolves_display_names_and_wakes_every_member() {
        let profiles = vec![
            ProfileMeta::new("p-sally", "Sally"),
            ProfileMeta::new("spencer", "Spencer"),
            ProfileMeta::new("nova", "Nova"),
        ];
        let raw = vec![
            "Sally".into(),
            "spencer".into(),
            "missing".into(),
            "NOVA".into(),
            "Sally".into(),
        ];
        let (resolved, unknown) = resolve_fanout_members(&raw, &profiles);
        let ids: Vec<_> = resolved.iter().map(|meta| meta.id.as_str()).collect();
        assert_eq!(ids, vec!["p-sally", "spencer", "nova"]);
        assert_eq!(unknown, vec!["missing".to_owned()]);
        assert!(
            resolved.len() > 1,
            "fan-out must not stop after the first speaker"
        );
    }

    #[test]
    fn fanout_matches_id_case_insensitively() {
        let profiles = vec![ProfileMeta::new("sally", "Agent Sally")];
        let (resolved, unknown) = resolve_fanout_members(&["Sally".into()], &profiles);
        assert!(unknown.is_empty());
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, "sally");
    }

    #[test]
    fn no_reply_token_is_quiet_not_a_message() {
        assert!(is_no_reply(""));
        assert!(is_no_reply("  NO_REPLY  "));
        assert!(is_no_reply("no reply."));
        assert!(!is_no_reply("NO_REPLY but actually here is a thought"));
    }

    #[test]
    fn room_membership_accepts_display_name() {
        let target = ProfileMeta::new("p-sally", "Sally");
        assert!(room_lists_member(
            &["Sally".into(), "spencer".into()],
            &target
        ));
        assert!(!room_lists_member(&["spencer".into()], &target));
    }
}
