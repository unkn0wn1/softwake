//! Interactive typed-command voice-state demo.
//!
//! There is no microphone in this loop. `wake` and `sleep` submit a configured
//! phrase to [`TextWakeDetector`] and apply the resulting hit on [`Machine`].
//! A phrase event is applied only after mock capture delivers a frame, so
//! hibernate (capture stopped) cannot take a voice transition. That same frame
//! is silence at 16 kHz and is scored by [`NullDetector`] through
//! [`crate::pcm::score_frame`]. The text hit still drives the transition.
//! Native `PipeWire` is feature-gated and is not opened here. `hibernate` and
//! `resume` are UI events.
//!
//! The clock is the `elapsed` argument of [`Demo::handle_line`]. Callers pass
//! wall time; tests pass exact durations. Nothing in here sleeps. Verbose
//! mode adds `verbose:` detail beside the same user-facing lines.
//!
//! A wake phrase is refused when the loaded soul pack is missing or invalid.
//! `reload-soul` reads the directory again; the new text applies on the next awake.
//! Entering awake opens a text session with those instructions. Sleep and
//! hibernate close it and drop a pending confirmation. `tool` runs a safe tool
//! while awake. A confirm-gated tool waits for `confirm` or `confirm-tool`.
//! `cancel` clears that pending call without running it.

use std::time::Duration;

use softwake_audio::{AudioCapture, MockAudioCapture};
#[cfg(test)]
use softwake_session::SessionPhase;
use softwake_session::TextStubSession;
use softwake_soul::SoulDir;
#[cfg(test)]
use softwake_state::VoiceState;
use softwake_state::{CooldownConfig, Effect, Event, Machine};
use softwake_voice::{MockStt, MockTts, TextToSpeech, TranscriptEvent};
use softwake_wake::{NullDetector, PhraseHit, PhraseTable, TextWakeDetector};

use crate::dispatch::{Hands, RequestOutcome};
use crate::pcm::score_frame;
use crate::soul::LoadedSoul;

