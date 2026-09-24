//! Text-only acting session.
//!
//! [`TextStubSession`] records the system instructions for one awake period
//! and can accept a synthetic user turn. There is no model client and no
//! network. Sleep and hibernate close the session.

/// Whether an acting session is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionPhase {
    /// No acting session. Instructions are not available.
    #[default]
    Closed,
    /// Instructions are stored and a synthetic user turn can be recorded.
    Open,
}

/// In-memory acting session for one awake period.
///
/// [`Self::open`] stores the soul-rendered instructions. [`Self::close`]
/// drops them. A closed session refuses further turns.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextStubSession {
    phase: SessionPhase,
    instructions: Option<String>,
    turns: Vec<String>,
}

impl TextStubSession {
    /// Open a session that holds `instructions`.
    #[must_use]
    pub fn open(instructions: impl Into<String>) -> Self {
        Self {
            phase: SessionPhase::Open,
            instructions: Some(instructions.into()),
            turns: Vec::new(),
        }
    }

    /// Close the session and drop its instructions and turns.
    pub fn close(&mut self) {
        self.phase = SessionPhase::Closed;
        self.instructions = None;
        self.turns.clear();
    }

    /// Whether this session is open.
    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// System instructions while the session is open.
    #[must_use]
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// Record one synthetic user turn.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::Closed`] when the session is not open.
    pub fn push_user_turn(&mut self, text: &str) -> Result<(), SessionError> {
        if self.phase != SessionPhase::Open {
            return Err(SessionError::Closed);
        }
        self.turns.push(text.to_owned());
        Ok(())
    }

    /// Turns recorded since [`Self::open`], in order.
    #[must_use]
    pub fn turns(&self) -> &[String] {
        &self.turns
    }
}

/// Failure from a session operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// [`TextStubSession::push_user_turn`] was called on a closed session.
    #[error("session is closed")]
    Closed,
}

#[cfg(test)]
mod tests {
    use super::{SessionError, SessionPhase, TextStubSession};

    #[test]
    fn open_stores_instructions_and_close_clears_them() {
        let mut session = TextStubSession::open("be brief");
        assert_eq!(session.phase(), SessionPhase::Open);
        assert_eq!(session.instructions(), Some("be brief"));
        assert_ne!(session.phase(), SessionPhase::Closed);

        session.push_user_turn("hello").expect("open session");
        assert_eq!(session.turns(), &["hello".to_owned()]);

        session.close();
        assert_eq!(session.phase(), SessionPhase::Closed);
        assert_eq!(session.instructions(), None);
        assert!(session.turns().is_empty());
        assert_eq!(session.push_user_turn("again"), Err(SessionError::Closed));
    }

    #[test]
    fn default_session_cannot_be_mistaken_for_open() {
        let session = TextStubSession::default();
        assert_eq!(session.phase(), SessionPhase::Closed);
        assert!(session.instructions().is_none());
        assert!(session.turns().is_empty());
        assert_ne!(session.phase(), SessionPhase::Open);
    }

    #[test]
    fn close_is_idempotent() {
        let mut session = TextStubSession::default();
        session.close();
        assert_eq!(session.phase(), SessionPhase::Closed);
        assert!(session.instructions().is_none());
    }
}
