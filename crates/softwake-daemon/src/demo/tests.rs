use std::time::Duration;

use softwake_session::SessionPhase;
use softwake_state::{CooldownConfig, VoiceState};
use softwake_wake::PhraseTable;

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
            "typed commands only — mic / PipeWire not wired yet".to_owned(),
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
    assert!(applied.contains("Tool allowlist: echo."));
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
    assert!(instructions.contains("Tool allowlist: echo."));

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
