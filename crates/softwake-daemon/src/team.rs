//! Agent team helpers: peer DM wake + `goal_run` outer loop (ADR-0052).

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use crate::room_recheck::{FanoutSay, RecheckDecision, decision_from_oneshot};
use softwake_soul::{
    ProfileMeta, ensure_profile_home, list_profiles, load_app_config, load_profile_meta,
    profile_pack_dir, resolve_config_dir, try_load_effective,
};
use softwake_tools::{
    AGENT_MESSAGE_TOOL, AgentMessageArgs, GOAL_RUN_TOOL, GoalBackend, GoalProgressEvent,
    GoalRunArgs, GoalRunResult, GoalStopReason, ROOM_COOLDOWN_MS, RoomLogKind, RoomLogLine,
    append_room_log, format_goal_result, mark_room_turn, resolve_rooms_dir,
    resolve_rooms_state_dir, room_cooldown_elapsed, room_log_context_line, select_goal_backend,
    softwake_plan_prompt, verify_acceptance,
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
    crate::room_recheck::is_quiet_reply(text)
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
            to_profile_id: None,
            to_name: None,
            reply: None,
        },
    );
    let _ = mark_room_turn(&rooms_dir, room_id, profile_id);
}

/// Peer DM: wake target profile with a oneshot ask (does not steal `active_profile`).
pub(crate) fn run_agent_message(
    runtime: &mut Runtime,
    args: &AgentMessageArgs,
    sender_id: Option<&str>,
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
    let sender = sender_id.and_then(|id| resolve_profile_meta(id).ok());
    if sender.as_ref().is_some_and(|from| from.id == target.id) {
        return Err("agent_message cannot target the sending profile".into());
    }
    let reply =
        runtime.oneshot_as_profile(&target.id, &instructions, meta.allow_all, &args.text)?;
    let target_name = if meta.name.trim().is_empty() {
        softwake_soul::DEFAULT_AGENT_NAME.to_owned()
    } else {
        meta.name.clone()
    };
    let sender_name = sender.as_ref().map_or_else(
        || "Operator".to_owned(),
        |from| {
            if from.name.trim().is_empty() {
                softwake_soul::DEFAULT_AGENT_NAME.to_owned()
            } else {
                from.name.clone()
            }
        },
    );
    let sender_profile = sender.as_ref().map_or("operator", |from| from.id.as_str());
    let stored_reply = if is_no_reply(&reply) {
        None
    } else {
        Some(reply.trim().to_owned())
    };
    if let Some(room_id) = args.room_id.as_deref() {
        if let Ok((rooms_dir, state_dir)) = rooms_dirs() {
            let _ = append_room_log(
                &state_dir,
                room_id,
                &RoomLogLine {
                    ts_ms: softwake_tools::now_ms(),
                    profile_id: sender_profile.to_owned(),
                    name: sender_name.clone(),
                    kind: RoomLogKind::Dm,
                    text: args.text.clone(),
                    iteration: None,
                    phase: None,
                    to_profile_id: Some(target.id.clone()),
                    to_name: Some(target_name.clone()),
                    reply: stored_reply.clone(),
                },
            );
            let _ = mark_room_turn(&rooms_dir, room_id, sender_profile);
        }
    }
    crate::hud_chat_write::append_dm_histories(
        sender.as_ref().map(|from| from.id.as_str()),
        &sender_name,
        &target.id,
        &target_name,
        &args.text,
        stored_reply.as_deref(),
    );
    let shown = stored_reply.as_deref().unwrap_or("(no reply)");
    Ok(format!(
        "{AGENT_MESSAGE_TOOL}: {sender_name} sent a message to {target_name} ({}):\n{shown}",
        target.id
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

/// One room-member line waiting to play, or staged before it is appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomSpeechClip {
    pub text: String,
    pub voice: String,
}

/// Counts from [`RoomSpeechQueue::clear`]. `generation` is the value after the bump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomSpeechClearReport {
    /// Generation after the bump. A stamp from before `clear` fails `allows`.
    pub generation: u64,
    /// Clips removed from the waiting-audio list.
    pub dropped_audio: usize,
    /// Replies removed before they were appended to the room log.
    pub dropped_unposted: usize,
    /// Re-checks removed before their result was applied.
    pub dropped_rechecks: usize,
}

/// One re-check started for a clip that is still waiting.
#[derive(Debug)]
struct PendingRecheck {
    clip: RoomSpeechClip,
    started: Instant,
    decision: Option<RecheckDecision>,
}

/// Result of looking at one clip's re-check under the queue lock.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecheckPoll {
    /// Nothing was started. Play the draft.
    Idle,
    /// Started, and the wait budget has not elapsed.
    Pending,
    /// The model returned before the budget.
    Ready(RecheckDecision),
    /// The budget elapsed with no decision. Play the draft.
    TimedOut,
    /// `clear` bumped the generation. Do not play or rewrite.
    Stale,
}

/// In-memory queue of room-member speech that has not started playback.
///
/// Generation starts at 0. `clear` drops waiting audio, unposted replies, and
/// pending re-checks, then bumps the generation with `wrapping_add`. It takes
/// no log path and does no I/O. A public say is appended when that clip is
/// about to play, so a clip `clear` drops has no row. A line already appended
/// stays in the log.
#[derive(Debug, Default)]
pub(crate) struct RoomSpeechQueue {
    generation: u64,
    waiting_audio: Vec<RoomSpeechClip>,
    unposted: Vec<RoomSpeechClip>,
    pending_rechecks: Vec<PendingRecheck>,
}

impl RoomSpeechQueue {
    pub(crate) fn enqueue_waiting_audio(&mut self, clip: RoomSpeechClip) {
        self.waiting_audio.push(clip);
    }

    /// Push `clip` when `captured` is still current.
    ///
    /// Returns false and does not push when `captured` is stale.
    #[must_use]
    pub(crate) fn stage_unposted(&mut self, clip: &RoomSpeechClip, captured: u64) -> bool {
        if !self.allows(captured) {
            return false;
        }
        self.unposted.push(clip.clone());
        true
    }

    /// Drop waiting audio and unposted replies, then bump the generation.
    ///
    /// Does not spawn a player, kill a process, or rewrite a room log.
    #[must_use]
    pub(crate) fn clear(&mut self) -> RoomSpeechClearReport {
        let dropped_audio = self.waiting_audio.len();
        let dropped_unposted = self.unposted.len();
        let dropped_rechecks = self.pending_rechecks.len();
        self.waiting_audio.clear();
        self.unposted.clear();
        self.pending_rechecks.clear();
        self.generation = self.generation.wrapping_add(1);
        RoomSpeechClearReport {
            generation: self.generation,
            dropped_audio,
            dropped_unposted,
            dropped_rechecks,
        }
    }

    /// True when `captured` is the generation this queue still honors.
    #[must_use]
    pub(crate) const fn allows(&self, captured: u64) -> bool {
        captured == self.generation
    }

    #[must_use]
    pub(crate) fn unposted_contains(&self, clip: &RoomSpeechClip) -> bool {
        self.unposted.iter().any(|item| item == clip)
    }

    /// Remove one matching staged reply. False when `clear` already dropped it.
    #[must_use]
    pub(crate) fn take_unposted(&mut self, clip: &RoomSpeechClip) -> bool {
        take_matching(&mut self.unposted, clip)
    }

    /// Remove one matching waiting clip. False when `clear` already dropped it.
    #[must_use]
    pub(crate) fn take_waiting(&mut self, clip: &RoomSpeechClip) -> bool {
        let removed = take_matching(&mut self.waiting_audio, clip);
        if removed {
            let _ = take_pending(&mut self.pending_rechecks, clip);
        }
        removed
    }

    /// Start at most one re-check for a clip that is still waiting.
    fn begin_recheck(&mut self, clip: &RoomSpeechClip, speech_gen: u64, started: Instant) -> bool {
        if !self.allows(speech_gen) || !self.waiting_audio.iter().any(|item| item == clip) {
            return false;
        }
        if self.pending_rechecks.iter().any(|item| item.clip == *clip) {
            return false;
        }
        self.pending_rechecks.push(PendingRecheck {
            clip: clip.clone(),
            started,
            decision: None,
        });
        true
    }

    /// Store a decision only while this generation still owns the waiting clip.
    fn store_recheck(
        &mut self,
        clip: &RoomSpeechClip,
        speech_gen: u64,
        decision: RecheckDecision,
    ) -> bool {
        if !self.allows(speech_gen) || !self.waiting_audio.iter().any(|item| item == clip) {
            return false;
        }
        let Some(pending) = self
            .pending_rechecks
            .iter_mut()
            .find(|item| item.clip == *clip && item.decision.is_none())
        else {
            return false;
        };
        pending.decision = Some(decision);
        true
    }

    /// Read the re-check. Ready and timed-out remove the pending row.
    fn take_recheck(
        &mut self,
        clip: &RoomSpeechClip,
        speech_gen: u64,
        now: Instant,
    ) -> RecheckPoll {
        if !self.allows(speech_gen) {
            return RecheckPoll::Stale;
        }
        let Some(index) = self
            .pending_rechecks
            .iter()
            .position(|item| item.clip == *clip)
        else {
            return RecheckPoll::Idle;
        };
        if self.pending_rechecks[index].decision.is_some() {
            let pending = self.pending_rechecks.remove(index);
            if let Some(decision) = pending.decision {
                return RecheckPoll::Ready(decision);
            }
            return RecheckPoll::Pending;
        }
        let started = self.pending_rechecks[index].started;
        if now.saturating_duration_since(started) >= crate::room_recheck::ROOM_REQUEUE_RECHECK_WAIT
        {
            self.pending_rechecks.remove(index);
            return RecheckPoll::TimedOut;
        }
        RecheckPoll::Pending
    }

