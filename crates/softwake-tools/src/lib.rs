//! Tool allowlist boundary.
//!
//! No runners are registered. [`Allowlist`] rejects every name so an empty
//! registry cannot accidentally run a tool.

/// Names the daemon is willing to run while awake.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Allowlist;

impl Allowlist {
    /// Whether `name` is allowlisted.
    ///
    /// The empty registry has no names, so this is always `false`.
    #[allow(clippy::unused_self)] // No names are registered in this milestone.
    #[must_use]
    pub const fn contains(self, name: &str) -> bool {
        let _ = name;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::Allowlist;

    #[test]
    fn empty_allowlist_contains_nothing() {
        let allowlist = Allowlist;
        assert!(!allowlist.contains("volume"));
        assert!(!allowlist.contains(""));
    }
}
