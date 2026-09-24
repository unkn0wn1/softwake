//! Gate a tool call on the voice state, then on the registry.
//!
//! [`softwake_state::Machine::permit_tool_dispatch`] runs first. Sleep and
//! hibernate never reach the registry. While awake, [`ToolRisk::Safe`] runs
//! immediately, [`ToolRisk::Confirm`] becomes one pending confirmation, and
//! [`ToolRisk::Deny`] is refused. The confirm-gated tool does not run, and
//! the notification sink does not change, until [`Hands::confirm`].
//!
//! One pending confirmation at a time. A second confirm-gated request is
//! rejected and leaves the first in place. [`Hands::cancel`] clears it in any
//! voice state. [`Hands::confirm`] is refused when the machine is not awake.
//! Sleep and hibernate should call [`Hands::clear_pending`].

use std::collections::VecDeque;

use softwake_state::{Machine, StateError, VoiceState};
use softwake_tools::{NOTIFY_TOOL, ToolError, ToolRegistry, ToolResult, ToolRisk};

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
    description: &'static str,
}

/// Registry gate, one pending confirmation, the notification sink, and the tool log.
#[derive(Debug)]
pub(crate) struct Hands {
    registry: ToolRegistry,
    pending: Option<Pending>,
    sink: VecDeque<String>,
    log: VecDeque<ToolLogEntry>,
    next_seq: u64,
    next_pending: u64,
}

impl Hands {
    /// Empty sink, empty log, and the phase-2 registry.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            registry: ToolRegistry::phase2(),
            pending: None,
            sink: VecDeque::new(),
            log: VecDeque::new(),
            next_seq: 0,
            next_pending: 0,
        }
    }

    /// The confirmation that is waiting, if any.
    #[must_use]
    pub(crate) fn pending(&self) -> Option<PendingToolCall> {
        self.pending.as_ref().map(|pending| PendingToolCall {
            pending_id: pending.id.clone(),
            name: pending.name.clone(),
            args: pending.args.clone(),
            description: pending.description.to_owned(),
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
    /// A confirm-gated tool does not run and does not touch the sink.
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
        let Some(meta) = self.registry.lookup(name) else {
            self.record(name, None, ToolOutcome::Unknown, None);
            return Err(DispatchError::Unknown {
                name: name.to_owned(),
            });
        };
        let risk = meta.risk;
        let description = meta.description;
        match risk {
            ToolRisk::Deny => {
                self.record(name, Some(risk), ToolOutcome::Denied, None);
                Err(DispatchError::Denied {
                    name: name.to_owned(),
                })
            }
            ToolRisk::Safe => match self.registry.invoke(name, args) {
                Ok(ToolResult { detail }) => {
                    self.record(name, Some(risk), ToolOutcome::Ran, Some(detail.clone()));
                    Ok(RequestOutcome::Ran(RanTool {
                        name: name.to_owned(),
                        detail,
                    }))
                }
                Err(error) => Err(self.fail_tool(error)),
            },
            ToolRisk::Confirm => {
                if let Some(pending_id) = self.pending.as_ref().map(|pending| pending.id.clone()) {
                    self.record(
                        name,
                        Some(risk),
                        ToolOutcome::Pending,
                        Some(format!("busy: {pending_id}")),
                    );
                    return Err(DispatchError::Busy { pending_id });
                }
                self.next_pending = self.next_pending.saturating_add(1);
                let pending_id = self.next_pending.to_string();
                self.pending = Some(Pending {
                    id: pending_id.clone(),
                    name: name.to_owned(),
                    args: args.to_vec(),
                    description,
                });
                self.record(name, Some(risk), ToolOutcome::Pending, None);
                Ok(RequestOutcome::Pending(PendingToolCall {
                    pending_id,
                    name: name.to_owned(),
                    args: args.to_vec(),
                    description: description.to_owned(),
                }))
            }
        }
    }

    /// Run the pending tool once when `pending_id` matches and the machine is awake.
    ///
    /// `expected_name`, when set, must match the stored tool. A mismatch leaves
    /// the pending record in place. The sink changes only after a successful
    /// `notify` run.
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
        let detail = match self.registry.invoke_confirmed(&pending.name, &pending.args) {
            Ok(ToolResult { detail }) => detail,
            Err(error) => return Err(self.fail_tool(error)),
        };
        if pending.name == NOTIFY_TOOL {
            self.push_notification(detail.clone());
        }
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

    fn fail_tool(&mut self, error: ToolError) -> DispatchError {
        let (recorded, risk, outcome, dispatch) = match error {
            ToolError::Denied { name } => (
                name.clone(),
                Some(ToolRisk::Deny),
                ToolOutcome::Denied,
                DispatchError::Denied { name },
            ),
            ToolError::Unknown { name }
            | ToolError::NeedsConfirm { name }
            | ToolError::NotConfirmGated { name } => (
                name.clone(),
                None,
                ToolOutcome::Unknown,
                DispatchError::Unknown { name },
            ),
        };
        self.record(&recorded, risk, outcome, None);
        dispatch
    }

    fn push_notification(&mut self, line: String) {
        if self.sink.len() == SINK_CAP {
            self.sink.pop_front();
        }
        self.sink.push_back(line);
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
        assert_eq!(hands.last_tool_line().as_deref(), Some("shell deny denied"));
        hands.confirm(&machine, "1", None).expect_err("no pending");
        assert!(hands.notifications().is_empty());

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
}
