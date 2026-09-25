//! Static connector registry.
//!
//! Each entry is confirm or deny. There is no safe variant. [`ConnectorRegistry::invoke`]
//! is the blind path and never succeeds for a registered action.
//! [`ConnectorRegistry::authorize_confirmed`] allows a confirm action and does
//! not perform it. Neither method accepts an [`crate::EmailConnector`], so
//! neither can send.

/// Connector name for email.
pub const EMAIL: &str = "email";

/// Confirm-gated email action.
///
/// [`crate::MockEmail::send`] is what appends a message. Calling it is the
/// caller's step after [`ConnectorRegistry::authorize_confirmed`].
pub const EMAIL_SEND: &str = "send";

/// Denied email action. It has no backend.
pub const EMAIL_DELETE: &str = "delete";

/// Connector name for Drive. Actions stay denied until a backend exists.
pub const DRIVE: &str = "drive";

/// Denied Drive action. It has no backend.
pub const DRIVE_LIST: &str = "list";

/// Connector name for calendar. Actions stay denied until a backend exists.
pub const CALENDAR: &str = "calendar";

/// Denied calendar action. It has no backend.
pub const CALENDAR_LIST: &str = "list";

/// How a registered connector action may be treated.
///
/// There is no safe variant. A world action is confirm or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorRisk {
    /// The caller may perform the action only after a confirmation held outside this crate.
    Confirm,
    /// The action never runs, including after a confirm attempt.
    Deny,
}

impl ConnectorRisk {
    /// Stable spelling for logs and operator text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Deny => "deny",
        }
    }
}

impl std::fmt::Display for ConnectorRisk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One registered connector action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectorMeta {
    /// Capability name, such as [`EMAIL`].
    pub connector: &'static str,
    /// Action name, such as [`EMAIL_SEND`].
    pub action: &'static str,
    /// Whether the action may be authorized or is refused.
    pub risk: ConnectorRisk,
    /// Short operator-facing description.
    pub description: &'static str,
}

const PHASE3: &[ConnectorMeta] = &[
    ConnectorMeta {
        connector: EMAIL,
        action: EMAIL_SEND,
        risk: ConnectorRisk::Confirm,
        description: "Send one email through the connector. Runs only after confirmation.",
    },
    ConnectorMeta {
        connector: EMAIL,
        action: EMAIL_DELETE,
        risk: ConnectorRisk::Deny,
        description: "Delete email. Denied.",
    },
    ConnectorMeta {
        connector: DRIVE,
        action: DRIVE_LIST,
        risk: ConnectorRisk::Deny,
        description: "List Drive files. Denied until a backend exists.",
    },
    ConnectorMeta {
        connector: CALENDAR,
        action: CALENDAR_LIST,
        risk: ConnectorRisk::Deny,
        description: "List calendar events. Denied until a backend exists.",
    },
];

/// Failure from a registry classification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectorError {
    /// The connector and action pair is not registered.
    #[error("unknown connector action: {connector}/{action}")]
    Unknown {
        /// Connector name that was rejected.
        connector: String,
        /// Action name that was rejected.
        action: String,
    },

    /// The pair is registered as [`ConnectorRisk::Deny`].
    #[error("connector action denied: {connector}/{action}")]
    Denied {
        /// Connector name that was rejected.
        connector: String,
        /// Action name that was rejected.
        action: String,
    },

    /// The pair is [`ConnectorRisk::Confirm`] and this call is the blind path.
    #[error("connector action requires confirmation: {connector}/{action}")]
    NeedsConfirm {
        /// Connector name that was not authorized.
        connector: String,
        /// Action name that was not authorized.
        action: String,
    },
}

/// Registered connector actions.
///
/// The table is static. There is no method that inserts an action, so a caller
/// cannot add a safe send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectorRegistry {
    actions: &'static [ConnectorMeta],
}

impl ConnectorRegistry {
    /// Registry for this slice: email send, email delete, Drive list, calendar list.
    #[must_use]
    pub const fn phase3() -> Self {
        Self { actions: PHASE3 }
    }

