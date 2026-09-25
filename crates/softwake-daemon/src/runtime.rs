//! Voice machine, mock capture, the PCM wake stand-in, and the loaded soul pack.
//!
//! UI commands map onto [`softwake_state::Event`]. Capture starts in sleep,
//! stops for hibernate, and starts again when hibernate returns to sleep.
//! [`Command::ReloadSoul`] re-reads the soul directory. That read applies on
//! the next transition into awake, not in the middle of an awake session.
//! A wake phrase with a missing or invalid pack leaves the machine where it is.
//! A successful awake opens a text session with the rendered instructions.
//! Sleep and hibernate close that session and drop a pending confirmation.
//! Safe tools run only while awake. A confirm-gated tool waits for
//! `confirm_tool`. `cancel_tool` clears it without running.
//! Mock STT/TTS act only while awake ([ADR 0007](../../docs/ADR-0007-awake-stt-tts.md)).
//! `ask` completes one provider turn on the open session while awake
//! ([ADR 0013](../../docs/ADR-0013-session-provider.md)).

use std::time::Instant;

use softwake_audio::{AudioCapture, MockAudioCapture};
use softwake_ipc::VoiceState as WireState;
use softwake_ipc::{Command, Event as WireEvent, IpcError, PendingTool, ResponseBody, Status};
use softwake_session::{SessionPhase, TextStubSession};
use softwake_soul::SoulDir;
use softwake_state::{CooldownConfig, Effect, Event, Machine, StateError, VoiceState};
use softwake_voice::{MockStt, MockTts, TextToSpeech, TranscriptEvent};
use softwake_wake::{NullDetector, PhraseHit};

use crate::dispatch::{
    CancelledTool, ConfirmedTool, DispatchError, Hands, PendingToolCall, RanTool, RequestOutcome,
};
use crate::pcm::score_frame;
use crate::soul::LoadedSoul;

#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) body: ResponseBody,
    pub(crate) events: Vec<WireEvent>,
}

pub(crate) struct Runtime {
    machine: Machine,
    /// Wall clock for [`Self::tick`]. Phrase cooldowns use machine time only.
    last_tick: Instant,
    capture: MockAudioCapture,
    pcm: NullDetector,
    /// Last PCM score from [`Self::drain_pcm`]. `None` means the queue was empty.
    #[cfg_attr(not(test), allow(dead_code))]
    last_pcm_hit: Option<PhraseHit>,
    soul: LoadedSoul,
    session: TextStubSession,
    hands: Hands,
    stt: MockStt,
    tts: MockTts,
    /// In-test provider. Absent in the serve binary, so ask uses the disk path.
    #[cfg(test)]
    chat_fixture: Option<crate::chat::ChatFixture>,
}

impl Runtime {
    /// Sleep, with mock capture already running and the soul directory read.
    pub(crate) fn new(soul_dir: SoulDir) -> Self {
        let mut capture = MockAudioCapture::default();
        // The mock device cannot fail to open.
        let Ok(()) = capture.start();
        Self {
            machine: Machine::new(CooldownConfig::default()),
            last_tick: Instant::now(),
            capture,
            pcm: NullDetector,
            last_pcm_hit: None,
            soul: LoadedSoul::open(soul_dir),
            session: TextStubSession::default(),
            hands: Hands::from_disk(),
            stt: MockStt::default(),
            tts: MockTts::default(),
            #[cfg(test)]
            chat_fixture: None,
        }
    }

    /// Apply one client command.
    ///
    /// A voice-state change yields a [`WireEvent::StateChanged`] for every
    /// connected client. A rejection leaves the machine untouched and does
    /// not emit that event.
    pub(crate) fn handle(&mut self, command: Command) -> Outcome {
        // Score PCM that arrived before this command. Serve's queue is empty
        // unless a test pushed samples. The typed demo enters awake on its own.
        self.drain_pcm();
        match command {
            Command::GetStatus => Self::quiet(self.snapshot(None, None)),
            Command::ReloadSoul => {
                self.soul.reload();
                Self::quiet(self.snapshot(Some(self.soul.reload_summary()), None))
            }
            Command::Hibernate | Command::WakeFromUi | Command::Sleep => self.transition(command),
        }
    }

    /// Enter awake from a wake phrase when the loaded pack is valid.
    ///
    /// An invalid pack does not change the voice state. The pack applied here
    /// is the last startup or `reload_soul` read, not a fresh disk read.
    ///
    /// Serve does not open a microphone. This method does not call
    /// [`Self::drain_pcm`]. The typed demo applies the same [`LoadedSoul`]
    /// gate on its own machine.
    pub(crate) fn wake_phrase(&mut self) -> Outcome {
        self.tick();
        if let Some(reason) = self.soul.refusal() {
            return Self::rejected(IpcError::protocol(reason));
        }
        self.apply_voice(Event::WakePhrase, |error| {
            IpcError::protocol(error.to_string())
        })
    }

    /// Move the phrase clock forward by the wall time since the previous tick.
    ///
    /// Called at the start of [`Self::wake_phrase`] and [`Self::transition`].
    /// A rejection still advances the clock and does not re-arm the gate.
    fn tick(&mut self) {
        let now = Instant::now();
        self.machine
            .advance(now.saturating_duration_since(self.last_tick));
        self.last_tick = now;
    }

    /// Instructions stored the last time awake was entered successfully.
    #[cfg(test)]
    pub(crate) fn applied_instructions(&self) -> Option<&str> {
        self.soul.applied_instructions()
    }

    /// Phase of the text session. Closed unless awake opened it.
    #[cfg(test)]
    pub(crate) fn session_phase(&self) -> SessionPhase {
        self.session.phase()
    }

    /// Instructions held by the open session.
    #[cfg(test)]
    pub(crate) fn session_instructions(&self) -> Option<&str> {
        self.session.instructions()
    }

    /// User lines recorded on the open session.
    #[cfg(test)]
    pub(crate) fn session_turns(&self) -> &[String] {
        self.session.turns()
    }

    /// Install the in-test provider. Production serve leaves this unset.
    #[cfg(test)]
    pub(crate) fn install_chat_fixture(&mut self, fixture: crate::chat::ChatFixture) {
        self.chat_fixture = Some(fixture);
    }

