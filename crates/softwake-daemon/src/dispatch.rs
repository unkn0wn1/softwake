//! Gate a tool call on the voice state, then on the policy engine.
//!
//! [`softwake_state::Machine::permit_tool_dispatch`] runs first. Sleep and
//! hibernate never reach the engine. While awake, [`PolicyEngine::evaluate`]
//! classifies the name. [`PolicyDecision::Safe`] runs immediately,
//! [`PolicyDecision::Confirm`] becomes one pending confirmation, and
//! [`PolicyDecision::Deny`] is refused. A denied name that is not registered
//! is reported as unknown. The confirm-gated tool does not run, and the
//! notification sink does not change, until [`Hands::confirm`]. `email_send`
//! uses that same confirmation. The outbox changes only after the engine
//! allows `email` / `send` and [`crate::email_tool::commit_email_send`]
//! accepts the message.
//!
//! One pending confirmation at a time. A second confirm-gated request is
//! rejected and leaves the first in place. [`Hands::cancel`] clears it in any
//! voice state. [`Hands::confirm`] is refused when the machine is not awake.
//! Sleep and hibernate should call [`Hands::clear_pending`].

use std::collections::VecDeque;

use softwake_connectors::{
    ConnectorRegistry, EMAIL, EMAIL_SEND, EmailBackend, EmailSettings, FileEmailSettings,
    OutboundEmail, resolve_email_file,
};
use softwake_policy::{PolicyDecision, PolicyEngine, Subject, permits_confirmed_connector};
use softwake_soul::Glossary;
use softwake_state::{Machine, StateError, VoiceState};
use softwake_tools::{
    EMAIL_SEND_TOOL, FileToolsSettings, NOTIFY_TOOL, SHELL_TOOL, ToolError, ToolRegistry,
    ToolResult, ToolRisk, ToolsSettings, format_shell_output, parse_email_send_args,
    resolve_tools_file, run_shell,
};

use crate::email_tool::{commit_detail, commit_email_send};

/// How many tool-log entries the runtime keeps.
const LOG_CAP: usize = 64;

/// How many notification lines the in-memory sink keeps.
const SINK_CAP: usize = 64;

/// Why a tool call, confirm, or cancel did not do what the caller asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DispatchError {
    /// The daemon is not awake, so the tool was not considered.
    #[error("cannot run {name} while {state}")]
    Forbidden {
        /// Tool the caller named.
        name: String,
        /// State that refused the call.
        state: VoiceState,
    },

    /// The name is not registered.
    #[error("unknown tool: {name}")]
    Unknown {
        /// Name that was rejected.
        name: String,
    },

    /// The name is registered as deny.
    #[error("tool denied: {name}")]
    Denied {
        /// Name that was rejected.
        name: String,
    },

    /// A confirm-gated tool is already waiting.
    #[error("a confirmation is already pending: {pending_id}")]
    Busy {
        /// Id of the confirmation that is already waiting.
        pending_id: String,
    },

    /// No pending confirmation has this id.
    #[error("unknown pending confirmation: {pending_id}")]
    UnknownPending {
        /// Id the caller sent.
        pending_id: String,
    },

    /// The optional name did not match the pending tool.
    #[error("pending confirmation {pending_id} is not {name}")]
    PendingMismatch {
        /// Id the caller sent.
        pending_id: String,
        /// Name the caller expected.
        name: String,
    },

    /// Confirm was refused because the machine is not awake.
    ///
    /// The pending record is left in place.
    #[error("cannot confirm {pending_id} while {state}")]
    ConfirmForbidden {
        /// Id the caller sent.
        pending_id: String,
        /// State that refused the confirm.
        state: VoiceState,
    },

    /// `email_send` did not include to, subject, and body.
    ///
    /// No pending record is stored when this happens on a request. On confirm,
    /// the existing record stays and the outbox is unchanged.
    #[error("{name} needs to, subject, and body")]
    InvalidArgs {
        /// Tool the caller named.
        name: String,
    },

    /// The connector refused the send after the tool confirmation was accepted.
    ///
    /// The pending record stays. The outbox is unchanged.
    #[error("{message}")]
    Connector {
        /// Registry text for the refused pair.
        message: String,
    },

    /// Shell did not start or returned an operator-safe failure before spawn completed.
    ///
    /// The pending record stays when this happens on confirm.
    #[error("{message}")]
    Shell {
        /// Operator-facing error (no command line, no secrets).
        message: String,
    },
}

/// What happened to one tool attempt, for the log and for status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolOutcome {
    /// A safe tool ran.
    Ran,
    /// A confirm-gated tool is waiting, or a second request was refused.
    Pending,
    /// A pending tool was confirmed and ran.
    Confirmed,
    /// A pending tool was cleared without running.
    Cancelled,
    /// A denied name was refused while awake.
    Denied,
    /// The voice state refused the call.
    Forbidden,
    /// The name or pending id was not found.
    Unknown,
}

impl ToolOutcome {
    /// Stable log spelling.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Ran => "ran",
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Cancelled => "cancelled",
            Self::Denied => "denied",
            Self::Forbidden => "forbidden",
            Self::Unknown => "unknown",
        }
    }
}

