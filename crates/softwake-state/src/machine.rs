use std::time::Duration;

use crate::transition::{Applied, decide};
use crate::{Event, StateError, VoiceState};

/// How long phrase events are ignored after a transition.
///
/// Defaults follow the voice-state configuration: 800 ms after wake and 800 ms
/// after sleep. UI events are never held by this clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CooldownConfig {
    /// Ignore a sleep phrase for this long after entering awake.
    pub post_wake: Duration,
    /// Ignore a wake phrase for this long after entering sleep.
    pub post_sleep: Duration,
}

impl Default for CooldownConfig {
    fn default() -> Self {
        Self {
            post_wake: Duration::from_millis(800),
            post_sleep: Duration::from_millis(800),
        }
    }
}

/// Phrase-loop gate. UI events do not consult it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhraseGate {
    Open,
    /// Block [`Event::WakePhrase`] until `until`.
    BlockWake {
        until: Duration,
    },
    /// Block [`Event::SleepPhrase`] until `until`.
    BlockSleep {
        until: Duration,
    },
}

/// Voice-state machine.
///
/// The clock is caller-supplied. [`Machine::advance`] moves it, so a test
/// never sleeps and a stalled daemon cannot expire a cooldown by accident.
///
/// The machine is not [`Copy`]: copying it would let a transition land on one
/// value and leave the other behind.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(unknown_lints)] // Not every toolchain ships the pedantic lint below.
#[allow(clippy::missing_copy_implementations)] // A copy would miss transitions applied to the original.
#[allow(clippy::module_name_repetitions)] // `Machine` is the public type for this crate.
pub struct Machine {
    state: VoiceState,
    now: Duration,
    gate: PhraseGate,
    config: CooldownConfig,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new(CooldownConfig::default())
    }
}

impl Machine {
    /// Start in [`VoiceState::Sleep`] with the phrase gate open.
    ///
    /// Sleep permits capture. This constructor does not open a device.
    #[must_use]
    pub const fn new(config: CooldownConfig) -> Self {
        Self {
            state: VoiceState::Sleep,
            now: Duration::ZERO,
            gate: PhraseGate::Open,
            config,
        }
    }

    /// Current state.
    #[must_use]
    pub const fn state(&self) -> VoiceState {
        self.state
    }

    /// Move the phrase-cooldown clock forward.
    ///
    /// Resolution is the full [`Duration`]. A zero duration leaves the clock
    /// where it is.
    pub const fn advance(&mut self, elapsed: Duration) {
        self.now = self.now.saturating_add(elapsed);
    }

    /// Apply one event.
    ///
    /// On success the new state is stored before this returns. Acting
    /// resources are therefore released within the call when the effect list
    /// says so: the caller observes a state that already forbids tools.
    ///
    /// # Errors
    ///
    /// - [`StateError::IllegalTransition`] when the event is not legal here.
    /// - [`StateError::Cooldown`] when a wake or sleep phrase is still inside
    ///   its cooldown.
    ///
    /// Both failures leave the machine unchanged.
    pub fn apply(&mut self, event: Event) -> Result<Applied, StateError> {
        let span = tracing::info_span!("voice_state.transition", from = %self.state, %event);
        let _entered = span.enter();

        let decision = match decide(self.state, event) {
            Ok(decision) => decision,
            Err(error) => {
                tracing::debug!(%error, "rejected voice transition");
                return Err(error);
            }
        };

        if let Some(remaining) = self.cooldown_remaining(event) {
            let error = StateError::Cooldown { event, remaining };
            tracing::debug!(%error, "rejected voice transition");
            return Err(error);
        }

        let from = self.state;
        self.state = decision.to;
        self.arm_cooldown(event);
        tracing::info!(to = %self.state, "applied voice transition");
        Ok(Applied {
            from,
            to: self.state,
            event,
            effects: decision.effects,
        })
    }