    /// Posts recorded by the in-test transport. Empty when no fixture is installed.
    #[cfg(test)]
    pub(crate) fn chat_posts(&self) -> Vec<crate::chat::RecordedPost> {
        self.chat_fixture
            .as_ref()
            .map(|fixture| fixture.transport.posts())
            .unwrap_or_default()
    }

    /// Complete one ask while awake.
    ///
    /// Blank text is rejected before the voice check. Sleep and hibernate
    /// refuse the turn before Settings are loaded. The assistant text is
    /// [`Status::message`]. No event is emitted. The bearer is not copied
    /// onto the status.
    pub(crate) fn ask(&mut self, text: &str) -> Outcome {
        if text.trim().is_empty() {
            return Self::chat_rejected("ask needs text");
        }
        if self.machine.permit_tool_dispatch().is_err() {
            return Self::chat_rejected(&format!(
                "ask while {} (chat acts only while awake)",
                wire_state(self.machine.state())
            ));
        }
        if self.session.phase() != SessionPhase::Open {
            return Self::chat_rejected("session is closed");
        }
        #[cfg(test)]
        if self.chat_fixture.is_some() {
            return self.ask_fixture(text);
        }
        self.ask_disk(text)
    }

    fn chat_rejected(message: &str) -> Outcome {
        Self::rejected(IpcError::ChatRejected {
            message: message.to_owned(),
        })
    }

