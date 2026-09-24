//! Voice machine, mock capture, and the loaded soul pack.
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

use softwake_audio::{AudioCapture, MockAudioCapture};
use softwake_ipc::VoiceState as WireState;
use softwake_ipc::{Command, Event as WireEvent, IpcError, PendingTool, ResponseBody, Status};
#[cfg(test)]
use softwake_session::SessionPhase;
use softwake_session::TextStubSession;
use softwake_soul::SoulDir;
use softwake_state::{CooldownConfig, Effect, Event, Machine, StateError, VoiceState};

use crate::dispatch::{
    CancelledTool, ConfirmedTool, DispatchError, Hands, PendingToolCall, RanTool, RequestOutcome,
};
use crate::soul::LoadedSoul;

#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) body: ResponseBody,
    pub(crate) events: Vec<WireEvent>,
}

pub(crate) struct Runtime {
    machine: Machine,
    capture: MockAudioCapture,
    soul: LoadedSoul,
    session: TextStubSession,
    hands: Hands,
}

impl Runtime {
    /// Sleep, with mock capture already running and the soul directory read.
    pub(crate) fn new(soul_dir: SoulDir) -> Self {
        let mut capture = MockAudioCapture::default();
        // The mock device cannot fail to open.
        let Ok(()) = capture.start();
        Self {
            machine: Machine::new(CooldownConfig::default()),
            capture,
            soul: LoadedSoul::open(soul_dir),
            session: TextStubSession::default(),
            hands: Hands::new(),
        }
    }

    /// Apply one client command.
    ///
    /// A voice-state change yields a [`WireEvent::StateChanged`] for every
    /// connected client. A rejection leaves the machine untouched and does
    /// not emit that event.
    pub(crate) fn handle(&mut self, command: Command) -> Outcome {
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
    /// Serve does not feed capture into the wake engine yet. The typed demo
    /// applies the same [`LoadedSoul`] gate itself; this is the serve entry
    /// for that later capture loop, and tests call it now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn wake_phrase(&mut self) -> Outcome {
        if let Some(reason) = self.soul.refusal() {
            return Self::rejected(IpcError::protocol(reason));
        }
        self.apply_voice(Event::WakePhrase, |error| {
            IpcError::protocol(error.to_string())
        })
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

    fn transition(&mut self, command: Command) -> Outcome {
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
        assert!(applied.contains("# Runtime policy"));
        assert!(applied.contains("State: awake."));
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
}