    fn rename_waiting(&mut self, clip: &RoomSpeechClip, new_text: &str) -> bool {
        let Some(slot) = self.waiting_audio.iter_mut().find(|item| *item == clip) else {
            return false;
        };
        new_text.clone_into(&mut slot.text);
        true
    }
}

fn take_matching(clips: &mut Vec<RoomSpeechClip>, clip: &RoomSpeechClip) -> bool {
    let Some(index) = clips.iter().position(|item| item == clip) else {
        return false;
    };
    clips.remove(index);
    true
}

fn take_pending(pending: &mut Vec<PendingRecheck>, clip: &RoomSpeechClip) -> bool {
    let Some(index) = pending.iter().position(|item| item.clip == *clip) else {
        return false;
    };
    pending.remove(index);
    true
}

fn room_speech_queue() -> std::sync::MutexGuard<'static, RoomSpeechQueue> {
    static LOCK: OnceLock<Mutex<RoomSpeechQueue>> = OnceLock::new();
    let lock = LOCK.get_or_init(|| Mutex::new(RoomSpeechQueue::default()));
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Clear queued member speech. Production also interrupts playback.
///
/// [`RoomSpeechQueue::clear`] does not touch the player. This wrapper does,
/// outside tests, so a clip that wins the race after the generation check
/// still dies. No log path: a line already appended stays. A clip that was
/// still waiting was not appended, so it leaves no row.
pub(crate) fn clear_room_speech_queue() {
    let _report = room_speech_queue().clear();
    #[cfg(not(test))]
    softwake_voice::interrupt_playback();
}

/// Generation stamped onto a fan-out when the operator line is committed.
///
/// Reading does not bump. [`clear_room_speech_queue`] bumps.
#[must_use]
pub(crate) fn room_speech_generation() -> u64 {
    room_speech_queue().generation
}

#[must_use]
pub(crate) fn room_speech_allows(captured: u64) -> bool {
    room_speech_queue().allows(captured)
}

fn stage_unposted_room_reply(clip: &RoomSpeechClip, captured: u64) -> bool {
    room_speech_queue().stage_unposted(clip, captured)
}

fn claim_unposted_room_reply(clip: &RoomSpeechClip, captured: u64) -> bool {
    let mut queue = room_speech_queue();
    if !queue.allows(captured) || !queue.unposted_contains(clip) {
        return false;
    }
    queue.take_unposted(clip)
}

/// Record a member line whose clip has not started.
///
/// The room log row is written later, when the clip is about to play.
/// Returns false when `captured` is already stale, and does not push.
#[must_use]
pub(crate) fn enqueue_waiting_room_audio(clip: RoomSpeechClip, captured: u64) -> bool {
    let mut queue = room_speech_queue();
    if !queue.allows(captured) {
        return false;
    }
    queue.enqueue_waiting_audio(clip);
    true
}

#[cfg(not(test))]
fn begin_room_recheck(clip: &RoomSpeechClip, speech_gen: u64, started: Instant) -> bool {
    room_speech_queue().begin_recheck(clip, speech_gen, started)
}

fn store_room_recheck(clip: &RoomSpeechClip, speech_gen: u64, decision: RecheckDecision) -> bool {
    room_speech_queue().store_recheck(clip, speech_gen, decision)
}

#[cfg(not(test))]
fn take_room_recheck(clip: &RoomSpeechClip, speech_gen: u64, now: Instant) -> RecheckPoll {
    room_speech_queue().take_recheck(clip, speech_gen, now)
}

fn rename_waiting_room_audio(clip: &RoomSpeechClip, new_text: &str, speech_gen: u64) -> bool {
    let mut queue = room_speech_queue();
    if !queue.allows(speech_gen) {
        return false;
    }
    queue.rename_waiting(clip, new_text)
}

/// Take the waiting clip only when `captured` is current and `clear` left it.
#[must_use]
pub(crate) fn claim_waiting_room_audio(clip: &RoomSpeechClip, captured: u64) -> bool {
    let mut queue = room_speech_queue();
    if !queue.allows(captured) {
        return false;
    }
    queue.take_waiting(clip)
}

/// True when `clip` is the oldest waiting line.
///
/// Playback uses this so a later reply cannot jump the queue.
#[must_use]
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "order check used by speak_room_member_line, which tests do not compile"
    )
)]
pub(crate) fn room_speech_front_is(clip: &RoomSpeechClip) -> bool {
    room_speech_queue().waiting_audio.first() == Some(clip)
}

/// True when `clip` is still waiting to play. False after clear or claim.
#[must_use]
pub(crate) fn room_speech_waiting_has(clip: &RoomSpeechClip) -> bool {
    room_speech_queue()
        .waiting_audio
        .iter()
        .any(|item| item == clip)
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
    /// Speech-queue generation at operator commit. Not bumped here.
    pub speech_gen: u64,
    /// Operator line, then member lines, copied into profile chats.
    pub history: Arc<Mutex<Vec<RoomHistoryTurn>>>,
    /// Public replies in this fan-out, in finish order.
    ///
    /// Text is the draft until the play-time decision stores the final line.
    /// The room log does not have that row yet.
    pub fanout_says: Arc<Mutex<Vec<FanoutSay>>>,
}

/// One room line copied into a profile's own chat. Text includes room id and speaker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomHistoryTurn {
    /// Profile whose HUD chat receives the line.
    pub profile_id: String,
    /// `user` when someone else spoke, `assistant` when this profile spoke.
    pub role: String,
    /// Speaker display name.
    pub name: String,
    /// Labeled body the private session can read.
    pub text: String,
}

/// `[room standup] Sally: hello`
#[must_use]
pub(crate) fn room_history_label(room_id: &str, speaker: &str, text: &str) -> String {
    format!("[room {room_id}] {speaker}: {}", text.trim())
}

fn name_mentioned(text: &str, name: &str) -> bool {
    let name = name.trim();
    if name.chars().count() < 2 {
        return false;
    }
    let hay = text.to_lowercase();
    let needle = name.to_lowercase();
    let bytes = hay.as_bytes();
    let mut start = 0;
    while let Some(rel) = hay[start..].find(&needle) {
        let abs = start + rel;
        let before_ok = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
        let after = abs + needle.len();
        let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

/// Public room line -> per-profile turns.
///
/// Operator posts (`to_every_member`) go to every member. A member say goes to
/// the speaker and to members named in the text. Other members' lines, including
/// private notes, are not copied here.
#[must_use]
pub(crate) fn profile_history_for_public_line(
    room_id: &str,
    speaker_id: &str,
    speaker_name: &str,
    text: &str,
    members: &[(&str, &str)],
    to_every_member: bool,
) -> Vec<RoomHistoryTurn> {
    let text = text.trim();
    if text.is_empty() || room_id.trim().is_empty() {
        return Vec::new();
    }
    let speaker_name = if speaker_name.trim().is_empty() {
        speaker_id
    } else {
        speaker_name.trim()
    };
    let labeled = room_history_label(room_id, speaker_name, text);
    let mut out = Vec::new();
    for (id, display) in members {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let is_speaker = id == speaker_id;
        let addressed = to_every_member
            || is_speaker
            || name_mentioned(text, display)
            || name_mentioned(text, id);
        if !addressed {
            continue;
        }
        let role = if is_speaker && !to_every_member {
            "assistant"
        } else {
            "user"
        };
        out.push(RoomHistoryTurn {
            profile_id: id.to_owned(),
            role: role.to_owned(),
            name: speaker_name.to_owned(),
            text: labeled.clone(),
        });
    }
    out
}

fn persist_room_history(turns: &[RoomHistoryTurn]) {
    for turn in turns {
        crate::hud_chat_write::append_role_turn(
            &turn.profile_id,
            &turn.role,
            &turn.name,
            &turn.text,
        );
    }
}

fn remember_room_history(job: &RoomFanoutJob, turns: Vec<RoomHistoryTurn>) {
    if turns.is_empty() {
        return;
    }
    persist_room_history(&turns);
    if let Ok(mut guard) = job.history.lock() {
        guard.extend(turns);
    }
}

fn display_of(meta: &ProfileMeta) -> String {
    if meta.name.trim().is_empty() {
        softwake_soul::DEFAULT_AGENT_NAME.to_owned()
    } else {
        meta.name.clone()
    }
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
        .map(room_log_context_line)
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
            to_profile_id: None,
            to_name: None,
            reply: None,
        },
    )?;
    let _ = mark_room_turn(&rooms_dir, room_id, "operator");
    // Stamp at commit, before profile lookup, and do not bump. An interrupt
    // during lookup must still invalidate this fan-out.
    let speech_gen = room_speech_generation();

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
    let displays: Vec<(String, String)> = members
        .iter()
        .map(|meta| (meta.id.clone(), display_of(meta)))
        .collect();
    let history_members: Vec<(&str, &str)> = displays
        .iter()
        .map(|(id, name)| (id.as_str(), name.as_str()))
        .collect();
    let operator_history = profile_history_for_public_line(
        room_id,
        "operator",
        "Operator",
        text,
        &history_members,
        true,
    );
    persist_room_history(&operator_history);
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
        speech_gen,
        history: Arc::new(Mutex::new(operator_history)),
        fanout_says: Arc::new(Mutex::new(Vec::new())),
    };
    Ok((message, Some(job)))
}

