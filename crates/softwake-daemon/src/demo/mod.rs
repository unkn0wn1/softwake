//! Interactive voice-state demo.
//!
//! `wake` and `sleep` submit a configured phrase to [`TextWakeDetector`] and
//! apply the resulting hit on [`Machine`]. A phrase event is applied only
//! after mock capture delivers a frame, so hibernate (capture stopped) cannot
//! take a voice transition. `hibernate` and `resume` are UI events.
//!
//! The clock is the `elapsed` argument of [`Demo::handle_line`]. Callers pass
//! wall time; tests pass exact durations. Nothing in here sleeps.

use std::time::Duration;

use softwake_audio::{AudioCapture, MockAudioCapture};
#[cfg(test)]
use softwake_state::VoiceState;
use softwake_state::{CooldownConfig, Effect, Event, Machine};
use softwake_wake::{PhraseHit, PhraseTable, TextWakeDetector};

const COMMANDS: &str = "commands: wake, sleep, hibernate, resume, status, quit";

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

/// Sleeping daemon with mock capture and the text phrase spike.
#[derive(Debug)]
pub(crate) struct Demo {
    machine: Machine,
    capture: MockAudioCapture,
    detector: TextWakeDetector,
}

impl Demo {
    /// Start in sleep and start mock capture.
    pub(crate) fn new(table: PhraseTable, cooldown: CooldownConfig) -> Self {
        let mut capture = MockAudioCapture::default();
        // The mock device cannot fail to open.
        let Ok(()) = capture.start();
        Self {
            machine: Machine::new(cooldown),
            capture,
            detector: TextWakeDetector::new(table),
        }
    }

    /// Lines printed before the first command.
    pub(crate) fn banner_lines(&self) -> Vec<String> {
        let mut lines = vec!["softwaked demo".to_owned()];
        lines.extend(self.status_lines());
        lines.push(COMMANDS.to_owned());
        lines
    }

    /// Apply one stdin line.
    ///
    /// `elapsed` is how long passed since the previous call. It advances the
    /// phrase-cooldown clock before the command is interpreted.
    pub(crate) fn handle_line(&mut self, line: &str, elapsed: Duration) -> CommandResult {
        self.machine.advance(elapsed);
        match parse_line(line) {
            Parsed::Empty => CommandResult::stay(Vec::new()),
            Parsed::Wake => CommandResult::stay(self.voice(VoiceCommand::Wake)),
            Parsed::Sleep => CommandResult::stay(self.voice(VoiceCommand::Sleep)),
            Parsed::Hibernate => CommandResult::stay(self.ui(Event::UiHibernate)),
            Parsed::Resume => CommandResult::stay(self.ui(Event::UiResume)),
            Parsed::Status => CommandResult::stay(self.status_lines()),
            Parsed::Quit => CommandResult::quit(vec!["quit".to_owned()]),
            Parsed::Unknown => CommandResult::stay(vec![
                format!("unknown command: {}", line.trim()),
                COMMANDS.to_owned(),
            ]),
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
            return self.with_status(vec![format!(
                "rejected: no {} phrase configured",
                command.as_str()
            )]);
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
            return self.with_status(vec![format!(
                "rejected: voice input while capture is stopped ({})",
                self.machine.state()
            )]);
        }
        let hit = self.detector.push_text(text);
        let mut lines = vec![format!("heard: \"{text}\" -> {hit}")];
        if let Some(expected) = expected {
            if hit != expected {
                lines.push(format!("rejected: phrase scored as {hit}, not {expected}"));
                return self.with_status(lines);
            }
        }
        match hit {
            PhraseHit::Wake => lines.extend(self.transition(Event::WakePhrase)),
            PhraseHit::Sleep => lines.extend(self.transition(Event::SleepPhrase)),
            PhraseHit::None => {}
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

    fn transition(&mut self, event: Event) -> Vec<String> {
        match self.machine.apply(event) {
            Ok(applied) => {
                let mut lines = vec![format!(
                    "transition {} -> {} ({})",
                    applied.from, applied.to, applied.event
                )];
                for &effect in applied.effects {
                    self.run_effect(effect);
                    lines.push(format!("effect: {}", effect_label(effect)));
                }
                lines
            }
            Err(error) => vec![format!("rejected: {error}")],
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
        vec![
            format!("state: {}", self.machine.state()),
            format!("capture: {capture}"),
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
