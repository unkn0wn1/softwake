use std::time::Duration;

use softwake_state::{CooldownConfig, VoiceState};
use softwake_wake::PhraseTable;

use super::{COMMANDS, CommandResult, Demo};

fn demo_with(cooldown: CooldownConfig) -> Demo {
    Demo::new(PhraseTable::default(), cooldown)
}

fn no_cooldown() -> CooldownConfig {
    CooldownConfig {
        post_wake: Duration::ZERO,
        post_sleep: Duration::ZERO,
    }
}

#[test]
fn new_demo_is_asleep_with_capture_running() {
    let demo = demo_with(CooldownConfig::default());
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
            COMMANDS.to_owned(),
        ]
    );
}

#[test]
fn wake_command_submits_the_configured_phrase() {
    let mut demo = demo_with(no_cooldown());
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "heard: \"hey softwake\" -> wake".to_owned(),
            "transition sleep -> awake (wake phrase)".to_owned(),
            "effect: open session".to_owned(),
            "state: awake".to_owned(),
            "capture: running".to_owned(),
        ]
    );
    assert!(demo.permits_tools());
    assert!(demo.capture_running());
}

#[test]
fn unmatched_text_does_not_transition() {
    let mut demo = demo_with(CooldownConfig::default());
    let lines = demo.hear("just chatting", None);
    assert_eq!(
        lines,
        vec![
            "heard: \"just chatting\" -> none".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(!demo.permits_tools());
}

#[test]
fn mixed_case_text_wakes_through_the_detector() {
    let mut demo = demo_with(no_cooldown());
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
    let mut demo = Demo::new(table, no_cooldown());
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "heard: \"go to sleep\" -> sleep".to_owned(),
            "rejected: phrase scored as sleep, not wake".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(!demo.permits_tools());
}

#[test]
fn sleep_while_asleep_reaches_the_machine() {
    let mut demo = demo_with(CooldownConfig::default());
    let result = demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "heard: \"softwake sleep\" -> sleep".to_owned(),
            "rejected: cannot apply sleep phrase from sleep: already asleep".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
        ]
    );
    assert!(demo.capture_running());
}

#[test]
fn post_wake_cooldown_blocks_sleep_until_the_clock_advances() {
    let mut demo = demo_with(CooldownConfig::default());
    demo.handle_line("wake", Duration::ZERO);
    let blocked = demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(
        blocked.lines,
        vec![
            "heard: \"softwake sleep\" -> sleep".to_owned(),
            "rejected: cannot apply sleep phrase during cooldown (800ms remaining)".to_owned(),
            "state: awake".to_owned(),
            "capture: running".to_owned(),
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
        ]
    );
    assert!(!demo.permits_tools());
    assert!(demo.capture_running());
}

#[test]
fn configured_cooldown_is_what_the_demo_waits_for() {
    let mut demo = demo_with(CooldownConfig {
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
    let mut demo = demo_with(no_cooldown());
    demo.handle_line("wake", Duration::ZERO);
    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
    assert!(demo.capture_running());
    assert!(!demo.permits_tools());
}

#[test]
fn hibernate_from_sleep_stops_capture() {
    let mut demo = demo_with(CooldownConfig::default());
    let result = demo.handle_line("hibernate", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "transition sleep -> hibernate (UI hibernate)".to_owned(),
            "effect: stop capture".to_owned(),
            "state: hibernate".to_owned(),
            "capture: stopped".to_owned(),
        ]
    );
    assert!(!demo.capture_running());
    assert!(!demo.permits_tools());
}

#[test]
fn hibernate_stops_capture_and_rejects_voice_until_resume() {
    let mut demo = demo_with(CooldownConfig::default());
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
    let mut demo = demo_with(CooldownConfig::default());
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
    let mut demo = Demo::new(table, CooldownConfig::default());
    let result = demo.handle_line("wake", Duration::ZERO);
    assert_eq!(
        result.lines,
        vec![
            "rejected: no wake phrase configured".to_owned(),
            "state: sleep".to_owned(),
            "capture: running".to_owned(),
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn blank_line_advances_the_cooldown_clock_without_printing() {
    let mut demo = demo_with(CooldownConfig::default());
    demo.handle_line("wake", Duration::ZERO);
    let blank = demo.handle_line("   ", Duration::from_millis(800));
    assert_eq!(blank, CommandResult::stay(Vec::new()));
    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn status_quit_and_unknown_commands() {
    let mut demo = demo_with(CooldownConfig::default());
    assert_eq!(
        demo.handle_line("status", Duration::ZERO).lines,
        vec!["state: sleep".to_owned(), "capture: running".to_owned()]
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
    let mut demo = demo_with(no_cooldown()).with_verbose(true);
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
        ]
    );
}

#[test]
fn verbose_unknown_names_the_raw_input() {
    let mut demo = demo_with(CooldownConfig::default()).with_verbose(true);
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
    let mut demo = demo_with(CooldownConfig::default()).with_verbose(true);
    demo.handle_line("wake", Duration::ZERO);
    let blank = demo.handle_line("   ", Duration::from_millis(800));
    assert_eq!(blank, CommandResult::stay(Vec::new()));
    demo.handle_line("sleep", Duration::ZERO);
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn verbose_cooldown_rejection_names_the_reason() {
    let mut demo = demo_with(CooldownConfig::default()).with_verbose(true);
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
        ]
    );
    assert_eq!(demo.state(), VoiceState::Awake);
}

#[test]
fn verbose_capture_stopped_names_the_reason_without_a_hit() {
    let mut demo = demo_with(CooldownConfig::default()).with_verbose(true);
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
    let mut demo = Demo::new(table, CooldownConfig::default()).with_verbose(true);
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
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
}

#[test]
fn verbose_hit_mismatch_names_the_reason() {
    let table = PhraseTable::new(["go to sleep"], ["go to sleep"]).expect("phrases");
    let mut demo = Demo::new(table, no_cooldown()).with_verbose(true);
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
        ]
    );
    assert_eq!(demo.state(), VoiceState::Sleep);
}
