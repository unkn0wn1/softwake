use std::time::Duration;

use crate::{Event, VoiceState};

/// Failure from a transition or a tool-permission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// The event is not legal from the current state.
    #[error("cannot apply {event} from {from}: {reason}")]
    IllegalTransition {
        /// State the machine was in.
        from: VoiceState,
        /// Event that was rejected.
        event: Event,
        /// Why that pair is rejected.
        reason: &'static str,
    },

    /// A wake or sleep phrase arrived inside its cooldown.
    #[error("cannot apply {event} during cooldown ({remaining:?} remaining)")]
    Cooldown {
        /// Phrase event that was ignored.
        event: Event,
        /// Time left before that phrase can apply.
        remaining: Duration,
    },

    /// Tool dispatch is only permitted while [`VoiceState::Awake`].
    #[error("tool dispatch is forbidden while {state}")]
    ToolsForbidden {
        /// State that rejected the dispatch.
        state: VoiceState,
    },
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{Event, StateError, VoiceState};

    #[test]
    fn errors_name_the_rule() {
        assert_eq!(
            StateError::IllegalTransition {
                from: VoiceState::Hibernate,
                event: Event::WakePhrase,
                reason: "hibernate does not capture audio; only the UI can leave hibernate",
            }
            .to_string(),
            "cannot apply wake phrase from hibernate: hibernate does not capture audio; only the UI can leave hibernate"
        );
        assert_eq!(
            StateError::Cooldown {
                event: Event::SleepPhrase,
                remaining: Duration::from_millis(800),
            }
            .to_string(),
            "cannot apply sleep phrase during cooldown (800ms remaining)"
        );
        assert_eq!(
            StateError::ToolsForbidden {
                state: VoiceState::Sleep,
            }
            .to_string(),
            "tool dispatch is forbidden while sleep"
        );
    }
}
