//! Interactive typed-command voice-state demo.
//!
//! There is no microphone in this loop, and `PipeWire` is not wired. `wake` and
//! `sleep` submit a configured phrase to [`TextWakeDetector`] and apply the
//! resulting hit on [`Machine`]. A phrase event is applied only after mock
//! capture delivers a frame, so hibernate (capture stopped) cannot take a
//! voice transition. `hibernate` and `resume` are UI events.
//!
//! The clock is the `elapsed` argument of [`Demo::handle_line`]. Callers pass
//! wall time; tests pass exact durations. Nothing in here sleeps. Verbose
//! mode adds `verbose:` detail beside the same user-facing lines.
//!
//! A wake phrase is refused when the loaded soul pack is missing or invalid.
//! `reload-soul` reads the directory again; the new text applies on the next awake.

use std::time::Duration;

use softwake_audio::{AudioCapture, MockAudioCapture};
use softwake_soul::SoulDir;
#[cfg(test)]
use softwake_state::VoiceState;
use softwake_state::{CooldownConfig, Effect, Event, Machine};
use softwake_wake::{PhraseHit, PhraseTable, TextWakeDetector};

use crate::soul::LoadedSoul;

const COMMANDS: &str = "commands: wake, sleep, hibernate, resume, status, reload-soul, quit";
const TYPED_ONLY: &str = "typed commands only — mic / PipeWire not wired yet";

/// One step of the demo loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandResult {
    pub(crate) lines: Vec<String>,
    pub(crate) quit: bool,
}

impl CommandResult {
    fn stay(lines: Vec<String>) -> Self {
        Self { lines, quit: false }
    }