/// One labeled line for a reply already spoken in this fan-out.
#[must_use]
fn room_peer_said_line(display: &str, text: &str) -> String {
    format!(
        "{display} (another agent in this room) said: {}",
        text.trim()
    )
}

/// Text one room member sees for this fan-out.
///
/// `recent_ctx` is the log tail captured before the operator line. `this_turn`
/// is replies from this fan-out that were already in the room log when this
/// prompt was built, labeled, in the order they were logged. It is a snapshot.
/// A peer who has not spoken yet is left out. Empty recent context is `(none)`.
/// No logged replies yet is `(none yet)`.
/// The member may answer peers, address another member by name in the room
/// message, answer the operator, or stay quiet.
#[must_use]
fn room_member_stimulus(
    room_id: &str,
    title: &str,
    display: &str,
    member_id: &str,
    recent_ctx: &str,
    this_turn: &str,
    operator_text: &str,
) -> String {
    let recent = if recent_ctx.is_empty() {
        "(none)"
    } else {
        recent_ctx
    };
    let peers = if this_turn.is_empty() {
        "(none yet)"
    } else {
        this_turn
    };
    format!(
        "\
You are in Softwake room `{room_id}` (title: {title}). You are {display} (profile id `{member_id}`).

Recent room context:
{recent}

Operator message:
{operator_text}

Other members this turn:
{peers}

Other members may be answering at the same time. You only see peers who have already spoken. If none have, answer the operator. Do not wait for them. Another member already speaking does not cancel your turn.
You may reply to a peer, address another member by name in this room message (for example, Sarah: what do you think?), reply to the operator, send a private note to one other member, or stay quiet (NO_REPLY).
A room message is spoken in the room. It is not a private note.
Decide for yourself:
- If you should reply, write your room message only (no preamble).
- If you should stay quiet, reply with exactly: NO_REPLY
- To send a private note to exactly one other member instead of speaking, reply with exactly: PRIVATE <name>: <message>
A private note is not spoken. The room only shows that you sent it. The note and any reply are written to both chats."
    )
}

/// Concurrent member oneshots. Playback stays one clip at a time.
///
/// Each member runs on its own thread. The 1500ms room cool-down may run once
/// before any of them start. It does not sit between LLM calls. Prompt build
/// snapshots peers already appended and does not wait for the others. When a
/// reply returns, it is enqueued immediately. The room log stays unchanged
/// until that clip is about to play. `NO_REPLY` skips are left out. One TTS
/// path plays the queue. This does not open a Voice Agent session per member.
///
/// When `job.speech_gen` is stale, a reply that is not logged yet is dropped.
/// A line already appended at play time stays in the log.
pub(crate) fn run_room_fanout<F>(job: &RoomFanoutJob, oneshot: F)
where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    let _fanout_flight = room_fanout_single_flight();
    if !job.members.is_empty() {
        wait_room_cooldown(&job.room_path);
    }
    let mut loaded = Vec::with_capacity(job.members.len());
    let mut skipped_early = Vec::new();
    for raw in &job.unknown {
        skipped_early.push(format!("{raw} (unknown profile)"));
    }
    for member in &job.members {
        if !room_speech_allows(job.speech_gen) {
            break;
        }
        let member_id = member.id.as_str();
        let Ok((meta, instructions)) = load_profile_instructions(member_id) else {
            skipped_early.push(format!("{member_id} (unknown profile)"));
            continue;
        };
        let _ = softwake_soul::ensure_profile_home(member_id);
        let display = if meta.name.trim().is_empty() {
            softwake_soul::DEFAULT_AGENT_NAME.to_owned()
        } else {
            meta.name.clone()
        };
        loaded.push(LoadedRoomMember {
            id: member_id.to_owned(),
            display,
            instructions,
            allow_all: meta.allow_all,
            voice: meta.tts_voice.clone(),
        });
    }
    let recheck_enabled = room_requeue_recheck_enabled();
    run_loaded_room_fanout(job, &loaded, "", &skipped_early, &oneshot, recheck_enabled);
}

fn room_requeue_recheck_enabled() -> bool {
    let xdg = env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    let Ok(config) = resolve_config_dir(xdg.as_deref(), home.as_deref()) else {
        return true;
    };
    load_app_config(&config)
        .map(|app| app.room_requeue_recheck)
        .unwrap_or(true)
}

/// Profile pack already read. Member threads only run the oneshot and the log.
struct LoadedRoomMember {
    id: String,
    display: String,
    instructions: String,
    allow_all: bool,
    voice: String,
}

fn push_line(bucket: &Mutex<Vec<String>>, line: String) {
    bucket
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(line);
}

/// `initial_peers` is room history already spoken before these prompts are built.
/// Empty means nobody in this fan-out has been logged yet.
fn run_loaded_room_fanout<F>(
    job: &RoomFanoutJob,
    members: &[LoadedRoomMember],
    initial_peers: &str,
    skipped_early: &[String],
    oneshot: &F,
    recheck_enabled: bool,
) where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    let peers = Mutex::new(initial_peers.to_owned());
    let replied = Mutex::new(Vec::new());
    let skipped = Mutex::new(skipped_early.to_vec());
    let (recheck_tx, recheck_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| run_recheck_worker(recheck_rx, oneshot));
        for member in members {
            let peers = &peers;
            let replied = &replied;
            let skipped = &skipped;
            let recheck_tx = recheck_tx.clone();
            scope.spawn(move || {
                run_one_room_member(
                    job,
                    member,
                    members,
                    peers,
                    replied,
                    skipped,
                    oneshot,
                    &recheck_tx,
                    recheck_enabled,
                );
            });
        }
        drop(recheck_tx);
    });
    let replied = replied
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let skipped = skipped
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let room_id = job.room_id.as_str();
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

#[allow(
    clippy::too_many_arguments,
    reason = "room member turn stays next to the speech queue and the re-check"
)]
fn run_one_room_member<F>(
    job: &RoomFanoutJob,
    member: &LoadedRoomMember,
    roster: &[LoadedRoomMember],
    peers: &Mutex<String>,
    replied: &Mutex<Vec<String>>,
    skipped: &Mutex<Vec<String>>,
    oneshot: &F,
    recheck_tx: &Sender<RecheckRequest>,
    recheck_enabled: bool,
) where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    if !room_speech_allows(job.speech_gen) {
        return;
    }
    let this_turn = peers
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let stimulus = room_member_stimulus(
        job.room_id.as_str(),
        job.title.as_str(),
        &member.display,
        member.id.as_str(),
        &job.recent_ctx,
        &this_turn,
        job.operator_text.as_str(),
    );
    if !room_speech_allows(job.speech_gen) {
        return;
    }
    let reply = match oneshot(
        member.id.as_str(),
        &member.instructions,
        member.allow_all,
        &stimulus,
    ) {
        Ok(reply) => reply,
        Err(error) => {
            if room_speech_allows(job.speech_gen) {
                push_line(skipped, format!("{} (error: {error})", member.id));
            }
            return;
        }
    };
    if !room_speech_allows(job.speech_gen) {
        return;
    }
    if is_no_reply(&reply) {
        push_line(skipped, format!("{} (NO_REPLY)", member.id));
        return;
    }
    if looks_like_private_note(&reply) {
        publish_private_member_note(
            job, member, roster, &reply, peers, replied, skipped, oneshot,
        );
        return;
    }
    publish_logged_member_reply(
        job,
        member,
        roster,
        reply.trim(),
        peers,
        replied,
        skipped,
        oneshot,
        recheck_tx,
        recheck_enabled,
    );
}

/// `PRIVATE <name>: <message>` is a note to one member, not a room line.
fn looks_like_private_note(reply: &str) -> bool {
    crate::room_recheck::looks_like_private_note(reply)
}

/// Recipient token and note body. `None` when this is not a private note.
fn parse_private_note(reply: &str) -> Option<(String, String)> {
    crate::room_recheck::parse_private_note(reply)
}

fn find_loaded_member<'a>(
    roster: &'a [LoadedRoomMember],
    needle: &str,
) -> Option<&'a LoadedRoomMember> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    if let Some(member) = roster.iter().find(|member| member.id == needle) {
        return Some(member);
    }
    if let Some(member) = roster
        .iter()
        .find(|member| member.id.eq_ignore_ascii_case(needle))
    {
        return Some(member);
    }
    let lower = needle.to_ascii_lowercase();
    roster
        .iter()
        .find(|member| member.display.trim().to_ascii_lowercase() == lower)
}

fn private_reply_stimulus(
    room_id: &str,
    sender_display: &str,
    recipient_display: &str,
    recipient_id: &str,
    text: &str,
) -> String {
    format!(
        "\
You are {recipient_display} (profile id `{recipient_id}`) in Softwake room `{room_id}`.
{sender_display} sent you a private message. It is not spoken in the room.

Private message:
{text}

Reply to {sender_display} only. Write the reply text alone, or exactly NO_REPLY if you have nothing to say.
Do not write a room message."
    )
}

/// Collapsed peer line. The note body stays out of the room prompt.
#[must_use]
fn room_private_peer_line(sender_display: &str, target_display: &str) -> String {
    format!("{sender_display} (another agent in this room) sent a message to {target_display}")
}