/// One structured tool-log entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolLogEntry {
    /// Monotonic counter, starting at 1.
    pub(crate) seq: u64,
    /// Tool name, or the pending id when the tool is not known.
    pub(crate) name: String,
    /// Registry risk, when the call was classified.
    pub(crate) risk: Option<ToolRisk>,
    /// What the gate did.
    pub(crate) outcome: ToolOutcome,
    /// Optional operator detail. Not a transcript of arguments.
    pub(crate) detail: Option<String>,
}

/// A safe tool ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RanTool {
    /// Tool name.
    pub(crate) name: String,
    /// Result text.
    pub(crate) detail: String,
}

/// A confirm-gated tool is waiting and has not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingToolCall {
    /// Id to confirm or cancel.
    pub(crate) pending_id: String,
    /// Tool name.
    pub(crate) name: String,
    /// Arguments stored until confirm or cancel.
    pub(crate) args: Vec<String>,
    /// Registry description.
    pub(crate) description: String,
}

/// Result of asking to run a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequestOutcome {
    /// A safe tool ran.
    Ran(RanTool),
    /// A confirm-gated tool is waiting.
    Pending(PendingToolCall),
}

/// A confirm-gated tool ran once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfirmedTool {
    /// Id that was consumed.
    pub(crate) pending_id: String,
    /// Tool name.
    pub(crate) name: String,
    /// Result text. For `notify`, this is also the sink line.
    pub(crate) detail: String,
}

/// A pending confirmation was cleared without running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CancelledTool {
    /// Id that was cleared.
    pub(crate) pending_id: String,
    /// Tool name.
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    id: String,
    name: String,
    args: Vec<String>,
    description: String,
}

/// Policy engine, registries, one pending confirmation, the notification sink, the email outbox, and the tool log.
#[derive(Debug)]
pub(crate) struct Hands {
    policy: PolicyEngine,
    registry: ToolRegistry,
    connectors: ConnectorRegistry,
    email: EmailBackend,
    glossary: Glossary,
    /// Test-only Tools Settings override. Production always reads disk.
    tools_settings_override: Option<ToolsSettings>,
    pending: Option<Pending>,
    sink: VecDeque<String>,
    log: VecDeque<ToolLogEntry>,
    next_seq: u64,
    next_pending: u64,
}

