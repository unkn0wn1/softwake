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

mod chat_schema;
mod cloud_read;
mod mcp_config;
mod messengers;
mod schedule;
mod settings;
mod shell;
mod softwake_ctl;

pub use chat_schema::{advertise_chat_tools, tool_args_from_json};
pub use cloud_read::{
    CalendarGetArgs, CalendarListArgs, DriveGetArgs, DriveListArgs, DriveSearchArgs, EmailGetArgs,
    EmailListArgs, EmailSearchArgs, parse_calendar_get_args, parse_calendar_list_args,
    parse_drive_get_args, parse_drive_list_args, parse_drive_search_args, parse_email_get_args,
    parse_email_list_args, parse_email_search_args,
};
pub use mcp_config::{
    MAX_MCP_BYTES, MCP_FILE_NAME, McpConfigError, McpFile, McpServerConfig, McpTransport,
    is_mcp_tool_name, load_mcp, mcp_tool_name, parse_mcp_permission, resolve_mcp_file,
    sanitize_mcp_id, save_mcp, split_mcp_tool_name,
};
pub use messengers::{
    CHANNEL_DESKTOP, CHANNEL_TELEGRAM, ChannelFlags, HUD_CHAT_INBOX_FILE_NAME, HudChatInbox,
    InboxTurn, MESSENGERS_FILE_NAME, MessengersError, MessengersFile, TelegramChannel,
    append_hud_inbox, find_profile_for_telegram_chat, load_messengers,
    resolve_active_messengers_file, resolve_hud_chat_inbox, resolve_messengers_file,
    save_messengers, take_hud_inbox, wants_ask_fanout, wants_timer_push,
};
pub use schedule::{
    CATCH_UP_GRACE_MS, CronExpr, MAX_ENTRIES, SCHEDULES_FILE_NAME, ScheduleAction, ScheduleEntry,
    ScheduleError, ScheduleKind, SchedulesFile, TIMEZONE_LOCAL, advance_after_fire, apply_action,
    compute_next_fire_ms, fire_notify_line, fire_speak_line, list_profile_ids, load_schedules,
    new_schedule_id, now_ms, parse_schedule_args, refresh_next_fire, resolve_active_schedules_file,
    resolve_schedules_file, save_schedules, should_fire, skip_missed, validate_entry,
};
pub use settings::{
    ConfirmPolicy, EmailOauthStatus, FileToolsSettings, TOOLS_FILE_NAME, TOOLS_PERMISSIONS_LEAD,
    ToolPermission, ToolsSettings, ToolsSettingsError, default_permission, parse_confirm_policy,
    parse_tool_permission, resolve_tools_file, resolve_tools_file_from, tools_permissions_appendix,
};
pub use shell::{
    DEFAULT_OUTPUT_CAP, DEFAULT_SHELL_TIMEOUT, ShellError, ShellOutput, format_shell_output,
    run_shell, run_shell_with,
};
pub use softwake_ctl::{
    SOFTWAKE_CTL_TOOLS, SoftwakeCtlEffect, is_softwake_ctl, parse_softwake_ctl,
    softwake_ctl_is_list,
};

/// Name of the safe tool. Behaviour matches phase 1.
pub const ECHO_TOOL: &str = "echo";

/// Confirm-gated tool. The daemon appends the formatted line to an in-memory sink.
pub const NOTIFY_TOOL: &str = "notify";

/// Confirm-gated send. The daemon appends one in-memory message after confirm.
pub const EMAIL_SEND_TOOL: &str = "email_send";

/// Inbox list (Gmail / Graph). Default Always allow.
pub const EMAIL_LIST_TOOL: &str = "email_list";

/// Inbox search. Default Ask.
pub const EMAIL_SEARCH_TOOL: &str = "email_search";

/// Inbox get-by-id. Default Ask.
pub const EMAIL_GET_TOOL: &str = "email_get";

/// Upcoming calendar events. Default Always allow.
pub const CALENDAR_LIST_TOOL: &str = "calendar_list";

/// Calendar get-by-id. Default Ask.
pub const CALENDAR_GET_TOOL: &str = "calendar_get";

/// Drive / `AppFolder` list. Default Always allow.
pub const DRIVE_LIST_TOOL: &str = "drive_list";

/// Drive search. Default Ask.
pub const DRIVE_SEARCH_TOOL: &str = "drive_search";

/// Drive metadata (+ optional cheap text). Default Ask.
pub const DRIVE_GET_TOOL: &str = "drive_get";