const COMMANDS: &str = "commands: wake, sleep, hibernate, resume, status, reload-soul, tool, confirm, cancel, hear, say, quit";
const TYPED_ONLY: &str = "typed commands only — mock capture; native PipeWire is feature-gated";

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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Parsed {
    Empty,
    Wake,
    Sleep,
    Hibernate,
    Resume,
    Status,
    ReloadSoul,
    Tool { name: String, args: Vec<String> },
    ToolMissingName,
    Confirm { id: Option<String> },
    Cancel { id: Option<String> },
    ConfirmExtra,
    CancelExtra,
    Hear { text: String },
    HearMissingText,
    Say { text: String },
    SayMissingText,
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
    fn as_str(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Wake => "wake",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
            Self::Resume => "resume",
            Self::Status => "status",
            Self::ReloadSoul => "reload-soul",
            Self::Tool { .. } | Self::ToolMissingName => "tool",
            Self::Confirm { .. } | Self::ConfirmExtra => "confirm",
            Self::Cancel { .. } | Self::CancelExtra => "cancel",
            Self::Hear { .. } | Self::HearMissingText => "hear",
            Self::Say { .. } | Self::SayMissingText => "say",
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
    /// PCM stand-in. Typed commands do not follow its hit.
    pcm: NullDetector,
    /// Last PCM score from a delivered frame. `None` means no frame was scored.
    #[cfg_attr(not(test), allow(dead_code))]
    last_pcm_hit: Option<PhraseHit>,
    soul: LoadedSoul,
    session: TextStubSession,
    hands: Hands,
    /// Mock STT. Typed `hear` injects while awake.
    stt: MockStt,
    /// Mock TTS. Typed `say` records while awake.
    tts: MockTts,
    /// Last STT events produced by a successful `hear` (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    last_transcripts: Vec<TranscriptEvent>,
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
            pcm: NullDetector,
            last_pcm_hit: None,
            soul: LoadedSoul::open(soul_dir),
            session: TextStubSession::default(),
            hands: Hands::new(),
            stt: MockStt::default(),
            tts: MockTts::default(),
            last_transcripts: Vec::new(),
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
        let parsed_label = parsed.as_str();
        let quit = matches!(parsed, Parsed::Quit);
        let mut lines = match parsed {
            Parsed::Empty => return CommandResult::stay(Vec::new()),
            Parsed::Wake => self.voice(VoiceCommand::Wake),
            Parsed::Sleep => self.voice(VoiceCommand::Sleep),
            Parsed::Hibernate => self.ui(Event::UiHibernate),
            Parsed::Resume => self.ui(Event::UiResume),
            Parsed::Status => self.status_lines(),
            Parsed::ReloadSoul => self.reload(),
            Parsed::Tool { name, args } => self.tool(&name, &args),
            Parsed::Confirm { id } => self.confirm_pending(id),
            Parsed::Cancel { id } => self.cancel_pending(id),
            Parsed::ConfirmExtra => self.reject_extra("confirm"),
            Parsed::CancelExtra => self.reject_extra("cancel"),
            Parsed::Hear { text } => self.inject_transcript(&text),
            Parsed::HearMissingText => {
                let mut lines = vec!["rejected: hear needs text".to_owned(), COMMANDS.to_owned()];
                if self.verbose {
                    lines.insert(0, "verbose: rejected: hear needs text".to_owned());
                }
                lines
            }
            Parsed::Say { text } => self.speak_text(&text),
            Parsed::SayMissingText => {
                let mut lines = vec!["rejected: say needs text".to_owned(), COMMANDS.to_owned()];
                if self.verbose {
                    lines.insert(0, "verbose: rejected: say needs text".to_owned());
                }
                lines
            }
            Parsed::ToolMissingName => {
                let mut lines = vec![
                    "rejected: tool needs a name".to_owned(),
                    COMMANDS.to_owned(),
                ];
                if self.verbose {
                    lines.insert(0, "verbose: rejected: tool needs a name".to_owned());
                }
                lines
            }
            Parsed::Quit => vec!["quit".to_owned()],
            Parsed::Unknown => vec![
                format!("unknown command: {}", line.trim()),
                COMMANDS.to_owned(),
            ],
        };
        if self.verbose {
            let mut prefixed = Vec::with_capacity(lines.len() + 2);
            prefixed.push(format!("verbose: raw: {line:?}"));
            prefixed.push(format!("verbose: parsed: {parsed_label}"));
            prefixed.append(&mut lines);
            lines = prefixed;
        }
        if quit {
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

    /// PCM hit from the last frame [`Self::deliver_frame`] scored.
    #[cfg(test)]
    fn last_pcm_hit(&self) -> Option<PhraseHit> {
        self.last_pcm_hit
    }

    #[cfg(test)]
    fn permits_tools(&self) -> bool {
        self.machine.permit_tool_dispatch().is_ok()
    }

    #[cfg(test)]
    fn spoken(&self) -> &[String] {
        self.tts.spoken()
    }

    #[cfg(test)]
    fn last_transcripts(&self) -> &[TranscriptEvent] {
        &self.last_transcripts
    }

    #[cfg(test)]
    fn applied_instructions(&self) -> Option<&str> {
        self.soul.applied_instructions()
    }

    #[cfg(test)]
    fn session_phase(&self) -> SessionPhase {
        self.session.phase()
    }

    #[cfg(test)]
    fn session_instructions(&self) -> Option<&str> {
        self.session.instructions()
    }

    #[cfg(test)]
    fn notifications(&self) -> Vec<String> {
        self.hands.notifications().iter().cloned().collect()
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

    /// Inject a mock STT partial and final while awake.
    ///
    /// Sleep and hibernate refuse the channel. This is the typed stand-in for
    /// a streaming ASR partial/final pair ([ADR 0007](../../../../docs/ADR-0007-awake-stt-tts.md)).
    fn inject_transcript(&mut self, text: &str) -> Vec<String> {
        if self.machine.permit_tool_dispatch().is_err() {
            let rejected = format!(
                "rejected: hear while {} (STT acts only while awake)",
                self.machine.state()
            );
            let mut lines = Vec::new();
            self.push_verbose(&mut lines, &rejected);
            lines.push(rejected);
            return self.with_status(lines);
        }
        self.stt.inject_partial(text);
        self.stt.inject_final(text);
        let events = self.stt.drain();
        self.last_transcripts.clone_from(&events);
        let mut lines = Vec::new();
        for event in &events {
            match event {
                TranscriptEvent::Partial { text } => {
                    lines.push(format!("partial transcript: \"{text}\""));
                }
                TranscriptEvent::Final { text } => {
                    lines.push(format!("final transcript: \"{text}\""));
                }
            }
        }
        self.with_status(lines)
    }

    /// Record mock TTS while awake.
    ///
    /// Sleep and hibernate prefer silence and refuse `say`.
    fn speak_text(&mut self, text: &str) -> Vec<String> {
        if self.machine.permit_tool_dispatch().is_err() {
            let rejected = format!(
                "rejected: say while {} (TTS acts only while awake)",
                self.machine.state()
            );
            let mut lines = Vec::new();
            self.push_verbose(&mut lines, &rejected);
            lines.push(rejected);
            return self.with_status(lines);
        }
        // MockTts::speak is infallible.
        let Ok(()) = TextToSpeech::speak(&mut self.tts, text);
        let mut lines = Vec::new();
        self.push_verbose(&mut lines, format!("tts: {text:?}"));
        lines.push(format!("said: \"{text}\""));
        self.with_status(lines)
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
    /// encoder, so the frame is 10 ms of silence (160 samples at 16 kHz) and
    /// the transcript is scored beside it. The silence still goes through
    /// [`score_frame`] so the PCM boundary is the one a microphone frame will
    /// use. `stop` makes [`MockAudioCapture::poll_frame`] return nothing.
    fn deliver_frame(&mut self) -> bool {
        // 10 ms at [`softwake_audio::AudioFormat::WAKE`].
        const SILENCE_10MS: &[i16] = &[0; 160];
        if !self.capture.push_frame(SILENCE_10MS) {
            return false;
        }
        let Ok(Some(frame)) = AudioCapture::poll_frame(&mut self.capture) else {
            return false;
        };
        self.last_pcm_hit = Some(score_frame(&mut self.pcm, &frame));
        true
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

    fn tool(&mut self, name: &str, args: &[String]) -> Vec<String> {
        let mut lines = Vec::new();
        self.push_verbose(&mut lines, format!("tool: {name}"));
        self.push_verbose(&mut lines, format!("args: {args:?}"));
        match self.hands.request(&self.machine, name, args) {
            Ok(RequestOutcome::Ran(ran)) => {
                lines.push(format!("tool {}: {}", ran.name, ran.detail));
            }
            Ok(RequestOutcome::Pending(pending)) => {
                lines.push(format!(
                    "pending {}: {} — {}",
                    pending.pending_id, pending.name, pending.description
                ));
                lines.push("waiting for confirm".to_owned());
            }
            Err(error) => {
                let rejected = format!("rejected: {error}");
                self.push_verbose(&mut lines, &rejected);
                lines.push(rejected);
            }
        }
        self.with_status(lines)
    }

    fn confirm_pending(&mut self, id: Option<String>) -> Vec<String> {
        let Some(pending_id) = self.chosen_pending_id(id) else {
            return self.with_status(vec!["rejected: no pending confirmation".to_owned()]);
        };
        let mut lines = Vec::new();
        match self.hands.confirm(&self.machine, &pending_id, None) {
            Ok(confirmed) => {
                lines.push(format!(
                    "confirmed {}: tool {}: {}",
                    confirmed.pending_id, confirmed.name, confirmed.detail
                ));
            }
            Err(error) => {
                let rejected = format!("rejected: {error}");
                self.push_verbose(&mut lines, &rejected);
                lines.push(rejected);
            }
        }
        self.with_status(lines)
    }

    fn cancel_pending(&mut self, id: Option<String>) -> Vec<String> {
        let Some(pending_id) = self.chosen_pending_id(id) else {
            return self.with_status(vec!["rejected: no pending confirmation".to_owned()]);
        };
        let mut lines = Vec::new();
        match self.hands.cancel(&pending_id, None) {
            Ok(cancelled) => {
                lines.push(format!(
                    "cancelled {}: {}",
                    cancelled.pending_id, cancelled.name
                ));
            }
            Err(error) => {
                let rejected = format!("rejected: {error}");
                self.push_verbose(&mut lines, &rejected);
                lines.push(rejected);
            }
        }
        self.with_status(lines)
    }

    fn chosen_pending_id(&self, id: Option<String>) -> Option<String> {
        id.or_else(|| self.hands.pending_id())
    }

    fn reject_extra(&mut self, command: &str) -> Vec<String> {
        let rejected = format!("rejected: {command} takes one pending id");
        let mut lines = Vec::new();
        self.push_verbose(&mut lines, &rejected);
        lines.push(rejected);
        lines.push(COMMANDS.to_owned());
        lines
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
                    if let Some(note) = self.run_effect(effect) {
                        lines.push(note);
                    }
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

    fn run_effect(&mut self, effect: Effect) -> Option<String> {
        match effect {
            Effect::OpenSession => {
                self.open_session();
                None
            }
            Effect::ReleaseActingResources => {
                self.session.close();
                // Drop any queued mock STT. Prefer TTS silence outside awake:
                // do not speak on the way out; leave the spoken log for status.
                let _ = self.stt.drain();
                self.last_transcripts.clear();
                self.hands.clear_pending().map(|cancelled| {
                    format!("cancelled {}: {}", cancelled.pending_id, cancelled.name)
                })
            }
            Effect::StopCapture => {
                let Ok(()) = self.capture.stop();
                None
            }
            Effect::StartCapture => {
                let Ok(()) = self.capture.start();
                None
            }
        }
    }

    fn open_session(&mut self) {
        // `commit_awake` runs before effects on a wake phrase. A missing pack
        // never reaches this effect; leave the session closed if it does.
        let Some(instructions) = self.soul.applied_instructions() else {
            return;
        };
        self.session = TextStubSession::open(instructions.to_owned());
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
        let mut lines = vec![
            format!("state: {}", self.machine.state()),
            format!("capture: {capture}"),
            format!("soul: {soul}"),
        ];
        if let Some(pending) = self.hands.pending() {
            let args = pending.args.join(" ");
            if args.is_empty() {
                lines.push(format!("pending: {} {}", pending.pending_id, pending.name));
            } else {
                lines.push(format!(
                    "pending: {} {} {args}",
                    pending.pending_id, pending.name
                ));
            }
        }
        if let Some(last) = self.hands.last_tool_line() {
            lines.push(format!("last tool: {last}"));
        }
        if let Some(notification) = self.hands.notifications().back() {
            lines.push(format!("notification: {notification}"));
        }
        if let Some(said) = self.tts.spoken().last() {
            lines.push(format!("last said: {said}"));
        }
        lines
    }

    fn with_status(&self, mut lines: Vec<String>) -> Vec<String> {
        lines.extend(self.status_lines());
        lines
    }
}

fn parse_line(line: &str) -> Parsed {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Parsed::Empty;
    }
    let mut parts = trimmed.split_whitespace();
    let Some(head) = parts.next() else {
        return Parsed::Empty;
    };
    if head.eq_ignore_ascii_case("tool") {
        let Some(name) = parts.next() else {
            return Parsed::ToolMissingName;
        };
        return Parsed::Tool {
            name: name.to_ascii_lowercase(),
            args: parts.map(str::to_owned).collect(),
        };
    }
    if head.eq_ignore_ascii_case("confirm") || head.eq_ignore_ascii_case("confirm-tool") {
        return parse_pending_id(parts, true);
    }
    if head.eq_ignore_ascii_case("cancel") || head.eq_ignore_ascii_case("cancel-tool") {
        return parse_pending_id(parts, false);
    }
    if head.eq_ignore_ascii_case("hear") {
        let rest = trimmed[head.len()..].trim();
        if rest.is_empty() {
            return Parsed::HearMissingText;
        }
        return Parsed::Hear {
            text: rest.to_owned(),
        };
    }
    if head.eq_ignore_ascii_case("say") {
        let rest = trimmed[head.len()..].trim();
        if rest.is_empty() {
            return Parsed::SayMissingText;
        }
        return Parsed::Say {
            text: rest.to_owned(),
        };
    }
    match trimmed.to_ascii_lowercase().as_str() {
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

fn parse_pending_id<'a>(mut parts: impl Iterator<Item = &'a str>, confirm: bool) -> Parsed {
    let id = parts.next().map(str::to_owned);
    if parts.next().is_some() {
        return if confirm {
            Parsed::ConfirmExtra
        } else {
            Parsed::CancelExtra
        };
    }
    if confirm {
        Parsed::Confirm { id }
    } else {
        Parsed::Cancel { id }
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