    fn ask_ok(&self, reply: String) -> Outcome {
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(reply), None)),
            events: Vec::new(),
        }
    }

    #[cfg(test)]
    fn ask_fixture(&mut self, text: &str) -> Outcome {
        let extracted = self.chat_fixture.as_ref().map(|fixture| {
            let prepared = crate::chat::prepare_fixture(fixture);
            (prepared, std::sync::Arc::clone(&fixture.transport))
        });
        let Some((prepared, transport)) = extracted else {
            return self.ask_disk(text);
        };
        match prepared {
            Ok((prepared, bearer)) => {
                let call = prepared.clone();
                // Disabled memory keeps the appendix empty. A developer
                // `memory.json` must not change the fixture post.
                let appendix = crate::chat::appendix_for_ask(
                    text,
                    Some(&softwake_memory::MockMemory::default()),
                );
                match crate::chat::perform_ask(
                    &mut self.session,
                    text,
                    &appendix,
                    move |system, user| {
                        crate::chat::complete_fixture(&transport, &call, &bearer, system, user)
                    },
                ) {
                    Ok(reply) => self.ask_ok(reply),
                    Err(error) => Self::chat_rejected(error.sentence()),
                }
            }
            Err(message) => Self::chat_rejected(&message),
        }
    }

    fn ask_disk(&mut self, text: &str) -> Outcome {
        let ready = match crate::chat::load_disk_chat() {
            Ok(ready) => ready,
            Err(message) => return Self::chat_rejected(&message),
        };
        let crate::chat::DiskChat { prepared, bearer } = ready;
        if let Err(message) = crate::chat::gate_live_http(&prepared, &bearer, text) {
            return Self::chat_rejected(&message);
        }
        let call = prepared.clone();
        let appendix =
            crate::chat::appendix_for_ask(text, Option::<&softwake_memory::MockMemory>::None);
        match crate::chat::perform_ask(&mut self.session, text, &appendix, move |system, user| {
            crate::chat::finish_prepared_chat(&call, &bearer, system, user)
        }) {
            Ok(reply) => self.ask_ok(reply),
            Err(error) => Self::chat_rejected(error.sentence()),
        }
    }

    /// Run one safe tool, or stage a confirm-gated tool.
    ///
    /// Sleep and hibernate refuse the call before the registry. A safe run
    /// emits [`WireEvent::ToolStarted`] and then [`WireEvent::ToolFinished`].
    /// A confirm-gated tool does not run: the outcome is
    /// [`WireEvent::ToolConfirmPending`] and [`Status::pending_tool`]. A
    /// refusal emits neither.
    pub(crate) fn invoke_tool(&mut self, name: &str, args: &[String]) -> Outcome {
        let step = self.hands.request(&self.machine, name, args);
        self.outcome_for_request(step)
    }

    /// Run the pending tool once when the id matches and the daemon is awake.
    ///
    /// `name`, when set, must match the pending tool. A second confirm of the
    /// same id fails because the first confirm clears the record.
    pub(crate) fn confirm_tool(&mut self, pending_id: &str, name: Option<&str>) -> Outcome {
        match self.hands.confirm(&self.machine, pending_id, name) {
            Ok(confirmed) => self.confirmed_outcome(confirmed),
            Err(error) => Self::rejected(tool_ipc_error(&error)),
        }
    }

    /// Clear a matching pending confirmation without running the tool.
    ///
    /// This is allowed when the daemon is not awake, so an operator can drop
    /// a confirmation that survived a state change. Sleep and hibernate also
    /// clear it on their own.
    pub(crate) fn cancel_tool(&mut self, pending_id: &str, name: Option<&str>) -> Outcome {
        match self.hands.cancel(pending_id, name) {
            Ok(cancelled) => self.cancelled_outcome(cancelled),
            Err(error) => Self::rejected(tool_ipc_error(&error)),
        }
    }

    /// Inject mock STT text while awake and emit transcript IPC events.
    ///
    /// Sleep and hibernate refuse the channel. Used by tests and as the serve
    /// stand-in until a microphone ASR path is wired.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn inject_transcript(&mut self, text: &str) -> Outcome {
        if self.machine.permit_tool_dispatch().is_err() {
            return Self::rejected(IpcError::protocol(format!(
                "cannot hear while {}",
                wire_state(self.machine.state())
            )));
        }
        self.stt.inject_partial(text);
        self.stt.inject_final(text);
        let events = self.stt.drain();
        let mut wire = Vec::with_capacity(events.len());
        for event in events {
            match event {
                TranscriptEvent::Partial { text } => {
                    wire.push(WireEvent::PartialTranscript { text });
                }
                TranscriptEvent::Final { text } => {
                    wire.push(WireEvent::FinalTranscript { text });
                }
            }
        }
        let detail = Some(format!("transcript events: {}", wire.len()));
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(format!("heard: {text}")), detail)),
            events: wire,
        }
    }

    /// Record mock TTS while awake.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn speak(&mut self, text: &str) -> Outcome {
        if self.machine.permit_tool_dispatch().is_err() {
            return Self::rejected(IpcError::protocol(format!(
                "cannot say while {}",
                wire_state(self.machine.state())
            )));
        }
        let Ok(()) = TextToSpeech::speak(&mut self.tts, text);
        Outcome {
            body: ResponseBody::ok(
                self.snapshot(Some(format!("said: {text}")), Some("mock tts".to_owned())),
            ),
            events: Vec::new(),
        }
    }

    /// Strings recorded by mock TTS (tests).
    #[cfg(test)]
    pub(crate) fn spoken(&self) -> &[String] {
        self.tts.spoken()
    }

    /// Pull every queued frame into the PCM detector.
    ///
    /// Returns the last hit, or [`PhraseHit::None`] when the queue is empty.
    /// [`NullDetector`] never matches. After `stop`, the mock drops the queue,
    /// so hibernate does not score a late frame.
    fn drain_pcm(&mut self) -> PhraseHit {
        let mut hit = PhraseHit::None;
        let mut scored = false;
        while let Ok(Some(frame)) = AudioCapture::poll_frame(&mut self.capture) {
            hit = score_frame(&mut self.pcm, &frame);
            scored = true;
        }
        if scored {
            self.last_pcm_hit = Some(hit);
        }
        hit
    }

    fn transition(&mut self, command: Command) -> Outcome {
        self.tick();
        let event = match command {
            Command::Hibernate => Event::UiHibernate,
            Command::WakeFromUi => Event::UiResume,
            Command::Sleep => Event::UiSleep,
            Command::GetStatus | Command::ReloadSoul => {
                return protocol_outcome("command is not a voice transition");
            }
        };
        self.apply_voice(event, |error| map_error(command, error))
    }

    fn apply_voice(
        &mut self,
        event: Event,
        map_err: impl FnOnce(StateError) -> IpcError,
    ) -> Outcome {
        let previous = wire_state(self.machine.state());
        match self.machine.apply(event) {
            Ok(applied) => {
                if event == Event::WakePhrase {
                    self.soul.commit_awake();
                }
                let cleared = self.apply_effects(applied.effects);
                let state = wire_state(self.machine.state());
                let capture_running = self.capture.is_running();
                let detail = Some(format!("{previous} -> {state}"));
                let mut events = vec![WireEvent::StateChanged {
                    state,
                    previous,
                    capture_running,
                    detail: detail.clone(),
                }];
                events.extend(cleared);
                Outcome {
                    body: ResponseBody::ok(self.snapshot(None, detail)),
                    events,
                }
            }
            Err(error) => Self::rejected(map_err(error)),
        }
    }

    fn quiet(snapshot: Status) -> Outcome {
        Outcome {
            body: ResponseBody::ok(snapshot),
            events: Vec::new(),
        }
    }

    fn rejected(error: IpcError) -> Outcome {
        Outcome {
            body: ResponseBody::Err { error },
            events: Vec::new(),
        }
    }

    fn apply_effects(&mut self, effects: &[Effect]) -> Vec<WireEvent> {
        let mut extra = Vec::new();
        for effect in effects {
            match effect {
                Effect::OpenSession => self.open_session(),
                Effect::ReleaseActingResources => {
                    self.session.close();
                    let _ = self.stt.drain();
                    if let Some(cancelled) = self.hands.clear_pending() {
                        extra.push(WireEvent::ToolConfirmResolved {
                            pending_id: cancelled.pending_id,
                            accepted: false,
                        });
                    }
                }
                Effect::StopCapture => {
                    let Ok(()) = self.capture.stop();
                }
                Effect::StartCapture => {
                    let Ok(()) = self.capture.start();
                }
            }
        }
        extra
    }

    fn open_session(&mut self) {
        // `commit_awake` runs before effects on a wake phrase. A missing pack
        // never reaches this effect; leave the session closed if it does.
        let Some(instructions) = self.soul.applied_instructions() else {
            return;
        };
        self.session = TextStubSession::open(instructions.to_owned());
    }

    fn snapshot(&self, message: Option<String>, detail: Option<String>) -> Status {
        Status {
            state: wire_state(self.machine.state()),
            capture_running: self.capture.is_running(),
            soul_reload_pending: self.soul.reload_pending(),
            soul: Some(self.soul.report()),
            message,
            detail,
            pending_tool: self.pending_wire(),
            last_tool: self.hands.last_tool_line(),
        }
    }

    fn pending_wire(&self) -> Option<PendingTool> {
        self.hands.pending().map(|pending| PendingTool {
            pending_id: pending.pending_id,
            name: pending.name,
            args: pending.args,
            description: pending.description,
        })
    }

    fn outcome_for_request(&self, step: Result<RequestOutcome, DispatchError>) -> Outcome {
        match step {
            Ok(RequestOutcome::Ran(ran)) => self.ran_outcome(ran),
            Ok(RequestOutcome::Pending(pending)) => self.pending_outcome(pending),
            Err(error) => Self::rejected(tool_ipc_error(&error)),
        }
    }

    fn ran_outcome(&self, ran: RanTool) -> Outcome {
        let RanTool { name, detail } = ran;
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(detail.clone()), Some(detail.clone()))),
            events: vec![
                WireEvent::ToolStarted { name: name.clone() },
                WireEvent::ToolFinished {
                    name,
                    detail: Some(detail),
                },
            ],
        }
    }

    fn pending_outcome(&self, pending: PendingToolCall) -> Outcome {
        let PendingToolCall {
            pending_id,
            name,
            args,
            description,
        } = pending;
        let message = format!("pending confirmation {pending_id} for {name}");
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(message), Some(description.clone()))),
            events: vec![WireEvent::ToolConfirmPending {
                pending_id,
                name,
                args,
                description,
            }],
        }
    }

    fn confirmed_outcome(&self, confirmed: ConfirmedTool) -> Outcome {
        let ConfirmedTool {
            pending_id,
            name,
            detail,
        } = confirmed;
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(detail.clone()), Some(detail.clone()))),
            events: vec![
                WireEvent::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
                WireEvent::ToolStarted { name: name.clone() },
                WireEvent::ToolFinished {
                    name,
                    detail: Some(detail),
                },
            ],
        }
    }

    fn cancelled_outcome(&self, cancelled: CancelledTool) -> Outcome {
        let message = format!("cancelled {}", cancelled.pending_id);
        Outcome {
            body: ResponseBody::ok(self.snapshot(Some(message), None)),
            events: vec![WireEvent::ToolConfirmResolved {
                pending_id: cancelled.pending_id,
                accepted: false,
            }],
        }
    }
}