/// Confirm-gated shell. Operator default is deny; the daemon spawns only after Ask or Always allow.
pub const SHELL_TOOL: &str = "shell";

/// Confirm-gated skill write. Daemon saves Markdown after confirm.
pub const SKILL_SAVE_TOOL: &str = "skill_save";

/// List saved skills (id + title). Default Always allow.
pub const SKILL_LIST_TOOL: &str = "skill_list";

/// Get one skill by id (full sections). Default Ask.
pub const SKILL_GET_TOOL: &str = "skill_get";

/// Confirm-gated schedule mutate (create/edit/delete). `list` is included and confirm-gated.
pub const SCHEDULE_TOOL: &str = "schedule";

/// Softwake ctl: `/status` mirror. Default Always allow.
pub const SOFTWAKE_STATUS_TOOL: &str = softwake_ctl::SOFTWAKE_STATUS_TOOL;
/// Softwake ctl: list chat/voice models. Default Always allow.
pub const SOFTWAKE_LIST_MODELS_TOOL: &str = softwake_ctl::SOFTWAKE_LIST_MODELS_TOOL;
/// Softwake ctl: list TTS voices. Default Always allow.
pub const SOFTWAKE_LIST_VOICES_TOOL: &str = softwake_ctl::SOFTWAKE_LIST_VOICES_TOOL;
/// Softwake ctl: list profiles. Default Always allow.
pub const SOFTWAKE_LIST_PROFILES_TOOL: &str = softwake_ctl::SOFTWAKE_LIST_PROFILES_TOOL;
/// Softwake ctl: set chat or voice model. Default Ask.
pub const SOFTWAKE_SET_MODEL_TOOL: &str = softwake_ctl::SOFTWAKE_SET_MODEL_TOOL;
/// Softwake ctl: set TTS voice. Default Ask.
pub const SOFTWAKE_SET_VOICE_TOOL: &str = softwake_ctl::SOFTWAKE_SET_VOICE_TOOL;
/// Softwake ctl: switch profile. Default Ask.
pub const SOFTWAKE_SET_PROFILE_TOOL: &str = softwake_ctl::SOFTWAKE_SET_PROFILE_TOOL;
/// Softwake ctl: sleep. Default Ask.
pub const SOFTWAKE_SLEEP_TOOL: &str = softwake_ctl::SOFTWAKE_SLEEP_TOOL;
/// Softwake ctl: hibernate. Default Ask.
pub const SOFTWAKE_HIBERNATE_TOOL: &str = softwake_ctl::SOFTWAKE_HIBERNATE_TOOL;
/// Softwake ctl: resume from hibernate. Default Ask.
pub const SOFTWAKE_RESUME_TOOL: &str = softwake_ctl::SOFTWAKE_RESUME_TOOL;
/// Softwake ctl: `/new` fresh session. Default Ask.
pub const SOFTWAKE_NEW_SESSION_TOOL: &str = softwake_ctl::SOFTWAKE_NEW_SESSION_TOOL;
/// Softwake ctl: full refresh (profile soul + clear session + HUD reseed + MCP). Default Ask.
pub const SOFTWAKE_REFRESH_TOOL: &str = softwake_ctl::SOFTWAKE_REFRESH_TOOL;

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
        description: "Email tool: draft or send one message (to, subject, body).",
    },
    ToolMeta {
        name: EMAIL_LIST_TOOL,
        risk: ToolRisk::Confirm,
        description: "List recent inbox messages (Gmail or Microsoft Graph via Email OAuth).",
    },
    ToolMeta {
        name: EMAIL_SEARCH_TOOL,
        risk: ToolRisk::Confirm,
        description: "Search inbox (Gmail q or Graph search) via Email OAuth.",
    },
    ToolMeta {
        name: EMAIL_GET_TOOL,
        risk: ToolRisk::Confirm,
        description: "Get one inbox message by id via Email OAuth.",
    },
    ToolMeta {
        name: CALENDAR_LIST_TOOL,
        risk: ToolRisk::Confirm,
        description: "List upcoming calendar events via Email OAuth.",
    },
    ToolMeta {
        name: CALENDAR_GET_TOOL,
        risk: ToolRisk::Confirm,
        description: "Get one calendar event by id via Email OAuth.",
    },
    ToolMeta {
        name: DRIVE_LIST_TOOL,
        risk: ToolRisk::Confirm,
        description: "List Drive/AppFolder files visible under Email OAuth scopes.",
    },
    ToolMeta {
        name: DRIVE_SEARCH_TOOL,
        risk: ToolRisk::Confirm,
        description: "Search Drive/AppFolder files via Email OAuth.",
    },
    ToolMeta {
        name: DRIVE_GET_TOOL,
        risk: ToolRisk::Confirm,
        description: "Get Drive file metadata; optional cheap text body via Email OAuth.",
    },
    ToolMeta {
        name: SHELL_TOOL,
        risk: ToolRisk::Confirm,
        description: "Run a shell command after confirm. Off until enabled in Tools Settings.",
    },
    ToolMeta {
        name: SKILL_SAVE_TOOL,
        risk: ToolRisk::Confirm,
        description: "Skills tool: save a Markdown skill (procedure / pitfalls / verify) after confirm.",
    },
    ToolMeta {
        name: SKILL_LIST_TOOL,
        risk: ToolRisk::Confirm,
        description: "List saved skills (id and title).",
    },
    ToolMeta {
        name: SKILL_GET_TOOL,
        risk: ToolRisk::Confirm,
        description: "Get one saved skill by id (procedure, pitfalls, verify).",
    },
    ToolMeta {
        name: SCHEDULE_TOOL,
        risk: ToolRisk::Confirm,
        description: "Timers/schedule tool: create, edit, delete, or list a per-profile timer/reminder/cron after confirm.",
    },
    ToolMeta {
        name: SOFTWAKE_STATUS_TOOL,
        risk: ToolRisk::Confirm,
        description: "Softwake status: voice state, profile, model, context meter (mirrors /status).",
    },
    ToolMeta {
        name: SOFTWAKE_LIST_MODELS_TOOL,
        risk: ToolRisk::Confirm,
        description: "List configured chat and voice/STT models (mirrors /model).",
    },
    ToolMeta {
        name: SOFTWAKE_LIST_VOICES_TOOL,
        risk: ToolRisk::Confirm,
        description: "List TTS voices for the selected provider (mirrors /voice list).",
    },
    ToolMeta {
        name: SOFTWAKE_LIST_PROFILES_TOOL,
        risk: ToolRisk::Confirm,
        description: "List Softwake profiles (mirrors /profile).",
    },
    ToolMeta {
        name: SOFTWAKE_SET_MODEL_TOOL,
        risk: ToolRisk::Confirm,
        description: "Set chat (ai) or voice/STT model id after confirm (mirrors /model ai|voice).",
    },
    ToolMeta {
        name: SOFTWAKE_SET_VOICE_TOOL,
        risk: ToolRisk::Confirm,
        description: "Set TTS voice id after confirm (mirrors /voice <id>).",
    },
    ToolMeta {
        name: SOFTWAKE_SET_PROFILE_TOOL,
        risk: ToolRisk::Confirm,
        description: "Switch active Softwake profile after confirm (mirrors /profile <name>).",
    },
    ToolMeta {
        name: SOFTWAKE_SLEEP_TOOL,
        risk: ToolRisk::Confirm,
        description: "Enter sleep after confirm (mirrors /sleep).",
    },
    ToolMeta {
        name: SOFTWAKE_HIBERNATE_TOOL,
        risk: ToolRisk::Confirm,
        description: "Enter hibernate after confirm (mirrors /hibernate).",
    },
    ToolMeta {
        name: SOFTWAKE_RESUME_TOOL,
        risk: ToolRisk::Confirm,
        description: "Resume from hibernate to sleep after confirm (mirrors /resume).",
    },
    ToolMeta {
        name: SOFTWAKE_NEW_SESSION_TOOL,
        risk: ToolRisk::Confirm,
        description: "Reload soul pack and open a fresh awake session after confirm (mirrors /new).",
    },
    ToolMeta {
        name: SOFTWAKE_REFRESH_TOOL,
        risk: ToolRisk::Confirm,
        description: "Full refresh: reload active profile soul, clear model session, reseed HUD, rediscover MCP (mirrors /refresh).",
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
                if name == SKILL_LIST_TOOL {
                    parse_skill_list_args(args)?;
                }
                if name == SKILL_GET_TOOL {
                    parse_skill_get_args(args)?;
                }
                if name == SCHEDULE_TOOL {
                    parse_schedule_args(args).map_err(|_| ToolError::InvalidArgs {
                        name: SCHEDULE_TOOL.to_owned(),
                    })?;
                }
                if name == EMAIL_LIST_TOOL {
                    parse_email_list_args(args)?;
                }
                if name == EMAIL_SEARCH_TOOL {
                    parse_email_search_args(args)?;
                }
                if name == EMAIL_GET_TOOL {
                    parse_email_get_args(args)?;
                }
                if name == CALENDAR_LIST_TOOL {
                    parse_calendar_list_args(args)?;
                }
                if name == CALENDAR_GET_TOOL {
                    parse_calendar_get_args(args)?;
                }
                if name == DRIVE_LIST_TOOL {
                    parse_drive_list_args(args)?;
                }
                if name == DRIVE_SEARCH_TOOL {
                    parse_drive_search_args(args)?;
                }
                if name == DRIVE_GET_TOOL {
                    parse_drive_get_args(args)?;
                }
                if softwake_ctl::is_softwake_ctl(name) {
                    softwake_ctl::parse_softwake_ctl(name, args)?;
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
        SCHEDULE_TOOL => match parse_schedule_args(args) {
            Ok(action) => format!("schedule {action:?}"),
            Err(error) => error.to_string(),
        },
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

/// `skill_list` takes no arguments.
///
/// # Errors
///
/// Unexpected args.
pub fn parse_skill_list_args(args: &[String]) -> Result<(), ToolError> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(ToolError::InvalidArgs {
            name: SKILL_LIST_TOOL.to_owned(),
        })
    }
}