#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "room private-note publish stays next to the spoken path"
)]
fn publish_private_member_note<F>(
    job: &RoomFanoutJob,
    member: &LoadedRoomMember,
    roster: &[LoadedRoomMember],
    raw_reply: &str,
    peers: &Mutex<String>,
    replied: &Mutex<Vec<String>>,
    skipped: &Mutex<Vec<String>>,
    oneshot: &F,
) where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    let Some((to, text)) = parse_private_note(raw_reply) else {
        push_line(skipped, format!("{} (private note malformed)", member.id));
        return;
    };
    if !room_speech_allows(job.speech_gen) {
        return;
    }
    let Some(target) = find_loaded_member(roster, &to) else {
        push_line(
            skipped,
            format!("{} (private note to unknown `{to}`)", member.id),
        );
        return;
    };
    if target.id == member.id {
        push_line(skipped, format!("{} (private note to self)", member.id));
        return;
    }
    let stimulus = private_reply_stimulus(
        job.room_id.as_str(),
        &member.display,
        &target.display,
        target.id.as_str(),
        &text,
    );
    if !room_speech_allows(job.speech_gen) {
        return;
    }
    let reply_text = match oneshot(
        target.id.as_str(),
        &target.instructions,
        target.allow_all,
        &stimulus,
    ) {
        Ok(reply) => reply,
        Err(error) => {
            if room_speech_allows(job.speech_gen) {
                push_line(
                    skipped,
                    format!("{} (private reply error: {error})", member.id),
                );
            }
            return;
        }
    };
    if !room_speech_allows(job.speech_gen) {
        return;
    }
    let stored_reply = if is_no_reply(&reply_text) {
        None
    } else {
        Some(reply_text.trim().to_owned())
    };
    {
        let mut peers_guard = peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !room_speech_allows(job.speech_gen) {
            return;
        }
        if let Err(error) = append_room_log(
            &job.state_dir,
            job.room_id.as_str(),
            &RoomLogLine {
                ts_ms: softwake_tools::now_ms(),
                profile_id: member.id.clone(),
                name: member.display.clone(),
                kind: RoomLogKind::Dm,
                text: text.clone(),
                iteration: None,
                phase: None,
                to_profile_id: Some(target.id.clone()),
                to_name: Some(target.display.clone()),
                reply: stored_reply.clone(),
            },
        ) {
            drop(peers_guard);
            push_line(skipped, format!("{} (log error: {error})", member.id));
            return;
        }
        let _ = mark_room_turn(&job.rooms_dir, job.room_id.as_str(), member.id.as_str());
        if !peers_guard.is_empty() {
            peers_guard.push('\n');
        }
        peers_guard.push_str(&room_private_peer_line(&member.display, &target.display));
        push_line(
            replied,
            format!(
                "{} ({}) private to {}",
                member.display, member.id, target.display
            ),
        );
    }
    #[cfg(not(test))]
    crate::hud_chat_write::append_dm_histories(
        Some(member.id.as_str()),
        &member.display,
        &target.id,
        &target.display,
        &text,
        stored_reply.as_deref(),
    );
}

#[allow(
    clippy::too_many_arguments,
    reason = "room reply publish stays next to the speech queue and the re-check"
)]
fn publish_logged_member_reply<F>(
    job: &RoomFanoutJob,
    member: &LoadedRoomMember,
    roster: &[LoadedRoomMember],
    trimmed: &str,
    peers: &Mutex<String>,
    replied: &Mutex<Vec<String>>,
    skipped: &Mutex<Vec<String>>,
    oneshot: &F,
    recheck_tx: &Sender<RecheckRequest>,
    recheck_enabled: bool,
) where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    let clip = RoomSpeechClip {
        text: trimmed.to_owned(),
        voice: member.voice.clone(),
    };
    if !stage_unposted_room_reply(&clip, job.speech_gen) {
        return;
    }
    if !claim_unposted_room_reply(&clip, job.speech_gen) {
        return;
    }
    if !room_speech_allows(job.speech_gen) {
        return;
    }
    record_fanout_say(job, member, trimmed);
    if !enqueue_waiting_room_audio(clip.clone(), job.speech_gen) {
        revise_fanout_say(job, member.id.as_str(), trimmed, None);
        return;
    }
    #[cfg(not(test))]
    {
        let env = RecheckEnv {
            job,
            oneshot,
            recheck_tx,
            recheck_enabled,
            peers,
            replied,
            skipped,
            roster,
        };
        drive_queued_room_line(&env, member, &clip);
    }
    #[cfg(test)]
    {
        let _ = (
            roster,
            peers,
            replied,
            skipped,
            oneshot,
            recheck_tx,
            recheck_enabled,
        );
    }
}

/// Model work for one waiting clip. The worker applies nothing itself.
struct RecheckRequest {
    profile_id: String,
    instructions: String,
    allow_all: bool,
    prompt: String,
    clip: RoomSpeechClip,
    speech_gen: u64,
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "the worker owns the receiver so the scoped thread can move it"
)]
fn run_recheck_worker<F>(rx: Receiver<RecheckRequest>, oneshot: &F)
where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    while let Ok(request) = rx.recv() {
        if !room_speech_allows(request.speech_gen) || !room_speech_waiting_has(&request.clip) {
            continue;
        }
        let decision = decision_from_oneshot(oneshot(
            &request.profile_id,
            &request.instructions,
            request.allow_all,
            &request.prompt,
        ));
        let _stored = store_room_recheck(&request.clip, request.speech_gen, decision);
    }
}

#[cfg(not(test))]
struct RecheckEnv<'a, F> {
    job: &'a RoomFanoutJob,
    oneshot: &'a F,
    recheck_tx: &'a Sender<RecheckRequest>,
    recheck_enabled: bool,
    peers: &'a Mutex<String>,
    replied: &'a Mutex<Vec<String>>,
    skipped: &'a Mutex<Vec<String>>,
    roster: &'a [LoadedRoomMember],
}

fn record_fanout_say(job: &RoomFanoutJob, member: &LoadedRoomMember, text: &str) {
    job.fanout_says
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(FanoutSay {
            profile_id: member.id.clone(),
            name: member.display.clone(),
            text: text.to_owned(),
        });
}

fn revise_fanout_say(job: &RoomFanoutJob, profile_id: &str, draft: &str, revised: Option<&str>) {
    let mut says = job
        .fanout_says
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(index) = says
        .iter()
        .position(|say| say.profile_id == profile_id && say.text == draft)
    else {
        return;
    };
    if let Some(revised) = revised {
        revised.clone_into(&mut says[index].text);
    } else {
        says.remove(index);
    }
}

/// Borrowed fan-out state for the moment a clip is about to play.
struct PlayEnv<'a, F> {
    job: &'a RoomFanoutJob,
    roster: &'a [LoadedRoomMember],
    peers: &'a Mutex<String>,
    replied: &'a Mutex<Vec<String>>,
    skipped: &'a Mutex<Vec<String>>,
    oneshot: &'a F,
}

/// What playback should do after the final line is chosen.
enum PlayChoice {
    /// The log holds this text. Speak it.
    Speak(String),
    /// Nothing to speak. The queue clip was claimed when the line was dropped.
    Silent,
    /// Generation changed, or the clip is already gone.
    Stop,
}

/// Append the final public line once.
///
/// The draft was not logged at enqueue. `text` is the draft or the revision.
/// On success the peer snapshot, profile mirrors, and `job.history` match
/// that one line. On failure the clip is dropped and nothing is appended.
fn commit_final_public_line<F>(
    env: &PlayEnv<'_, F>,
    member: &LoadedRoomMember,
    clip: &RoomSpeechClip,
    text: &str,
) -> bool
where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    if !room_speech_allows(env.job.speech_gen) || !room_speech_waiting_has(clip) {
        return false;
    }
    let text = text.trim();
    if text.is_empty() {
        revise_fanout_say(env.job, member.id.as_str(), clip.text.as_str(), None);
        let _ = claim_waiting_room_audio(clip, env.job.speech_gen);
        push_line(env.skipped, format!("{} (empty)", member.id));
        return false;
    }
    if let Err(error) = append_room_log(
        &env.job.state_dir,
        env.job.room_id.as_str(),
        &RoomLogLine {
            ts_ms: softwake_tools::now_ms(),
            profile_id: member.id.clone(),
            name: member.display.clone(),
            kind: RoomLogKind::Say,
            text: text.to_owned(),
            iteration: None,
            phase: None,
            to_profile_id: None,
            to_name: None,
            reply: None,
        },
    ) {
        eprintln!("softwaked: room line could not be logged: {error}");
        revise_fanout_say(env.job, member.id.as_str(), clip.text.as_str(), None);
        let _ = claim_waiting_room_audio(clip, env.job.speech_gen);
        push_line(env.skipped, format!("{} (log error: {error})", member.id));
        return false;
    }
    let _ = mark_room_turn(
        &env.job.rooms_dir,
        env.job.room_id.as_str(),
        member.id.as_str(),
    );
    {
        let mut peers_guard = env
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !peers_guard.is_empty() {
            peers_guard.push('\n');
        }
        peers_guard.push_str(&room_peer_said_line(&member.display, text));
    }
    let roster_pairs: Vec<(&str, &str)> = env
        .roster
        .iter()
        .map(|peer| (peer.id.as_str(), peer.display.as_str()))
        .collect();
    let turns = profile_history_for_public_line(
        env.job.room_id.as_str(),
        member.id.as_str(),
        member.display.as_str(),
        text,
        &roster_pairs,
        false,
    );
    remember_room_history(env.job, turns);
    push_line(env.replied, format!("{} ({})", member.display, member.id));
    revise_fanout_say(env.job, member.id.as_str(), clip.text.as_str(), Some(text));
    true
}

