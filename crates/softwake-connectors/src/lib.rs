//! Connector boundary.
//!
//! World I/O is a separate surface from the tool registry. [`ConnectorRegistry`]
//! classifies an action as confirm or deny. There is no safe world action.
//! [`invoke`](ConnectorRegistry::invoke) never sends. [`authorize_confirmed`](ConnectorRegistry::authorize_confirmed)
//! does not send either: it only reports that a confirm action may proceed.
//! [`MockEmail`] stores outbound messages in memory. This crate does not open
//! a socket and does not read credentials.

mod email;
mod mock;
mod registry;

pub use email::{EmailConnector, OutboundEmail, SendReceipt};
pub use mock::MockEmail;
pub use registry::{
    CALENDAR, CALENDAR_LIST, ConnectorError, ConnectorMeta, ConnectorRegistry, ConnectorRisk,
    DRIVE, DRIVE_LIST, EMAIL, EMAIL_DELETE, EMAIL_SEND,
};

#[cfg(test)]
mod tests {
    use super::{
        CALENDAR, CALENDAR_LIST, ConnectorError, ConnectorRegistry, DRIVE, DRIVE_LIST, EMAIL,
        EMAIL_DELETE, EMAIL_SEND, EmailConnector, MockEmail, OutboundEmail, SendReceipt,
    };

    fn message(to: &str, subject: &str, body: &str) -> OutboundEmail {
        OutboundEmail {
            to: to.to_owned(),
            subject: subject.to_owned(),
            body: body.to_owned(),
        }
    }

    fn deliver<C: EmailConnector>(
        connector: &mut C,
        message: &OutboundEmail,
    ) -> Result<SendReceipt, C::Error> {
        connector.send(message)
    }

    #[test]
    fn classification_does_not_send() {
        let registry = ConnectorRegistry::phase3();
        let mut email = MockEmail::default();
        let stored = message("ada@example.com", "hi", "body");
        assert!(registry.invoke(EMAIL, EMAIL_SEND).is_err());
        assert!(email.outbox().is_empty());
        assert!(registry.authorize_confirmed(EMAIL, EMAIL_SEND).is_ok());
        assert!(email.outbox().is_empty());
        let receipt = email.send(&stored);
        assert_eq!(receipt.id, 1);
        assert_eq!(email.outbox(), &[stored]);
    }

    #[test]
    fn denied_and_unknown_do_not_authorize() {
        let registry = ConnectorRegistry::phase3();
        let email = MockEmail::default();
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
        assert_eq!(
            registry.authorize_confirmed("gmail", "send"),
            Err(ConnectorError::Unknown {
                connector: "gmail".to_owned(),
                action: "send".to_owned(),
            })
        );
        assert!(email.outbox().is_empty());
    }

    #[test]
    fn mock_email_satisfies_email_connector() {
        let stored = message("ada@example.com", "hello", "body");
        let mut email = MockEmail::default();
        let receipt = deliver(&mut email, &stored).expect("mock send");
        assert_eq!(receipt.id, 1);
        assert_eq!(email.outbox(), &[stored]);
    }
}
