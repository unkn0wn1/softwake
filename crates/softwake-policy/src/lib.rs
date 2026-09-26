//! Policy engine.
//!
//! [`PolicyEngine::evaluate`] is the classification path for a tool name and
//! for a connector pair. It reads [`softwake_tools::ToolRegistry`] and
//! [`softwake_connectors::ConnectorRegistry`] and does not copy their rows.
//! An unknown subject is [`PolicyDecision::Deny`]. A connector subject never
//! evaluates to [`PolicyDecision::Safe`].
//!
//! [`tighten`] raises risk and ignores a weaker request. This crate does not
//! read a soul file, send mail, or open a socket.

mod tighten;

use softwake_connectors::{ConnectorRegistry, ConnectorRisk};
use softwake_tools::{ToolPermission, ToolRegistry, ToolRisk};

pub use tighten::tighten;

/// Operator grant composed with the registry floor and a tighten-only soul request.
///
/// Operator Always allow may loosen a confirm floor. A registry deny stays deny.
/// Soul requests only tighten the operator result. This function is not
/// `tighten(floor, operator)`.
#[must_use]
pub fn effective_tool_decision(
    floor: PolicyDecision,
    operator: ToolPermission,
    soul_tighten: Option<PolicyDecision>,
) -> PolicyDecision {
    if floor == PolicyDecision::Deny {
        return PolicyDecision::Deny;
    }
    let chosen = match operator {
        ToolPermission::Deny => PolicyDecision::Deny,
        ToolPermission::Ask => PolicyDecision::Confirm,
        ToolPermission::AlwaysAllow => PolicyDecision::Safe,
    };
    match soul_tighten {
        Some(requested) => tighten(chosen, requested),
        None => chosen,
    }
}

/// How a subject may be treated after policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Run while awake, with no extra confirmation.
    Safe,
    /// Do not run until the operator confirms that pending call.
    Confirm,
    /// Do not run.
    Deny,
}

impl PolicyDecision {
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

impl std::fmt::Display for PolicyDecision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What [`PolicyEngine::evaluate`] classifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject<'a> {
    /// A tool-bus name, such as `echo`.
    Tool {
        /// Registry name. Matching is case-sensitive.
        name: &'a str,
    },
    /// A connector capability and action, such as `email` / `send`.
    Connector {
        /// Capability name.
        connector: &'a str,
        /// Action name.
        action: &'a str,
    },
}

/// Requests that may raise the risk of a known row.
///
/// A request weaker than the registry floor is ignored. Several requests for
/// one row keep the most restrictive request. An unknown subject is denied
/// without consulting these rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyOverrides {
    tools: Vec<(String, PolicyDecision)>,
    connectors: Vec<((String, String), PolicyDecision)>,
}

impl PolicyOverrides {
    /// Store override requests. The vectors may be empty.
    #[must_use]
    pub fn new(
        tools: Vec<(String, PolicyDecision)>,
        connectors: Vec<((String, String), PolicyDecision)>,
    ) -> Self {
        Self { tools, connectors }
    }

    fn tool(&self, name: &str) -> Option<PolicyDecision> {
        tighten::strictest(
            self.tools
                .iter()
                .filter_map(|(candidate, decision)| (candidate == name).then_some(*decision)),
        )
    }

    fn connector(&self, connector: &str, action: &str) -> Option<PolicyDecision> {
        tighten::strictest(self.connectors.iter().filter_map(
            |((candidate, candidate_action), decision)| {
                (candidate == connector && candidate_action == action).then_some(*decision)
            },
        ))
    }
}

/// Builtin allowlists plus an optional tighten-only override map.
///
/// The tool table is [`ToolRegistry::phase2`]. The connector table is
/// [`ConnectorRegistry::phase3`]. There is no method that replaces either table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyEngine {
    tools: ToolRegistry,
    connectors: ConnectorRegistry,
    overrides: PolicyOverrides,
}

impl PolicyEngine {
    /// Phase-2 tools, phase-3 connectors, and an empty override map.
    #[must_use]
    pub fn builtin() -> Self {
        Self::with_overrides(PolicyOverrides::new(Vec::new(), Vec::new()))
    }

    /// Same tables as [`Self::builtin`], with `overrides` applied on known rows.
    #[must_use]
    pub fn with_overrides(overrides: PolicyOverrides) -> Self {
        Self {
            tools: ToolRegistry::phase2(),
            connectors: ConnectorRegistry::phase3(),
            overrides,
        }
    }