fn protocol_outcome(message: &str) -> Outcome {
    Outcome {
        body: ResponseBody::Err {
            error: IpcError::protocol(message),
        },
        events: Vec::new(),
    }
}

fn tool_ipc_error(error: &DispatchError) -> IpcError {
    match error {
        DispatchError::Forbidden { name, state } => IpcError::ToolForbidden {
            name: name.clone(),
            state: wire_state(*state),
        },
        DispatchError::Unknown { name } => IpcError::UnknownTool { name: name.clone() },
        DispatchError::Denied { name } => IpcError::ToolDenied { name: name.clone() },
        DispatchError::Busy { pending_id } => IpcError::ConfirmationPending {
            pending_id: pending_id.clone(),
        },
        DispatchError::UnknownPending { pending_id } => IpcError::UnknownPending {
            pending_id: pending_id.clone(),
        },
        DispatchError::PendingMismatch { pending_id, name } => IpcError::PendingMismatch {
            pending_id: pending_id.clone(),
            name: name.clone(),
        },
        DispatchError::ConfirmForbidden { pending_id, state } => IpcError::ConfirmForbidden {
            pending_id: pending_id.clone(),
            state: wire_state(*state),
        },
        DispatchError::InvalidArgs { .. } | DispatchError::Connector { .. } => {
            IpcError::protocol(error.to_string())
        }
    }
}

fn wire_state(state: VoiceState) -> WireState {
    match state {
        VoiceState::Sleep => WireState::Sleep,
        VoiceState::Awake => WireState::Awake,
        VoiceState::Hibernate => WireState::Hibernate,
    }
}

