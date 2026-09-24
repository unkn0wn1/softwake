use std::fmt;

/// Fixed voice-state vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `VoiceState` is the public domain name.
pub enum VoiceState {
    /// Listening for a wake phrase. The acting session and tools are off.
    Sleep,
    /// Acting session and allowlisted tools may run. Capture stays on.
    Awake,
    /// Capture and the wake engine are off. Only the UI can leave.
    Hibernate,
}

impl VoiceState {
    /// Stable log and UI spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::Awake => "awake",
            Self::Hibernate => "hibernate",
        }
    }

    /// What the daemon may do in this state.
    #[must_use]
    pub const fn capabilities(self) -> Capabilities {
        match self {
            Self::Sleep => Capabilities {
                capture: true,
                wake_engine: true,
                acting: false,
            },
            Self::Awake => Capabilities {
                capture: true,
                wake_engine: true,
                acting: true,
            },
            Self::Hibernate => Capabilities {
                capture: false,
                wake_engine: false,
                acting: false,
            },
        }
    }

    /// Microphone capture is permitted.
    #[must_use]
    pub const fn allows_capture(self) -> bool {
        self.capabilities().capture
    }

    /// A tool dispatcher may invoke one allowlisted tool.
    ///
    /// Session and tools are one privilege: both follow [`Capabilities::acting`].
    #[must_use]
    pub const fn allows_tool_dispatch(self) -> bool {
        self.capabilities().acting
    }

    /// An acting session may be open.
    ///
    /// Same privilege as [`Self::allows_tool_dispatch`].
    #[must_use]
    pub const fn allows_session(self) -> bool {
        self.capabilities().acting
    }
}

impl fmt::Display for VoiceState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Permissions implied by a [`VoiceState`].
///
/// Capture and the wake engine are separate columns in the voice-state table
/// even though they move together today. Acting covers the session and tools,
/// which are granted together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Microphone capture is permitted.
    pub capture: bool,
    /// The wake engine may score frames.
    pub wake_engine: bool,
    /// An acting session and tool dispatch are permitted.
    pub acting: bool,
}

/// Something that happened outside the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The local wake engine accepted a wake phrase.
    WakePhrase,
    /// The local wake engine accepted a sleep phrase.
    SleepPhrase,
    /// The UI asked to end the acting session and return to sleep.
    UiSleep,
    /// The UI asked to hibernate. Legal from sleep and from awake.
    UiHibernate,
    /// The UI asked to leave hibernate.
    ///
    /// Lands in [`VoiceState::Sleep`], never awake. Hibernate must not resume
    /// straight into an acting session. The operator gets a passive listener
    /// first; a later wake phrase, after cooldown, is what enters awake.
    UiResume,
}

impl Event {
    /// Stable log spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WakePhrase => "wake phrase",
            Self::SleepPhrase => "sleep phrase",
            Self::UiSleep => "UI sleep",
            Self::UiHibernate => "UI hibernate",
            Self::UiResume => "UI resume",
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Work the daemon must do because a transition succeeded.
///
/// When more than one effect is present, apply them in slice order. Releasing
/// the acting session comes before capture stops, so a tool cannot run while
/// the device is closing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Open an acting session. Emitted only for sleep → awake.
    OpenSession,
    /// Drop the acting session and any tool permits.
    ///
    /// [`crate::Machine::apply`] stores the new state before it returns, so
    /// this release is bounded by that call. There is no deferred acting step.
    ReleaseActingResources,
    /// Stop capture and the wake engine. The microphone must go idle.
    StopCapture,
    /// Capture may start. Used when hibernate returns to sleep.
    StartCapture,
}

#[cfg(test)]
mod tests {
    use super::{Capabilities, VoiceState};

    #[test]
    fn capabilities_follow_the_voice_table() {
        assert_eq!(
            VoiceState::Sleep.capabilities(),
            Capabilities {
                capture: true,
                wake_engine: true,
                acting: false,
            }
        );
        assert_eq!(
            VoiceState::Awake.capabilities(),
            Capabilities {
                capture: true,
                wake_engine: true,
                acting: true,
            }
        );
        assert_eq!(
            VoiceState::Hibernate.capabilities(),
            Capabilities {
                capture: false,
                wake_engine: false,
                acting: false,
            }
        );
    }

    #[test]
    fn hibernate_has_no_capture() {
        assert!(!VoiceState::Hibernate.allows_capture());
        assert!(!VoiceState::Hibernate.capabilities().capture);
        assert!(!VoiceState::Hibernate.capabilities().wake_engine);
    }

    #[test]
    fn sleep_and_hibernate_cannot_act() {
        assert!(!VoiceState::Sleep.allows_tool_dispatch());
        assert!(!VoiceState::Sleep.allows_session());
        assert!(!VoiceState::Hibernate.allows_tool_dispatch());
        assert!(!VoiceState::Hibernate.allows_session());
        assert!(VoiceState::Awake.allows_tool_dispatch());
        assert!(VoiceState::Awake.allows_session());
    }

    #[test]
    fn display_uses_the_fixed_vocabulary() {
        assert_eq!(VoiceState::Sleep.to_string(), "sleep");
        assert_eq!(VoiceState::Awake.to_string(), "awake");
        assert_eq!(VoiceState::Hibernate.to_string(), "hibernate");
    }
}
