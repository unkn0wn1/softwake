//! Tool registry.
//!
//! Each name has a risk: [`ToolRisk::Safe`] runs immediately, [`ToolRisk::Confirm`]
//! waits for an explicit confirmation held by the daemon, and [`ToolRisk::Deny`]
//! never runs. [`invoke`](ToolRegistry::invoke) runs safe tools only.
//! [`invoke_confirmed`](ToolRegistry::invoke_confirmed) runs confirm-gated tools
//! and is the daemon's step after the operator accepts. Neither function
//! spawns a process, writes a file, or opens a socket. The notification sink
//! and the email outbox live in the daemon. This crate classifies names and
//! shapes `email_send` arguments. It does not send.
//!
//! [`SHELL_TOOL`] is confirm-gated in the registry. The operator default is deny.
//! Tools Settings choose always allow, ask, or deny. The daemon expands glossary
//! aliases, then may spawn `/bin/sh -c` via [`shell`]. This crate still does not spawn on invoke.

mod settings;
mod shell;

pub use settings::{
    ConfirmPolicy, FileToolsSettings, TOOLS_FILE_NAME, ToolPermission, ToolsSettings,
    ToolsSettingsError, default_permission, parse_confirm_policy, parse_tool_permission,
    resolve_tools_file, resolve_tools_file_from,
};
pub use shell::{
    DEFAULT_OUTPUT_CAP, DEFAULT_SHELL_TIMEOUT, ShellError, ShellOutput, format_shell_output,
    run_shell, run_shell_with,
};

/// Name of the safe tool. Behaviour matches phase 1.
pub const ECHO_TOOL: &str = "echo";

/// Confirm-gated tool. The daemon appends the formatted line to an in-memory sink.
pub const NOTIFY_TOOL: &str = "notify";

/// Confirm-gated send. The daemon appends one in-memory message after confirm.
pub const EMAIL_SEND_TOOL: &str = "email_send";

/// Confirm-gated shell. Operator default is deny; the daemon spawns only after Ask or Always allow.
pub const SHELL_TOOL: &str = "shell";

/// Confirm-gated skill write. Daemon saves Markdown after confirm.
pub const SKILL_SAVE_TOOL: &str = "skill_save";

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
        name: EMAIL_SEND_TOOL,
        risk: ToolRisk::Confirm,
        description: "Append one message to the in-memory outbox.",
    },
    ToolMeta {
        name: SHELL_TOOL,
        risk: ToolRisk::Confirm,
        description: "Run a shell command after confirm. Off until enabled in Tools Settings.",
    },
    ToolMeta {
        name: SKILL_SAVE_TOOL,
        risk: ToolRisk::Confirm,
        description: "Save a Markdown skill (procedure / pitfalls / verify) after confirm.",
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

    /// [`EMAIL_SEND_TOOL`] was missing to, subject, or body.
    #[error("{name} needs to, subject, and body")]
    InvalidArgs {
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
    /// Registry used by the daemon: `echo`, `notify`, `email_send`, and `shell`.
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
    /// [`EMAIL_SEND_TOOL`] parses to, subject, and body, then returns an empty
    /// detail. The receipt id is chosen by the daemon when it sends.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError::Unknown`] when `name` is not registered,
    /// [`ToolError::Denied`] for a denied tool,
    /// [`ToolError::NotConfirmGated`] for a safe tool, and
    /// [`ToolError::InvalidArgs`] when [`EMAIL_SEND_TOOL`] has fewer than
    /// three arguments.
    pub fn invoke_confirmed(&self, name: &str, args: &[String]) -> Result<ToolResult, ToolError> {
        match self.lookup(name).map(|tool| tool.risk) {
            Some(ToolRisk::Confirm) => {
                if name == EMAIL_SEND_TOOL {
                    parse_email_send_args(args)?;
                }
                if name == SKILL_SAVE_TOOL {
                    parse_skill_save_args(args)?;
                }
                Ok(ToolResult {
                    detail: render(name, args),
                })
            }
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
        NOTIFY_TOOL | SHELL_TOOL => args.join(" "),
        SKILL_SAVE_TOOL => match parse_skill_save_args(args) {
            Ok(parsed) => format!("skill_save: {}", parsed.title),
            Err(_) => args.join(" "),
        },
        _ => String::new(),
    }
}

/// Slots for [`EMAIL_SEND_TOOL`].
///
/// The strings are stored unchanged. Nothing here checks that `to` is an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSendArgs {
    /// Recipient text.
    pub to: String,
    /// Subject line. One argument, which may itself contain spaces.
    pub subject: String,
    /// Body text. The remaining arguments joined by a single space.
    pub body: String,
}

/// Split `email_send` arguments into to, subject, and body.
///
/// `args[0]` is `to`, `args[1]` is `subject`, and `args[2..]` joined by a
/// single space is `body`. A present slot may be empty. Missing slots are rejected.
///
/// # Errors
///
/// Returns [`ToolError::InvalidArgs`] when `args` has fewer than three elements.
pub fn parse_email_send_args(args: &[String]) -> Result<EmailSendArgs, ToolError> {
    let Some((to, rest)) = args.split_first() else {
        return Err(invalid_email_args());
    };
    let Some((subject, body)) = rest.split_first() else {
        return Err(invalid_email_args());
    };
    if body.is_empty() {
        return Err(invalid_email_args());
    }
    Ok(EmailSendArgs {
        to: to.clone(),
        subject: subject.clone(),
        body: body.join(" "),
    })
}

/// Slots for [`SKILL_SAVE_TOOL`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSaveArgs {
    /// Skill title.
    pub title: String,
    /// Procedure section.
    pub procedure: String,
    /// Pitfalls section.
    pub pitfalls: String,
    /// Verify section.
    pub verify: String,
}