    /// Ask whether one tool call may run.
    ///
    /// This crate does not run tools. A dispatcher must call this and skip
    /// the tool when it returns an error, so sleep and hibernate produce zero
    /// invocations.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::ToolsForbidden`] unless the state is awake.
    #[must_use = "skip the tool when dispatch is forbidden"]
    pub const fn permit_tool_dispatch(&self) -> Result<(), StateError> {
        if self.state.allows_tool_dispatch() {
            Ok(())
        } else {
            Err(StateError::ToolsForbidden { state: self.state })
        }
    }

    // Each arm names the phrase that gate blocks. `let...else` hides that pairing.
    #[allow(clippy::manual_let_else)]
    fn cooldown_remaining(&self, event: Event) -> Option<Duration> {
        let until = match (self.gate, event) {
            (PhraseGate::BlockWake { until }, Event::WakePhrase)
            | (PhraseGate::BlockSleep { until }, Event::SleepPhrase) => until,
            _ => return None,
        };
        until
            .checked_sub(self.now)
            .filter(|remaining| !remaining.is_zero())
    }

    /// Entering awake blocks the sleep phrase. Entering sleep blocks the wake
    /// phrase, including UI resume, so a buffered wake phrase cannot carry
    /// hibernate straight into awake. Entering hibernate clears the gate
    /// because the microphone is off.
    fn arm_cooldown(&mut self, event: Event) {
        self.gate = match event {
            Event::WakePhrase => PhraseGate::BlockSleep {
                until: self.now.saturating_add(self.config.post_wake),
            },
            Event::SleepPhrase | Event::UiSleep | Event::UiResume => PhraseGate::BlockWake {
                until: self.now.saturating_add(self.config.post_sleep),
            },
            Event::UiHibernate => PhraseGate::Open,
        };
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{CooldownConfig, Machine};
    use crate::{Effect, Event, StateError, VoiceState};

    fn without_cooldown() -> CooldownConfig {
        CooldownConfig {
            post_wake: Duration::ZERO,
            post_sleep: Duration::ZERO,
        }
    }

    fn machine_in(state: VoiceState) -> Machine {
        let mut machine = Machine::new(without_cooldown());
        match state {
            VoiceState::Sleep => {}
            VoiceState::Awake => {
                machine.apply(Event::WakePhrase).expect("enter awake");
            }
            VoiceState::Hibernate => {
                machine.apply(Event::UiHibernate).expect("enter hibernate");
            }
        }
        machine
    }

    #[test]
    fn new_machine_starts_asleep_and_passive() {
        let machine = Machine::default();
        assert_eq!(machine.state(), VoiceState::Sleep);
        assert!(machine.state().allows_capture());
        assert_eq!(accepted_dispatches(&machine, 8), 0);
    }

    #[test]
    fn default_cooldowns_are_800ms() {
        let config = CooldownConfig::default();
        assert_eq!(config.post_wake, Duration::from_millis(800));
        assert_eq!(config.post_sleep, Duration::from_millis(800));
    }

    #[test]
    fn sleep_cannot_dispatch_tools() {
        let machine = machine_in(VoiceState::Sleep);
        assert_eq!(accepted_dispatches(&machine, 10), 0);
        assert_eq!(
            machine.permit_tool_dispatch(),
            Err(StateError::ToolsForbidden {
                state: VoiceState::Sleep
            })
        );
    }

    #[test]
    fn hibernate_cannot_dispatch_tools_or_capture() {
        let machine = machine_in(VoiceState::Hibernate);
        assert_eq!(accepted_dispatches(&machine, 10), 0);
        assert!(!machine.state().allows_capture());
        assert_eq!(
            machine.permit_tool_dispatch(),
            Err(StateError::ToolsForbidden {
                state: VoiceState::Hibernate
            })
        );
    }

    #[test]
    fn awake_can_dispatch_tools() {
        let machine = machine_in(VoiceState::Awake);
        assert_eq!(machine.permit_tool_dispatch(), Ok(()));
        assert_eq!(accepted_dispatches(&machine, 3), 3);
    }

    #[test]
    fn illegal_events_do_not_change_state() {
        let cases = [
            (VoiceState::Sleep, Event::SleepPhrase),
            (VoiceState::Sleep, Event::UiSleep),
            (VoiceState::Sleep, Event::UiResume),
            (VoiceState::Awake, Event::WakePhrase),
            (VoiceState::Awake, Event::UiResume),
            (VoiceState::Hibernate, Event::WakePhrase),
            (VoiceState::Hibernate, Event::SleepPhrase),
            (VoiceState::Hibernate, Event::UiSleep),
            (VoiceState::Hibernate, Event::UiHibernate),
        ];
        for (state, event) in cases {
            let mut machine = machine_in(state);
            let before = machine.clone();
            let error = machine.apply(event).expect_err("illegal event");
            assert!(
                matches!(error, StateError::IllegalTransition { .. }),
                "{event} from {state} produced {error}"
            );
            assert_eq!(machine, before, "{event} from {state}");
        }
    }

    #[test]
    fn awake_to_sleep_releases_acting_before_return() {
        for event in [Event::SleepPhrase, Event::UiSleep] {
            let mut machine = machine_in(VoiceState::Awake);
            let applied = machine.apply(event).expect("leave awake");
            assert_eq!(applied.from, VoiceState::Awake);
            assert_eq!(applied.to, VoiceState::Sleep);
            assert_eq!(applied.effects, &[Effect::ReleaseActingResources]);
            assert_eq!(machine.state(), VoiceState::Sleep);
            assert_eq!(accepted_dispatches(&machine, 5), 0);
            assert!(machine.state().allows_capture());
        }
    }

    #[test]
    fn awake_to_hibernate_releases_acting_and_stops_capture() {
        let mut machine = machine_in(VoiceState::Awake);
        let applied = machine.apply(Event::UiHibernate).expect("hibernate");
        assert_eq!(
            applied.effects,
            &[Effect::ReleaseActingResources, Effect::StopCapture]
        );
        assert_eq!(machine.state(), VoiceState::Hibernate);
        assert!(!machine.state().allows_capture());
        assert_eq!(accepted_dispatches(&machine, 4), 0);
    }

    #[test]
    fn hibernate_resume_lands_in_sleep_not_awake() {
        let mut machine = machine_in(VoiceState::Hibernate);
        let applied = machine.apply(Event::UiResume).expect("resume");
        assert_eq!(applied.to, VoiceState::Sleep);
        assert_eq!(machine.state(), VoiceState::Sleep);
        assert_eq!(applied.effects, &[Effect::StartCapture]);
        assert!(!applied.effects.contains(&Effect::OpenSession));
        assert!(!machine.state().allows_tool_dispatch());
        assert!(machine.state().allows_capture());
    }

    #[test]
    fn post_wake_cooldown_blocks_only_the_sleep_phrase() {
        let mut machine = Machine::new(CooldownConfig::default());
        machine.apply(Event::WakePhrase).expect("wake");
        let blocked = machine.apply(Event::SleepPhrase).expect_err("cooldown");
        assert_eq!(
            blocked,
            StateError::Cooldown {
                event: Event::SleepPhrase,
                remaining: Duration::from_millis(800),
            }
        );
        assert_eq!(machine.state(), VoiceState::Awake);

        machine.advance(Duration::from_millis(799));
        assert!(matches!(
            machine.apply(Event::SleepPhrase),
            Err(StateError::Cooldown { .. })
        ));
        assert_eq!(machine.state(), VoiceState::Awake);

        machine.advance(Duration::from_millis(1));
        let applied = machine.apply(Event::SleepPhrase).expect("cooldown elapsed");
        assert_eq!(applied.to, VoiceState::Sleep);
    }

    #[test]
    fn post_sleep_cooldown_blocks_only_the_wake_phrase() {
        let mut machine = Machine::new(CooldownConfig::default());
        machine.apply(Event::WakePhrase).expect("wake");
        machine.advance(CooldownConfig::default().post_wake);
        machine.apply(Event::SleepPhrase).expect("sleep");

        let blocked = machine.apply(Event::WakePhrase).expect_err("cooldown");
        assert_eq!(
            blocked,
            StateError::Cooldown {
                event: Event::WakePhrase,
                remaining: Duration::from_millis(800),
            }
        );
        assert_eq!(machine.state(), VoiceState::Sleep);

        machine.advance(Duration::from_millis(800));
        let applied = machine.apply(Event::WakePhrase).expect("cooldown elapsed");
        assert_eq!(applied.to, VoiceState::Awake);
    }

    #[test]
    fn ui_commands_ignore_phrase_cooldown() {
        let mut machine = Machine::new(CooldownConfig::default());
        machine.apply(Event::WakePhrase).expect("wake");
        let applied = machine
            .apply(Event::UiSleep)
            .expect("ui sleep during cooldown");
        assert_eq!(applied.to, VoiceState::Sleep);

        machine
            .apply(Event::UiHibernate)
            .expect("hibernate during cooldown");
        assert_eq!(machine.state(), VoiceState::Hibernate);
        assert!(!machine.state().allows_capture());
    }

    #[test]
    fn resume_cooldown_blocks_an_immediate_wake_phrase() {
        let mut machine = Machine::new(CooldownConfig::default());
        machine.apply(Event::UiHibernate).expect("hibernate");
        machine.apply(Event::UiResume).expect("resume");
        assert_eq!(machine.state(), VoiceState::Sleep);
        assert!(matches!(
            machine.apply(Event::WakePhrase),
            Err(StateError::Cooldown {
                event: Event::WakePhrase,
                ..
            })
        ));
        assert_eq!(machine.state(), VoiceState::Sleep);

        machine.advance(CooldownConfig::default().post_sleep);
        assert_eq!(
            machine.apply(Event::WakePhrase).expect("later wake").to,
            VoiceState::Awake
        );
    }

    #[test]
    fn zero_cooldown_allows_an_immediate_reverse_phrase() {
        let mut machine = Machine::new(without_cooldown());
        machine.apply(Event::WakePhrase).expect("wake");
        assert_eq!(
            machine
                .apply(Event::SleepPhrase)
                .expect("immediate sleep")
                .to,
            VoiceState::Sleep
        );
        assert_eq!(
            machine.apply(Event::WakePhrase).expect("immediate wake").to,
            VoiceState::Awake
        );
    }

    #[test]
    fn rejected_cooldown_does_not_consume_the_gate() {
        let mut machine = Machine::new(CooldownConfig::default());
        machine.apply(Event::WakePhrase).expect("wake");
        let _ = machine.apply(Event::SleepPhrase).expect_err("cooldown");
        machine.advance(Duration::from_millis(100));
        let error = machine
            .apply(Event::SleepPhrase)
            .expect_err("still cooling");
        assert_eq!(
            error,
            StateError::Cooldown {
                event: Event::SleepPhrase,
                remaining: Duration::from_millis(700),
            }
        );
        assert_eq!(machine.state(), VoiceState::Awake);
    }

    #[test]
    fn operator_loop_returns_to_a_passive_listener() {
        let mut machine = Machine::new(without_cooldown());
        assert_eq!(accepted_dispatches(&machine, 2), 0);

        let wake = machine.apply(Event::WakePhrase).expect("wake");
        assert_eq!(wake.effects, &[Effect::OpenSession]);
        assert_eq!(accepted_dispatches(&machine, 1), 1);

        let sleep = machine.apply(Event::SleepPhrase).expect("sleep");
        assert_eq!(sleep.effects, &[Effect::ReleaseActingResources]);
        assert_eq!(accepted_dispatches(&machine, 2), 0);
        assert!(machine.state().allows_capture());

        machine.apply(Event::UiHibernate).expect("hibernate");
        assert!(!machine.state().allows_capture());
        assert_eq!(accepted_dispatches(&machine, 2), 0);
        assert!(machine.apply(Event::WakePhrase).is_err());

        let resume = machine.apply(Event::UiResume).expect("resume");
        assert_eq!(resume.to, VoiceState::Sleep);
        assert_ne!(resume.to, VoiceState::Awake);
        assert_eq!(accepted_dispatches(&machine, 2), 0);
    }

    fn accepted_dispatches(machine: &Machine, attempts: usize) -> usize {
        let mut accepted = 0;
        for _ in 0..attempts {
            if machine.permit_tool_dispatch().is_ok() {
                accepted += 1;
            }
        }
        accepted
    }
}