    fn quit(lines: Vec<String>) -> Self {
        Self { lines, quit: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Parsed {
    Empty,
    Wake,
    Sleep,
    Hibernate,
    Resume,
    Status,
    ReloadSoul,
    Quit,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceCommand {
    Wake,
    Sleep,
}

impl VoiceCommand {
    const fn hit(self) -> PhraseHit {
        match self {
            Self::Wake => PhraseHit::Wake,
            Self::Sleep => PhraseHit::Sleep,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Wake => "wake",
            Self::Sleep => "sleep",
        }
    }
}

impl Parsed {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Wake => "wake",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
            Self::Resume => "resume",
            Self::Status => "status",
            Self::ReloadSoul => "reload-soul",
            Self::Quit => "quit",
            Self::Unknown => "unknown",
        }
    }
}

/// Sleeping daemon with mock capture and the text phrase spike.
#[derive(Debug)]
pub(crate) struct Demo {
    machine: Machine,
    capture: MockAudioCapture,
    detector: TextWakeDetector,
    soul: LoadedSoul,
    verbose: bool,
}

impl Demo {
    /// Start in sleep and start mock capture.
    ///
    /// User-facing lines stay short. [`Self::with_verbose`] adds processing detail.
    pub(crate) fn new(table: PhraseTable, cooldown: CooldownConfig, soul_dir: SoulDir) -> Self {
        let mut capture = MockAudioCapture::default();
        // The mock device cannot fail to open.
        let Ok(()) = capture.start();
        Self {
            machine: Machine::new(cooldown),
            capture,
            detector: TextWakeDetector::new(table),
            soul: LoadedSoul::open(soul_dir),
            verbose: false,
        }
    }

    /// Add `verbose:` lines for each non-empty command.
    #[must_use]
    pub(crate) fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Lines printed before the first command.
    pub(crate) fn banner_lines(&self) -> Vec<String> {
        let mut lines = vec!["softwaked demo".to_owned(), TYPED_ONLY.to_owned()];
        lines.extend(self.status_lines());
        lines.push(COMMANDS.to_owned());
        lines
    }

    /// Apply one stdin line.
    ///
    /// `elapsed` is how long passed since the previous call. It advances the
    /// phrase-cooldown clock before the command is interpreted. Blank lines
    /// advance that clock and print nothing, including in verbose mode.
    pub(crate) fn handle_line(&mut self, line: &str, elapsed: Duration) -> CommandResult {
        self.machine.advance(elapsed);
        let parsed = parse_line(line);
        let mut lines = match parsed {
            Parsed::Empty => return CommandResult::stay(Vec::new()),
            Parsed::Wake => self.voice(VoiceCommand::Wake),
            Parsed::Sleep => self.voice(VoiceCommand::Sleep),
            Parsed::Hibernate => self.ui(Event::UiHibernate),
            Parsed::Resume => self.ui(Event::UiResume),
            Parsed::Status => self.status_lines(),
            Parsed::ReloadSoul => self.reload(),
            Parsed::Quit => vec!["quit".to_owned()],
            Parsed::Unknown => vec![
                format!("unknown command: {}", line.trim()),
                COMMANDS.to_owned(),
            ],
        };
        if self.verbose {
            let mut prefixed = Vec::with_capacity(lines.len() + 2);
            prefixed.push(format!("verbose: raw: {line:?}"));
            prefixed.push(format!("verbose: parsed: {}", parsed.as_str()));
            prefixed.append(&mut lines);
            lines = prefixed;
        }
        if parsed == Parsed::Quit {
            CommandResult::quit(lines)
        } else {
            CommandResult::stay(lines)
        }
    }

    #[cfg(test)]
    fn state(&self) -> VoiceState {
        self.machine.state()
    }

    #[cfg(test)]
    fn capture_running(&self) -> bool {
        self.capture.is_running()
    }

    #[cfg(test)]
    fn permits_tools(&self) -> bool {
        self.machine.permit_tool_dispatch().is_ok()
    }

    #[cfg(test)]
    fn applied_instructions(&self) -> Option<&str> {
        self.soul.applied_instructions()
    }

    fn voice(&mut self, command: VoiceCommand) -> Vec<String> {
        let phrase = match command {
            VoiceCommand::Wake => self
                .detector
                .table()
                .primary_wake_phrase()
                .map(str::to_owned),
            VoiceCommand::Sleep => self
                .detector
                .table()
                .primary_sleep_phrase()
                .map(str::to_owned),
        };
        let Some(phrase) = phrase else {
            let rejected = format!("rejected: no {} phrase configured", command.as_str());
            let mut lines = Vec::new();
            self.push_verbose(&mut lines, &rejected);
            lines.push(rejected);
            return self.with_status(lines);
        };
        self.hear(&phrase, Some(command.hit()))
    }

    /// Score `text` when capture delivers a frame, then apply that hit.
    ///
    /// `expected` rejects a hit that does not match the command. [`None`]
    /// applies whatever the detector returns, which is how tests show that a
    /// non-matching window leaves the machine alone.
    fn hear(&mut self, text: &str, expected: Option<PhraseHit>) -> Vec<String> {
        if !self.deliver_frame() {
            let mut lines = Vec::new();
            self.push_verbose(&mut lines, format!("phrase: {text:?}"));
            self.push_verbose(&mut lines, "rejected: capture stopped");
            lines.push(format!(
                "rejected: voice input while capture is stopped ({})",
                self.machine.state()
            ));
            return self.with_status(lines);
        }
        let hit = self.detector.push_text(text);
        let mut lines = Vec::new();
        self.push_verbose(&mut lines, format!("phrase: {text:?}"));
        self.push_verbose(&mut lines, format!("hit: {hit}"));
        lines.push(format!("heard: \"{text}\" -> {hit}"));
        if let Some(expected) = expected {
            if hit != expected {
                self.push_verbose(
                    &mut lines,
                    format!("rejected: hit mismatch (scored {hit}, expected {expected})"),
                );
                lines.push(format!("rejected: phrase scored as {hit}, not {expected}"));
                return self.with_status(lines);
            }
        }
        match hit {
            PhraseHit::Wake => lines.extend(self.transition(Event::WakePhrase)),
            PhraseHit::Sleep => lines.extend(self.transition(Event::SleepPhrase)),
            PhraseHit::None => self.push_verbose(&mut lines, "transition: none"),
        }
        self.with_status(lines)
    }

    /// A voice command is a frame plus a transcript. The text spike has no PCM
    /// encoder, so the frame is the capture token and the transcript is scored
    /// beside it. `stop` makes [`MockAudioCapture::poll_frame`] return nothing.
    fn deliver_frame(&mut self) -> bool {
        let accepted = self.capture.push_frame(&[]);
        let delivered = self.capture.poll_frame().is_some();
        accepted && delivered
    }

    fn ui(&mut self, event: Event) -> Vec<String> {
        let mut lines = self.transition(event);
        lines.extend(self.status_lines());
        lines
    }

    fn reload(&mut self) -> Vec<String> {
        self.soul.reload();
        self.with_status(vec![self.soul.reload_summary()])
    }

    fn transition(&mut self, event: Event) -> Vec<String> {
        if event == Event::WakePhrase {
            if let Some(reason) = self.soul.refusal() {
                let rejected = format!("rejected: {reason}");
                let mut lines = Vec::new();
                self.push_verbose(&mut lines, &rejected);
                lines.push(rejected);
                return lines;
            }
        }
        match self.machine.apply(event) {
            Ok(applied) => {
                if event == Event::WakePhrase {
                    self.soul.commit_awake();
                }
                let summary = format!("{} -> {} ({})", applied.from, applied.to, applied.event);
                let mut lines = Vec::new();
                self.push_verbose(&mut lines, format!("transition: ok {summary}"));
                lines.push(format!("transition {summary}"));
                for &effect in applied.effects {
                    self.run_effect(effect);
                    lines.push(format!("effect: {}", effect_label(effect)));
                }
                lines
            }
            Err(error) => {
                let rejected = format!("rejected: {error}");
                let mut lines = Vec::new();
                self.push_verbose(&mut lines, &rejected);
                lines.push(rejected);
                lines
            }
        }
    }

    fn push_verbose(&self, lines: &mut Vec<String>, detail: impl std::fmt::Display) {
        if self.verbose {
            lines.push(format!("verbose: {detail}"));
        }
    }

    fn run_effect(&mut self, effect: Effect) {
        match effect {
            // The session crate is a later milestone. `Machine::apply` has
            // already stored the new state, so tool dispatch follows it.
            Effect::OpenSession | Effect::ReleaseActingResources => {}
            Effect::StopCapture => {
                let Ok(()) = self.capture.stop();
            }
            Effect::StartCapture => {
                let Ok(()) = self.capture.start();
            }
        }
    }

    fn status_lines(&self) -> Vec<String> {
        let capture = if self.capture.is_running() {
            "running"
        } else {
            "stopped"
        };
        let soul = if self.soul.is_valid() {
            "ok"
        } else {
            "missing"
        };
        vec![
            format!("state: {}", self.machine.state()),
            format!("capture: {capture}"),
            format!("soul: {soul}"),
        ]
    }

    fn with_status(&self, mut lines: Vec<String>) -> Vec<String> {
        lines.extend(self.status_lines());
        lines
    }
}

fn parse_line(line: &str) -> Parsed {
    match line.trim().to_ascii_lowercase().as_str() {
        "" => Parsed::Empty,
        "wake" => Parsed::Wake,
        "sleep" => Parsed::Sleep,
        "hibernate" => Parsed::Hibernate,
        "resume" => Parsed::Resume,
        "status" => Parsed::Status,
        "reload-soul" => Parsed::ReloadSoul,
        "quit" => Parsed::Quit,
        _ => Parsed::Unknown,
    }
}

fn effect_label(effect: Effect) -> &'static str {
    match effect {
        Effect::OpenSession => "open session",
        Effect::ReleaseActingResources => "release acting resources",
        Effect::StopCapture => "stop capture",
        Effect::StartCapture => "start capture",
    }
}

#[cfg(test)]
mod tests;