    /// Metadata in registration order.
    #[must_use]
    pub const fn entries(self) -> &'static [ConnectorMeta] {
        self.actions
    }

    /// Metadata for the pair, if it is registered.
    ///
    /// Matching is case-sensitive and does not trim.
    #[must_use]
    pub fn lookup(&self, connector: &str, action: &str) -> Option<&ConnectorMeta> {
        self.actions
            .iter()
            .find(|entry| entry.connector == connector && entry.action == action)
    }

    /// Risk for the pair, if it is registered.
    #[must_use]
    pub fn risk(&self, connector: &str, action: &str) -> Option<ConnectorRisk> {
        self.lookup(connector, action).map(|entry| entry.risk)
    }

    /// Classify a connector action with no confirmation.
    ///
    /// This method does not accept a connector value, so it cannot send.
    /// A confirm action returns [`ConnectorError::NeedsConfirm`]. A deny
    /// action returns [`ConnectorError::Denied`]. An unknown pair returns
    /// [`ConnectorError::Unknown`].
    ///
    /// # Errors
    ///
    /// See the variants above. No registered action returns `Ok`.
    pub fn invoke(&self, connector: &str, action: &str) -> Result<(), ConnectorError> {
        match self.risk(connector, action) {
            Some(ConnectorRisk::Confirm) => Err(needs_confirm(connector, action)),
            Some(ConnectorRisk::Deny) => Err(denied(connector, action)),
            None => Err(unknown(connector, action)),
        }
    }

    /// Allow a confirm action after the daemon has already accepted a confirmation.
    ///
    /// This function does not check a token and does not send. `Ok(())` means
    /// the caller may perform the action. The only confirm action in
    /// [`Self::phase3`] is [`EMAIL`] / [`EMAIL_SEND`].
    ///
    /// # Errors
    ///
    /// Returns [`ConnectorError::Denied`] for a deny action and
    /// [`ConnectorError::Unknown`] when the pair is not registered.
    pub fn authorize_confirmed(&self, connector: &str, action: &str) -> Result<(), ConnectorError> {
        match self.risk(connector, action) {
            Some(ConnectorRisk::Confirm) => Ok(()),
            Some(ConnectorRisk::Deny) => Err(denied(connector, action)),
            None => Err(unknown(connector, action)),
        }
    }
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::phase3()
    }
}

fn unknown(connector: &str, action: &str) -> ConnectorError {
    ConnectorError::Unknown {
        connector: connector.to_owned(),
        action: action.to_owned(),
    }
}

fn denied(connector: &str, action: &str) -> ConnectorError {
    ConnectorError::Denied {
        connector: connector.to_owned(),
        action: action.to_owned(),
    }
}

