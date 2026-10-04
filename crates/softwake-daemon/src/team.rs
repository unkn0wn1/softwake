//! Agent team helpers: peer DM wake + `goal_run` outer loop (ADR-0052).

use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

use softwake_soul::{
    ProfileMeta, ensure_profile_home, list_profiles, load_profile_meta, profile_pack_dir,
    resolve_config_dir, try_load_effective,
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
    let sender_profile = sender
        .as_ref()
        .map_or("operator", |from| from.id.as_str());
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
}

/// In-memory queue of room-member speech that has not started playback.
///
/// Generation starts at 0. `clear` drops both lists and bumps the generation
/// with `wrapping_add`. It takes no log path and does no I/O, so a line already
/// stored in the room log stays there.
#[derive(Debug, Default)]
pub(crate) struct RoomSpeechQueue {
    generation: u64,
    waiting_audio: Vec<RoomSpeechClip>,
    unposted: Vec<RoomSpeechClip>,
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
        self.waiting_audio.clear();
        self.unposted.clear();
        self.generation = self.generation.wrapping_add(1);
        RoomSpeechClearReport {
            generation: self.generation,
            dropped_audio,
            dropped_unposted,
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
        take_matching(&mut self.waiting_audio, clip)
    }
}

fn take_matching(clips: &mut Vec<RoomSpeechClip>, clip: &RoomSpeechClip) -> bool {
    let Some(index) = clips.iter().position(|item| item == clip) else {
        return false;
    };
    clips.remove(index);
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
/// still dies. No log path: text already in the room log is left alone.
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

/// Record a member line whose log row exists and whose clip has not started.
///
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

/// Take the waiting clip only when `captured` is current and `clear` left it.
#[must_use]
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "called from speak_room_member_line, which tests do not compile"
    )
)]
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
/// snapshots peers already logged and does not wait for the others. When a
/// reply returns, it is appended and enqueued immediately. `NO_REPLY` skips
/// are left out. One TTS path plays the queue. This does not open a Voice
/// Agent session per member.
///
/// When `job.speech_gen` is stale, a reply that is not logged yet is dropped.
/// Lines already appended stay in the log.
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
    run_loaded_room_fanout(job, &loaded, "", &skipped_early, &oneshot);
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
) where
    F: Fn(&str, &str, bool, &str) -> Result<String, String> + Sync,
{
    let peers = Mutex::new(initial_peers.to_owned());
    let replied = Mutex::new(Vec::new());
    let skipped = Mutex::new(skipped_early.to_vec());
    std::thread::scope(|scope| {
        for member in members {
            let peers = &peers;
            let replied = &replied;
            let skipped = &skipped;
            scope.spawn(move || {
                run_one_room_member(job, member, members, peers, replied, skipped, oneshot);
            });
        }
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

fn run_one_room_member<F>(
    job: &RoomFanoutJob,
    member: &LoadedRoomMember,
    roster: &[LoadedRoomMember],
    peers: &Mutex<String>,
    replied: &Mutex<Vec<String>>,
    skipped: &Mutex<Vec<String>>,
    oneshot: &F,
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
    publish_logged_member_reply(job, member, reply.trim(), peers, replied, skipped);
}

/// `PRIVATE <name>: <message>` is a note to one member, not a room line.
fn looks_like_private_note(reply: &str) -> bool {
    private_note_body(reply).is_some()
}

/// Recipient token and note body. `None` when this is not a private note.
fn parse_private_note(reply: &str) -> Option<(String, String)> {
    let rest = private_note_body(reply)?;
    let (to, text) = rest.split_once(':')?;
    let to = to.trim();
    let text = text.trim();
    if to.is_empty() || text.is_empty() || to.contains('\n') || to.contains(',') {
        return None;
    }
    Some((to.to_owned(), text.to_owned()))
}

fn private_note_body(reply: &str) -> Option<&str> {
    let trimmed = reply.trim();
    let prefix = "PRIVATE";
    let rest = trimmed.get(prefix.len()..)?;
    if !trimmed[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return None;
    }
    if !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
        return None;
    }
    Some(rest.trim_start())
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

fn publish_logged_member_reply(
    job: &RoomFanoutJob,
    member: &LoadedRoomMember,
    trimmed: &str,
    peers: &Mutex<String>,
    replied: &Mutex<Vec<String>>,
    skipped: &Mutex<Vec<String>>,
) {
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
                kind: RoomLogKind::Say,
                text: trimmed.to_owned(),
                iteration: None,
                phase: None,
                to_profile_id: None,
                to_name: None,
                reply: None,
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
        peers_guard.push_str(&room_peer_said_line(&member.display, trimmed));
        push_line(replied, format!("{} ({})", member.display, member.id));
    }
    let queued = enqueue_waiting_room_audio(clip, job.speech_gen);
    #[cfg(not(test))]
    if queued {
        crate::talk::speak_room_member_line(&member.voice, trimmed, job.speech_gen);
    }
    #[cfg(test)]
    let _ = queued;
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
        run_loaded_room_fanout(&job, &members, "", &[], &oneshot);
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
        let log = std::fs::read_to_string(dir.join("parallel").join("log.jsonl")).expect("log");
        assert!(log.contains(sara_text));
        assert!(log.contains(spencer_text));
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
        run_loaded_room_fanout(&job, &members, &seed, &[], &oneshot);
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
        run_loaded_room_fanout(&job, &members, "", &[], &oneshot);
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
                .any(|line| line.kind == RoomLogKind::Say && line.text == say)
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
        run_loaded_room_fanout(&job, &members, "", &[], &oneshot);
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
}
