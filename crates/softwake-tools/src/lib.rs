//! Tool registry.
//!
//! Each name has a risk: [`ToolRisk::Safe`] runs immediately, [`ToolRisk::Confirm`]
//! waits for an explicit confirmation held by the daemon, and [`ToolRisk::Deny`]
//! never runs. [`invoke`](ToolRegistry::invoke) runs safe tools only.
//! [`invoke_confirmed`](ToolRegistry::invoke_confirmed) runs confirm-gated tools
//! and is the daemon's step after the operator accepts. Neither function
//! spawns a process, writes a file, or opens a socket. The notification sink
//! lives in the daemon; this crate only formats the line.

/// Name of the safe tool. Behaviour matches phase 1.
pub const ECHO_TOOL: &str = "echo";

/// Confirm-gated tool. The daemon appends the formatted line to an in-memory sink.
pub const NOTIFY_TOOL: &str = "notify";

/// Registered deny name. It is never runnable.
pub const SHELL_TOOL: &str = "shell";

/// How the daemon may treat a registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRisk {
    /// Run while awake, with no extra confirmation.
    Safe,
    /// Do not run until the operator confirms that pending call.
    Confirm,
    /// Never run, including while awake and after a confirm attempt.
    Deny,
}

impl ToolRisk {
    /// Stable spelling for logs and operator text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Confirm => "confirm",
            Self::Deny => "deny",
        }
    }
}

impl std::fmt::Display for ToolRisk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolMeta {
    /// Protocol and command name.
    pub name: &'static str,
    /// Whether the tool may run, must wait, or is refused.
    pub risk: ToolRisk,
    /// Short operator-facing description.
    pub description: &'static str,
}

const PHASE2: &[ToolMeta] = &[
    ToolMeta {
        name: ECHO_TOOL,
        risk: ToolRisk::Safe,
        description: "Repeat the arguments. No side effect.",
    },
    ToolMeta {
        name: NOTIFY_TOOL,
        risk: ToolRisk::Confirm,
        description: "Append a notification to the in-memory sink.",
    },
    ToolMeta {
        name: SHELL_TOOL,
        risk: ToolRisk::Deny,
        description: "Run a shell. Denied.",
    },
];

/// Text a tool returns to the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    /// Deterministic summary safe to show to an operator.
    pub detail: String,
}

/// Failure from a registry invoke.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolError {
    /// `name` is not registered.
    #[error("unknown tool: {name}")]
    Unknown {
        /// Name that was rejected.
        name: String,
    },

    /// `name` is registered as [`ToolRisk::Deny`].
    #[error("tool denied: {name}")]
    Denied {
        /// Name that was rejected.
        name: String,
    },

    /// `name` is [`ToolRisk::Confirm`] and this call did not carry a confirmation.
    #[error("tool requires confirmation: {name}")]
    NeedsConfirm {
        /// Name that was not run.
        name: String,
    },

    /// [`ToolRegistry::invoke_confirmed`] was asked to run a tool that is not confirm-gated.
    #[error("tool is not confirm-gated: {name}")]
    NotConfirmGated {
        /// Name that was rejected.
        name: String,
    },
}

/// Registered tools and their pure runners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolRegistry {
    tools: &'static [ToolMeta],
}

impl ToolRegistry {
    /// Registry used by the daemon: `echo`, `notify`, and `shell`.
    #[must_use]
    pub const fn phase2() -> Self {
        Self { tools: PHASE2 }
    }

    /// Metadata in registration order.
    #[must_use]
    pub const fn entries(self) -> &'static [ToolMeta] {
        self.tools
    }

    /// Metadata for `name`, if it is registered.
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&ToolMeta> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// Risk for `name`, if it is registered.
    #[must_use]
    pub fn risk(&self, name: &str) -> Option<ToolRisk> {
        self.lookup(name).map(|tool| tool.risk)
    }

    /// Run a [`ToolRisk::Safe`] tool.
    ///
    /// Confirm-gated and denied names do not run. [`ECHO_TOOL`] with no
    /// arguments returns `pong`. With arguments it returns `echo:` plus those
    /// arguments joined by spaces. The arguments are not interpreted.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Unknown`] when `name` is not registered,
    /// [`ToolError::NeedsConfirm`] for a confirm-gated tool, and
    /// [`ToolError::Denied`] for a denied tool.
    pub fn invoke(&self, name: &str, args: &[String]) -> Result<ToolResult, ToolError> {
        self.invoke_safe(name, args)
    }

    /// Run a [`ToolRisk::Safe`] tool. Same rules as [`Self::invoke`].
    ///
    /// # Errors
    ///
    /// See [`Self::invoke`].
    pub fn invoke_safe(&self, name: &str, args: &[String]) -> Result<ToolResult, ToolError> {
        match self.lookup(name).map(|tool| tool.risk) {
            Some(ToolRisk::Safe) => Ok(ToolResult {
                detail: render(name, args),
            }),
            Some(ToolRisk::Confirm) => Err(ToolError::NeedsConfirm {
                name: name.to_owned(),
            }),
            Some(ToolRisk::Deny) => Err(ToolError::Denied {
                name: name.to_owned(),
            }),
            None => Err(ToolError::Unknown {
                name: name.to_owned(),
            }),
        }
    }

    /// Run a [`ToolRisk::Confirm`] tool after the daemon has accepted it.
    ///
    /// This function does not check a token. The daemon calls it only after
    /// confirmation. [`NOTIFY_TOOL`] returns the arguments joined by spaces.
    /// That string is the notification line; this crate does not store it.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Unknown`] when `name` is not registered,
    /// [`ToolError::Denied`] for a denied tool, and
    /// [`ToolError::NotConfirmGated`] for a safe tool.
    pub fn invoke_confirmed(&self, name: &str, args: &[String]) -> Result<ToolResult, ToolError> {
        match self.lookup(name).map(|tool| tool.risk) {
            Some(ToolRisk::Confirm) => Ok(ToolResult {
                detail: render(name, args),
            }),
            Some(ToolRisk::Deny) => Err(ToolError::Denied {
                name: name.to_owned(),
            }),
            Some(ToolRisk::Safe) => Err(ToolError::NotConfirmGated {
                name: name.to_owned(),
            }),
            None => Err(ToolError::Unknown {
                name: name.to_owned(),
            }),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::phase2()
    }
}

