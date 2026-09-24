//! Voice machine, mock capture, and the loaded soul pack.
//!
//! UI commands map onto [`softwake_state::Event`]. Capture starts in sleep,
//! stops for hibernate, and starts again when hibernate returns to sleep.
//! [`Command::ReloadSoul`] re-reads the soul directory. That read applies on
//! the next transition into awake, not in the middle of an awake session.
//! A wake phrase with a missing or invalid pack leaves the machine where it is.

use softwake_audio::{AudioCapture, MockAudioCapture};
use softwake_ipc::VoiceState as WireState;
use softwake_ipc::{Command, Event as WireEvent, IpcError, ResponseBody, Status};
use softwake_soul::SoulDir;
use softwake_state::{CooldownConfig, Effect, Event, Machine, StateError, VoiceState};

use crate::soul::LoadedSoul;

#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) body: ResponseBody,
    pub(crate) event: Option<WireEvent>,
}

pub(crate) struct Runtime {
    machine: Machine,
    capture: MockAudioCapture,
    soul: LoadedSoul,
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
        }
    }

    /// Apply one client command.
    ///
    /// A voice-state change yields a [`WireEvent::StateChanged`] for every
    /// connected client. A rejection leaves the machine untouched and does
    /// not emit that event.
    pub(crate) fn handle(&mut self, command: Command) -> Outcome {
        match command {
            Command::GetStatus => Outcome {
                body: ResponseBody::ok(self.snapshot(None, None)),
                event: None,
            },
            Command::ReloadSoul => {
                self.soul.reload();
                Outcome {
                    body: ResponseBody::ok(self.snapshot(Some(self.soul.reload_summary()), None)),
                    event: None,
                }
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
            return Outcome {
                body: ResponseBody::Err {
                    error: IpcError::protocol(reason),
                },
                event: None,
            };
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
                apply_effects(&mut self.capture, applied.effects);
                let state = wire_state(self.machine.state());
                let capture_running = self.capture.is_running();
                let detail = Some(format!("{previous} -> {state}"));
                Outcome {
                    body: ResponseBody::ok(self.snapshot(None, detail.clone())),
                    event: Some(WireEvent::StateChanged {
                        state,
                        previous,
                        capture_running,
                        detail,
                    }),
                }
            }
            Err(error) => Outcome {
                body: ResponseBody::Err {
                    error: map_err(error),
                },
                event: None,
            },
        }
    }

    fn snapshot(&self, message: Option<String>, detail: Option<String>) -> Status {
        Status {
            state: wire_state(self.machine.state()),
            capture_running: self.capture.is_running(),
            soul_reload_pending: self.soul.reload_pending(),
            soul: Some(self.soul.report()),
            message,
            detail,
        }
    }
}

fn protocol_outcome(message: &str) -> Outcome {
    Outcome {
        body: ResponseBody::Err {
            error: IpcError::protocol(message),
        },
        event: None,
    }
}

/// Session open and release are recorded by [`Machine::apply`] itself.
/// The session crate arrives in a later milestone.
fn apply_effects(capture: &mut MockAudioCapture, effects: &[Effect]) {
    for effect in effects {
        match effect {
            Effect::OpenSession | Effect::ReleaseActingResources => {}
            Effect::StopCapture => {
                let Ok(()) = capture.stop();
            }
            Effect::StartCapture => {
                let Ok(()) = capture.start();
            }
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
            slept.event,
            Some(Event::StateChanged {
                state: VoiceState::Sleep,
                previous: VoiceState::Awake,
                capture_running: true,
                ..
            })
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
            hibernated.event,
            Some(Event::StateChanged {
                capture_running: false,
                ..
            })
        ));
    }

    #[test]
    fn illegal_sleep_does_not_emit_a_state_change() {
        let (mut runtime, _soul) = valid_runtime();
        let outcome = runtime.handle(Command::Sleep);
        assert!(outcome.event.is_none());
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
        assert!(status.event.is_none());
        assert_eq!(
            status.body.status().expect("status").state,
            VoiceState::Sleep
        );
    }

    #[test]
    fn reload_sticks_across_hibernate_without_a_state_event() {
        let (mut runtime, _soul) = valid_runtime();
        let reloaded = runtime.handle(Command::ReloadSoul);
        assert!(reloaded.event.is_none());
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
        assert!(refused.event.is_none());
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
        assert!(runtime.wake_phrase().event.is_none());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);

        soul.write("fixed soul\n", "fixed user\n");
        // Files changed on disk. Without reload the cached miss still refuses.
        assert!(runtime.wake_phrase().event.is_none());
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
            woke.event,
            Some(Event::StateChanged {
                state: VoiceState::Awake,
                previous: VoiceState::Sleep,
                ..
            })
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
        assert!(refused.event.is_none());
        assert_eq!(runtime.machine.state(), softwake_state::VoiceState::Sleep);
        assert!(runtime.machine.permit_tool_dispatch().is_err());
        assert!(runtime.soul.reload_pending());
        assert_eq!(runtime.applied_instructions(), Some(original.as_str()));
    }
}
