//! Pure transition table.
//!
//! Hibernate never targets awake. Leaving hibernate is [`Event::UiResume`],
//! which lands in sleep. Phrase cooldowns live on [`crate::Machine`], not here.

use crate::{Effect, Event, StateError, VoiceState};

/// A legal transition, before cooldown is considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decision {
    pub to: VoiceState,
    pub effects: &'static [Effect],
}

/// The transition [`crate::Machine::apply`] committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    /// State before the event.
    pub from: VoiceState,
    /// State after the event.
    pub to: VoiceState,
    /// Event that was applied.
    pub event: Event,
    /// Side effects in the order the daemon should run them.
    pub effects: &'static [Effect],
}

const OPEN_SESSION: &[Effect] = &[Effect::OpenSession];
const RELEASE: &[Effect] = &[Effect::ReleaseActingResources];
const STOP_CAPTURE: &[Effect] = &[Effect::StopCapture];
const RELEASE_AND_STOP: &[Effect] = &[Effect::ReleaseActingResources, Effect::StopCapture];
const START_CAPTURE: &[Effect] = &[Effect::StartCapture];

pub(crate) fn decide(from: VoiceState, event: Event) -> Result<Decision, StateError> {
    match (from, event) {
        (VoiceState::Sleep, Event::WakePhrase) => Ok(Decision {
            to: VoiceState::Awake,
            effects: OPEN_SESSION,
        }),
        (VoiceState::Awake, Event::SleepPhrase | Event::UiSleep) => Ok(Decision {
            to: VoiceState::Sleep,
            effects: RELEASE,
        }),
        (VoiceState::Sleep, Event::UiHibernate | Event::HibernatePhrase) => Ok(Decision {
            to: VoiceState::Hibernate,
            effects: STOP_CAPTURE,
        }),
        (VoiceState::Awake, Event::UiHibernate | Event::HibernatePhrase) => Ok(Decision {
            to: VoiceState::Hibernate,
            effects: RELEASE_AND_STOP,
        }),
        (VoiceState::Hibernate, Event::UiResume) => Ok(Decision {
            to: VoiceState::Sleep,
            effects: START_CAPTURE,
        }),
        (VoiceState::Sleep, Event::SleepPhrase | Event::UiSleep) => {
            Err(illegal(from, event, "already asleep"))
        }
        (VoiceState::Sleep | VoiceState::Awake, Event::UiResume) => Err(illegal(
            from,
            event,
            "UI resume only leaves hibernate, and it lands in sleep",
        )),
        (VoiceState::Awake, Event::WakePhrase) => Err(illegal(
            from,
            event,
            "already awake; a wake phrase does not open a second session",
        )),
        (VoiceState::Hibernate, Event::WakePhrase | Event::HibernatePhrase) => Err(illegal(
            from,
            event,
            "hibernate does not capture audio; only the UI can leave hibernate",
        )),
        (VoiceState::Hibernate, Event::SleepPhrase | Event::UiSleep) => Err(illegal(
            from,
            event,
            "hibernate has no acting session; only the UI can leave hibernate",
        )),
        (VoiceState::Hibernate, Event::UiHibernate) => {
            Err(illegal(from, event, "already hibernating"))
        }
    }
}

fn illegal(from: VoiceState, event: Event, reason: &'static str) -> StateError {
    StateError::IllegalTransition {
        from,
        event,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::decide;
    use crate::{Effect, Event, StateError, VoiceState};

    #[test]
    fn legal_transitions_and_effects() {
        let cases = [
            (
                VoiceState::Sleep,
                Event::WakePhrase,
                VoiceState::Awake,
                &[Effect::OpenSession][..],
            ),
            (
                VoiceState::Awake,
                Event::SleepPhrase,
                VoiceState::Sleep,
                &[Effect::ReleaseActingResources][..],
            ),
            (
                VoiceState::Awake,
                Event::UiSleep,
                VoiceState::Sleep,
                &[Effect::ReleaseActingResources][..],
            ),
            (
                VoiceState::Sleep,
                Event::UiHibernate,
                VoiceState::Hibernate,
                &[Effect::StopCapture][..],
            ),
            (
                VoiceState::Awake,
                Event::UiHibernate,
                VoiceState::Hibernate,
                &[Effect::ReleaseActingResources, Effect::StopCapture][..],
            ),
            (
                VoiceState::Sleep,
                Event::HibernatePhrase,
                VoiceState::Hibernate,
                &[Effect::StopCapture][..],
            ),
            (
                VoiceState::Awake,
                Event::HibernatePhrase,
                VoiceState::Hibernate,
                &[Effect::ReleaseActingResources, Effect::StopCapture][..],
            ),
            (
                VoiceState::Hibernate,
                Event::UiResume,
                VoiceState::Sleep,
                &[Effect::StartCapture][..],
            ),
        ];

        for (from, event, to, effects) in cases {
            let decision = decide(from, event).expect("legal transition");
            assert_eq!(decision.to, to, "{event} from {from}");
            assert_eq!(decision.effects, effects, "{event} from {from}");
        }
    }

    #[test]
    fn illegal_transitions_name_the_rule() {
        let cases = [
            (VoiceState::Sleep, Event::SleepPhrase, "already asleep"),
            (VoiceState::Sleep, Event::UiSleep, "already asleep"),
            (
                VoiceState::Sleep,
                Event::UiResume,
                "UI resume only leaves hibernate, and it lands in sleep",
            ),
            (
                VoiceState::Awake,
                Event::WakePhrase,
                "already awake; a wake phrase does not open a second session",
            ),
            (
                VoiceState::Awake,
                Event::UiResume,
                "UI resume only leaves hibernate, and it lands in sleep",
            ),
            (
                VoiceState::Hibernate,
                Event::WakePhrase,
                "hibernate does not capture audio; only the UI can leave hibernate",
            ),
            (
                VoiceState::Hibernate,
                Event::SleepPhrase,
                "hibernate has no acting session; only the UI can leave hibernate",
            ),
            (
                VoiceState::Hibernate,
                Event::UiSleep,
                "hibernate has no acting session; only the UI can leave hibernate",
            ),
            (
                VoiceState::Hibernate,
                Event::UiHibernate,
                "already hibernating",
            ),
            (
                VoiceState::Hibernate,
                Event::HibernatePhrase,
                "hibernate does not capture audio; only the UI can leave hibernate",
            ),
        ];

        for (from, event, reason) in cases {
            let error = decide(from, event).expect_err("illegal transition");
            assert_eq!(
                error,
                StateError::IllegalTransition {
                    from,
                    event,
                    reason,
                },
                "{event} from {from}"
            );
        }
    }

    #[test]
    fn hibernate_resume_effect_does_not_open_a_session() {
        let decision = decide(VoiceState::Hibernate, Event::UiResume).expect("resume");
        assert_eq!(decision.to, VoiceState::Sleep);
        assert!(!decision.effects.contains(&Effect::OpenSession));
        assert!(!decision.effects.contains(&Effect::ReleaseActingResources));
    }

    #[test]
    fn awake_to_sleep_keeps_capture_up() {
        for event in [Event::SleepPhrase, Event::UiSleep] {
            let decision = decide(VoiceState::Awake, event).expect("sleep");
            assert_eq!(decision.effects, &[Effect::ReleaseActingResources]);
            assert!(!decision.effects.contains(&Effect::StopCapture));
        }
    }
}