fn render(name: &str, args: &[String]) -> String {
    match name {
        ECHO_TOOL => echo_detail(args),
        NOTIFY_TOOL => args.join(" "),
        _ => String::new(),
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
    use super::{
        ECHO_TOOL, NOTIFY_TOOL, SHELL_TOOL, ToolError, ToolRegistry, ToolResult, ToolRisk,
    };

    fn registry() -> ToolRegistry {
        ToolRegistry::phase2()
    }

    #[test]
    fn phase2_registers_safe_confirm_and_deny() {
        let registry = registry();
        assert_eq!(
            registry
                .entries()
                .iter()
                .map(|tool| (tool.name, tool.risk))
                .collect::<Vec<_>>(),
            vec![
                (ECHO_TOOL, ToolRisk::Safe),
                (NOTIFY_TOOL, ToolRisk::Confirm),
                (SHELL_TOOL, ToolRisk::Deny),
            ]
        );
        assert_eq!(registry.risk("echo"), Some(ToolRisk::Safe));
        assert_eq!(registry.risk("notify"), Some(ToolRisk::Confirm));
        assert_eq!(registry.risk("shell"), Some(ToolRisk::Deny));
        assert_eq!(registry.risk("volume"), None);
        assert_eq!(registry.risk("Echo"), None);
        assert_eq!(registry.risk(""), None);
        assert!(
            registry.lookup("echo").is_some_and(|tool| {
                tool.risk == ToolRisk::Safe && !tool.description.is_empty()
            })
        );
        assert!(
            registry
                .lookup("notify")
                .is_some_and(|tool| tool.risk == ToolRisk::Confirm)
        );
        assert_eq!(ToolRegistry::default().entries(), registry.entries());
    }

    #[test]
    fn echo_is_deterministic_and_auto_runnable() {
        let registry = registry();
        assert_eq!(invoke_ok(&registry, "echo", &[]), "pong");
        assert_eq!(invoke_ok(&registry, "echo", &["hello"]), "echo: hello");
        assert_eq!(
            invoke_ok(&registry, "echo", &["hello", "world"]),
            "echo: hello world"
        );
        assert_eq!(invoke_ok(&registry, "echo", &["a;b", "c"]), "echo: a;b c");
        assert_eq!(
            registry
                .invoke_safe("echo", &["hello".to_owned()])
                .expect("safe")
                .detail,
            "echo: hello"
        );
        assert_eq!(
            registry.invoke_confirmed("echo", &[]),
            Err(ToolError::NotConfirmGated {
                name: "echo".to_owned()
            })
        );
    }

    #[test]
    fn notify_does_not_run_until_invoke_confirmed() {
        let registry = registry();
        let args = vec!["hello".to_owned(), "there".to_owned()];
        assert_eq!(
            registry.invoke("notify", &args),
            Err(ToolError::NeedsConfirm {
                name: "notify".to_owned()
            })
        );
        assert_eq!(
            registry
                .invoke("notify", &args)
                .expect_err("blind")
                .to_string(),
            "tool requires confirmation: notify"
        );
        assert_eq!(
            registry
                .invoke_confirmed("notify", &args)
                .expect("confirmed")
                .detail,
            "hello there"
        );
        assert_eq!(
            registry
                .invoke_confirmed("notify", &[])
                .expect("empty")
                .detail,
            ""
        );
    }

    #[test]
    fn shell_is_denied_from_both_entry_points() {
        let registry = registry();
        let denied = ToolError::Denied {
            name: "shell".to_owned(),
        };
        assert_eq!(
            registry.invoke("shell", &["rm".to_owned()]),
            Err(denied.clone())
        );
        assert_eq!(
            registry.invoke_confirmed("shell", &["rm".to_owned()]),
            Err(denied)
        );
        assert_eq!(
            registry.invoke("shell", &[]).expect_err("deny").to_string(),
            "tool denied: shell"
        );
    }

    #[test]
    fn unknown_names_are_rejected() {
        let registry = registry();
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
        assert!(registry.invoke_confirmed("volume", &[]).is_err());
    }

    fn invoke_ok(registry: &ToolRegistry, name: &str, args: &[&str]) -> String {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let ToolResult { detail } = registry.invoke(name, &owned).expect(name);
        detail
    }
}