fn map_error(command: Command, error: StateError) -> IpcError {
    match error {
        StateError::IllegalTransition { from, reason, .. } => IpcError::IllegalTransition {
            from: wire_state(from),
            command,
            reason: reason.to_owned(),
        },
        StateError::Cooldown { remaining, .. } => IpcError::Cooldown {
            command,
            remaining_ms: u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX),
            detail: Some(error.to_string()),
        },
        StateError::ToolsForbidden { .. } => IpcError::protocol(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Runtime;
    use crate::soul::{TestSoulDir, reload_message};
    use softwake_ipc::{Command, Event, IpcError, ResponseBody, VoiceState};
    use softwake_session::SessionPhase;
    use softwake_wake::PhraseHit;

    fn valid_runtime() -> (Runtime, TestSoulDir) {
        let dir = TestSoulDir::valid();
        let runtime = Runtime::new(dir.soul_dir());
        (runtime, dir)
    }

    fn wake(runtime: &mut Runtime) {
        let outcome = runtime.wake_phrase();
        assert!(
            outcome.body.status().is_some(),
            "wake should apply, got {outcome:?}"
        );
    }

    #[test]
    fn queued_silence_is_scored_before_a_command_and_dropped_after_hibernate() {
        let (mut runtime, _soul) = valid_runtime();
        assert_eq!(runtime.last_pcm_hit, None);
        assert!(runtime.capture.push_frame(&[0; 160]));
        let status = runtime.handle(Command::GetStatus);
        assert_eq!(
            status.body.status().expect("status").state,
            VoiceState::Sleep
        );
        assert_eq!(runtime.last_pcm_hit, Some(PhraseHit::None));
        assert!(runtime.capture.poll_frame().is_none());

        runtime.handle(Command::Hibernate);
        assert!(!runtime.capture.push_frame(&[0; 160]));
        runtime.last_pcm_hit = None;
        runtime.handle(Command::GetStatus);
        assert_eq!(runtime.last_pcm_hit, None);
    }

    #[test]
    fn sleep_from_awake_keeps_capture_and_hibernate_stops_it() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        let slept = runtime.handle(Command::Sleep);
        let status = slept.body.status().expect("slept");
        assert_eq!(status.state, VoiceState::Sleep);
        assert!(status.capture_running);
        assert_eq!(status.detail.as_deref(), Some("awake -> sleep"));
        assert!(status.soul.as_ref().is_some_and(|soul| soul.ok));
        assert!(matches!(
            slept.events.as_slice(),
            [Event::StateChanged {
                state: VoiceState::Sleep,
                previous: VoiceState::Awake,
                capture_running: true,
                ..
            }]
        ));

        runtime
            .machine
            .advance(std::time::Duration::from_millis(800));
        wake(&mut runtime);
        let hibernated = runtime.handle(Command::Hibernate);
        let status = hibernated.body.status().expect("hibernated");
        assert_eq!(status.state, VoiceState::Hibernate);
        assert!(!status.capture_running);
        assert!(matches!(
            hibernated.events.as_slice(),
            [Event::StateChanged {
                capture_running: false,
                ..
            }]
        ));
    }

    #[test]
    fn wake_after_sleep_and_resume_respects_the_phrase_cooldown() {
        let (mut runtime, _soul) = valid_runtime();
        let woke = runtime.wake_phrase();
        assert_eq!(woke.body.status().expect("awake").state, VoiceState::Awake);

        runtime.handle(Command::Sleep);
        let cooled = runtime.wake_phrase();
        match cooled.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => assert!(
                message.contains("cannot apply wake phrase during cooldown"),
                "{message}"
            ),
            other => panic!("expected cooldown, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        runtime.machine.advance(Duration::from_millis(800));
        let again = runtime.wake_phrase();
        assert_eq!(
            again.body.status().expect("awake after cooldown").state,
            VoiceState::Awake
        );

        runtime.handle(Command::Hibernate);
        let resumed = runtime.handle(Command::WakeFromUi);
        assert_eq!(
            resumed.body.status().expect("resume").state,
            VoiceState::Sleep
        );
        let cooled = runtime.wake_phrase();
        match cooled.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => assert!(message.contains("during cooldown"), "{message}"),
            other => panic!("expected cooldown after resume, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        runtime.machine.advance(Duration::from_millis(800));
        let woke = runtime.wake_phrase();
        assert_eq!(
            woke.body
                .status()
                .expect("awake after resume cooldown")
                .state,
            VoiceState::Awake
        );
    }

    #[test]
    fn illegal_sleep_does_not_emit_a_state_change() {
        let (mut runtime, _soul) = valid_runtime();
        let outcome = runtime.handle(Command::Sleep);
        assert!(outcome.events.is_empty());
        match outcome.body {
            ResponseBody::Err {
                error: IpcError::IllegalTransition { from, command, .. },
            } => {
                assert_eq!(from, VoiceState::Sleep);
                assert_eq!(command, Command::Sleep);
            }
            other => panic!("expected illegal transition, got {other:?}"),
        }
        let status = runtime.handle(Command::GetStatus);
        assert!(status.events.is_empty());
        assert_eq!(
            status.body.status().expect("status").state,
            VoiceState::Sleep
        );
    }

    #[test]
    fn reload_sticks_across_hibernate_without_a_state_event() {
        let (mut runtime, _soul) = valid_runtime();
        let reloaded = runtime.handle(Command::ReloadSoul);
        assert!(reloaded.events.is_empty());
        let status = reloaded.body.status().expect("reload");
        assert!(status.soul_reload_pending);
        assert!(status.soul.as_ref().is_some_and(|soul| soul.ok));
        let expected = reload_message(true, None);
        assert!(expected.contains("reloaded soul pack; applies on next awake"));
        assert!(expected.contains("rules.md"));
        assert!(expected.contains("glossary.md"));
        assert_eq!(status.message.as_deref(), Some(expected.as_str()));

        runtime.handle(Command::Hibernate);
        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Hibernate);
        assert!(status.soul_reload_pending);
        assert!(status.message.is_none());
        assert!(status.soul.as_ref().is_some_and(|soul| soul.ok));
    }

    #[test]
    fn missing_soul_refuses_awake_and_leaves_hibernate_usable() {
        let soul = TestSoulDir::empty();
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        match refused.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => {
                assert!(message.contains("refusing awake"), "{message}");
                assert!(message.contains("missing soul.md"), "{message}");
            }
            other => panic!("expected a soul refusal, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.capture.is_running());
        assert!(runtime.machine.permit_tool_dispatch().is_err());
        assert!(runtime.applied_instructions().is_none());

        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Sleep);
        assert!(status.capture_running);
        assert!(!status.soul_reload_pending);
        let report = status.soul.expect("soul report");
        assert!(!report.ok);
        assert!(
            report
                .reason
                .unwrap_or_default()
                .contains("missing soul.md")
        );

        let hibernated = runtime.handle(Command::Hibernate);
        assert_eq!(
            hibernated.body.status().expect("hibernate").state,
            VoiceState::Hibernate
        );
        assert!(!runtime.capture.is_running());
        let resumed = runtime.handle(Command::WakeFromUi);
        assert_eq!(
            resumed.body.status().expect("resume").state,
            VoiceState::Sleep
        );
        assert!(runtime.capture.is_running());
        let slept = runtime.handle(Command::Sleep);
        assert!(matches!(
            slept.body,
            ResponseBody::Err {
                error: IpcError::IllegalTransition { .. }
            }
        ));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
    }

    #[test]
    fn reload_then_wake_applies_the_fixed_pack() {
        let soul = TestSoulDir::empty();
        let mut runtime = Runtime::new(soul.soul_dir());
        assert!(runtime.wake_phrase().events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        soul.write("fixed soul\n", "fixed user\n");
        // Files changed on disk. Without reload the cached miss still refuses.
        assert!(runtime.wake_phrase().events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        let reloaded = runtime.handle(Command::ReloadSoul);
        let status = reloaded.body.status().expect("reload");
        assert!(status.soul_reload_pending);
        assert!(status.soul.as_ref().is_some_and(|report| report.ok));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.applied_instructions().is_none());

        let woke = runtime.wake_phrase();
        let status = woke.body.status().expect("awake");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(!status.soul_reload_pending);
        assert!(matches!(
            woke.events.as_slice(),
            [Event::StateChanged {
                state: VoiceState::Awake,
                previous: VoiceState::Sleep,
                ..
            }]
        ));
        assert!(runtime.machine.permit_tool_dispatch().is_ok());
        let applied = runtime.applied_instructions().expect("applied");
        assert!(applied.contains("# Identity"));
        assert!(applied.contains("fixed soul"));
        assert!(applied.contains("# User profile"));
        assert!(applied.contains("fixed user"));
        assert!(applied.contains("# Rules"));
        assert!(applied.contains("# Glossary"));
        assert!(applied.contains("# Runtime policy"));
        assert!(applied.contains("State: awake."));
    }

    #[test]
    fn missing_rules_refuses_awake_and_leaves_other_transitions() {
        let soul = TestSoulDir::valid();
        std::fs::remove_file(soul.path().join("rules.md")).expect("remove rules");
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        match refused.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => {
                assert!(message.contains("refusing awake"), "{message}");
                assert!(message.contains("rules.md"), "{message}");
            }
            other => panic!("expected a soul refusal, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.capture.is_running());
        assert!(runtime.applied_instructions().is_none());

        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Sleep);
        let report = status.soul.expect("soul report");
        assert!(!report.ok);
        assert!(report.reason.unwrap_or_default().contains("rules.md"));

        let hibernated = runtime.handle(Command::Hibernate);
        assert_eq!(
            hibernated.body.status().expect("hibernate").state,
            VoiceState::Hibernate
        );
        assert!(!runtime.capture.is_running());
        let resumed = runtime.handle(Command::WakeFromUi);
        assert_eq!(
            resumed.body.status().expect("resume").state,
            VoiceState::Sleep
        );
        assert!(runtime.capture.is_running());
        let slept = runtime.handle(Command::Sleep);
        assert!(matches!(
            slept.body,
            ResponseBody::Err {
                error: IpcError::IllegalTransition { .. }
            }
        ));
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
    }

    #[test]
    fn duplicate_glossary_alias_refuses_awake() {
        let soul = TestSoulDir::valid();
        std::fs::write(
            soul.path().join("glossary.md"),
            "docs → /path/to/docs\ndocs → /path/to/other\n",
        )
        .expect("glossary");
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.applied_instructions().is_none());
        let status = runtime
            .handle(Command::GetStatus)
            .body
            .status()
            .expect("status")
            .clone();
        assert_eq!(status.state, VoiceState::Sleep);
        let report = status.soul.expect("soul report");
        assert!(!report.ok);
        assert!(
            report
                .reason
                .unwrap_or_default()
                .contains("duplicate alias docs")
        );
    }

    #[test]
    fn empty_file_refuses_awake_until_reload() {
        let soul = TestSoulDir::valid();
        soul.write("   \n", "user\n");
        let mut runtime = Runtime::new(soul.soul_dir());
        let refused = runtime.wake_phrase();
        match refused.body {
            ResponseBody::Err {
                error: IpcError::Protocol { message },
            } => assert!(message.contains("empty"), "{message}"),
            other => panic!("expected a soul refusal, got {other:?}"),
        }
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        soul.write("soul\n", "user\n");
        let reloaded = runtime.handle(Command::ReloadSoul);
        let expected = reload_message(true, None);
        assert_eq!(
            reloaded.body.status().expect("reload").message.as_deref(),
            Some(expected.as_str())
        );
        wake(&mut runtime);
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Awake);
        assert!(!runtime.soul.reload_pending());
    }

    #[test]
    fn reload_while_awake_waits_for_the_next_awake_entry() {
        let (mut runtime, soul) = valid_runtime();
        wake(&mut runtime);
        let original = runtime.applied_instructions().expect("applied").to_owned();
        assert!(original.contains("test soul"));

        soul.write("updated soul\n", "updated user\n");
        let reloaded = runtime.handle(Command::ReloadSoul);
        let status = reloaded.body.status().expect("reload");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(status.soul_reload_pending);
        assert_eq!(runtime.applied_instructions(), Some(original.as_str()));

        runtime.handle(Command::Sleep);
        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        let applied = runtime.applied_instructions().expect("new pack");
        assert!(applied.contains("updated soul"));
        assert!(applied.contains("updated user"));
        assert!(!applied.contains("test soul"));
        assert!(!runtime.soul.reload_pending());
    }

    #[test]
    fn invalid_reload_while_awake_keeps_the_session_and_blocks_the_next_wake() {
        let (mut runtime, soul) = valid_runtime();
        wake(&mut runtime);
        let original = runtime.applied_instructions().expect("applied").to_owned();

        std::fs::remove_file(soul.path().join("user.md")).expect("remove user");
        let reloaded = runtime.handle(Command::ReloadSoul);
        let status = reloaded.body.status().expect("reload");
        assert_eq!(status.state, VoiceState::Awake);
        assert!(status.soul_reload_pending);
        assert!(status.soul.as_ref().is_some_and(|report| !report.ok));
        let reason = status
            .soul
            .as_ref()
            .and_then(|report| report.reason.clone());
        let expected = reload_message(false, reason.as_deref());
        assert_eq!(status.message.as_deref(), Some(expected.as_str()));
        assert_eq!(runtime.applied_instructions(), Some(original.as_str()));
        assert!(runtime.machine.permit_tool_dispatch().is_ok());

        runtime.handle(Command::Sleep);
        runtime.machine.advance(Duration::from_millis(800));
        let refused = runtime.wake_phrase();
        assert!(refused.events.is_empty());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.machine.permit_tool_dispatch().is_err());
        assert!(runtime.soul.reload_pending());
        assert_eq!(runtime.applied_instructions(), Some(original.as_str()));
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());
    }

    #[test]
    fn echo_runs_only_while_awake() {
        let (mut runtime, _soul) = valid_runtime();

        let asleep = runtime.invoke_tool("echo", &["hello".to_owned()]);
        assert!(asleep.events.is_empty());
        assert!(matches!(
            asleep.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    name,
                    state: VoiceState::Sleep,
                }
            } if name == "echo"
        ));

        let unknown_asleep = runtime.invoke_tool("volume", &[]);
        assert!(unknown_asleep.events.is_empty());
        assert!(matches!(
            unknown_asleep.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));

        wake(&mut runtime);
        let ran = runtime.invoke_tool("echo", &["hello".to_owned(), "world".to_owned()]);
        let status = ran.body.status().expect("echo");
        assert_eq!(status.message.as_deref(), Some("echo: hello world"));
        assert_eq!(status.detail.as_deref(), Some("echo: hello world"));
        assert!(matches!(
            ran.events.as_slice(),
            [
                Event::ToolStarted { name: started },
                Event::ToolFinished {
                    name: finished,
                    detail: Some(detail),
                },
            ] if started == "echo" && finished == "echo" && detail == "echo: hello world"
        ));
        assert_eq!(
            runtime
                .invoke_tool("echo", &[])
                .body
                .status()
                .expect("pong")
                .message
                .as_deref(),
            Some("pong")
        );

        let unknown = runtime.invoke_tool("volume", &[]);
        assert!(unknown.events.is_empty());
        assert!(matches!(
            unknown.body,
            ResponseBody::Err {
                error: IpcError::UnknownTool { name }
            } if name == "volume"
        ));

        runtime.handle(Command::Sleep);
        assert!(matches!(
            runtime.invoke_tool("echo", &[]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        runtime.handle(Command::Hibernate);
        let hibernated = runtime.invoke_tool("echo", &["x".to_owned()]);
        assert!(hibernated.events.is_empty());
        assert!(matches!(
            hibernated.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Hibernate,
                    ..
                }
            }
        ));
    }

    #[test]
    fn session_opens_with_soul_instructions_and_closes_when_acting_stops() {
        let (mut runtime, soul) = valid_runtime();
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());

        wake(&mut runtime);
        assert_eq!(runtime.session_phase(), SessionPhase::Open);
        let instructions = runtime
            .session_instructions()
            .expect("session opened")
            .to_owned();
        assert!(instructions.contains("test soul"));
        assert!(instructions.contains("test user"));
        assert!(instructions.contains("echo (safe)"));
        assert!(instructions.contains("notify (confirm)"));
        assert!(instructions.contains("email_send (confirm)"));
        assert_eq!(runtime.applied_instructions(), Some(instructions.as_str()));

        soul.write("updated soul\n", "updated user\n");
        runtime.handle(Command::ReloadSoul);
        assert_eq!(runtime.session_instructions(), Some(instructions.as_str()));

        runtime.handle(Command::Sleep);
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());
        assert!(
            runtime
                .applied_instructions()
                .expect("last applied pack")
                .contains("test soul")
        );

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        let reopened = runtime.session_instructions().expect("reopened");
        assert!(reopened.contains("updated soul"));
        assert!(reopened.contains("updated user"));
        assert!(reopened.contains("echo (safe)"));
        assert!(reopened.contains("notify (confirm)"));
        assert!(reopened.contains("email_send (confirm)"));

        runtime.handle(Command::Hibernate);
        assert_eq!(
            runtime.machine.state(),
            softwake_state::VoiceState::Hibernate
        );
        assert_eq!(runtime.session_phase(), SessionPhase::Closed);
        assert!(runtime.session_instructions().is_none());
    }

    #[test]
    fn notify_stays_pending_until_confirm_and_runs_once() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);

        let pending = runtime.invoke_tool("notify", &["hello".to_owned()]);
        let status = pending.body.status().expect("pending");
        assert_eq!(
            status.message.as_deref(),
            Some("pending confirmation 1 for notify")
        );
        let waiting = status.pending_tool.clone().expect("pending tool");
        assert_eq!(waiting.pending_id, "1");
        assert_eq!(waiting.name, "notify");
        assert_eq!(waiting.args, ["hello"]);
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            pending.events.as_slice(),
            [Event::ToolConfirmPending {
                pending_id,
                name,
                ..
            }] if pending_id == "1" && name == "notify"
        ));
        assert!(pending.events.iter().all(|event| !matches!(
            event,
            Event::ToolStarted { .. } | Event::ToolFinished { .. }
        )));

        let busy = runtime.invoke_tool("notify", &["other".to_owned()]);
        assert!(matches!(
            busy.body,
            ResponseBody::Err {
                error: IpcError::ConfirmationPending { ref pending_id }
            } if pending_id == "1"
        ));
        assert!(runtime.hands.notifications().is_empty());
        assert_eq!(runtime.hands.pending_id().as_deref(), Some("1"));

        let confirmed = runtime.confirm_tool("1", Some("notify"));
        let status = confirmed.body.status().expect("confirmed");
        assert_eq!(status.message.as_deref(), Some("hello"));
        assert!(status.pending_tool.is_none());
        assert_eq!(
            runtime.hands.notifications().iter().collect::<Vec<_>>(),
            ["hello"]
        );
        assert!(matches!(
            confirmed.events.as_slice(),
            [
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
                Event::ToolStarted { name: started },
                Event::ToolFinished {
                    name: finished,
                    detail: Some(detail),
                },
            ] if pending_id == "1"
                && started == "notify"
                && finished == "notify"
                && detail == "hello"
        ));

        let again = runtime.confirm_tool("1", None);
        assert!(matches!(
            again.body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { ref pending_id }
            } if pending_id == "1"
        ));
        assert_eq!(runtime.hands.notifications().len(), 1);
    }

    #[test]
    fn email_send_confirms_once_onto_the_outbox() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        let args = vec![
            "ada@example.com".to_owned(),
            "hello there".to_owned(),
            "line one".to_owned(),
        ];

        let pending = runtime.invoke_tool("email_send", &args);
        let status = pending.body.status().expect("pending");
        assert_eq!(
            status.message.as_deref(),
            Some("pending confirmation 1 for email_send")
        );
        let waiting = status.pending_tool.clone().expect("pending tool");
        assert_eq!(waiting.name, "email_send");
        assert_eq!(waiting.args, args);
        assert!(runtime.hands.outbox().is_empty());
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            pending.events.as_slice(),
            [Event::ToolConfirmPending { name, .. }] if name == "email_send"
        ));
        assert!(pending.events.iter().all(|event| !matches!(
            event,
            Event::ToolStarted { .. } | Event::ToolFinished { .. }
        )));

        let short = runtime.invoke_tool("email_send", &["ada@example.com".to_owned()]);
        assert!(matches!(
            short.body,
            ResponseBody::Err {
                error: IpcError::ConfirmationPending { .. }
            }
        ));
        assert!(runtime.hands.outbox().is_empty());

        let confirmed = runtime.confirm_tool("1", Some("email_send"));
        let status = confirmed.body.status().expect("confirmed");
        assert_eq!(status.message.as_deref(), Some("sent 1"));
        assert!(status.pending_tool.is_none());
        assert_eq!(runtime.hands.outbox().len(), 1);
        assert_eq!(runtime.hands.outbox()[0].to, "ada@example.com");
        assert_eq!(runtime.hands.outbox()[0].subject, "hello there");
        assert_eq!(runtime.hands.outbox()[0].body, "line one");
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            confirmed.events.as_slice(),
            [
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: true,
                },
                Event::ToolStarted { name: started },
                Event::ToolFinished {
                    name: finished,
                    detail: Some(detail),
                },
            ] if pending_id == "1"
                && started == "email_send"
                && finished == "email_send"
                && detail == "sent 1"
        ));

        let again = runtime.confirm_tool("1", None);
        assert!(matches!(
            again.body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { .. }
            }
        ));
        assert_eq!(runtime.hands.outbox().len(), 1);
    }

    fn email_body() -> Vec<String> {
        vec![
            "ada@example.com".to_owned(),
            "hello".to_owned(),
            "body".to_owned(),
        ]
    }

    #[test]
    fn email_send_cancel_sleep_and_short_args_do_not_send() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);

        let short = runtime.invoke_tool(
            "email_send",
            &["ada@example.com".to_owned(), "hello".to_owned()],
        );
        assert!(matches!(
            short.body,
            ResponseBody::Err {
                error: IpcError::Protocol { ref message }
            } if message == "email_send needs to, subject, and body"
        ));
        assert!(short.events.is_empty());
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.outbox().is_empty());

        runtime.invoke_tool("email_send", &email_body());
        let cancelled = runtime.cancel_tool("1", None);
        assert!(matches!(
            cancelled.events.as_slice(),
            [Event::ToolConfirmResolved {
                accepted: false,
                ..
            }]
        ));
        assert!(runtime.hands.outbox().is_empty());

        runtime.invoke_tool("email_send", &email_body());
        // Leave the pending record in place while the machine is no longer awake.
        // The sleep command itself clears that record; this is the confirm-forbidden case.
        runtime
            .machine
            .apply(softwake_state::Event::UiSleep)
            .expect("sleep without clearing");
        let refused = runtime.confirm_tool("2", None);
        assert!(matches!(
            refused.body,
            ResponseBody::Err {
                error: IpcError::ConfirmForbidden { .. }
            }
        ));
        assert!(runtime.hands.outbox().is_empty());
        assert!(runtime.hands.pending().is_some());
        runtime.cancel_tool("2", None);
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.outbox().is_empty());

        let forbidden = runtime.invoke_tool("email_send", &email_body());
        assert!(matches!(
            forbidden.body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));
        assert!(runtime.hands.outbox().is_empty());
    }

    #[test]
    fn sleep_and_hibernate_clear_a_pending_email_without_sending() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.invoke_tool("email_send", &email_body());

        let slept = runtime.handle(Command::Sleep);
        assert!(slept.events.iter().any(|event| matches!(
            event,
            Event::ToolConfirmResolved {
                accepted: false,
                ..
            }
        )));
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.outbox().is_empty());

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        runtime.invoke_tool("email_send", &email_body());
        let hibernated = runtime.handle(Command::Hibernate);
        assert!(hibernated.events.iter().any(|event| matches!(
            event,
            Event::ToolConfirmResolved {
                accepted: false,
                ..
            }
        )));
        assert!(runtime.hands.outbox().is_empty());
        assert!(matches!(
            runtime.invoke_tool("email_send", &email_body()).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Hibernate,
                    ..
                }
            }
        ));
    }

    #[test]
    fn cancel_does_not_append_and_a_name_mismatch_keeps_pending() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.invoke_tool("notify", &["hello".to_owned()]);

        let mismatch = runtime.confirm_tool("1", Some("echo"));
        assert!(matches!(
            mismatch.body,
            ResponseBody::Err {
                error: IpcError::PendingMismatch { .. }
            }
        ));
        assert!(runtime.hands.notifications().is_empty());
        assert!(runtime.hands.pending().is_some());

        let cancelled = runtime.cancel_tool("1", None);
        let status = cancelled.body.status().expect("cancelled");
        assert_eq!(status.message.as_deref(), Some("cancelled 1"));
        assert!(status.pending_tool.is_none());
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            cancelled.events.as_slice(),
            [Event::ToolConfirmResolved {
                accepted: false,
                pending_id,
            }] if pending_id == "1"
        ));

        let gone = runtime.cancel_tool("1", None);
        assert!(matches!(
            gone.body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { .. }
            }
        ));
    }

    #[test]
    fn shell_is_denied_and_unknown_is_rejected_while_awake() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        let denied = runtime.invoke_tool("shell", &["echo".to_owned()]);
        assert!(denied.events.is_empty());
        assert!(matches!(
            denied.body,
            ResponseBody::Err {
                error: IpcError::ToolDenied { ref name }
            } if name == "shell"
        ));
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.notifications().is_empty());
        runtime.confirm_tool("1", None);
        assert!(runtime.hands.notifications().is_empty());

        let unknown = runtime.invoke_tool("volume", &[]);
        assert!(matches!(
            unknown.body,
            ResponseBody::Err {
                error: IpcError::UnknownTool { ref name }
            } if name == "volume"
        ));
    }

    #[test]
    fn sleep_and_hibernate_clear_pending_and_still_refuse_tools() {
        let (mut runtime, _soul) = valid_runtime();
        wake(&mut runtime);
        runtime.invoke_tool("notify", &["hello".to_owned()]);
        assert!(runtime.hands.notifications().is_empty());

        let slept = runtime.handle(Command::Sleep);
        assert!(matches!(
            slept.events.as_slice(),
            [
                Event::StateChanged {
                    state: VoiceState::Sleep,
                    ..
                },
                Event::ToolConfirmResolved {
                    pending_id,
                    accepted: false,
                },
            ] if pending_id == "1"
        ));
        assert!(runtime.hands.pending().is_none());
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            runtime.confirm_tool("1", None).body,
            ResponseBody::Err {
                error: IpcError::UnknownPending { .. }
            }
        ));
        assert!(matches!(
            runtime.invoke_tool("notify", &["x".to_owned()]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));
        assert!(matches!(
            runtime.invoke_tool("shell", &[]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Sleep,
                    ..
                }
            }
        ));

        runtime.machine.advance(Duration::from_millis(800));
        wake(&mut runtime);
        runtime.invoke_tool("notify", &["later".to_owned()]);
        let hibernated = runtime.handle(Command::Hibernate);
        assert!(hibernated.events.iter().any(|event| matches!(
            event,
            Event::ToolConfirmResolved {
                accepted: false,
                ..
            }
        )));
        assert!(runtime.hands.notifications().is_empty());
        assert!(matches!(
            runtime.invoke_tool("echo", &[]).body,
            ResponseBody::Err {
                error: IpcError::ToolForbidden {
                    state: VoiceState::Hibernate,
                    ..
                }
            }
        ));
    }

    #[test]
    fn mock_stt_emits_partial_and_final_while_awake() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        wake(&mut runtime);

        let outcome = runtime.inject_transcript("hello there");
        assert!(outcome.body.status().is_some());
        assert_eq!(
            outcome.events,
            vec![
                Event::PartialTranscript {
                    text: "hello there".to_owned(),
                },
                Event::FinalTranscript {
                    text: "hello there".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn mock_stt_refused_while_asleep() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        let outcome = runtime.inject_transcript("nope");
        assert!(matches!(
            outcome.body,
            ResponseBody::Err {
                error: IpcError::Protocol { .. }
            }
        ));
        assert!(outcome.events.is_empty());
    }

    #[test]
    fn mock_tts_records_while_awake_and_refuses_asleep() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        assert!(matches!(
            runtime.speak("early").body,
            ResponseBody::Err {
                error: IpcError::Protocol { .. }
            }
        ));
        wake(&mut runtime);
        let said = runtime.speak("hello");
        assert!(said.body.status().is_some());
        assert_eq!(runtime.spoken(), &["hello".to_owned()]);
    }

    #[test]
    fn ask_rejects_blank_text_before_a_post() {
        let soul = TestSoulDir::valid();
        let mut runtime = Runtime::new(soul.soul_dir());
        wake(&mut runtime);
        runtime.install_chat_fixture(crate::chat::xai_key_fixture(
            true,
            Some("sk-test-secret"),
            "pong",
        ));
        let outcome = runtime.ask("   ");
        match outcome.body {
            ResponseBody::Err {
                error: IpcError::ChatRejected { ref message },
            } => assert_eq!(message, "ask needs text"),
            other => panic!("expected chat_rejected, got {other:?}"),
        }
        assert!(outcome.events.is_empty());
        assert!(runtime.chat_posts().is_empty());
        assert!(runtime.session_turns().is_empty());
        assert!(!format!("{outcome:?}").contains("sk-test-secret"));
    }
}
