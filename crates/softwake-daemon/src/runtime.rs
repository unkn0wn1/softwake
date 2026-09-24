//! Voice machine, mock capture, and the reload-soul stub.
//!
//! UI commands map onto [`softwake_state::Event`]. Capture starts in sleep,
//! stops for hibernate, and starts again when hibernate returns to sleep.
//! [`Command::ReloadSoul`] records that a later awake session should re-read
//! the soul pack. It does not parse one.

use softwake_audio::{AudioCapture, MockAudioCapture};
use softwake_ipc::VoiceState as WireState;
use softwake_ipc::{Command, Event as WireEvent, IpcError, ResponseBody, Status};
use softwake_state::{CooldownConfig, Effect, Event, Machine, StateError, VoiceState};

/// Sentence returned by a reload command. The flag on [`Status`] is what later
/// reads observe; this sentence is only on the reload reply.
pub(crate) const RELOAD_MESSAGE: &str = "reload requested; applies on next awake";

pub(crate) struct Outcome {
    pub(crate) body: ResponseBody,
    pub(crate) event: Option<WireEvent>,
}

pub(crate) struct Runtime {
    machine: Machine,
    capture: MockAudioCapture,
    soul_reload_pending: bool,
}

impl Runtime {
    /// Sleep, with mock capture already running.
    pub(crate) fn new() -> Self {
        let mut capture = MockAudioCapture::default();
        // The mock device cannot fail to open.
        let Ok(()) = capture.start();
        Self {
            machine: Machine::new(CooldownConfig::default()),
            capture,
            soul_reload_pending: false,
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
                self.soul_reload_pending = true;
                Outcome {
                    body: ResponseBody::ok(self.snapshot(Some(RELOAD_MESSAGE.to_owned()), None)),
                    event: None,
                }
            }
            Command::Hibernate | Command::WakeFromUi | Command::Sleep => self.transition(command),
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
        let previous = wire_state(self.machine.state());
        match self.machine.apply(event) {
            Ok(applied) => {
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
                    error: map_error(command, error),
                },
                event: None,
            },
        }
    }

    fn snapshot(&self, message: Option<String>, detail: Option<String>) -> Status {
        Status {
            state: wire_state(self.machine.state()),
            capture_running: self.capture.is_running(),
            soul_reload_pending: self.soul_reload_pending,
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
    use super::{RELOAD_MESSAGE, Runtime};
    use softwake_ipc::{Command, Event, IpcError, ResponseBody, VoiceState};

    #[test]
    fn sleep_from_awake_keeps_capture_and_hibernate_stops_it() {
        let mut runtime = Runtime::new();
        runtime.wake_for_test();
        let slept = runtime.handle(Command::Sleep);
        let status = slept.body.status().expect("slept");
        assert_eq!(status.state, VoiceState::Sleep);
        assert!(status.capture_running);
        assert_eq!(status.detail.as_deref(), Some("awake -> sleep"));
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
        runtime.wake_for_test();
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
        let mut runtime = Runtime::new();
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
        let mut runtime = Runtime::new();
        let reloaded = runtime.handle(Command::ReloadSoul);
        assert!(reloaded.event.is_none());
        let status = reloaded.body.status().expect("reload");
        assert!(status.soul_reload_pending);
        assert_eq!(status.message.as_deref(), Some(RELOAD_MESSAGE));

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
    }

    impl Runtime {
        fn wake_for_test(&mut self) {
            let applied = self
                .machine
                .apply(softwake_state::Event::WakePhrase)
                .expect("wake");
            super::apply_effects(&mut self.capture, applied.effects);
        }
    }
}