impl Hands {
    /// Empty sink, mock outbox, empty log, the builtin policy engine, and the builtin registries.
    ///
    /// Tests and default construction stay on [`EmailBackend::Mock`]. Serve and
    /// the typed demo call [`Hands::from_disk`] so an opted-in live Settings
    /// file can select the live scaffold.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::with_email(EmailBackend::default())
    }

    /// Like [`Hands::new`], using `email` as the backend.
    #[must_use]
    pub(crate) fn with_email(email: EmailBackend) -> Self {
        Self {
            policy: PolicyEngine::builtin(),
            registry: ToolRegistry::phase2(),
            connectors: ConnectorRegistry::phase3(),
            email,
            glossary: Glossary::parse("").expect("empty glossary"),
            tools_settings_override: None,
            pending: None,
            sink: VecDeque::new(),
            log: VecDeque::new(),
            next_seq: 0,
            next_pending: 0,
        }
    }

    /// Load non-secret email Settings and whether an SMTP password is saved.
    ///
    /// Missing Settings or secret bag errors fall back to the mock backend so
    /// the daemon still starts. Live email stays off unless Settings say so.
    #[must_use]
    pub(crate) fn from_disk() -> Self {
        Self::with_email(email_backend_from_disk())
    }

    /// Replace the glossary used for shell alias expansion.
    pub(crate) fn set_glossary(&mut self, glossary: Glossary) {
        self.glossary = glossary;
    }

    /// Glossary currently used for shell expansion.
    #[must_use]
    pub(crate) fn glossary(&self) -> &Glossary {
        &self.glossary
    }

    /// Override Tools Settings (tests). `None` restores disk loads.
    #[cfg(test)]
    pub(crate) fn set_tools_settings_for_test(&mut self, settings: Option<ToolsSettings>) {
        self.tools_settings_override = settings;
    }

    /// The confirmation that is waiting, if any.
    #[must_use]
    pub(crate) fn pending(&self) -> Option<PendingToolCall> {
        self.pending.as_ref().map(|pending| PendingToolCall {
            pending_id: pending.id.clone(),
            name: pending.name.clone(),
            args: pending.args.clone(),
            description: pending.description.clone(),
        })
    }

    /// Id of the confirmation that is waiting.
    #[must_use]
    pub(crate) fn pending_id(&self) -> Option<String> {
        self.pending.as_ref().map(|pending| pending.id.clone())
    }

    /// Notification lines, oldest first.
    #[must_use]
    pub(crate) fn notifications(&self) -> &VecDeque<String> {
        &self.sink
    }

    /// Messages accepted by the email backend (mock outbox or live drafts), oldest first.
    #[must_use]
    pub(crate) fn outbox(&self) -> &[OutboundEmail] {
        self.email.messages()
    }

    /// Tool log, oldest first. At most [`LOG_CAP`] entries.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn log(&self) -> &VecDeque<ToolLogEntry> {
        &self.log
    }

    /// `name risk outcome` for the latest log entry.
    #[must_use]
    pub(crate) fn last_tool_line(&self) -> Option<String> {
        let entry = self.log.back()?;
        Some(match entry.risk {
            Some(risk) => format!("{} {risk} {}", entry.name, entry.outcome.as_str()),
            None => format!("{} {}", entry.name, entry.outcome.as_str()),
        })
    }

    /// Run a safe tool, or stage a confirm-gated tool.
    ///
    /// Classification comes from [`PolicyEngine::evaluate`]. A confirm-gated
    /// tool does not run. The sink and the outbox stay as they are.
    pub(crate) fn request(
        &mut self,
        machine: &Machine,
        name: &str,
        args: &[String],
    ) -> Result<RequestOutcome, DispatchError> {
        if let Some(state) = forbidden_state(machine) {
            self.record(name, None, ToolOutcome::Forbidden, None);
            return Err(DispatchError::Forbidden {
                name: name.to_owned(),
                state,
            });
        }
        match self.policy.evaluate(&Subject::Tool { name }) {
            PolicyDecision::Deny => {
                if self.registry.lookup(name).is_none() {
                    return Err(self.unknown_tool(name));
                }
                self.record(name, Some(ToolRisk::Deny), ToolOutcome::Denied, None);
                Err(DispatchError::Denied {
                    name: name.to_owned(),
                })
            }
            PolicyDecision::Safe => {
                if self.registry.lookup(name).is_none() {
                    return Err(self.unknown_tool(name));
                }
                match self.registry.invoke(name, args) {
                    Ok(ToolResult { detail }) => {
                        self.record(
                            name,
                            Some(ToolRisk::Safe),
                            ToolOutcome::Ran,
                            Some(detail.clone()),
                        );
                        Ok(RequestOutcome::Ran(RanTool {
                            name: name.to_owned(),
                            detail,
                        }))
                    }
                    Err(error) => Err(self.fail_tool(error)),
                }
            }
            PolicyDecision::Confirm => {
                let Some(meta) = self.registry.lookup(name) else {
                    return Err(self.unknown_tool(name));
                };
                if let Some(pending_id) = self.pending.as_ref().map(|pending| pending.id.clone()) {
                    self.record(
                        name,
                        Some(ToolRisk::Confirm),
                        ToolOutcome::Pending,
                        Some(format!("busy: {pending_id}")),
                    );
                    return Err(DispatchError::Busy { pending_id });
                }
                if name == SHELL_TOOL {
                    return self.request_shell(args);
                }
                if name == EMAIL_SEND_TOOL {
                    if let Err(error) = parse_email_send_args(args) {
                        return Err(self.fail_tool(error));
                    }
                }
                let description = meta.description.to_owned();
                self.next_pending = self.next_pending.saturating_add(1);
                let pending_id = self.next_pending.to_string();
                self.pending = Some(Pending {
                    id: pending_id.clone(),
                    name: name.to_owned(),
                    args: args.to_vec(),
                    description: description.clone(),
                });
                self.record(name, Some(ToolRisk::Confirm), ToolOutcome::Pending, None);
                Ok(RequestOutcome::Pending(PendingToolCall {
                    pending_id,
                    name: name.to_owned(),
                    args: args.to_vec(),
                    description,
                }))
            }
        }
    }

    /// Run the pending tool once when `pending_id` matches and the machine is awake.
    ///
    /// `expected_name`, when set, must match the stored tool. A mismatch leaves
    /// the pending record in place. The sink changes only after a successful
    /// `notify` run. The outbox changes only after a successful `email_send`.
    pub(crate) fn confirm(
        &mut self,
        machine: &Machine,
        pending_id: &str,
        expected_name: Option<&str>,
    ) -> Result<ConfirmedTool, DispatchError> {
        let Some(pending) = self.pending.clone() else {
            self.record(
                pending_id,
                None,
                ToolOutcome::Unknown,
                Some("unknown pending".to_owned()),
            );
            return Err(DispatchError::UnknownPending {
                pending_id: pending_id.to_owned(),
            });
        };
        if pending.id != pending_id {
            self.record(
                pending_id,
                None,
                ToolOutcome::Unknown,
                Some("unknown pending".to_owned()),
            );
            return Err(DispatchError::UnknownPending {
                pending_id: pending_id.to_owned(),
            });
        }
        if let Some(expected) = expected_name {
            if expected != pending.name {
                self.record(
                    &pending.name,
                    Some(ToolRisk::Confirm),
                    ToolOutcome::Unknown,
                    Some(format!("name mismatch: {expected}")),
                );
                return Err(DispatchError::PendingMismatch {
                    pending_id: pending.id,
                    name: expected.to_owned(),
                });
            }
        }
        if let Some(state) = forbidden_state(machine) {
            self.record(
                &pending.name,
                Some(ToolRisk::Confirm),
                ToolOutcome::Forbidden,
                None,
            );
            return Err(DispatchError::ConfirmForbidden {
                pending_id: pending.id,
                state,
            });
        }
        let detail = self.confirmed_detail(&pending)?;
        self.pending = None;
        self.record(
            &pending.name,
            Some(ToolRisk::Confirm),
            ToolOutcome::Confirmed,
            Some(detail.clone()),
        );
        Ok(ConfirmedTool {
            pending_id: pending.id,
            name: pending.name,
            detail,
        })
    }

    /// Clear a matching pending confirmation without running the tool.
    ///
    /// Voice state is not consulted. A name mismatch leaves the record in place.
    pub(crate) fn cancel(
        &mut self,
        pending_id: &str,
        expected_name: Option<&str>,
    ) -> Result<CancelledTool, DispatchError> {
        let Some(pending) = self.pending.clone() else {
            self.record(
                pending_id,
                None,
                ToolOutcome::Unknown,
                Some("unknown pending".to_owned()),
            );
            return Err(DispatchError::UnknownPending {
                pending_id: pending_id.to_owned(),
            });
        };
        if pending.id != pending_id {
            self.record(
                pending_id,
                None,
                ToolOutcome::Unknown,
                Some("unknown pending".to_owned()),
            );
            return Err(DispatchError::UnknownPending {
                pending_id: pending_id.to_owned(),
            });
        }
        if let Some(expected) = expected_name {
            if expected != pending.name {
                self.record(
                    &pending.name,
                    Some(ToolRisk::Confirm),
                    ToolOutcome::Unknown,
                    Some(format!("name mismatch: {expected}")),
                );
                return Err(DispatchError::PendingMismatch {
                    pending_id: pending.id,
                    name: expected.to_owned(),
                });
            }
        }
        self.pending = None;
        self.record(
            &pending.name,
            Some(ToolRisk::Confirm),
            ToolOutcome::Cancelled,
            None,
        );
        Ok(CancelledTool {
            pending_id: pending.id,
            name: pending.name,
        })
    }

    /// Drop a pending confirmation because acting stopped.
    ///
    /// Returns the cleared record when one was waiting. The sink is unchanged.
    pub(crate) fn clear_pending(&mut self) -> Option<CancelledTool> {
        let pending = self.pending.take()?;
        self.record(
            &pending.name,
            Some(ToolRisk::Confirm),
            ToolOutcome::Cancelled,
            Some("cleared when acting stopped".to_owned()),
        );
        Some(CancelledTool {
            pending_id: pending.id,
            name: pending.name,
        })
    }

    /// Result text for a confirmation that already passed the awake check.
    ///
    /// A failure leaves `self.pending` in place. `email_send` appends only
    /// after the policy engine allows `email` / `send`, `invoke_confirmed`
    /// succeeds, and `authorize_confirmed` succeeds.
    fn confirmed_detail(&mut self, pending: &Pending) -> Result<String, DispatchError> {
        if pending.name == EMAIL_SEND_TOOL {
            let parsed = match parse_email_send_args(&pending.args) {
                Ok(parsed) => parsed,
                Err(error) => return Err(self.fail_tool(error)),
            };
            if let Err(error) = self.registry.invoke_confirmed(&pending.name, &pending.args) {
                return Err(self.fail_tool(error));
            }
            let message = OutboundEmail {
                to: parsed.to,
                subject: parsed.subject,
                body: parsed.body,
            };
            let decision = self.policy.evaluate(&Subject::Connector {
                connector: EMAIL,
                action: EMAIL_SEND,
            });
            if !permits_confirmed_connector(decision) {
                let denied = format!("connector action denied: {EMAIL}/{EMAIL_SEND}");
                self.record(
                    &pending.name,
                    Some(ToolRisk::Confirm),
                    ToolOutcome::Unknown,
                    Some(denied.clone()),
                );
                return Err(DispatchError::Connector { message: denied });
            }
            let receipt = match commit_email_send(
                &self.connectors,
                &mut self.email,
                EMAIL,
                EMAIL_SEND,
                &message,
            ) {
                Ok(receipt) => receipt,
                Err(error) => {
                    let message = error.to_string();
                    self.record(
                        &pending.name,
                        Some(ToolRisk::Confirm),
                        ToolOutcome::Unknown,
                        Some(message.clone()),
                    );
                    return Err(DispatchError::Connector { message });
                }
            };
            return Ok(commit_detail(&self.email, receipt));
        }
        if pending.name == SHELL_TOOL {
            return self.run_confirmed_shell(pending);
        }
        let detail = match self.registry.invoke_confirmed(&pending.name, &pending.args) {
            Ok(ToolResult { detail }) => detail,
            Err(error) => return Err(self.fail_tool(error)),
        };
        if pending.name == NOTIFY_TOOL {
            self.push_notification(detail.clone());
        }
        Ok(detail)
    }

    /// Stage or auto-run shell after Tools Settings + glossary expand + confirm policy.
    fn request_shell(&mut self, args: &[String]) -> Result<RequestOutcome, DispatchError> {
        let settings = load_tools_settings(self.tools_settings_override.as_ref());
        if !settings.shell_enabled {
            self.record(
                SHELL_TOOL,
                Some(ToolRisk::Deny),
                ToolOutcome::Denied,
                Some("shell disabled in Tools Settings".to_owned()),
            );
            return Err(DispatchError::Denied {
                name: SHELL_TOOL.to_owned(),
            });
        }
        let original = args.join(" ");
        if original.trim().is_empty() {
            return Err(DispatchError::InvalidArgs {
                name: SHELL_TOOL.to_owned(),
            });
        }
        let echo = self.glossary.confirm_echo(&original);
        let expanded_args = vec![echo.expanded.clone()];
        if settings.confirm_policy.must_confirm(echo.requires_readback) {
            let description = echo.readback.trim_end().to_owned();
            self.next_pending = self.next_pending.saturating_add(1);
            let pending_id = self.next_pending.to_string();
            self.pending = Some(Pending {
                id: pending_id.clone(),
                name: SHELL_TOOL.to_owned(),
                args: expanded_args.clone(),
                description: description.clone(),
            });
            self.record(
                SHELL_TOOL,
                Some(ToolRisk::Confirm),
                ToolOutcome::Pending,
                Some(echo.expanded),
            );
            return Ok(RequestOutcome::Pending(PendingToolCall {
                pending_id,
                name: SHELL_TOOL.to_owned(),
                args: expanded_args,
                description,
            }));
        }
        // Quiet path (mutating_only / allowlisted_quiet with no readback).
        match run_shell(&echo.expanded) {
            Ok(output) => {
                let detail = format_shell_output(&output);
                self.record(
                    SHELL_TOOL,
                    Some(ToolRisk::Confirm),
                    ToolOutcome::Ran,
                    Some(detail.clone()),
                );
                Ok(RequestOutcome::Ran(RanTool {
                    name: SHELL_TOOL.to_owned(),
                    detail,
                }))
            }
            Err(error) => {
                let message = error.to_string();
                self.record(
                    SHELL_TOOL,
                    Some(ToolRisk::Confirm),
                    ToolOutcome::Unknown,
                    Some(message.clone()),
                );
                Err(DispatchError::Shell { message })
            }
        }
    }

    fn run_confirmed_shell(&mut self, pending: &Pending) -> Result<String, DispatchError> {
        if let Err(error) = self.registry.invoke_confirmed(&pending.name, &pending.args) {
            return Err(self.fail_tool(error));
        }
        let command = pending.args.join(" ");
        match run_shell(&command) {
            Ok(output) => Ok(format_shell_output(&output)),
            Err(error) => {
                let message = error.to_string();
                self.record(
                    SHELL_TOOL,
                    Some(ToolRisk::Confirm),
                    ToolOutcome::Unknown,
                    Some(message.clone()),
                );
                Err(DispatchError::Shell { message })
            }
        }
    }

    fn fail_tool(&mut self, error: ToolError) -> DispatchError {
        let (recorded, risk, outcome, detail, dispatch) = match error {
            ToolError::Denied { name } => (
                name.clone(),
                Some(ToolRisk::Deny),
                ToolOutcome::Denied,
                None,
                DispatchError::Denied { name },
            ),
            ToolError::InvalidArgs { name } => (
                name.clone(),
                Some(ToolRisk::Confirm),
                ToolOutcome::Unknown,
                Some("needs to, subject, and body".to_owned()),
                DispatchError::InvalidArgs { name },
            ),
            ToolError::Unknown { name }
            | ToolError::NeedsConfirm { name }
            | ToolError::NotConfirmGated { name } => (
                name.clone(),
                None,
                ToolOutcome::Unknown,
                None,
                DispatchError::Unknown { name },
            ),
        };
        self.record(&recorded, risk, outcome, detail);
        dispatch
    }

    fn push_notification(&mut self, line: String) {
        if self.sink.len() == SINK_CAP {
            self.sink.pop_front();
        }
        self.sink.push_back(line);
    }

    fn unknown_tool(&mut self, name: &str) -> DispatchError {
        self.record(name, None, ToolOutcome::Unknown, None);
        DispatchError::Unknown {
            name: name.to_owned(),
        }
    }

    fn record(
        &mut self,
        name: &str,
        risk: Option<ToolRisk>,
        outcome: ToolOutcome,
        detail: Option<String>,
    ) {
        self.next_seq = self.next_seq.saturating_add(1);
        if self.log.len() == LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(ToolLogEntry {
            seq: self.next_seq,
            name: name.to_owned(),
            risk,
            outcome,
            detail,
        });
    }
}