fn speak_committed<F>(
    env: &PlayEnv<'_, F>,
    member: &LoadedRoomMember,
    clip: &RoomSpeechClip,
    text: &str,
) -> PlayChoice
where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    if commit_final_public_line(env, member, clip, text) {
        PlayChoice::Speak(text.to_owned())
    } else {
        PlayChoice::Stop
    }
}

fn speak_revised<F>(
    env: &PlayEnv<'_, F>,
    member: &LoadedRoomMember,
    clip: &RoomSpeechClip,
    revised: &str,
) -> PlayChoice
where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    if !commit_final_public_line(env, member, clip, revised) {
        return PlayChoice::Stop;
    }
    if !rename_waiting_room_audio(clip, revised, env.job.speech_gen) {
        return PlayChoice::Stop;
    }
    PlayChoice::Speak(revised.to_owned())
}

/// Append the final line for one re-check result, or append nothing.
///
/// KEEP, a skipped check, a timeout, and a model error log the draft.
/// A revision logs only the new text. `NO_REPLY` logs nothing.
/// `PRIVATE` uses the private-note path and does not speak.
fn apply_play_decision<F>(
    env: &PlayEnv<'_, F>,
    member: &LoadedRoomMember,
    clip: &RoomSpeechClip,
    decision: RecheckDecision,
) -> PlayChoice
where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    if !room_speech_allows(env.job.speech_gen) || !room_speech_waiting_has(clip) {
        return PlayChoice::Stop;
    }
    match decision {
        RecheckDecision::Keep => speak_committed(env, member, clip, clip.text.as_str()),
        RecheckDecision::Revise(text) => {
            let revised = text.trim();
            if revised.is_empty() || revised == clip.text {
                speak_committed(env, member, clip, clip.text.as_str())
            } else {
                speak_revised(env, member, clip, revised)
            }
        }
        RecheckDecision::Drop => {
            revise_fanout_say(env.job, member.id.as_str(), clip.text.as_str(), None);
            let _ = claim_waiting_room_audio(clip, env.job.speech_gen);
            push_line(env.skipped, format!("{} (NO_REPLY)", member.id));
            PlayChoice::Silent
        }
        RecheckDecision::Private { to, text } => {
            revise_fanout_say(env.job, member.id.as_str(), clip.text.as_str(), None);
            let _ = claim_waiting_room_audio(clip, env.job.speech_gen);
            let raw = format!("PRIVATE {to}: {text}");
            publish_private_member_note(
                env.job,
                member,
                env.roster,
                &raw,
                env.peers,
                env.replied,
                env.skipped,
                env.oneshot,
            );
            PlayChoice::Silent
        }
    }
}

#[cfg(not(test))]
fn request_recheck_for(
    env: &RecheckEnv<'_, impl Fn(&str, &str, bool, &str) -> Result<String, String> + Sync>,
    profile_id: &str,
    display: &str,
    instructions: &str,
    allow_all: bool,
    clip: &RoomSpeechClip,
) {
    if !env.recheck_enabled {
        return;
    }
    let ahead = {
        let says = env
            .job
            .fanout_says
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::room_recheck::ahead_for(&says, profile_id, clip.text.as_str())
    };
    if !crate::room_recheck::recheck_should_run(true, &ahead) {
        return;
    }
    if !begin_room_recheck(clip, env.job.speech_gen, Instant::now()) {
        return;
    }
    let prompt =
        crate::room_recheck::recheck_prompt(display, profile_id, clip.text.as_str(), &ahead);
    let _sent = env.recheck_tx.send(RecheckRequest {
        profile_id: profile_id.to_owned(),
        instructions: instructions.to_owned(),
        allow_all,
        prompt,
        clip: clip.clone(),
        speech_gen: env.job.speech_gen,
    });
}

#[cfg(not(test))]
enum WaitedRecheck {
    Draft,
    Ready(RecheckDecision),
    Stop,
}

#[cfg(not(test))]
fn wait_recheck(clip: &RoomSpeechClip, speech_gen: u64) -> WaitedRecheck {
    loop {
        if !room_speech_allows(speech_gen) {
            return WaitedRecheck::Stop;
        }
        match take_room_recheck(clip, speech_gen, Instant::now()) {
            RecheckPoll::Idle => return WaitedRecheck::Draft,
            RecheckPoll::TimedOut => {
                eprintln!("softwaked: room re-check timed out; playing the draft");
                return WaitedRecheck::Draft;
            }
            RecheckPoll::Ready(decision) => return WaitedRecheck::Ready(decision),
            RecheckPoll::Stale => return WaitedRecheck::Stop,
            RecheckPoll::Pending => std::thread::sleep(std::time::Duration::from_millis(40)),
        }
    }
}

