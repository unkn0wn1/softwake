//! Text session for one awake period.
//!
//! [`TextStubSession`] stores the rendered instructions and accepts an
//! injected completer. This crate has no HTTP client and no provider
//! dependency. Sleep and hibernate close the session.
//!
//! See [ADR 0013](../../docs/ADR-0013-session-provider.md).

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

    /// Record `user_text`, then call `complete(instructions, user_text)`.
    ///
    /// The assistant text is the return value. It is not stored on the session.
    ///
    /// # Errors
    ///
    /// [`SessionError::Empty`] when `user_text` is empty or whitespace.
    /// [`SessionError::Closed`] when the phase is closed. The closure is not called.
    /// [`SessionError::Complete`] when the closure returns `Err`. The user line stays recorded.
    pub fn ask(
        &mut self,
        user_text: &str,
        complete: impl FnOnce(&str, &str) -> Result<String, String>,
    ) -> Result<String, SessionError> {
        if user_text.trim().is_empty() {
            return Err(SessionError::Empty);
        }
        if self.phase != SessionPhase::Open {
            return Err(SessionError::Closed);
        }
        let Some(instructions) = self.instructions.clone() else {
            return Err(SessionError::Closed);
        };
        self.turns.push(user_text.to_owned());
        match complete(&instructions, user_text) {
            Ok(reply) => Ok(reply),
            Err(message) => Err(SessionError::Complete { message }),
        }
    }
}

/// Failure from a session operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The session is not open.
    #[error("session is closed")]
    Closed,
    /// `user_text` is empty or only whitespace.
    #[error("needs text")]
    Empty,
    /// The completer failed. The user line stays recorded.
    ///
    /// `message` is the caller-supplied sentence. This type does not wrap it.
    #[error("{message}")]
    Complete {
        /// Display text from the completer.
        message: String,
    },
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

    #[test]
    fn ask_records_the_user_line_and_returns_the_reply() {
        let mut session = TextStubSession::open("be brief");
        let reply = session
            .ask("hello", |instructions, user| {
                assert_eq!(instructions, "be brief");
                assert_eq!(user, "hello");
                Ok("pong".to_owned())
            })
            .expect("ask");
        assert_eq!(reply, "pong");
        assert_eq!(session.turns(), &["hello".to_owned()]);
        assert_eq!(session.instructions(), Some("be brief"));
    }

    #[test]
    fn ask_on_a_closed_session_does_not_call_the_completer() {
        let mut session = TextStubSession::default();
        let mut called = false;
        let error = session.ask("hello", |_, _| {
            called = true;
            Ok("no".to_owned())
        });
        assert_eq!(error, Err(SessionError::Closed));
        assert!(!called);
        assert!(session.turns().is_empty());
    }

    #[test]
    fn blank_ask_is_empty_and_does_not_record() {
        let mut session = TextStubSession::open("be brief");
        session.push_user_turn("kept").expect("open");
        let mut called = false;
        let error = session.ask("  \n", |_, _| {
            called = true;
            Ok("no".to_owned())
        });
        assert_eq!(error, Err(SessionError::Empty));
        assert!(!called);
        assert_eq!(session.turns(), &["kept".to_owned()]);
    }

    #[test]
    fn completer_error_keeps_the_user_line() {
        let mut session = TextStubSession::open("be brief");
        let error = session
            .ask("hello", |_, _| {
                Err("Provider rejected the credentials.".to_owned())
            })
            .expect_err("complete");
        assert_eq!(
            error,
            SessionError::Complete {
                message: "Provider rejected the credentials.".to_owned(),
            }
        );
        assert_eq!(error.to_string(), "Provider rejected the credentials.");
        assert_eq!(session.turns(), &["hello".to_owned()]);
    }

    #[test]
    fn close_then_ask_is_closed_and_close_stays_idempotent() {
        let mut session = TextStubSession::open("be brief");
        session.close();
        session.close();
        assert_eq!(session.phase(), SessionPhase::Closed);
        let mut called = false;
        let error = session.ask("hello", |_, _| {
            called = true;
            Ok("no".to_owned())
        });
        assert_eq!(error, Err(SessionError::Closed));
        assert!(!called);
        assert!(session.turns().is_empty());
        assert!(session.instructions().is_none());
    }
}