/// Parsed `skill_get` args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillGetArgs {
    /// Skill id (filename stem).
    pub id: String,
}

/// Parse `skill_get`: id.
///
/// # Errors
///
/// Missing id.
pub fn parse_skill_get_args(args: &[String]) -> Result<SkillGetArgs, ToolError> {
    match args {
        [id] if !id.trim().is_empty() => Ok(SkillGetArgs { id: id.clone() }),
        _ => Err(ToolError::InvalidArgs {
            name: SKILL_GET_TOOL.to_owned(),
        }),
    }
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
        CALENDAR_GET_TOOL, CALENDAR_LIST_TOOL, DRIVE_GET_TOOL, DRIVE_LIST_TOOL, DRIVE_SEARCH_TOOL,
        ECHO_TOOL, EMAIL_GET_TOOL, EMAIL_LIST_TOOL, EMAIL_SEARCH_TOOL, EMAIL_SEND_TOOL,
        EmailSendArgs, NOTIFY_TOOL, SCHEDULE_TOOL, SHELL_TOOL, SKILL_GET_TOOL, SKILL_LIST_TOOL,
        SKILL_SAVE_TOOL, SOFTWAKE_HIBERNATE_TOOL, SOFTWAKE_LIST_MODELS_TOOL,
        SOFTWAKE_LIST_PROFILES_TOOL, SOFTWAKE_LIST_VOICES_TOOL, SOFTWAKE_NEW_SESSION_TOOL,
        SOFTWAKE_REFRESH_TOOL, SOFTWAKE_RESUME_TOOL, SOFTWAKE_SET_MODEL_TOOL,
        SOFTWAKE_SET_PROFILE_TOOL, SOFTWAKE_SET_VOICE_TOOL, SOFTWAKE_SLEEP_TOOL,
        SOFTWAKE_STATUS_TOOL, ToolError, ToolRegistry, ToolResult, ToolRisk, parse_email_send_args,
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
                (EMAIL_LIST_TOOL, ToolRisk::Confirm),
                (EMAIL_SEARCH_TOOL, ToolRisk::Confirm),
                (EMAIL_GET_TOOL, ToolRisk::Confirm),
                (CALENDAR_LIST_TOOL, ToolRisk::Confirm),
                (CALENDAR_GET_TOOL, ToolRisk::Confirm),
                (DRIVE_LIST_TOOL, ToolRisk::Confirm),
                (DRIVE_SEARCH_TOOL, ToolRisk::Confirm),
                (DRIVE_GET_TOOL, ToolRisk::Confirm),
                (SHELL_TOOL, ToolRisk::Confirm),
                (SKILL_SAVE_TOOL, ToolRisk::Confirm),
                (SKILL_LIST_TOOL, ToolRisk::Confirm),
                (SKILL_GET_TOOL, ToolRisk::Confirm),
                (SCHEDULE_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_STATUS_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_LIST_MODELS_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_LIST_VOICES_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_LIST_PROFILES_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_SET_MODEL_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_SET_VOICE_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_SET_PROFILE_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_SLEEP_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_HIBERNATE_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_RESUME_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_NEW_SESSION_TOOL, ToolRisk::Confirm),
                (SOFTWAKE_REFRESH_TOOL, ToolRisk::Confirm),
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
            tool.risk == ToolRisk::Confirm && tool.description.contains("Email tool:")
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