/// Wait until this clip is next, apply one re-check, log the final line, then play or drop it.
///
/// The re-check starts here, when the clip becomes the oldest waiting line.
/// That is the moment the previous clip has been claimed and is playing, so
/// the model call overlaps that playback. A clip with nothing ahead skips the
/// call. The 1500 ms budget is measured from the start, then the draft is
/// logged and played. This clip stays out of the room log until that decision.
#[cfg(not(test))]
fn drive_queued_room_line<F>(
    env: &RecheckEnv<'_, F>,
    member: &LoadedRoomMember,
    clip: &RoomSpeechClip,
) where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    let started = Instant::now();
    let budget = std::time::Duration::from_secs(120);
    loop {
        if !room_speech_allows(env.job.speech_gen) || !room_speech_waiting_has(clip) {
            return;
        }
        if started.elapsed() >= budget {
            return;
        }
        if !room_speech_front_is(clip) {
            std::thread::sleep(std::time::Duration::from_millis(40));
            continue;
        }
        if env.recheck_enabled {
            request_recheck_for(
                env,
                member.id.as_str(),
                member.display.as_str(),
                member.instructions.as_str(),
                member.allow_all,
                clip,
            );
        }
        let decision = match wait_recheck(clip, env.job.speech_gen) {
            WaitedRecheck::Stop => return,
            WaitedRecheck::Draft => RecheckDecision::Keep,
            WaitedRecheck::Ready(decision) => decision,
        };
        let play = PlayEnv {
            job: env.job,
            roster: env.roster,
            peers: env.peers,
            replied: env.replied,
            skipped: env.skipped,
            oneshot: env.oneshot,
        };
        match apply_play_decision(&play, member, clip, decision) {
            PlayChoice::Speak(text) => {
                crate::talk::speak_room_member_line(&member.voice, &text, env.job.speech_gen);
            }
            PlayChoice::Silent | PlayChoice::Stop => {}
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clears the HUD history override when a re-check test returns.
    struct ResetHistoryOverride;

    impl Drop for ResetHistoryOverride {
        fn drop(&mut self) {
            *crate::hud_chat_write::HISTORY_CONFIG_OVERRIDE
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
    }

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

    #[test]
    fn clear_drops_queued_member_speech_and_leaves_the_log() {
        // Local queue, not the process global. No audio device and no network.
        let mut queue = RoomSpeechQueue::default();
        let stamped = 0_u64;
        assert!(queue.allows(stamped));
        queue.enqueue_waiting_audio(RoomSpeechClip {
            text: "the build is green".to_owned(),
            voice: "ara".to_owned(),
        });
        let unposted = RoomSpeechClip {
            text: "I can take the notes".to_owned(),
            voice: "rex".to_owned(),
        };
        assert!(queue.stage_unposted(&unposted, stamped));
        assert!(queue.unposted_contains(&unposted));
        // Operator line plus the member line already in the log. clear must
        // not receive this vec and must not rewrite it.
        let room_log = vec![
            "operator: are we shipping today".to_owned(),
            "Sally: the build is green".to_owned(),
        ];
        let room_log_before = room_log.clone();

        let report = queue.clear();

        assert_eq!(report.dropped_audio, 1);
        assert_eq!(report.dropped_unposted, 1);
        assert!(queue.waiting_audio.is_empty());
        assert!(queue.unposted.is_empty());
        assert!(!queue.allows(stamped));
        assert!(queue.allows(report.generation));
        assert_eq!(report.generation, stamped.wrapping_add(1));
        assert!(!queue.take_waiting(&RoomSpeechClip {
            text: "the build is green".to_owned(),
            voice: "ara".to_owned(),
        }));
        assert_eq!(room_log, room_log_before);
        assert!(!queue.unposted_contains(&unposted));
        // A stamp from before the interrupt cannot stage another reply.
        // The generation clear just published can.
        assert!(!queue.stage_unposted(&unposted, stamped));
        assert!(queue.unposted.is_empty());
        assert!(queue.stage_unposted(&unposted, report.generation));
        assert!(queue.unposted_contains(&unposted));
        assert_eq!(room_log, room_log_before);
    }

    #[test]
    fn first_member_stimulus_has_context_and_no_peers_yet() {
        let recent = "[say] Kai: yesterday we slipped the cut";
        let stimulus = room_member_stimulus(
            "standup",
            "Standup",
            "Spencer",
            "spencer",
            recent,
            "",
            "are we shipping today",
        );
        assert!(stimulus.contains("Operator message:\nare we shipping today"));
        assert!(stimulus.contains("Recent room context:\n[say] Kai: yesterday we slipped the cut"));
        assert!(stimulus.contains("Other members this turn:\n(none yet)"));
        assert!(
            stimulus.contains(
                "You may reply to a peer, address another member by name in this room message (for example, Sarah: what do you think?), reply to the operator, send a private note to one other member, or stay quiet (NO_REPLY)."
            )
        );
        assert!(!stimulus.contains("another agent in this room"));
        assert!(!stimulus.to_ascii_lowercase().contains("must reply"));

        let empty_recent = room_member_stimulus(
            "standup",
            "Standup",
            "Spencer",
            "spencer",
            "",
            "",
            "are we shipping today",
        );
        assert!(empty_recent.contains("Recent room context:\n(none)"));
        assert!(empty_recent.contains("(none yet)"));
    }

    #[test]
    fn later_member_stimulus_hears_earlier_peer() {
        let recent = "[say] Kai: yesterday we slipped the cut";
        let peer = room_peer_said_line("Sara Vale", "the build is green");
        assert_eq!(
            peer,
            "Sara Vale (another agent in this room) said: the build is green"
        );
        let stimulus = room_member_stimulus(
            "standup",
            "Standup",
            "Spencer",
            "spencer",
            recent,
            &peer,
            "are we shipping today",
        );
        assert!(stimulus.contains("Operator message:\nare we shipping today"));
        assert!(
            stimulus.contains("Sara Vale (another agent in this room) said: the build is green")
        );
        assert!(stimulus.contains("Recent room context:\n[say] Kai: yesterday we slipped the cut"));
        assert!(
            stimulus.contains(
                "You may reply to a peer, address another member by name in this room message (for example, Sarah: what do you think?), reply to the operator, send a private note to one other member, or stay quiet (NO_REPLY)."
            )
        );
        assert!(
            stimulus.contains("- If you should reply, write your room message only (no preamble).")
        );
        assert!(stimulus.contains("- If you should stay quiet, reply with exactly: NO_REPLY"));
        assert!(stimulus.contains("PRIVATE <name>: <message>"));
        assert!(stimulus.contains("A private note is not spoken."));
        let lower = stimulus.to_ascii_lowercase();
        assert!(!lower.contains("must reply"));
        assert!(!lower.contains("must respond"));
        assert!(!lower.contains("have to reply"));
        assert!(!stimulus.contains("[say] Sara Vale:"));
    }
    fn parallel_temp(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "softwake-room-parallel-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn parallel_job(dir: &std::path::Path, speech_gen: u64) -> RoomFanoutJob {
        RoomFanoutJob {
            room_id: "parallel".into(),
            operator_text: "are we shipping today".into(),
            title: "Standup".into(),
            members: Vec::new(),
            unknown: Vec::new(),
            recent_ctx: String::new(),
            rooms_dir: dir.to_path_buf(),
            state_dir: dir.to_path_buf(),
            room_path: dir.join("parallel.json"),
            speech_gen,
            history: Arc::new(Mutex::new(Vec::new())),
            fanout_says: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn loaded_member(id: &str, display: &str, voice: &str) -> LoadedRoomMember {
        LoadedRoomMember {
            id: id.to_owned(),
            display: display.to_owned(),
            instructions: "test".to_owned(),
            allow_all: false,
            voice: voice.to_owned(),
        }
    }

    /// The process speech queue is global. Keep these tests from interleaving.
    fn hold_speech_queue_tests() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    struct DrainWaiting {
        texts: Vec<String>,
    }

    impl Drop for DrainWaiting {
        fn drop(&mut self) {
            let mut queue = room_speech_queue();
            queue
                .waiting_audio
                .retain(|clip| !self.texts.iter().any(|text| text == &clip.text));
        }
    }

    #[test]
    fn two_members_can_be_in_flight_and_a_finished_reply_enqueues() {
        let _hold = hold_speech_queue_tests();
        let dir = parallel_temp("inflight");
        let sara_text = "the build is green";
        let spencer_text = "notes from spencer";
        let _drain = DrainWaiting {
            texts: vec![sara_text.to_owned(), spencer_text.to_owned()],
        };
        let job = parallel_job(&dir, room_speech_generation());
        let members = vec![
            loaded_member("sara", "Sara Vale", "ara"),
            loaded_member("spencer", "Spencer", "rex"),
        ];
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let inflight = std::sync::atomic::AtomicUsize::new(0);
        let max_inflight = std::sync::atomic::AtomicUsize::new(0);
        let saw_enqueued = std::sync::atomic::AtomicBool::new(false);
        let stimuli = Mutex::new(Vec::<(String, String)>::new());
        let oneshot = |id: &str, _instructions: &str, _allow_all: bool, stimulus: &str| {
            stimuli
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((id.to_owned(), stimulus.to_owned()));
            let now = inflight.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            max_inflight.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            barrier.wait();
            let result = if id == "sara" {
                Ok(sara_text.to_owned())
            } else {
                let clip = RoomSpeechClip {
                    text: sara_text.to_owned(),
                    voice: "ara".to_owned(),
                };
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    if room_speech_waiting_has(&clip) {
                        saw_enqueued.store(true, std::sync::atomic::Ordering::SeqCst);
                        break;
                    }
                    if std::time::Instant::now() > deadline {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(spencer_text.to_owned())
            };
            inflight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            result
        };
        run_loaded_room_fanout(&job, &members, "", &[], &oneshot, false);
        assert!(
            max_inflight.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "both member oneshots must overlap"
        );
        assert!(
            saw_enqueued.load(std::sync::atomic::Ordering::SeqCst),
            "a finished reply must enqueue while the other member is still in oneshot"
        );
        let stimuli = stimuli
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(stimuli.len(), 2);
        for (_id, stimulus) in stimuli.iter() {
            assert!(
                stimulus.contains("Other members this turn:\n(none yet)"),
                "{stimulus}"
            );
            assert!(
                !stimulus.contains("another agent in this room"),
                "{stimulus}"
            );
            assert!(stimulus.contains("Do not wait for them."));
            assert!(stimulus.contains("Sarah: what do you think?"));
        }
        let path = dir.join("parallel").join("log.jsonl");
        if path.is_file() {
            let log = std::fs::read_to_string(&path).expect("log");
            assert!(!log.contains(sara_text), "{log}");
            assert!(!log.contains(spencer_text), "{log}");
        }
        assert!(room_speech_waiting_has(&RoomSpeechClip {
            text: sara_text.to_owned(),
            voice: "ara".to_owned(),
        }));
        assert!(room_speech_waiting_has(&RoomSpeechClip {
            text: spencer_text.to_owned(),
            voice: "rex".to_owned(),
        }));
        let says = job
            .fanout_says
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            says.iter()
                .any(|say| say.profile_id == "sara" && say.text == sara_text)
        );
        assert!(
            says.iter()
                .any(|say| say.profile_id == "spencer" && say.text == spencer_text)
        );
    }

    #[test]
    fn already_logged_peer_is_in_the_prompt_without_waiting_to_start() {
        let _hold = hold_speech_queue_tests();
        let dir = parallel_temp("seed");
        let _drain = DrainWaiting {
            texts: vec!["alpha line".to_owned(), "beta line".to_owned()],
        };
        let job = parallel_job(&dir, room_speech_generation());
        let members = vec![
            loaded_member("sara", "Sara Vale", "ara"),
            loaded_member("spencer", "Spencer", "rex"),
        ];
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let inflight = std::sync::atomic::AtomicUsize::new(0);
        let max_inflight = std::sync::atomic::AtomicUsize::new(0);
        let stimuli = Mutex::new(Vec::<(String, String)>::new());
        let seed = room_peer_said_line("Kai", "I already spoke");
        let oneshot = |id: &str, _instructions: &str, _allow_all: bool, stimulus: &str| {
            stimuli
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((id.to_owned(), stimulus.to_owned()));
            let now = inflight.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            max_inflight.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            barrier.wait();
            inflight.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if id == "sara" {
                Ok("alpha line".to_owned())
            } else {
                Ok("beta line".to_owned())
            }
        };
        run_loaded_room_fanout(&job, &members, &seed, &[], &oneshot, false);
        assert!(
            max_inflight.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "seeded history must not serialize the oneshots"
        );
        let stimuli = stimuli
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(stimuli.len(), 2);
        for (_id, stimulus) in stimuli.iter() {
            assert!(stimulus.contains("Kai (another agent in this room) said: I already spoke"));
            assert!(!stimulus.contains("(none yet)"));
            assert!(stimulus.contains("Sarah: what do you think?"));
        }
    }

    #[test]
    fn private_note_parses_one_recipient_only() {
        let parsed = parse_private_note("PRIVATE Spencer: ping the notes").unwrap();
        assert_eq!(parsed.0, "Spencer");
        assert_eq!(parsed.1, "ping the notes");
        assert!(parse_private_note("private Sally: hi").is_some());
        assert!(parse_private_note("PRIVATE Spencer, Sally: hi").is_none());
        assert!(parse_private_note("hello room").is_none());
        assert!(!looks_like_private_note("hello room"));
        let line = room_private_peer_line("Sara", "Sally");
        assert_eq!(
            line,
            "Sara (another agent in this room) sent a message to Sally"
        );
        assert!(!line.contains("ping"));
    }

    #[test]
    fn private_note_is_logged_not_spoken_and_a_room_say_still_enqueues() {
        let _hold = hold_speech_queue_tests();
        let dir = parallel_temp("dm");
        let say = "shipping today";
        let note = "ping the notes";
        let _drain = DrainWaiting {
            texts: vec![say.to_owned()],
        };
        let job = parallel_job(&dir, room_speech_generation());
        let members = vec![
            loaded_member("sara", "Sara", "ara"),
            loaded_member("spencer", "Spencer", "rex"),
        ];
        let oneshot = |id: &str, _instructions: &str, _allow_all: bool, stimulus: &str| {
            if stimulus.contains("sent you a private message") {
                assert_eq!(id, "spencer");
                assert!(stimulus.contains(note));
                assert!(stimulus.contains("not spoken"));
                return Ok("got it".to_owned());
            }
            if id == "sara" {
                Ok(format!("PRIVATE Spencer: {note}"))
            } else {
                Ok(say.to_owned())
            }
        };
        run_loaded_room_fanout(&job, &members, "", &[], &oneshot, false);
        let raw = std::fs::read_to_string(dir.join("parallel").join("log.jsonl")).expect("log");
        let lines: Vec<softwake_tools::RoomLogLine> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("row"))
            .collect();
        let dm = lines
            .iter()
            .find(|line| line.kind == RoomLogKind::Dm)
            .expect("dm");
        assert_eq!(dm.name, "Sara");
        assert_eq!(dm.profile_id, "sara");
        assert_eq!(dm.text, note);
        assert_eq!(dm.to_name.as_deref(), Some("Spencer"));
        assert_eq!(dm.to_profile_id.as_deref(), Some("spencer"));
        assert_eq!(dm.reply.as_deref(), Some("got it"));
        assert!(
            lines
                .iter()
                .all(|line| !(line.kind == RoomLogKind::Say && line.text == say)),
            "public say waits until play: {raw}"
        );
        assert!(!raw.contains("PRIVATE Spencer"));
        let clip = RoomSpeechClip {
            text: note.to_owned(),
            voice: "ara".to_owned(),
        };
        assert!(!room_speech_waiting_has(&clip));
        assert!(room_speech_waiting_has(&RoomSpeechClip {
            text: say.to_owned(),
            voice: "rex".to_owned(),
        }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn interrupt_drops_a_private_note_that_is_not_logged_yet() {
        let _hold = hold_speech_queue_tests();
        let dir = parallel_temp("dm-clear");
        let job = parallel_job(&dir, room_speech_generation());
        let members = vec![
            loaded_member("sara", "Sara", "ara"),
            loaded_member("spencer", "Spencer", "rex"),
        ];
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let oneshot = |id: &str, _instructions: &str, _allow_all: bool, stimulus: &str| {
            assert!(
                !stimulus.contains("sent you a private message"),
                "interrupt must drop the note before a reply oneshot"
            );
            barrier.wait();
            if id == "sara" {
                clear_room_speech_queue();
                Ok("PRIVATE Spencer: ping the notes".to_owned())
            } else {
                Ok("NO_REPLY".to_owned())
            }
        };
        run_loaded_room_fanout(&job, &members, "", &[], &oneshot, false);
        let path = dir.join("parallel").join("log.jsonl");
        if path.is_file() {
            let raw = std::fs::read_to_string(&path).expect("log");
            assert!(!raw.contains("ping the notes"), "{raw}");
            assert!(!raw.contains("PRIVATE"), "{raw}");
        }
        assert!(!room_speech_waiting_has(&RoomSpeechClip {
            text: "ping the notes".to_owned(),
            voice: "ara".to_owned(),
        }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn operator_post_and_member_say_are_in_that_profile_history() {
        let _hold = crate::hud_chat_write::hold_history_override_tests();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "softwake-room-history-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp");
        *crate::hud_chat_write::HISTORY_CONFIG_OVERRIDE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(root.clone());

        let members = [("sally", "Sally"), ("joi", "Joi")];
        let operator = profile_history_for_public_line(
            "spncxrchat",
            "operator",
            "Operator",
            "Sally, you are Chief of Staff and my PA",
            &members,
            true,
        );
        let said = profile_history_for_public_line(
            "spncxrchat",
            "sally",
            "Sally",
            "Sorted. Chief of Staff and your PA it is.",
            &members,
            false,
        );
        persist_room_history(&operator);
        persist_room_history(&said);

        let sally = std::fs::read_to_string(root.join("profiles/sally/hud-chat.json"))
            .expect("sally history");
        let joi =
            std::fs::read_to_string(root.join("profiles/joi/hud-chat.json")).expect("joi history");
        assert!(sally.contains("[room spncxrchat] Operator:"), "{sally}");
        assert!(sally.contains("Chief of Staff and my PA"), "{sally}");
        assert!(sally.contains("[room spncxrchat] Sally:"), "{sally}");
        assert!(
            sally.contains("Sorted. Chief of Staff and your PA it is."),
            "{sally}"
        );
        assert!(sally.contains("\"role\": \"user\""), "{sally}");
        assert!(sally.contains("\"role\": \"assistant\""), "{sally}");
        assert!(joi.contains("[room spncxrchat] Operator:"), "{joi}");
        assert!(
            !joi.contains("Sorted. Chief of Staff"),
            "joi must not receive sally's room line: {joi}"
        );

        *crate::hud_chat_write::HISTORY_CONFIG_OVERRIDE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn recheck_budget_falls_back_and_clear_discards_a_late_result() {
        let mut queue = RoomSpeechQueue::default();
        let stamped = 0_u64;
        let clip = RoomSpeechClip {
            text: "draft line".to_owned(),
            voice: "ara".to_owned(),
        };
        let later = RoomSpeechClip {
            text: "second line".to_owned(),
            voice: "rex".to_owned(),
        };
        queue.enqueue_waiting_audio(clip.clone());
        queue.enqueue_waiting_audio(later);
        assert!(queue.rename_waiting(&clip, "draft final"));
        let renamed = RoomSpeechClip {
            text: "draft final".to_owned(),
            voice: "ara".to_owned(),
        };
        assert_eq!(queue.waiting_audio.first(), Some(&renamed));

        let elapsed =
            crate::room_recheck::ROOM_REQUEUE_RECHECK_WAIT + std::time::Duration::from_millis(5);
        let started = Instant::now().checked_sub(elapsed).expect("clock");
        assert!(queue.begin_recheck(&renamed, stamped, started));
        assert!(
            !queue.begin_recheck(&renamed, stamped, Instant::now()),
            "one re-check per reply"
        );
        assert!(queue.store_recheck(&renamed, stamped, RecheckDecision::Keep));
        assert_eq!(
            queue.take_recheck(&renamed, stamped, Instant::now()),
            RecheckPoll::Ready(RecheckDecision::Keep)
        );

        assert!(queue.begin_recheck(&renamed, stamped, started));
        assert_eq!(
            queue.take_recheck(&renamed, stamped, Instant::now()),
            RecheckPoll::TimedOut
        );
        assert_eq!(
            queue.take_recheck(&renamed, stamped, Instant::now()),
            RecheckPoll::Idle
        );

        assert!(queue.begin_recheck(&renamed, stamped, Instant::now()));
        assert_eq!(
            queue.take_recheck(&renamed, stamped, Instant::now()),
            RecheckPoll::Pending
        );
        let report = queue.clear();
        assert_eq!(report.dropped_rechecks, 1);
        assert_eq!(report.dropped_audio, 2);
        assert!(!queue.allows(stamped));
        assert!(!queue.store_recheck(
            &renamed,
            stamped,
            RecheckDecision::Revise("too late".to_owned())
        ));
        assert_eq!(
            queue.take_recheck(&renamed, stamped, Instant::now()),
            RecheckPoll::Stale
        );
    }

    fn arm_history_override(dir: &std::path::Path) -> std::path::PathBuf {
        let cfg = dir.join("cfg");
        std::fs::create_dir_all(&cfg).expect("cfg");
        *crate::hud_chat_write::HISTORY_CONFIG_OVERRIDE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cfg.clone());
        cfg
    }

    fn queue_draft(job: &RoomFanoutJob, member: &LoadedRoomMember, text: &str) -> RoomSpeechClip {
        let clip = RoomSpeechClip {
            text: text.to_owned(),
            voice: member.voice.clone(),
        };
        record_fanout_say(job, member, text);
        assert!(enqueue_waiting_room_audio(clip.clone(), job.speech_gen));
        clip
    }

    fn read_room_log(dir: &std::path::Path) -> String {
        let path = dir.join("parallel").join("log.jsonl");
        if path.is_file() {
            std::fs::read_to_string(&path).expect("log")
        } else {
            String::new()
        }
    }

    struct PlayFixture {
        dir: std::path::PathBuf,
        job: RoomFanoutJob,
        roster: Vec<LoadedRoomMember>,
        peers: Mutex<String>,
        replied: Mutex<Vec<String>>,
        skipped: Mutex<Vec<String>>,
    }

    fn play_fixture(label: &str) -> PlayFixture {
        let dir = parallel_temp(label);
        let job = parallel_job(&dir, room_speech_generation());
        PlayFixture {
            dir,
            job,
            roster: vec![
                loaded_member("sally", "Sally", "ara"),
                loaded_member("joi", "Joi", "rex"),
            ],
            peers: Mutex::new(String::new()),
            replied: Mutex::new(Vec::new()),
            skipped: Mutex::new(Vec::new()),
        }
    }

    fn apply_on<'a, F>(
        fixture: &'a PlayFixture,
        member_index: usize,
        clip: &RoomSpeechClip,
        decision: RecheckDecision,
        oneshot: &'a F,
    ) -> PlayChoice
    where
        F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
    {
        let env = PlayEnv {
            job: &fixture.job,
            roster: &fixture.roster,
            peers: &fixture.peers,
            replied: &fixture.replied,
            skipped: &fixture.skipped,
            oneshot,
        };
        apply_play_decision(&env, &fixture.roster[member_index], clip, decision)
    }

    fn assert_mirror_has(cfg: &std::path::Path, profile: &str, text: &str, absent: &str) {
        let body = std::fs::read_to_string(cfg.join(format!("profiles/{profile}/hud-chat.json")))
            .unwrap_or_else(|_| format!("missing {profile}"));
        assert!(body.contains(text), "{body}");
        assert!(!body.contains(absent), "{body}");
    }

    #[test]
    fn play_time_keep_appends_the_draft_once() {
        let _queue = hold_speech_queue_tests();
        let _history = crate::hud_chat_write::hold_history_override_tests();
        let fixture = play_fixture("keep");
        let cfg = arm_history_override(&fixture.dir);
        let _reset = ResetHistoryOverride;
        let draft = "Joi, the build is green";
        let _drain = DrainWaiting {
            texts: vec![draft.to_owned()],
        };
        let clip = queue_draft(&fixture.job, &fixture.roster[0], draft);
        let oneshot = |_id: &str, _instructions: &str, _allow_all: bool, _stimulus: &str| {
            Ok("unused".to_owned())
        };
        assert!(matches!(
            apply_on(&fixture, 0, &clip, RecheckDecision::Keep, &oneshot),
            PlayChoice::Speak(text) if text == draft
        ));
        let raw = read_room_log(&fixture.dir);
        let lines: Vec<softwake_tools::RoomLogLine> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("row"))
            .collect();
        assert_eq!(lines.len(), 1, "{raw}");
        assert_eq!(lines[0].kind, RoomLogKind::Say);
        assert_eq!(lines[0].profile_id, "sally");
        assert_eq!(lines[0].text, draft);
        assert_mirror_has(&cfg, "sally", draft, "slipped");
        assert_mirror_has(&cfg, "joi", draft, "slipped");
        let history = fixture
            .job
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(history.len(), 2);
        assert!(history.iter().all(|turn| turn.text.contains(draft)));
        drop(history);
        let peers = fixture
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(peers.contains(draft), "{peers}");
        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn play_time_revise_appends_only_the_final_line() {
        let _queue = hold_speech_queue_tests();
        let _history = crate::hud_chat_write::hold_history_override_tests();
        let fixture = play_fixture("revise");
        let cfg = arm_history_override(&fixture.dir);
        let _reset = ResetHistoryOverride;
        let draft = "Joi, the build is green";
        let revised = "Joi, the build slipped";
        let _drain = DrainWaiting {
            texts: vec![draft.to_owned(), revised.to_owned()],
        };
        let clip = queue_draft(&fixture.job, &fixture.roster[0], draft);
        let oneshot = |_id: &str, _instructions: &str, _allow_all: bool, _stimulus: &str| {
            Ok("unused".to_owned())
        };
        assert!(matches!(
            apply_on(
                &fixture,
                0,
                &clip,
                RecheckDecision::Revise(revised.to_owned()),
                &oneshot
            ),
            PlayChoice::Speak(text) if text == revised
        ));
        let raw = read_room_log(&fixture.dir);
        assert_eq!(raw.lines().count(), 1, "{raw}");
        assert!(raw.contains(revised), "{raw}");
        assert!(!raw.contains("is green"), "{raw}");
        assert_mirror_has(&cfg, "sally", revised, "is green");
        assert_mirror_has(&cfg, "joi", revised, "is green");
        let said = fixture
            .job
            .fanout_says
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(said.len(), 1);
        assert_eq!(said[0].text, revised);
        drop(said);
        let peers = fixture
            .peers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(peers.contains(revised), "{peers}");
        assert!(!peers.contains("is green"), "{peers}");
        let history = fixture
            .job
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(history.iter().all(|turn| turn.text.contains(revised)));
        assert!(history.iter().all(|turn| !turn.text.contains("is green")));
        assert!(room_speech_waiting_has(&RoomSpeechClip {
            text: revised.to_owned(),
            voice: "ara".to_owned(),
        }));
        assert!(!room_speech_waiting_has(&clip));
        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn play_time_no_reply_logs_nothing() {
        let _queue = hold_speech_queue_tests();
        let fixture = play_fixture("drop");
        let draft = "Joi, the build is green";
        let clip = queue_draft(&fixture.job, &fixture.roster[0], draft);
        let oneshot = |_id: &str, _instructions: &str, _allow_all: bool, _stimulus: &str| {
            Ok("unused".to_owned())
        };
        assert!(matches!(
            apply_on(&fixture, 0, &clip, RecheckDecision::Drop, &oneshot),
            PlayChoice::Silent
        ));
        let raw = read_room_log(&fixture.dir);
        assert!(!raw.contains(draft), "{raw}");
        assert!(
            !raw.contains("\"kind\":\"say\"") && !raw.contains("\"kind\": \"say\""),
            "{raw}"
        );
        assert!(
            fixture
                .job
                .fanout_says
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        assert!(
            fixture
                .job
                .history
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        assert!(!room_speech_waiting_has(&clip));
        let skipped = fixture
            .skipped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(skipped.iter().any(|line| line.contains("NO_REPLY")));
        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn play_time_private_logs_the_note_and_not_a_say() {
        let _queue = hold_speech_queue_tests();
        let fixture = play_fixture("play-dm");
        let draft = "hello room";
        let clip = queue_draft(&fixture.job, &fixture.roster[0], draft);
        let oneshot = |id: &str, _instructions: &str, _allow_all: bool, stimulus: &str| {
            assert_eq!(id, "joi");
            assert!(stimulus.contains("ping the notes"));
            assert!(stimulus.contains("not spoken"));
            Ok("got it".to_owned())
        };
        assert!(matches!(
            apply_on(
                &fixture,
                0,
                &clip,
                RecheckDecision::Private {
                    to: "Joi".to_owned(),
                    text: "ping the notes".to_owned(),
                },
                &oneshot
            ),
            PlayChoice::Silent
        ));
        let raw = read_room_log(&fixture.dir);
        let lines: Vec<softwake_tools::RoomLogLine> = raw
            .lines()
            .map(|line| serde_json::from_str(line).expect("row"))
            .collect();
        assert!(
            lines.iter().all(|line| line.kind != RoomLogKind::Say),
            "{raw}"
        );
        let dm = lines
            .iter()
            .find(|line| line.kind == RoomLogKind::Dm)
            .expect("dm");
        assert_eq!(dm.text, "ping the notes");
        assert_eq!(dm.profile_id, "sally");
        assert_eq!(dm.to_profile_id.as_deref(), Some("joi"));
        assert_eq!(dm.reply.as_deref(), Some("got it"));
        assert!(!room_speech_waiting_has(&clip));
        assert!(
            fixture
                .job
                .fanout_says
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&fixture.dir);
    }

    #[test]
    fn interrupt_before_play_leaves_no_orphan_draft() {
        let _queue = hold_speech_queue_tests();
        let dir = parallel_temp("orphan");
        let job = parallel_job(&dir, room_speech_generation());
        let members = vec![loaded_member("sara", "Sara", "ara")];
        let draft = "draft line";
        let oneshot = |_id: &str, _instructions: &str, _allow_all: bool, _stimulus: &str| {
            Ok(draft.to_owned())
        };
        run_loaded_room_fanout(&job, &members, "", &[], &oneshot, true);
        let raw = read_room_log(&dir);
        assert!(!raw.contains(draft), "{raw}");
        let clip = RoomSpeechClip {
            text: draft.to_owned(),
            voice: "ara".to_owned(),
        };
        assert!(room_speech_waiting_has(&clip));
        clear_room_speech_queue();
        let fixture = PlayFixture {
            dir: dir.clone(),
            job,
            roster: members,
            peers: Mutex::new(String::new()),
            replied: Mutex::new(Vec::new()),
            skipped: Mutex::new(Vec::new()),
        };
        assert!(matches!(
            apply_on(&fixture, 0, &clip, RecheckDecision::Keep, &oneshot),
            PlayChoice::Stop
        ));
        let raw = read_room_log(&dir);
        assert!(!raw.contains(draft), "{raw}");
        assert!(!room_speech_waiting_has(&clip));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recheck_off_still_logs_at_play_time() {
        let _queue = hold_speech_queue_tests();
        let _history = crate::hud_chat_write::hold_history_override_tests();
        let fixture = play_fixture("recheck-off");
        let cfg = arm_history_override(&fixture.dir);
        let _reset = ResetHistoryOverride;
        let draft = "Joi, the build is green";
        let _drain = DrainWaiting {
            texts: vec![draft.to_owned()],
        };
        let oneshot = |_id: &str, _instructions: &str, _allow_all: bool, _stimulus: &str| {
            Ok(draft.to_owned())
        };
        run_loaded_room_fanout(&fixture.job, &fixture.roster[..1], "", &[], &oneshot, false);
        let raw = read_room_log(&fixture.dir);
        assert!(!raw.contains(draft), "{raw}");
        let clip = RoomSpeechClip {
            text: draft.to_owned(),
            voice: "ara".to_owned(),
        };
        assert!(room_speech_waiting_has(&clip));
        assert!(matches!(
            apply_on(&fixture, 0, &clip, RecheckDecision::Keep, &oneshot),
            PlayChoice::Speak(text) if text == draft
        ));
        let raw = read_room_log(&fixture.dir);
        assert_eq!(raw.lines().count(), 1, "{raw}");
        assert!(raw.contains(draft), "{raw}");
        assert_mirror_has(&cfg, "sally", draft, "NO_REPLY");
        let history = fixture
            .job
            .history
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(history.iter().any(|turn| turn.text.contains(draft)));
        let _ = std::fs::remove_dir_all(&fixture.dir);
    }
}
