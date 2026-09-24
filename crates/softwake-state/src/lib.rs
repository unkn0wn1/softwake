//! Sleep, awake, and hibernate.
//!
//! Names match the voice-state vocabulary used by the UI and the logs.
//! [`Machine`] decides whether a transition is legal and which side effects
//! the daemon must run. It does not open a microphone, call a model, or
//! invoke a tool.
//!
//! Leaving hibernate is [`Event::UiResume`], which lands in [`VoiceState::Sleep`].
//! There is no hibernate → awake transition: a passive listener has to be up
//! before a wake phrase can open an acting session.

mod error;
mod machine;
mod transition;
mod voice_state;

pub use error::StateError;
pub use machine::{CooldownConfig, Machine};
pub use transition::Applied;
pub use voice_state::{Capabilities, Effect, Event, VoiceState};
