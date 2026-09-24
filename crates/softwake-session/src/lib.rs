//! Acting-session boundary.
//!
//! Model clients are not linked. [`Session`] reports closed so a caller cannot
//! mistake the placeholder for a live session.

/// Whether an acting session is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionPhase {
    /// No model session.
    #[default]
    Closed,
    /// Session is accepting acting input. Not constructed in this milestone.
    Open,
}

/// Placeholder acting session. It stays closed until a later milestone.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    phase: SessionPhase,
}

impl Session {
    /// Phase of this placeholder. Always [`SessionPhase::Closed`].
    #[must_use]
    pub const fn phase(self) -> SessionPhase {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::{Session, SessionPhase};

    #[test]
    fn placeholder_session_stays_closed() {
        assert_eq!(Session::default().phase(), SessionPhase::Closed);
    }
}
