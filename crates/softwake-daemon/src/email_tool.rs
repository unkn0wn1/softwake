//! Send one authorized email through the configured backend.
//!
//! [`commit_email_send`] is the only place this daemon calls
//! [`EmailBackend::send`](softwake_connectors::EmailBackend::send).
//! The registry is asked first. A denied or unknown pair leaves messages empty.

use softwake_connectors::{
    ConnectorError, ConnectorRegistry, EmailBackend, LiveEmailError, LiveEmailMode, OutboundEmail,
    SendReceipt,
};

/// Failure after the registry step or from the live backend.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CommitEmailError {
    /// Registry refused the pair.
    #[error(transparent)]
    Connector(#[from] ConnectorError),
    /// Live scaffold rejected the send.
    #[error(transparent)]
    Live(#[from] LiveEmailError),
}

/// Detail line after a successful commit (`sent N` or `draft N`).
#[must_use]
pub(crate) fn commit_detail(backend: &EmailBackend, receipt: SendReceipt) -> String {
    match backend.live_mode() {
        Some(LiveEmailMode::DraftOnly) => format!("draft {}", receipt.id),
        Some(LiveEmailMode::Send) | None => format!("sent {}", receipt.id),
    }
}

/// Authorize `connector` / `action`, then send or draft `message`.
///
/// # Errors
///
/// Returns the registry error when the pair is not a confirm action, or a live
/// backend error when configuration or transport refuses the message.
pub(crate) fn commit_email_send(
    connectors: &ConnectorRegistry,
    email: &mut EmailBackend,
    connector: &str,
    action: &str,
    message: &OutboundEmail,
) -> Result<SendReceipt, CommitEmailError> {
    connectors.authorize_confirmed(connector, action)?;
    Ok(email.send(message)?)
}

#[cfg(test)]
mod tests {
    use softwake_connectors::{
        ConnectorError, ConnectorRegistry, EMAIL, EMAIL_DELETE, EMAIL_SEND, EmailBackend,
        EmailSettings, LiveEmailMode, MockEmail, OutboundEmail,
    };

    use super::{CommitEmailError, commit_detail, commit_email_send};

    fn message() -> OutboundEmail {
        OutboundEmail {
            to: "ada@example.com".to_owned(),
            subject: "hello".to_owned(),
            body: "a short note".to_owned(),
        }
    }

    fn live_draft() -> EmailBackend {
        EmailBackend::from_settings(
            EmailSettings {
                live_enabled: true,
                smtp_host: "smtp.example.com".to_owned(),
                username: "ada".to_owned(),
                from_address: "ada@example.com".to_owned(),
                mode: LiveEmailMode::DraftOnly,
                ..EmailSettings::default()
            },
            true,
        )
    }

    #[test]
    fn authorized_send_appends_once_and_ids_increase() {
        let connectors = ConnectorRegistry::phase3();
        let mut email = EmailBackend::Mock(MockEmail::default());
        let stored = message();
        let first =
            commit_email_send(&connectors, &mut email, EMAIL, EMAIL_SEND, &stored).expect("send");
        assert_eq!(first.id, 1);
        assert_eq!(commit_detail(&email, first), "sent 1");
        assert_eq!(email.messages(), std::slice::from_ref(&stored));
        let second =
            commit_email_send(&connectors, &mut email, EMAIL, EMAIL_SEND, &stored).expect("second");
        assert_eq!(second.id, 2);
        assert_eq!(email.messages(), &[stored.clone(), stored]);
    }

    #[test]
    fn live_draft_commits_as_draft() {
        let connectors = ConnectorRegistry::phase3();
        let mut email = live_draft();
        let stored = message();
        let receipt =
            commit_email_send(&connectors, &mut email, EMAIL, EMAIL_SEND, &stored).expect("draft");
        assert_eq!(commit_detail(&email, receipt), "draft 1");
        assert_eq!(email.messages(), std::slice::from_ref(&stored));
    }

    #[test]
    fn denied_and_unknown_do_not_append() {
        let connectors = ConnectorRegistry::phase3();
        let mut email = EmailBackend::Mock(MockEmail::default());
        let stored = message();
        assert_eq!(
            commit_email_send(&connectors, &mut email, EMAIL, EMAIL_DELETE, &stored),
            Err(CommitEmailError::Connector(ConnectorError::Denied {
                connector: EMAIL.to_owned(),
                action: EMAIL_DELETE.to_owned(),
            }))
        );
        assert!(email.messages().is_empty());
        assert_eq!(
            commit_email_send(&connectors, &mut email, "gmail", "send", &stored),
            Err(CommitEmailError::Connector(ConnectorError::Unknown {
                connector: "gmail".to_owned(),
                action: "send".to_owned(),
            }))
        );
        assert!(email.messages().is_empty());
    }
}
