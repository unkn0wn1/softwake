use std::time::Duration;

use softwake_session::SessionPhase;
use softwake_state::{CooldownConfig, VoiceState};
use softwake_voice::TranscriptEvent;
use softwake_wake::{PhraseHit, PhraseTable};

use super::{COMMANDS, CommandResult, Demo};
use crate::soul::TestSoulDir;

fn demo_with(cooldown: CooldownConfig) -> (Demo, TestSoulDir) {
    let soul = TestSoulDir::valid();
    let demo = Demo::new(PhraseTable::default(), cooldown, soul.soul_dir());
    (demo, soul)
}

fn demo_table(table: PhraseTable, cooldown: CooldownConfig) -> (Demo, TestSoulDir) {
    let soul = TestSoulDir::valid();
    let demo = Demo::new(table, cooldown, soul.soul_dir());
    (demo, soul)
}

fn no_cooldown() -> CooldownConfig {
    CooldownConfig {
        post_wake: Duration::ZERO,
        post_sleep: Duration::ZERO,
    }
}

#[test]
fn new_demo_is_asleep_with_capture_running() {
    let (demo, _soul) = demo_with(CooldownConfig::default());
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
    assert!(!demo.permits_tools());
    let banner = demo.banner_lines();
    assert!(
        banner
            .iter()
            .any(|line| { line.contains("typed commands only") && line.contains("PipeWire") })
    );
    assert_eq!(
        banner,
        vec![
            "softwaked demo".to_owned(),
            "typed commands only — mock capture; native PipeWire is feature-gated".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
            COMMANDS.to_owned(),
        ]
    );
}

#[test]
fn wake_command_submits_the_configured_phrase() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "heard: \"hey softwake\" -> wake".to_owned(),
            "transition sleep -> awake (wake phrase)".to_owned(),
            "effect: open session".to_owned(),
            "state: awake".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert!(demo.permits_tools());
    assert!(demo.capture_running());
    assert_eq!(demo.last_pcm_hit(), Some(PhraseHit::None));
}

#[test]
fn hibernate_does_not_score_a_pcm_frame() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    demo.handle_line("hibernate", Duration::ZERO);
    assert!(!demo.capture_running());
    let result = demo.handle_line("wake", Duration::ZERO);
    assert!(
        result
            .lines
            .iter()
            .any(|line| line.contains("capture is stopped"))
    );
    assert_eq!(demo.last_pcm_hit(), None);
}

#[test]
fn unmatched_text_does_not_transition() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    let lines = demo.hear("just chatting", None);
    assert_eq!(
        lines,
        vec![
            "heard: \"just chatting\" -> none".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(!demo.permits_tools());
}

#[test]
fn mixed_case_text_wakes_through_the_detector() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let lines = demo.hear("  HeY SoFtWaKe  ", None);
    assert_eq!(demo.state(), VoiceState::Awake);
    assert!(
        lines
            .iter()
            .any(|line| line == "transition sleep -> awake (wake phrase)")
    );
}

#[test]
fn wake_command_rejects_when_the_detector_scores_sleep() {
    let table = PhraseTable::new(["go to sleep"], ["go to sleep"]).expect("phrases");
    let (mut demo, _soul) = demo_table(table, no_cooldown());
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "heard: \"go to sleep\" -> sleep".to_owned(),
            "rejected: phrase scored as sleep, not wake".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(!demo.permits_tools());
}

#[test]
fn sleep_while_asleep_reaches_the_machine() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    let result = demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "heard: \"softwake sleep\" -> sleep".to_owned(),
            "rejected: cannot apply sleep phrase from sleep: already asleep".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert!(demo.capture_running());
}

