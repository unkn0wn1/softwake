//! Phase-1 tool allowlist.
//!
//! Exactly one tool is registered: [`ECHO_TOOL`]. It is a pure function.
//! Nothing here spawns a process, writes a file, or opens a socket.

/// Name of the only tool phase 1 will run.
pub const ECHO_TOOL: &str = "echo";

/// Names the daemon is willing to run while awake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allowlist {
    names: &'static [&'static str],
}

impl Allowlist {
    /// The phase-1 allowlist. It contains [`ECHO_TOOL`] and nothing else.
    #[must_use]
    pub const fn phase1() -> Self {
        Self {
            names: &[ECHO_TOOL],
        }
    }

    /// Names in this allowlist, in registration order.
    #[must_use]
    pub const fn names(self) -> &'static [&'static str] {
        self.names
    }

    /// Whether `name` is allowlisted.
    #[must_use]
    pub fn contains(self, name: &str) -> bool {
        self.names.contains(&name)
    }
}

impl Default for Allowlist {
    fn default() -> Self {
        Self::phase1()
    }
}

/// Text a tool returns to the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    /// Deterministic summary safe to show to an operator.
    pub detail: String,
}

/// Failure from [`ToolRegistry::invoke`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    /// `name` is not on the allowlist.
    #[error("unknown tool: {name}")]
    Unknown {
        /// Name that was rejected.
        name: String,
    },
}

/// The phase-1 registry. It runs [`ECHO_TOOL`] and rejects every other name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolRegistry {
    allowlist: Allowlist,
}

impl ToolRegistry {
    /// Registry whose allowlist is [`Allowlist::phase1`].
    #[must_use]
    pub const fn phase1() -> Self {
        Self {
            allowlist: Allowlist::phase1(),
        }
    }

    /// Allowlist this registry consults.
    #[must_use]
    pub const fn allowlist(self) -> Allowlist {
        self.allowlist
    }

    /// Run `name` with `args`.
    ///
    /// [`ECHO_TOOL`] with no arguments returns `pong`. With arguments it
    /// returns `echo:` plus those arguments joined by spaces. The arguments
    /// are not interpreted.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Unknown`] when `name` is not allowlisted.
    pub fn invoke(&self, name: &str, args: &[String]) -> Result<ToolResult, ToolError> {
        if name == ECHO_TOOL && self.allowlist.contains(name) {
            Ok(ToolResult {
                detail: echo_detail(args),
            })
        } else {
            Err(ToolError::Unknown {
                name: name.to_owned(),
            })
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::phase1()
    }
}

fn echo_detail(args: &[String]) -> String {
    if args.is_empty() {
        "pong".to_owned()
    } else {
        let mut detail = String::from("echo:");
        for arg in args {
            detail.push(' ');
            detail.push_str(arg);
        }
        detail
    }
}

#[cfg(test)]
mod tests {
    use super::{Allowlist, ECHO_TOOL, ToolError, ToolRegistry};

    #[test]
    fn phase1_allowlist_is_exactly_echo() {
        let allowlist = Allowlist::phase1();
        assert_eq!(allowlist.names(), &[ECHO_TOOL]);
        assert!(allowlist.contains("echo"));
        assert!(!allowlist.contains("volume"));
        assert!(!allowlist.contains("notify_local"));
        assert!(!allowlist.contains("Echo"));
        assert!(!allowlist.contains(""));
        assert_eq!(Allowlist::default().names(), allowlist.names());
    }

    #[test]
    fn echo_is_deterministic() {
        let registry = ToolRegistry::phase1();
        assert_eq!(registry.invoke("echo", &[]).expect("echo").detail, "pong");
        assert_eq!(
            registry
                .invoke("echo", &["hello".to_owned()])
                .expect("echo")
                .detail,
            "echo: hello"
        );
        assert_eq!(
            registry
                .invoke("echo", &["hello".to_owned(), "world".to_owned()])
                .expect("echo")
                .detail,
            "echo: hello world"
        );
        assert_eq!(
            registry
                .invoke("echo", &["a;b".to_owned(), "c".to_owned()])
                .expect("echo")
                .detail,
            "echo: a;b c"
        );
    }

    #[test]
    fn unknown_names_are_rejected() {
        let registry = ToolRegistry::default();
        let error = registry
            .invoke("volume", &["1".to_owned()])
            .expect_err("volume");
        assert_eq!(
            error,
            ToolError::Unknown {
                name: "volume".to_owned()
            }
        );
        assert_eq!(error.to_string(), "unknown tool: volume");
        assert!(registry.invoke("", &[]).is_err());
    }
}