/// Split `skill_save` arguments into title, procedure, pitfalls, and verify.
///
/// `args[0]` title, `args[1]` procedure, `args[2]` pitfalls, `args[3..]` joined as verify.
/// Present slots may be empty except title.
///
/// # Errors
///
/// [`ToolError::InvalidArgs`] when fewer than four arguments.
pub fn parse_skill_save_args(args: &[String]) -> Result<SkillSaveArgs, ToolError> {
    if args.len() < 4 {
        return Err(ToolError::InvalidArgs {
            name: SKILL_SAVE_TOOL.to_owned(),
        });
    }
    let title = args[0].trim();
    if title.is_empty() {
        return Err(ToolError::InvalidArgs {
            name: SKILL_SAVE_TOOL.to_owned(),
        });
    }
    Ok(SkillSaveArgs {
        title: title.to_owned(),
        procedure: args[1].clone(),
        pitfalls: args[2].clone(),
        verify: args[3..].join(" "),
    })
}

fn invalid_email_args() -> ToolError {
    ToolError::InvalidArgs {
        name: EMAIL_SEND_TOOL.to_owned(),
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
        ECHO_TOOL, EMAIL_SEND_TOOL, EmailSendArgs, NOTIFY_TOOL, SHELL_TOOL, SKILL_SAVE_TOOL,
        ToolError, ToolRegistry, ToolResult, ToolRisk, parse_email_send_args,
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
                (EMAIL_SEND_TOOL, ToolRisk::Confirm),
                (SHELL_TOOL, ToolRisk::Confirm),
                (SKILL_SAVE_TOOL, ToolRisk::Confirm),
            ]
        );
        assert_eq!(registry.risk("echo"), Some(ToolRisk::Safe));
        assert_eq!(registry.risk("notify"), Some(ToolRisk::Confirm));
        assert_eq!(registry.risk("email_send"), Some(ToolRisk::Confirm));
        assert_eq!(registry.risk("shell"), Some(ToolRisk::Confirm));
        assert_eq!(registry.risk("Email_Send"), None);
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
        assert!(registry.lookup("email_send").is_some_and(|tool| {
            tool.risk == ToolRisk::Confirm
                && tool.description == "Append one message to the in-memory outbox."
        }));
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
    fn email_send_does_not_run_until_invoke_confirmed_and_parses_fields() {
        let registry = registry();
        let args = vec![
            "ada@example.com".to_owned(),
            "hello".to_owned(),
            "a".to_owned(),
            "short".to_owned(),
        ];
        assert_eq!(
            registry.invoke("email_send", &args),
            Err(ToolError::NeedsConfirm {
                name: "email_send".to_owned(),
            })
        );
        assert_eq!(
            registry
                .invoke("email_send", &[])
                .expect_err("blind")
                .to_string(),
            "tool requires confirmation: email_send"
        );
        let confirmed = registry
            .invoke_confirmed("email_send", &args)
            .expect("confirmed");
        assert_eq!(confirmed.detail, "");
        let again = registry
            .invoke_confirmed("email_send", &args)
            .expect("pure");
        assert_eq!(again, confirmed);
        assert_eq!(
            parse_email_send_args(&args).expect("parse"),
            EmailSendArgs {
                to: "ada@example.com".to_owned(),
                subject: "hello".to_owned(),
                body: "a short".to_owned(),
            }
        );
        let spaced = parse_email_send_args(&[
            " ada@example.com ".to_owned(),
            "hello there".to_owned(),
            "line one".to_owned(),
        ])
        .expect("spaces");
        assert_eq!(spaced.to, " ada@example.com ");
        assert_eq!(spaced.subject, "hello there");
        assert_eq!(spaced.body, "line one");
        assert_eq!(
            parse_email_send_args(&["not-an-address".to_owned(), "s".to_owned(), "b".to_owned(),])
                .expect("no address check")
                .to,
            "not-an-address"
        );
        for short in [
            vec![],
            vec!["only-to".to_owned()],
            vec!["only-to".to_owned(), "subject".to_owned()],
        ] {
            assert_eq!(
                registry.invoke_confirmed("email_send", &short),
                Err(ToolError::InvalidArgs {
                    name: "email_send".to_owned(),
                })
            );
            assert_eq!(
                parse_email_send_args(&short)
                    .expect_err("short")
                    .to_string(),
                "email_send needs to, subject, and body"
            );
            assert_eq!(
                registry.invoke("email_send", &short),
                Err(ToolError::NeedsConfirm {
                    name: "email_send".to_owned(),
                })
            );
        }
        let empty_body = parse_email_send_args(&[
            "ada@example.com".to_owned(),
            "hello".to_owned(),
            String::new(),
        ])
        .expect("empty body slot");
        assert_eq!(empty_body.body, "");
    }

    #[test]
    fn shell_is_confirm_gated_and_invoke_confirmed_returns_command() {
        let registry = registry();
        let args = vec!["echo".to_owned(), "hi".to_owned()];
        assert_eq!(
            registry.invoke("shell", &args),
            Err(ToolError::NeedsConfirm {
                name: "shell".to_owned()
            })
        );
        assert_eq!(
            registry
                .invoke_confirmed("shell", &args)
                .expect("confirmed")
                .detail,
            "echo hi"
        );
        assert_eq!(
            registry
                .invoke("shell", &[])
                .expect_err("confirm")
                .to_string(),
            "tool requires confirmation: shell"
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