    /// Classify `subject`.
    ///
    /// Unknown names return [`PolicyDecision::Deny`] and do not read the
    /// override map. A connector subject returns [`PolicyDecision::Confirm`]
    /// or [`PolicyDecision::Deny`].
    #[must_use]
    pub fn evaluate(&self, subject: &Subject<'_>) -> PolicyDecision {
        match *subject {
            Subject::Tool { name } => self.evaluate_tool(name),
            Subject::Connector { connector, action } => self.evaluate_connector(connector, action),
        }
    }

    /// Strictest override request for `name`, not the evaluated registry floor.
    ///
    /// `None` when the override map has no row for that name.
    #[must_use]
    pub fn tool_tighten_request(&self, name: &str) -> Option<PolicyDecision> {
        self.overrides.tool(name)
    }

    fn evaluate_tool(&self, name: &str) -> PolicyDecision {
        let Some(floor) = tool_floor(self.tools.risk(name)) else {
            return PolicyDecision::Deny;
        };
        apply(floor, self.overrides.tool(name))
    }

    fn evaluate_connector(&self, connector: &str, action: &str) -> PolicyDecision {
        let Some(floor) = connector_floor(self.connectors.risk(connector, action)) else {
            return PolicyDecision::Deny;
        };
        apply(floor, self.overrides.connector(connector, action))
    }
}

/// World I/O may proceed only when `decision` is [`PolicyDecision::Confirm`].
#[must_use]
pub const fn permits_confirmed_connector(decision: PolicyDecision) -> bool {
    matches!(decision, PolicyDecision::Confirm)
}

fn apply(floor: PolicyDecision, requested: Option<PolicyDecision>) -> PolicyDecision {
    match requested {
        Some(requested) => tighten(floor, requested),
        None => floor,
    }
}

fn tool_floor(risk: Option<ToolRisk>) -> Option<PolicyDecision> {
    match risk {
        Some(ToolRisk::Safe) => Some(PolicyDecision::Safe),
        Some(ToolRisk::Confirm) => Some(PolicyDecision::Confirm),
        Some(ToolRisk::Deny) => Some(PolicyDecision::Deny),
        None => None,
    }
}