#[test]
fn post_wake_cooldown_blocks_sleep_until_the_clock_advances() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    demo.handle_line("wake", Duration::ZERO);
    let blocked = demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(
        blocked.lines,
        vec![
            "heard: \"softwake sleep\" -> sleep".to_owned(),
            "rejected: cannot apply sleep phrase during cooldown (800ms remaining)".to_owned(),
            "state: awake".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Awake);

    let still = demo.handle_line("sleep", Duration::from_millis(799));
    assert!(still.lines.iter().any(|line| line.contains("cooldown")));
    assert_eq!(demo.state(), VoiceState::Awake);

    let slept = demo.handle_line("sleep", Duration::from_millis(1));
    assert_eq!(
        slept.lines,
        vec![
            "heard: \"softwake sleep\" -> sleep".to_owned(),
            "transition awake -> sleep (sleep phrase)".to_owned(),
            "effect: release acting resources".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert!(!demo.permits_tools());
    assert!(demo.capture_running());
}

#[test]
fn configured_cooldown_is_what_the_demo_waits_for() {
    let (mut demo, _soul) = demo_with(CooldownConfig {
        post_wake: Duration::from_millis(25),
        post_sleep: Duration::from_millis(25),
    });
    demo.handle_line("wake", Duration::ZERO);
    let blocked = demo.handle_line("sleep", Duration::from_millis(24));
    assert!(blocked.lines.iter().any(|line| line.contains("cooldown")));
    assert_eq!(demo.state(), VoiceState::Awake);
    demo.handle_line("sleep", Duration::from_millis(1));
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn zero_cooldown_allows_an_immediate_sleep_phrase() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    demo.handle_line("wake", Duration::ZERO);
    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
    assert!(!demo.permits_tools());
}

#[test]
fn hibernate_from_sleep_stops_capture() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    let result = demo.handle_line("hibernate", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "transition sleep -> hibernate (UI hibernate)".to_owned(),
            "effect: stop capture".to_owned(),
            "state: hibernate".to_owned(),
            "capture: stopped".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert!(!demo.capture_running());
    assert!(!demo.permits_tools());
}

#[test]
fn hibernate_stops_capture_and_rejects_voice_until_resume() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    demo.handle_line("wake", Duration::ZERO);

    let hibernate = demo.handle_line("hibernate", Duration::ZERO);
    assert_eq!(
        hibernate.lines,
        vec![
            "transition awake -> hibernate (UI hibernate)".to_owned(),
            "effect: release acting resources".to_owned(),
            "effect: stop capture".to_owned(),
            "state: hibernate".to_owned(),
            "capture: stopped".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert!(!demo.capture_running());
    assert!(!demo.permits_tools());

    let voice = demo.handle_line("WAKE", Duration::ZERO);
    assert_eq!(
        voice.lines,
        vec![
            "rejected: voice input while capture is stopped (hibernate)".to_owned(),
            "state: hibernate".to_owned(),
            "capture: stopped".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert!(voice.lines.iter().all(|line| !line.starts_with("heard:")));
    assert_eq!(demo.state(), VoiceState::Hibernate);
    assert!(!demo.capture_running());

    let resume = demo.handle_line("resume", Duration::ZERO);
    assert_eq!(
        resume.lines,
        vec![
            "transition hibernate -> sleep (UI resume)".to_owned(),
            "effect: start capture".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
    assert!(!demo.permits_tools());

    let too_soon = demo.handle_line("wake", Duration::ZERO);
    assert!(too_soon.lines.iter().any(|line| line.contains("cooldown")));
    assert_eq!(demo.state(), VoiceState::Sleep);

    demo.handle_line("wake", Duration::from_millis(800));
    assert_eq!(demo.state(), VoiceState::Awake);
    assert!(demo.permits_tools());
}

#[test]
fn resume_outside_hibernate_is_rejected() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    let result = demo.handle_line("resume", Duration::ZERO);
    assert_eq!(
        result.lines[0],
        "rejected: cannot apply UI resume from sleep: UI resume only leaves hibernate, and it lands in sleep"
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
}

#[test]
fn wake_without_a_configured_phrase_does_not_transition() {
    let table = PhraseTable::new(std::iter::empty::<&str>(), ["go to sleep"]).expect("table");
    let (mut demo, _soul) = demo_table(table, CooldownConfig::default());
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "rejected: no wake phrase configured".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn blank_line_advances_the_cooldown_clock_without_printing() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    demo.handle_line("wake", Duration::ZERO);
    let blank = demo.handle_line("   ", Duration::from_millis(800));
    assert_eq!(blank, CommandResult::stay(Vec::new()));
    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn status_quit_and_unknown_commands() {
    let (mut demo, _soul) = demo_with(CooldownConfig::default());
    assert_eq!(
        demo.handle_line("status", Duration::ZERO).lines,
        vec![
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    let unknown = demo.handle_line("dance", Duration::ZERO);
    assert_eq!(
        unknown.lines,
        vec!["unknown command: dance".to_owned(), COMMANDS.to_owned(),]
    );
    assert!(!unknown.quit);
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert_eq!(
        demo.handle_line("quit", Duration::ZERO),
        CommandResult::quit(vec!["quit".to_owned()])
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn verbose_wake_records_the_phrase_hit_and_transition() {
    let (demo, _soul) = demo_with(no_cooldown());
    let mut demo = demo.with_verbose(true);
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "verbose: raw: \"wake\"".to_owned(),
            "verbose: parsed: wake".to_owned(),
            "verbose: phrase: \"hey softwake\"".to_owned(),
            "verbose: hit: wake".to_owned(),
            "heard: \"hey softwake\" -> wake".to_owned(),
            "verbose: transition: ok sleep -> awake (wake phrase)".to_owned(),
            "transition sleep -> awake (wake phrase)".to_owned(),
            "effect: open session".to_owned(),
            "state: awake".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
}

#[test]
fn verbose_unknown_names_the_raw_input() {
    let (demo, _soul) = demo_with(CooldownConfig::default());
    let mut demo = demo.with_verbose(true);
    let unknown = demo.handle_line("dance", Duration::ZERO);
    assert_eq!(
        unknown.lines,
        vec![
            "verbose: raw: \"dance\"".to_owned(),
            "verbose: parsed: unknown".to_owned(),
            "unknown command: dance".to_owned(),
            COMMANDS.to_owned(),
        ]
    );
    assert!(!unknown.quit);
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn verbose_blank_line_stays_silent_and_advances_cooldown() {
    let (demo, _soul) = demo_with(CooldownConfig::default());
    let mut demo = demo.with_verbose(true);
    demo.handle_line("wake", Duration::ZERO);
    let blank = demo.handle_line("   ", Duration::from_millis(800));
    assert_eq!(blank, CommandResult::stay(Vec::new()));
    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn verbose_cooldown_rejection_names_the_reason() {
    let (demo, _soul) = demo_with(CooldownConfig::default());
    let mut demo = demo.with_verbose(true);
    demo.handle_line("wake", Duration::ZERO);
    let blocked = demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(
        blocked.lines,
        vec![
            "verbose: raw: \"sleep\"".to_owned(),
            "verbose: parsed: sleep".to_owned(),
            "verbose: phrase: \"softwake sleep\"".to_owned(),
            "verbose: hit: sleep".to_owned(),
            "heard: \"softwake sleep\" -> sleep".to_owned(),
            "verbose: rejected: cannot apply sleep phrase during cooldown (800ms remaining)"
                .to_owned(),
            "rejected: cannot apply sleep phrase during cooldown (800ms remaining)".to_owned(),
            "state: awake".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Awake);
}

#[test]
fn verbose_capture_stopped_names_the_reason_without_a_hit() {
    let (demo, _soul) = demo_with(CooldownConfig::default());
    let mut demo = demo.with_verbose(true);
    demo.handle_line("hibernate", Duration::ZERO);
    let voice = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        voice.lines,
        vec![
            "verbose: raw: \"wake\"".to_owned(),
            "verbose: parsed: wake".to_owned(),
            "verbose: phrase: \"hey softwake\"".to_owned(),
            "verbose: rejected: capture stopped".to_owned(),
            "rejected: voice input while capture is stopped (hibernate)".to_owned(),
            "state: hibernate".to_owned(),
            "capture: stopped".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert!(voice.lines.iter().all(|line| !line.starts_with("heard:")));
    assert!(
        voice
            .lines
            .iter()
            .all(|line| !line.starts_with("verbose: hit:"))
    );
}

#[test]
fn verbose_missing_phrase_names_the_reason() {
    let table = PhraseTable::new(std::iter::empty::<&str>(), ["go to sleep"]).expect("table");
    let (demo, _soul) = demo_table(table, CooldownConfig::default());
    let mut demo = demo.with_verbose(true);
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "verbose: raw: \"wake\"".to_owned(),
            "verbose: parsed: wake".to_owned(),
            "verbose: rejected: no wake phrase configured".to_owned(),
            "rejected: no wake phrase configured".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn verbose_hit_mismatch_names_the_reason() {
    let table = PhraseTable::new(["go to sleep"], ["go to sleep"]).expect("phrases");
    let (demo, _soul) = demo_table(table, no_cooldown());
    let mut demo = demo.with_verbose(true);
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "verbose: raw: \"wake\"".to_owned(),
            "verbose: parsed: wake".to_owned(),
            "verbose: phrase: \"go to sleep\"".to_owned(),
            "verbose: hit: sleep".to_owned(),
            "heard: \"go to sleep\" -> sleep".to_owned(),
            "verbose: rejected: hit mismatch (scored sleep, expected wake)".to_owned(),
            "rejected: phrase scored as sleep, not wake".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
            "soul: ok".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn missing_soul_refuses_wake_until_reload() {
    let soul = TestSoulDir::empty();
    let mut demo = Demo::new(PhraseTable::default(), no_cooldown(), soul.soul_dir());

    let refused = demo.handle_line("wake", Duration::ZERO);
    assert!(
        refused
            .lines
            .iter()
            .any(|line| line.contains("refusing awake"))
    );
    assert!(
        refused
            .lines
            .iter()
            .any(|line| line.contains("missing soul.md"))
    );
    assert!(refused.lines.iter().any(|line| line == "soul: missing"));
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
    assert!(!demo.permits_tools());
    assert!(demo.applied_instructions().is_none());

    demo.handle_line("hibernate", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Hibernate);
    assert!(!demo.capture_running());
    assert!(!demo.permits_tools());

    demo.handle_line("resume", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
    assert!(!demo.permits_tools());

    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());

    let still = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(
        still
            .lines
            .iter()
            .any(|line| line.contains("refusing awake"))
    );

    soul.write("Fresh soul\n", "Fresh user\n");
    let reloaded = demo.handle_line("reload-soul", Duration::ZERO);
    assert!(
        reloaded
            .lines
            .iter()
            .any(|line| line.contains("reloaded soul pack; applies on next awake"))
    );
    assert!(reloaded.lines.iter().any(|line| line == "soul: ok"));
    assert!(demo.applied_instructions().is_none());

    demo.handle_line("wake", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Awake);
    assert!(demo.capture_running());
    assert!(demo.permits_tools());
    let applied = demo.applied_instructions().expect("applied");
    assert!(applied.contains("# Identity"));
    assert!(applied.contains("Fresh soul"));
    assert!(applied.contains("# User profile"));
    assert!(applied.contains("Fresh user"));
    assert!(applied.contains("# Runtime policy"));
    assert!(applied.contains("State: awake."));
    assert!(applied.contains("echo (safe)"));
    assert!(applied.contains("notify (confirm)"));
    assert!(applied.contains("email_send (confirm)"));
    assert_eq!(demo.session_phase(), SessionPhase::Open);
    assert_eq!(demo.session_instructions(), Some(applied));
}

#[test]
fn tool_echo_requires_awake_and_the_session_opens_and_closes() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    assert_eq!(demo.session_phase(), SessionPhase::Closed);
    assert!(demo.session_instructions().is_none());

    let asleep = demo.handle_line("tool echo hello", Duration::ZERO);
    assert!(
        asleep
            .lines
            .iter()
            .any(|line| line.contains("cannot run echo while sleep"))
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert_eq!(demo.session_phase(), SessionPhase::Closed);

    let unknown_asleep = demo.handle_line("tool volume", Duration::ZERO);
    assert!(
        unknown_asleep
            .lines
            .iter()
            .any(|line| line.contains("cannot run volume while sleep"))
    );

    demo.handle_line("wake", Duration::ZERO);
    assert_eq!(demo.session_phase(), SessionPhase::Open);
    let instructions = demo.session_instructions().expect("open").to_owned();
    assert!(instructions.contains("test soul"));
    assert!(instructions.contains("test user"));
    assert!(instructions.contains("echo (safe)"));
    assert!(instructions.contains("notify (confirm)"));
    assert!(instructions.contains("email_send (confirm)"));

    let ran = demo.handle_line("tool echo Hello", Duration::ZERO);
    assert!(
        ran.lines
            .iter()
            .any(|line| line == "tool echo: echo: Hello")
    );
    assert_eq!(demo.state(), VoiceState::Awake);

    let pong = demo.handle_line("tool echo", Duration::ZERO);
    assert!(pong.lines.iter().any(|line| line == "tool echo: pong"));

    let multi = demo.handle_line("tool echo hello world", Duration::ZERO);
    assert!(
        multi
            .lines
            .iter()
            .any(|line| line == "tool echo: echo: hello world")
    );

    let unknown = demo.handle_line("tool volume", Duration::ZERO);
    assert!(
        unknown
            .lines
            .iter()
            .any(|line| line.contains("unknown tool: volume"))
    );
    assert_eq!(demo.state(), VoiceState::Awake);
    assert_eq!(demo.session_instructions(), Some(instructions.as_str()));

    let missing = demo.handle_line("tool", Duration::ZERO);
    assert!(
        missing
            .lines
            .iter()
            .any(|line| line.contains("tool needs a name"))
    );

    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
    assert_eq!(demo.session_phase(), SessionPhase::Closed);
    assert!(demo.session_instructions().is_none());
    let after_sleep = demo.handle_line("tool echo", Duration::ZERO);
    assert!(
        after_sleep
            .lines
            .iter()
            .any(|line| line.contains("cannot run echo while sleep"))
    );

    demo.handle_line("wake", Duration::ZERO);
    assert_eq!(demo.session_phase(), SessionPhase::Open);
    demo.handle_line("hibernate", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Hibernate);
    assert!(!demo.capture_running());
    assert_eq!(demo.session_phase(), SessionPhase::Closed);
    let hibernated = demo.handle_line("tool echo", Duration::ZERO);
    assert!(
        hibernated
            .lines
            .iter()
            .any(|line| line.contains("cannot run echo while hibernate"))
    );
}

#[test]
fn notify_waits_for_confirm_and_cancel_does_not_append() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let asleep = demo.handle_line("tool notify hello", Duration::ZERO);
    assert!(
        asleep
            .lines
            .iter()
            .any(|line| line.contains("cannot run notify while sleep"))
    );
    assert!(demo.notifications().is_empty());

    demo.handle_line("wake", Duration::ZERO);
    let pending = demo.handle_line("tool notify hello", Duration::ZERO);
    assert!(
        pending
            .lines
            .iter()
            .any(|line| line.starts_with("pending 1: notify — "))
    );
    assert!(
        pending
            .lines
            .iter()
            .any(|line| line == "waiting for confirm")
    );
    assert!(
        pending
            .lines
            .iter()
            .any(|line| line == "pending: 1 notify hello")
    );
    assert!(demo.notifications().is_empty());

    let echo = demo.handle_line("tool echo hi", Duration::ZERO);
    assert!(echo.lines.iter().any(|line| line == "tool echo: echo: hi"));
    assert!(demo.notifications().is_empty());

    let busy = demo.handle_line("tool notify other", Duration::ZERO);
    assert!(
        busy.lines
            .iter()
            .any(|line| line.contains("a confirmation is already pending: 1"))
    );
    assert!(demo.notifications().is_empty());

    let cancelled = demo.handle_line("cancel", Duration::ZERO);
    assert!(
        cancelled
            .lines
            .iter()
            .any(|line| line == "cancelled 1: notify")
    );
    assert!(demo.notifications().is_empty());
    let again = demo.handle_line("confirm", Duration::ZERO);
    assert!(
        again
            .lines
            .iter()
            .any(|line| line.contains("no pending confirmation"))
    );

    demo.handle_line("tool notify hello world", Duration::ZERO);
    let confirmed = demo.handle_line("confirm-tool 2", Duration::ZERO);
    assert!(
        confirmed
            .lines
            .iter()
            .any(|line| { line == "confirmed 2: tool notify: hello world" })
    );
    assert_eq!(demo.notifications(), ["hello world"]);
    assert!(
        confirmed
            .lines
            .iter()
            .any(|line| line == "notification: hello world")
    );
    let spent = demo.handle_line("confirm 2", Duration::ZERO);
    assert!(
        spent
            .lines
            .iter()
            .any(|line| line.contains("unknown pending confirmation: 2"))
    );
    assert_eq!(demo.notifications(), ["hello world"]);
}

#[test]
fn sleep_clears_a_pending_notification_without_appending() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    demo.handle_line("wake", Duration::ZERO);
    demo.handle_line("tool notify later", Duration::ZERO);
    let slept = demo.handle_line("sleep", Duration::ZERO);
    assert!(slept.lines.iter().any(|line| line == "cancelled 1: notify"));
    assert!(demo.notifications().is_empty());
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn shell_is_denied_and_hibernate_refuses_notify() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    demo.handle_line("wake", Duration::ZERO);
    let denied = demo.handle_line("tool shell rm", Duration::ZERO);
    assert!(
        denied
            .lines
            .iter()
            .any(|line| line.contains("tool denied: shell"))
    );
    let unknown = demo.handle_line("tool volume", Duration::ZERO);
    assert!(
        unknown
            .lines
            .iter()
            .any(|line| line.contains("unknown tool: volume"))
    );
    assert!(demo.notifications().is_empty());

    demo.handle_line("tool notify bye", Duration::ZERO);
    demo.handle_line("hibernate", Duration::ZERO);
    assert!(demo.notifications().is_empty());
    let refused = demo.handle_line("tool notify bye", Duration::ZERO);
    assert!(
        refused
            .lines
            .iter()
            .any(|line| line.contains("cannot run notify while hibernate"))
    );
}

#[test]
fn confirm_and_cancel_reject_extra_arguments() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let confirm = demo.handle_line("confirm 1 extra", Duration::ZERO);
    assert!(
        confirm
            .lines
            .iter()
            .any(|line| line.contains("confirm takes one pending id"))
    );
    let cancel = demo.handle_line("CANCEL-TOOL 1 extra", Duration::ZERO);
    assert!(
        cancel
            .lines
            .iter()
            .any(|line| line.contains("cancel takes one pending id"))
    );
}

#[test]
fn hear_injects_transcript_while_awake() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    demo.handle_line("wake", Duration::ZERO);
    let result = demo.handle_line("hear hello there", Duration::from_millis(800));
    assert!(
        result
            .lines
            .iter()
            .any(|line| line == "partial transcript: \"hello there\"")
    );
    assert!(
        result
            .lines
            .iter()
            .any(|line| line == "final transcript: \"hello there\"")
    );
    assert_eq!(
        demo.last_transcripts(),
        &[
            TranscriptEvent::Partial {
                text: "hello there".to_owned(),
            },
            TranscriptEvent::Final {
                text: "hello there".to_owned(),
            },
        ]
    );
}

#[test]
fn hear_refused_while_asleep_and_hibernating() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let asleep = demo.handle_line("hear nope", Duration::ZERO);
    assert!(
        asleep
            .lines
            .iter()
            .any(|line| line.contains("hear while sleep"))
    );
    assert!(demo.last_transcripts().is_empty());

    demo.handle_line("hibernate", Duration::ZERO);
    let hibernating = demo.handle_line("hear nope", Duration::ZERO);
    assert!(
        hibernating
            .lines
            .iter()
            .any(|line| line.contains("hear while hibernate"))
    );
}

#[test]
fn say_records_speech_while_awake_and_refuses_asleep() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let asleep = demo.handle_line("say hello", Duration::ZERO);
    assert!(
        asleep
            .lines
            .iter()
            .any(|line| line.contains("say while sleep"))
    );
    assert!(demo.spoken().is_empty());

    demo.handle_line("wake", Duration::ZERO);
    let said = demo.handle_line("say hello", Duration::from_millis(800));
    assert!(said.lines.iter().any(|line| line == "said: \"hello\""));
    assert_eq!(demo.spoken(), &["hello".to_owned()]);
    assert!(said.lines.iter().any(|line| line == "last said: hello"));
}

#[test]
fn hear_and_say_need_text() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    demo.handle_line("wake", Duration::ZERO);
    let hear = demo.handle_line("hear", Duration::from_millis(800));
    assert!(
        hear.lines
            .iter()
            .any(|line| line.contains("hear needs text"))
    );
    let say = demo.handle_line("say", Duration::ZERO);
    assert!(say.lines.iter().any(|line| line.contains("say needs text")));
}

#[test]
fn email_send_waits_for_confirm_and_then_appends_once() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    let asleep = demo.handle_line("tool email_send ada@example.com hello body", Duration::ZERO);
    assert!(
        asleep
            .lines
            .iter()
            .any(|line| { line.contains("cannot run email_send while sleep") })
    );
    assert!(demo.outbox().is_empty());

    demo.handle_line("wake", Duration::ZERO);
    let short = demo.handle_line("tool email_send ada@example.com hello", Duration::ZERO);
    assert!(
        short
            .lines
            .iter()
            .any(|line| { line == "rejected: email_send needs to, subject, and body" })
    );
    assert!(short.lines.iter().all(|line| line != "waiting for confirm"));
    assert!(demo.outbox().is_empty());

    let pending = demo.handle_line(
        "tool email_send ada@example.com hello a short note",
        Duration::ZERO,
    );
    assert!(pending.lines.iter().any(|line| {
        line == "pending 1: email_send — Append one message to the in-memory outbox."
    }));
    assert!(
        pending
            .lines
            .iter()
            .any(|line| line == "waiting for confirm")
    );
    assert!(
        pending
            .lines
            .iter()
            .any(|line| { line == "pending: 1 email_send ada@example.com hello a short note" })
    );
    assert!(
        pending
            .lines
            .iter()
            .any(|line| { line == "last tool: email_send confirm pending" })
    );
    assert!(pending.lines.iter().all(|line| !line.starts_with("email:")));
    assert!(demo.outbox().is_empty());

    let echo = demo.handle_line("tool echo hi", Duration::ZERO);
    assert!(echo.lines.iter().any(|line| line == "tool echo: echo: hi"));
    assert!(demo.outbox().is_empty());

    let busy = demo.handle_line("tool email_send ada@example.com other note", Duration::ZERO);
    assert!(
        busy.lines
            .iter()
            .any(|line| { line.contains("confirmation is already pending: 1") })
    );
    assert!(demo.outbox().is_empty());

    let confirmed = demo.handle_line("confirm", Duration::ZERO);
    assert!(
        confirmed
            .lines
            .iter()
            .any(|line| { line == "confirmed 1: tool email_send: sent 1" })
    );
    assert!(
        confirmed
            .lines
            .iter()
            .any(|line| { line == "last tool: email_send confirm confirmed" })
    );
    assert!(
        confirmed
            .lines
            .iter()
            .any(|line| { line == "email: ada@example.com | hello | a short note" })
    );
    assert_eq!(demo.outbox().len(), 1);
    assert_eq!(demo.outbox()[0].to, "ada@example.com");
    assert_eq!(demo.outbox()[0].subject, "hello");
    assert_eq!(demo.outbox()[0].body, "a short note");
    assert!(demo.notifications().is_empty());

    let spent = demo.handle_line("confirm", Duration::ZERO);
    assert!(
        spent
            .lines
            .iter()
            .any(|line| { line.contains("no pending confirmation") })
    );
    assert_eq!(demo.outbox().len(), 1);

    let slept = demo.handle_line("sleep", Duration::ZERO);
    assert!(
        slept
            .lines
            .iter()
            .any(|line| { line == "email: ada@example.com | hello | a short note" })
    );
    assert_eq!(demo.outbox().len(), 1);
}

#[test]
fn cancel_sleep_and_hibernate_do_not_send_email() {
    let (mut demo, _soul) = demo_with(no_cooldown());
    demo.handle_line("wake", Duration::ZERO);
    demo.handle_line(
        "tool email_send ada@example.com hello later",
        Duration::ZERO,
    );
    let cancelled = demo.handle_line("cancel", Duration::ZERO);
    assert!(
        cancelled
            .lines
            .iter()
            .any(|line| line == "cancelled 1: email_send")
    );
    assert!(
        cancelled
            .lines
            .iter()
            .all(|line| !line.starts_with("email:"))
    );
    assert!(demo.outbox().is_empty());

    demo.handle_line(
        "tool email_send ada@example.com hello later",
        Duration::ZERO,
    );
    let slept = demo.handle_line("sleep", Duration::ZERO);
    assert!(
        slept
            .lines
            .iter()
            .any(|line| line == "cancelled 2: email_send")
    );
    assert!(demo.outbox().is_empty());

    demo.handle_line("wake", Duration::ZERO);
    demo.handle_line(
        "tool email_send ada@example.com hello later",
        Duration::ZERO,
    );
    let hibernated = demo.handle_line("hibernate", Duration::ZERO);
    assert!(
        hibernated
            .lines
            .iter()
            .any(|line| line == "cancelled 3: email_send")
    );
    assert!(demo.outbox().is_empty());
    let refused = demo.handle_line(
        "tool email_send ada@example.com hello later",
        Duration::ZERO,
    );
    assert!(
        refused
            .lines
            .iter()
            .any(|line| { line.contains("cannot run email_send while hibernate") })
    );
    assert!(demo.outbox().is_empty());
}
