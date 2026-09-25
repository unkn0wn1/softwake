//! Send one authorized email through the in-memory connector.
//!
//! [`commit_email_send`] is the only place this daemon calls
//! [`EmailConnector::send`](softwake_connectors::EmailConnector::send).
//! The registry is asked first. A denied or unknown pair leaves the outbox empty.

use softwake_connectors::{
    ConnectorError, ConnectorRegistry, EmailConnector, MockEmail, OutboundEmail, SendReceipt,
};

/// Authorize `connector` / `action`, then send `message`.
///
/// # Errors
///
/// Returns the registry error when the pair is not a confirm action.
/// [`MockEmail`] does not fail after authorization.
pub(crate) fn commit_email_send(
    connectors: &ConnectorRegistry,
    email: &mut MockEmail,
    connector: &str,
    action: &str,
    message: &OutboundEmail,
) -> Result<SendReceipt, ConnectorError> {
    connectors.authorize_confirmed(connector, action)?;
    match EmailConnector::send(email, message) {
        Ok(receipt) => Ok(receipt),
        Err(error) => match error {},
    }
}

#[cfg(test)]
mod tests {
    use softwake_connectors::{
        ConnectorError, ConnectorRegistry, EMAIL, EMAIL_DELETE, EMAIL_SEND, MockEmail,
        OutboundEmail,
    };

    use super::commit_email_send;

    fn message() -> OutboundEmail {
        OutboundEmail {
            to: "ada@example.com".to_owned(),
            subject: "hello".to_owned(),
            body: "a short note".to_owned(),
        }
    }

    #[test]
    fn authorized_send_appends_once_and_ids_increase() {
        let connectors = ConnectorRegistry::phase3();
        let mut email = MockEmail::default();
        let stored = message();
        let first =
            commit_email_send(&connectors, &mut email, EMAIL, EMAIL_SEND, &stored).expect("send");
        assert_eq!(first.id, 1);
        assert_eq!(email.outbox(), std::slice::from_ref(&stored));
        let second =
            commit_email_send(&connectors, &mut email, EMAIL, EMAIL_SEND, &stored).expect("second");
        assert_eq!(second.id, 2);
        assert_eq!(email.outbox(), &[stored.clone(), stored]);
    }

    #[test]
    fn denied_and_unknown_do_not_append() {
        let connectors = ConnectorRegistry::phase3();
        let mut email = MockEmail::default();
        let stored = message();
        assert_eq!(
            commit_email_send(&connectors, &mut email, EMAIL, EMAIL_DELETE, &stored),
            Err(ConnectorError::Denied {
                connector: EMAIL.to_owned(),
                action: EMAIL_DELETE.to_owned(),
            })
        );
        assert!(email.outbox().is_empty());
        assert_eq!(
            commit_email_send(&connectors, &mut email, "gmail", "send", &stored),
            Err(ConnectorError::Unknown {
                connector: "gmail".to_owned(),
                action: "send".to_owned(),
            })
        );
        assert!(email.outbox().is_empty());
    }
}