fn connector_floor(risk: Option<ConnectorRisk>) -> Option<PolicyDecision> {
    match risk {
        Some(ConnectorRisk::Confirm) => Some(PolicyDecision::Confirm),
        Some(ConnectorRisk::Deny) => Some(PolicyDecision::Deny),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use softwake_connectors::{
        CALENDAR, CALENDAR_DELETE, CALENDAR_LIST, ConnectorRegistry, ConnectorRisk, DRIVE,
        DRIVE_DELETE, DRIVE_LIST, EMAIL, EMAIL_DELETE, EMAIL_SEND,
    };
    use softwake_tools::{
        ECHO_TOOL, EMAIL_SEND_TOOL, NOTIFY_TOOL, SHELL_TOOL, ToolRegistry, ToolRisk,
    };

    use softwake_tools::ToolPermission;

    use super::{
        PolicyDecision, PolicyEngine, PolicyOverrides, Subject, effective_tool_decision,
        permits_confirmed_connector,
    };

    fn builtin() -> PolicyEngine {
        PolicyEngine::builtin()
    }

    fn tool_engine(name: &str, decision: PolicyDecision) -> PolicyEngine {
        PolicyEngine::with_overrides(PolicyOverrides::new(
            vec![(name.to_owned(), decision)],
            Vec::new(),
        ))
    }

    fn connector_engine(connector: &str, action: &str, decision: PolicyDecision) -> PolicyEngine {
        PolicyEngine::with_overrides(PolicyOverrides::new(
            Vec::new(),
            vec![((connector.to_owned(), action.to_owned()), decision)],
        ))
    }

    fn eval_tool(engine: &PolicyEngine, name: &str) -> PolicyDecision {
        engine.evaluate(&Subject::Tool { name })
    }

    fn eval_connector(engine: &PolicyEngine, connector: &str, action: &str) -> PolicyDecision {
        let decision = engine.evaluate(&Subject::Connector { connector, action });
        assert_ne!(decision, PolicyDecision::Safe);
        decision
    }

    #[test]
    fn builtin_matches_the_registered_allowlists() {
        let engine = builtin();
        assert_eq!(eval_tool(&engine, ECHO_TOOL), PolicyDecision::Safe);
        assert_eq!(eval_tool(&engine, NOTIFY_TOOL), PolicyDecision::Confirm);
        assert_eq!(eval_tool(&engine, EMAIL_SEND_TOOL), PolicyDecision::Confirm);
        assert_eq!(eval_tool(&engine, SHELL_TOOL), PolicyDecision::Confirm);
        assert_eq!(
            eval_connector(&engine, EMAIL, EMAIL_SEND),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_connector(&engine, EMAIL, EMAIL_DELETE),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(&engine, DRIVE, DRIVE_LIST),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_connector(&engine, DRIVE, DRIVE_DELETE),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(&engine, CALENDAR, CALENDAR_LIST),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_connector(&engine, CALENDAR, CALENDAR_DELETE),
            PolicyDecision::Deny
        );
        let empty = PolicyEngine::with_overrides(PolicyOverrides::new(Vec::new(), Vec::new()));
        assert_eq!(engine, empty);
    }

    #[test]
    fn builtin_tools_match_registry_rows() {
        let engine = builtin();
        let registry = ToolRegistry::phase2();
        for tool in registry.entries() {
            let decision = eval_tool(&engine, tool.name);
            let expected = match tool.risk {
                ToolRisk::Safe => PolicyDecision::Safe,
                ToolRisk::Confirm => PolicyDecision::Confirm,
                ToolRisk::Deny => PolicyDecision::Deny,
            };
            assert_eq!(decision, expected);
            assert_eq!(decision.as_str(), tool.risk.as_str());
            assert_eq!(decision.as_str(), decision.to_string());
        }
    }

    #[test]
    fn builtin_connectors_match_registry_rows_and_are_never_safe() {
        let engine = builtin();
        let registry = ConnectorRegistry::phase3();
        for entry in registry.entries() {
            let decision = eval_connector(&engine, entry.connector, entry.action);
            let expected = match entry.risk {
                ConnectorRisk::Confirm => PolicyDecision::Confirm,
                ConnectorRisk::Deny => PolicyDecision::Deny,
            };
            assert_eq!(decision, expected);
            assert_eq!(decision.as_str(), entry.risk.as_str());
            assert_ne!(decision.as_str(), "safe");
        }
    }

    #[test]
    fn unknown_subjects_are_denied() {
        let engine = builtin();
        for name in ["volume", "Echo", "Email_Send", ""] {
            assert_eq!(eval_tool(&engine, name), PolicyDecision::Deny);
        }
        for (connector, action) in [
            ("gmail", "send"),
            ("email", "forward"),
            ("email", "Send"),
            ("email", ""),
            ("", "send"),
            ("drive", "upload"),
            ("calendar", "create"),
            ("Drive", "list"),
            ("drive", "List"),
        ] {
            assert_eq!(
                eval_connector(&engine, connector, action),
                PolicyDecision::Deny
            );
        }
    }

    #[test]
    fn permits_confirmed_connector_accepts_only_confirm() {
        assert!(permits_confirmed_connector(PolicyDecision::Confirm));
        assert!(!permits_confirmed_connector(PolicyDecision::Safe));
        assert!(!permits_confirmed_connector(PolicyDecision::Deny));
    }

    #[test]
    fn known_tool_overrides_only_tighten() {
        assert_eq!(
            eval_tool(&tool_engine(ECHO_TOOL, PolicyDecision::Confirm), ECHO_TOOL),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_tool(&tool_engine(ECHO_TOOL, PolicyDecision::Deny), ECHO_TOOL),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_tool(&tool_engine(ECHO_TOOL, PolicyDecision::Safe), ECHO_TOOL),
            PolicyDecision::Safe
        );
        assert_eq!(
            eval_tool(&tool_engine(NOTIFY_TOOL, PolicyDecision::Safe), NOTIFY_TOOL),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_tool(&tool_engine(NOTIFY_TOOL, PolicyDecision::Deny), NOTIFY_TOOL),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_tool(
                &tool_engine(EMAIL_SEND_TOOL, PolicyDecision::Safe),
                EMAIL_SEND_TOOL
            ),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_tool(&tool_engine(SHELL_TOOL, PolicyDecision::Safe), SHELL_TOOL),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_tool(
                &tool_engine(SHELL_TOOL, PolicyDecision::Confirm),
                SHELL_TOOL
            ),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_tool(&tool_engine(SHELL_TOOL, PolicyDecision::Deny), SHELL_TOOL),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn known_connector_overrides_only_tighten_and_never_return_safe() {
        assert_eq!(
            eval_connector(
                &connector_engine(EMAIL, EMAIL_SEND, PolicyDecision::Safe),
                EMAIL,
                EMAIL_SEND
            ),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_connector(
                &connector_engine(EMAIL, EMAIL_SEND, PolicyDecision::Deny),
                EMAIL,
                EMAIL_SEND
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine(EMAIL, EMAIL_DELETE, PolicyDecision::Confirm),
                EMAIL,
                EMAIL_DELETE
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine(EMAIL, EMAIL_DELETE, PolicyDecision::Safe),
                EMAIL,
                EMAIL_DELETE
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine(DRIVE, DRIVE_LIST, PolicyDecision::Safe),
                DRIVE,
                DRIVE_LIST
            ),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_connector(
                &connector_engine(DRIVE, DRIVE_LIST, PolicyDecision::Deny),
                DRIVE,
                DRIVE_LIST
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine(CALENDAR, CALENDAR_LIST, PolicyDecision::Safe),
                CALENDAR,
                CALENDAR_LIST
            ),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_connector(
                &connector_engine(CALENDAR, CALENDAR_LIST, PolicyDecision::Confirm),
                CALENDAR,
                CALENDAR_LIST
            ),
            PolicyDecision::Confirm
        );
        assert_eq!(
            eval_connector(
                &connector_engine(DRIVE, DRIVE_DELETE, PolicyDecision::Confirm),
                DRIVE,
                DRIVE_DELETE
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine(DRIVE, DRIVE_DELETE, PolicyDecision::Safe),
                DRIVE,
                DRIVE_DELETE
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine(CALENDAR, CALENDAR_DELETE, PolicyDecision::Confirm),
                CALENDAR,
                CALENDAR_DELETE
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine(CALENDAR, CALENDAR_DELETE, PolicyDecision::Safe),
                CALENDAR,
                CALENDAR_DELETE
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn unknown_overrides_do_not_authorize() {
        assert_eq!(
            eval_tool(&tool_engine("volume", PolicyDecision::Safe), "volume"),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_tool(&tool_engine("volume", PolicyDecision::Confirm), "volume"),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine("gmail", "send", PolicyDecision::Confirm),
                "gmail",
                "send"
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine("gmail", "send", PolicyDecision::Safe),
                "gmail",
                "send"
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine("drive", "upload", PolicyDecision::Confirm),
                "drive",
                "upload"
            ),
            PolicyDecision::Deny
        );
        assert_eq!(
            eval_connector(
                &connector_engine("calendar", "create", PolicyDecision::Confirm),
                "calendar",
                "create"
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn effective_tool_decision_composes_operator_grant_then_soul_tighten() {
        use PolicyDecision::{Confirm, Deny, Safe};
        use ToolPermission::{AlwaysAllow, Ask, Deny as OperatorDeny};

        let rows = [
            (Safe, AlwaysAllow, None, Safe),
            (Confirm, AlwaysAllow, None, Safe),
            (Safe, Ask, None, Confirm),
            (Confirm, Ask, None, Confirm),
            (Safe, OperatorDeny, None, Deny),
            (Confirm, OperatorDeny, None, Deny),
            (Deny, AlwaysAllow, None, Deny),
            (Deny, Ask, None, Deny),
            (Deny, OperatorDeny, None, Deny),
            (Deny, AlwaysAllow, Some(Safe), Deny),
            (Deny, Ask, Some(Confirm), Deny),
            (Deny, OperatorDeny, Some(Deny), Deny),
            (Safe, AlwaysAllow, Some(Confirm), Confirm),
            (Confirm, AlwaysAllow, Some(Confirm), Confirm),
            (Safe, AlwaysAllow, Some(Deny), Deny),
            (Confirm, AlwaysAllow, Some(Deny), Deny),
            (Safe, Ask, Some(Safe), Confirm),
            (Confirm, Ask, Some(Safe), Confirm),
            (Safe, OperatorDeny, Some(Safe), Deny),
            (Confirm, OperatorDeny, Some(Safe), Deny),
        ];
        for (floor, operator, soul, expected) in rows {
            assert_eq!(
                effective_tool_decision(floor, operator, soul),
                expected,
                "floor {floor} operator {operator} soul {soul:?}"
            );
        }
        assert!(
            PolicyEngine::builtin()
                .tool_tighten_request(ECHO_TOOL)
                .is_none()
        );
        assert_eq!(
            tool_engine(NOTIFY_TOOL, Deny).tool_tighten_request(NOTIFY_TOOL),
            Some(Deny)
        );
    }

    #[test]
    fn duplicate_tool_requests_keep_the_stricter_one() {
        for tools in [
            vec![
                (ECHO_TOOL.to_owned(), PolicyDecision::Confirm),
                (ECHO_TOOL.to_owned(), PolicyDecision::Deny),
            ],
            vec![
                (ECHO_TOOL.to_owned(), PolicyDecision::Deny),
                (ECHO_TOOL.to_owned(), PolicyDecision::Confirm),
            ],
        ] {
            let engine = PolicyEngine::with_overrides(PolicyOverrides::new(tools, Vec::new()));
            assert_eq!(eval_tool(&engine, ECHO_TOOL), PolicyDecision::Deny);
        }
    }
}