fn needs_confirm(connector: &str, action: &str) -> ConnectorError {
    ConnectorError::NeedsConfirm {
        connector: connector.to_owned(),
        action: action.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CALENDAR, CALENDAR_LIST, ConnectorError, ConnectorRegistry, ConnectorRisk, DRIVE,
        DRIVE_LIST, EMAIL, EMAIL_DELETE, EMAIL_SEND,
    };

    fn registry() -> ConnectorRegistry {
        ConnectorRegistry::phase3()
    }

    #[test]
    fn phase3_registers_confirm_and_deny_and_default_matches() {
        let registry = registry();
        assert_eq!(
            registry
                .entries()
                .iter()
                .map(|entry| (entry.connector, entry.action, entry.risk))
                .collect::<Vec<_>>(),
            vec![
                (EMAIL, EMAIL_SEND, ConnectorRisk::Confirm),
                (EMAIL, EMAIL_DELETE, ConnectorRisk::Deny),
                (DRIVE, DRIVE_LIST, ConnectorRisk::Deny),
                (CALENDAR, CALENDAR_LIST, ConnectorRisk::Deny),
            ]
        );
        assert!(
            registry
                .entries()
                .iter()
                .all(|entry| !entry.description.is_empty())
        );
        assert_eq!(
            registry
                .entries()
                .iter()
                .map(|entry| entry.description)
                .collect::<Vec<_>>(),
            vec![
                "Send one email through the connector. Runs only after confirmation.",
                "Delete email. Denied.",
                "List Drive files. Denied until a backend exists.",
                "List calendar events. Denied until a backend exists.",
            ]
        );
        let confirmations = registry
            .entries()
            .iter()
            .filter(|entry| entry.risk == ConnectorRisk::Confirm)
            .count();
        assert_eq!(confirmations, 1);
        for entry in registry.entries() {
            match entry.risk {
                ConnectorRisk::Confirm | ConnectorRisk::Deny => {
                    assert_ne!(entry.risk.as_str(), "safe");
                }
            }
            assert_eq!(entry.risk.as_str(), entry.risk.to_string());
        }
        assert_eq!(
            registry.risk(EMAIL, EMAIL_SEND),
            Some(ConnectorRisk::Confirm)
        );
        assert_eq!(
            registry.risk(EMAIL, EMAIL_DELETE),
            Some(ConnectorRisk::Deny)
        );
        assert_eq!(registry.risk(DRIVE, DRIVE_LIST), Some(ConnectorRisk::Deny));
        assert_eq!(
            registry.risk(CALENDAR, CALENDAR_LIST),
            Some(ConnectorRisk::Deny)
        );
        assert_eq!(registry.risk("Email", EMAIL_SEND), None);
        assert_eq!(registry.risk(EMAIL, "SEND"), None);
        assert_eq!(registry.risk("", EMAIL_SEND), None);
        assert_eq!(registry.risk(EMAIL, ""), None);
        assert_eq!(ConnectorRegistry::default().entries(), registry.entries());
        assert_eq!(ConnectorRegistry::default(), registry);
    }

    #[test]
    fn blind_invoke_never_succeeds() {
        let registry = registry();
        for entry in registry.entries() {
            assert!(registry.invoke(entry.connector, entry.action).is_err());
        }
        let needs_confirm = registry.invoke(EMAIL, EMAIL_SEND).expect_err("blind");
        assert_eq!(
            needs_confirm,
            ConnectorError::NeedsConfirm {
                connector: EMAIL.to_owned(),
                action: EMAIL_SEND.to_owned(),
            }
        );
        assert_eq!(
            needs_confirm.to_string(),
            "connector action requires confirmation: email/send"
        );
        for (connector, action) in [
            (EMAIL, EMAIL_DELETE),
            (DRIVE, DRIVE_LIST),
            (CALENDAR, CALENDAR_LIST),
        ] {
            let denied = registry.invoke(connector, action).expect_err("deny");
            assert_eq!(
                denied,
                ConnectorError::Denied {
                    connector: connector.to_owned(),
                    action: action.to_owned(),
                }
            );
            assert_eq!(
                denied.to_string(),
                format!("connector action denied: {connector}/{action}")
            );
        }
    }

    #[test]
    fn authorize_confirmed_allows_only_email_send() {
        let registry = registry();
        assert_eq!(registry.authorize_confirmed(EMAIL, EMAIL_SEND), Ok(()));
        for (connector, action) in [
            (EMAIL, EMAIL_DELETE),
            (DRIVE, DRIVE_LIST),
            (CALENDAR, CALENDAR_LIST),
        ] {
            assert_eq!(
                registry.authorize_confirmed(connector, action),
                Err(ConnectorError::Denied {
                    connector: connector.to_owned(),
                    action: action.to_owned(),
                })
            );
        }
    }

    #[test]
    fn unknown_pairs_are_rejected() {
        let registry = registry();
        for (connector, action) in [
            ("gmail", "send"),
            ("email", "forward"),
            ("email", "Send"),
            ("drive", "send"),
            ("", "send"),
            ("email", ""),
        ] {
            let error = registry.invoke(connector, action).expect_err("unknown");
            assert_eq!(
                error,
                ConnectorError::Unknown {
                    connector: connector.to_owned(),
                    action: action.to_owned(),
                }
            );
            assert_eq!(
                error.to_string(),
                format!("unknown connector action: {connector}/{action}")
            );
        }
    }
}