fn load_tools_settings(override_settings: Option<&ToolsSettings>) -> ToolsSettings {
    if let Some(settings) = override_settings {
        return settings.clone();
    }
    match resolve_tools_file().and_then(FileToolsSettings::new) {
        Ok(store) => store.load().unwrap_or_default(),
        Err(_) => ToolsSettings::default(),
    }
}

fn email_backend_from_disk() -> EmailBackend {
    let settings = match resolve_email_file().and_then(FileEmailSettings::new) {
        Ok(store) => store.load().unwrap_or_default(),
        Err(_) => EmailSettings::default(),
    };
    let password_present = softwake_providers::resolve_secrets_file()
        .ok()
        .and_then(|path| softwake_providers::open_store(&path).ok())
        .and_then(|store| store.load().ok())
        .is_some_and(|bag| {
            bag.email_smtp_password
                .as_ref()
                .is_some_and(|password| !password.is_empty())
        });
    EmailBackend::from_settings(settings, password_present)
}

impl Default for Hands {
    fn default() -> Self {
        Self::new()
    }
}

fn forbidden_state(machine: &Machine) -> Option<VoiceState> {
    match machine.permit_tool_dispatch() {
        Ok(()) => None,
        Err(StateError::ToolsForbidden { state }) => Some(state),
        Err(_) => Some(machine.state()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{DispatchError, Hands, RequestOutcome, ToolOutcome};
    use softwake_state::{CooldownConfig, Event, Machine, VoiceState};
    use softwake_tools::ToolRisk;

    fn awake() -> Machine {
        let mut machine = Machine::new(CooldownConfig {
            post_wake: Duration::ZERO,
            post_sleep: Duration::ZERO,
        });
        machine.apply(Event::WakePhrase).expect("awake");
        machine
    }

    fn asleep() -> Machine {
        Machine::new(CooldownConfig::default())
    }

    #[test]
    fn echo_runs_while_awake_and_is_refused_asleep() {
        let mut hands = Hands::new();
        let machine = asleep();
        let refused = hands
            .request(&machine, "echo", &["hello".to_owned()])
            .expect_err("asleep");
        assert_eq!(
            refused,
            DispatchError::Forbidden {
                name: "echo".to_owned(),
                state: VoiceState::Sleep,
            }
        );
        assert!(hands.notifications().is_empty());

        let machine = awake();
        let ran = hands
            .request(&machine, "echo", &["hello".to_owned()])
            .expect("echo");
        match ran {
            RequestOutcome::Ran(ran) => {
                assert_eq!(ran.name, "echo");
                assert_eq!(ran.detail, "echo: hello");
            }
            RequestOutcome::Pending(_) => panic!("echo is safe"),
        }
        assert_eq!(hands.last_tool_line().as_deref(), Some("echo safe ran"));
        let entry = hands.log().back().expect("log");
        assert_eq!(entry.outcome, ToolOutcome::Ran);
        assert_eq!(entry.risk, Some(ToolRisk::Safe));
        assert_eq!(entry.detail.as_deref(), Some("echo: hello"));
    }

    #[test]
    fn notify_waits_and_confirm_appends_once() {
        let mut hands = Hands::new();
        let machine = awake();
        let pending = hands
            .request(&machine, "notify", &["hello".to_owned()])
            .expect("pending");
        let RequestOutcome::Pending(pending) = pending else {
            panic!("notify is confirm-gated");
        };
        assert_eq!(pending.pending_id, "1");
        assert_eq!(pending.name, "notify");
        assert!(hands.notifications().is_empty());
        assert_eq!(
            hands.last_tool_line().as_deref(),
            Some("notify confirm pending")
        );

        let busy = hands
            .request(&machine, "notify", &["other".to_owned()])
            .expect_err("busy");
        assert_eq!(
            busy,
            DispatchError::Busy {
                pending_id: "1".to_owned()
            }
        );
        assert!(hands.notifications().is_empty());
        assert_eq!(hands.pending_id().as_deref(), Some("1"));

        let echo = hands
            .request(&machine, "echo", &[])
            .expect("echo still runs");
        assert!(matches!(echo, RequestOutcome::Ran(_)));

        let confirmed = hands
            .confirm(&machine, "1", Some("notify"))
            .expect("confirm");
        assert_eq!(confirmed.detail, "hello");
        assert_eq!(hands.notifications().iter().collect::<Vec<_>>(), ["hello"]);
        assert!(hands.pending().is_none());
        assert_eq!(
            hands.last_tool_line().as_deref(),
            Some("notify confirm confirmed")
        );

        let again = hands.confirm(&machine, "1", None).expect_err("spent");
        assert_eq!(
            again,
            DispatchError::UnknownPending {
                pending_id: "1".to_owned()
            }
        );
        assert_eq!(hands.notifications().len(), 1);
    }

    #[test]
    fn cancel_leaves_the_sink_unchanged_in_any_state() {
        let mut hands = Hands::new();
        let machine = awake();
        hands
            .request(&machine, "notify", &["hello".to_owned()])
            .expect("pending");
        let mut asleep = machine;
        asleep.apply(Event::SleepPhrase).expect("sleep");
        let cancelled = hands.cancel("1", None).expect("cancel while asleep");
        assert_eq!(cancelled.name, "notify");
        assert!(hands.notifications().is_empty());
        assert!(hands.pending().is_none());
        assert_eq!(
            hands.log().back().expect("log").outcome,
            ToolOutcome::Cancelled
        );
        let missing = hands.cancel("1", None).expect_err("gone");
        assert!(matches!(missing, DispatchError::UnknownPending { .. }));
        assert!(hands.notifications().is_empty());
    }

    #[test]
    fn confirm_is_refused_when_no_longer_awake_and_cancel_still_clears() {
        let mut hands = Hands::new();
        let machine = awake();
        hands
            .request(&machine, "notify", &["hello".to_owned()])
            .expect("pending");
        let mut asleep = machine;
        asleep.apply(Event::SleepPhrase).expect("sleep");
        let refused = hands.confirm(&asleep, "1", None).expect_err("asleep");
        assert_eq!(
            refused,
            DispatchError::ConfirmForbidden {
                pending_id: "1".to_owned(),
                state: VoiceState::Sleep,
            }
        );
        assert!(hands.notifications().is_empty());
        assert!(hands.pending().is_some());
        hands.cancel("1", None).expect("cancel");
        assert!(hands.notifications().is_empty());
    }

    #[test]
    fn name_mismatch_does_not_run_or_clear() {
        let mut hands = Hands::new();
        let machine = awake();
        hands
            .request(&machine, "notify", &["hello".to_owned()])
            .expect("pending");
        let mismatch = hands
            .confirm(&machine, "1", Some("echo"))
            .expect_err("name");
        assert_eq!(
            mismatch,
            DispatchError::PendingMismatch {
                pending_id: "1".to_owned(),
                name: "echo".to_owned(),
            }
        );
        assert!(hands.notifications().is_empty());
        assert!(hands.pending().is_some());
        hands.cancel("1", Some("nope")).expect_err("cancel name");
        assert!(hands.pending().is_some());
        hands.cancel("1", Some("notify")).expect("cancel");
        assert!(hands.notifications().is_empty());
    }

    #[test]
    fn shell_never_runs_and_unknown_is_rejected_only_while_awake() {
        let mut hands = Hands::new();
        let machine = awake();
        assert_eq!(
            hands.request(&machine, "shell", &["echo".to_owned()]),
            Err(DispatchError::Denied {
                name: "shell".to_owned()
            })
        );
        assert!(hands.pending().is_none());
        assert!(hands.notifications().is_empty());
        assert!(hands.outbox().is_empty());
        assert_eq!(hands.last_tool_line().as_deref(), Some("shell deny denied"));
        hands.confirm(&machine, "1", None).expect_err("no pending");
        assert!(hands.notifications().is_empty());
        assert!(hands.outbox().is_empty());

        assert_eq!(
            hands.request(&machine, "volume", &[]),
            Err(DispatchError::Unknown {
                name: "volume".to_owned()
            })
        );

        let asleep = asleep();
        assert!(matches!(
            hands.request(&asleep, "shell", &[]),
            Err(DispatchError::Forbidden {
                state: VoiceState::Sleep,
                ..
            })
        ));
        assert!(matches!(
            hands.request(&asleep, "volume", &[]),
            Err(DispatchError::Forbidden { .. })
        ));
    }

    #[test]
    fn shell_enabled_expands_glossary_and_stages_confirm() {
        let mut hands = Hands::new();
        let settings = softwake_tools::ToolsSettings {
            shell_enabled: true,
            confirm_policy: softwake_tools::ConfirmPolicy::Always,
            ..softwake_tools::ToolsSettings::default()
        };
        hands.set_tools_settings_for_test(Some(settings));
        hands.set_glossary(
            softwake_soul::Glossary::parse("aau → echo hello-tools\n").expect("glossary"),
        );
        let machine = awake();
        let outcome = hands
            .request(&machine, "shell", &["aau".to_owned()])
            .expect("pending");
        let RequestOutcome::Pending(pending) = outcome else {
            panic!("expected pending shell");
        };
        assert_eq!(pending.name, "shell");
        assert_eq!(pending.args, vec!["echo hello-tools".to_owned()]);
        assert!(pending.description.contains("echo hello-tools"));

        let confirmed = hands
            .confirm(&machine, &pending.pending_id, Some("shell"))
            .expect("confirm");
        assert_eq!(confirmed.name, "shell");
        assert!(confirmed.detail.contains("hello-tools"));
    }

    #[test]
    fn clear_pending_on_sleep_does_not_append() {
        let mut hands = Hands::new();
        let machine = awake();
        hands
            .request(
                &machine,
                "notify",
                &["hello".to_owned(), "there".to_owned()],
            )
            .expect("pending");
        let cleared = hands.clear_pending().expect("cleared");
        assert_eq!(cleared.pending_id, "1");
        assert!(hands.notifications().is_empty());
        assert!(hands.clear_pending().is_none());
        assert_eq!(
            hands.log().back().expect("log").detail.as_deref(),
            Some("cleared when acting stopped")
        );
    }

    #[test]
    fn log_and_sink_keep_the_newest_entries() {
        let mut hands = Hands::new();
        let machine = awake();
        for index in 0..70 {
            hands
                .request(&machine, "echo", &[index.to_string()])
                .expect("echo");
        }
        assert_eq!(hands.log().len(), 64);
        assert_eq!(hands.log().front().expect("oldest").seq, 7);
        assert_eq!(hands.log().back().expect("newest").seq, 70);

        for index in 0..70 {
            hands
                .request(&machine, "notify", &[index.to_string()])
                .expect("stage");
            hands
                .confirm(&machine, &(index + 1).to_string(), None)
                .expect("confirm");
        }
        assert_eq!(hands.notifications().len(), 64);
        assert_eq!(hands.notifications().front().map(String::as_str), Some("6"));
        assert_eq!(hands.notifications().back().map(String::as_str), Some("69"));
    }

    fn email_args(body: &[&str]) -> Vec<String> {
        let mut args = vec!["ada@example.com".to_owned(), "hello".to_owned()];
        args.extend(body.iter().map(|part| (*part).to_owned()));
        args
    }

    fn hibernating() -> Machine {
        let mut machine = asleep();
        machine.apply(Event::UiHibernate).expect("hibernate");
        machine
    }

    #[test]
    fn email_send_waits_and_confirm_appends_once() {
        let mut hands = Hands::new();
        let machine = awake();
        let args = email_args(&["a", "short", "note"]);
        let pending = hands
            .request(&machine, "email_send", &args)
            .expect("pending");
        let RequestOutcome::Pending(pending) = pending else {
            panic!("email_send is confirm-gated");
        };
        assert_eq!(pending.pending_id, "1");
        assert_eq!(pending.name, "email_send");
        assert_eq!(
            pending.description,
            "Append one message to the in-memory outbox."
        );
        assert!(hands.outbox().is_empty());
        assert!(hands.notifications().is_empty());
        assert_eq!(
            hands.last_tool_line().as_deref(),
            Some("email_send confirm pending")
        );

        let busy = hands
            .request(&machine, "email_send", &email_args(&["other"]))
            .expect_err("busy");
        assert_eq!(
            busy,
            DispatchError::Busy {
                pending_id: "1".to_owned()
            }
        );
        let notify_busy = hands
            .request(&machine, "notify", &["other".to_owned()])
            .expect_err("notify busy");
        assert!(matches!(notify_busy, DispatchError::Busy { .. }));
        assert!(hands.outbox().is_empty());

        let echo = hands
            .request(&machine, "echo", &[])
            .expect("echo still runs");
        assert!(matches!(echo, RequestOutcome::Ran(_)));
        assert!(hands.outbox().is_empty());

        let confirmed = hands
            .confirm(&machine, "1", Some("email_send"))
            .expect("confirm");
        assert_eq!(confirmed.detail, "sent 1");
        assert_eq!(hands.outbox().len(), 1);
        assert_eq!(hands.outbox()[0].to, "ada@example.com");
        assert_eq!(hands.outbox()[0].subject, "hello");
        assert_eq!(hands.outbox()[0].body, "a short note");
        assert!(hands.notifications().is_empty());
        assert!(hands.pending().is_none());
        assert_eq!(
            hands.last_tool_line().as_deref(),
            Some("email_send confirm confirmed")
        );

        let again = hands.confirm(&machine, "1", None).expect_err("spent");
        assert_eq!(
            again,
            DispatchError::UnknownPending {
                pending_id: "1".to_owned()
            }
        );
        assert_eq!(hands.outbox().len(), 1);
        assert!(hands.clear_pending().is_none());
        assert_eq!(hands.outbox().len(), 1);

        hands
            .request(&machine, "email_send", &email_args(&["second"]))
            .expect("second pending");
        let second = hands.confirm(&machine, "2", None).expect("second confirm");
        assert_eq!(second.detail, "sent 2");
        assert_eq!(hands.outbox().len(), 2);
        assert_eq!(hands.outbox()[1].body, "second");
    }

    #[test]
    fn short_email_send_does_not_stage() {
        let mut hands = Hands::new();
        let machine = awake();
        let refused = hands
            .request(
                &machine,
                "email_send",
                &["ada@example.com".to_owned(), "hello".to_owned()],
            )
            .expect_err("short");
        assert_eq!(
            refused,
            DispatchError::InvalidArgs {
                name: "email_send".to_owned()
            }
        );
        assert_eq!(
            refused.to_string(),
            "email_send needs to, subject, and body"
        );
        assert!(hands.pending().is_none());
        assert!(hands.outbox().is_empty());
        assert_eq!(
            hands.last_tool_line().as_deref(),
            Some("email_send confirm unknown")
        );
    }

    #[test]
    fn email_send_is_forbidden_when_not_awake() {
        let mut hands = Hands::new();
        let args = email_args(&["body"]);
        let asleep = hands
            .request(&asleep(), "email_send", &args)
            .expect_err("asleep");
        assert_eq!(
            asleep,
            DispatchError::Forbidden {
                name: "email_send".to_owned(),
                state: VoiceState::Sleep,
            }
        );
        let hibernate = hands
            .request(&hibernating(), "email_send", &["only".to_owned()])
            .expect_err("hibernate");
        assert_eq!(
            hibernate,
            DispatchError::Forbidden {
                name: "email_send".to_owned(),
                state: VoiceState::Hibernate,
            }
        );
        assert!(hands.pending().is_none());
        assert!(hands.outbox().is_empty());
    }

    #[test]
    fn cancel_and_sleep_do_not_send_email() {
        let mut hands = Hands::new();
        let machine = awake();
        hands
            .request(&machine, "email_send", &email_args(&["body"]))
            .expect("pending");
        let mismatch = hands
            .confirm(&machine, "1", Some("notify"))
            .expect_err("name");
        assert!(matches!(mismatch, DispatchError::PendingMismatch { .. }));
        assert!(hands.outbox().is_empty());
        assert!(hands.pending().is_some());
        hands.cancel("1", Some("notify")).expect_err("cancel name");
        assert!(hands.pending().is_some());

        let mut asleep = machine;
        asleep.apply(Event::SleepPhrase).expect("sleep");
        let refused = hands.confirm(&asleep, "1", None).expect_err("asleep");
        assert!(matches!(refused, DispatchError::ConfirmForbidden { .. }));
        assert!(hands.outbox().is_empty());
        assert!(hands.pending().is_some());
        hands.cancel("1", None).expect("cancel");
        assert!(hands.pending().is_none());
        assert!(hands.outbox().is_empty());

        let machine = awake();
        hands
            .request(&machine, "email_send", &email_args(&["later"]))
            .expect("stage");
        let cleared = hands.clear_pending().expect("cleared");
        assert_eq!(cleared.name, "email_send");
        assert!(hands.outbox().is_empty());
        assert_eq!(
            hands.log().back().expect("log").detail.as_deref(),
            Some("cleared when acting stopped")
        );
    }

    #[test]
    fn email_send_stores_subject_and_body_unchanged() {
        let mut hands = Hands::new();
        let machine = awake();
        let args = vec![
            "ada@example.com".to_owned(),
            "hello there".to_owned(),
            "line one".to_owned(),
        ];
        hands
            .request(&machine, "email_send", &args)
            .expect("pending");
        hands.confirm(&machine, "1", None).expect("confirm");
        assert_eq!(hands.outbox().len(), 1);
        assert_eq!(hands.outbox()[0].subject, "hello there");
        assert_eq!(hands.outbox()[0].body, "line one");
        assert!(hands.notifications().is_empty());
    }
}
