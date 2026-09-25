//! Connector boundary.
//!
//! World I/O is a separate surface from the tool registry. [`ConnectorRegistry`]
//! classifies an action as confirm or deny. There is no safe world action.
//! [`invoke`](ConnectorRegistry::invoke) never sends or lists.
//! [`authorize_confirmed`](ConnectorRegistry::authorize_confirmed) does not send
//! or list either: it only reports that a confirm action may proceed.
//! [`MockEmail`] stores outbound messages in memory. [`LiveEmail`] is an opt-in
//! scaffold (off by default) that can store drafts after confirm and does not
//! open a socket. [`MockDrive`] and [`MockCalendar`] list entries stored on that
//! value. Default builds do not read credentials for connectors; the live email
//! password lives in the provider secret bag and is only checked as a bool here.

mod calendar;
mod drive;
mod email;
mod email_settings;
mod live_email;
mod mock;
mod registry;

pub use calendar::{CalendarConnector, CalendarEvent};
pub use drive::{DriveConnector, DriveFile};
pub use email::{EmailConnector, OutboundEmail, SendReceipt};
pub use email_settings::{
    DEFAULT_SMTP_PORT, EMAIL_FILE_NAME, EmailSettings, EmailSettingsError, FileEmailSettings,
    LiveEmailMode, MAX_EMAIL_SETTINGS_BYTES, parse_live_email_mode, resolve_email_file,
    resolve_email_file_from,
};
pub use live_email::{
    EmailBackend, LIVE_EMAIL_DISABLED, LIVE_EMAIL_NOT_CONFIGURED, LIVE_EMAIL_TEST_DRAFT_OK,
    LIVE_EMAIL_TEST_SEND_SCAFFOLD, LIVE_EMAIL_TRANSPORT_NOT_WIRED, LiveEmail, LiveEmailError,
};
pub use mock::{MockCalendar, MockDrive, MockEmail};
pub use registry::{
    CALENDAR, CALENDAR_DELETE, CALENDAR_LIST, ConnectorError, ConnectorMeta, ConnectorRegistry,
    ConnectorRisk, DRIVE, DRIVE_DELETE, DRIVE_LIST, EMAIL, EMAIL_DELETE, EMAIL_SEND,
};

#[cfg(test)]
mod tests {
    use super::{
        CALENDAR, CALENDAR_DELETE, CALENDAR_LIST, CalendarConnector, ConnectorError,
        ConnectorRegistry, DRIVE, DRIVE_DELETE, DRIVE_LIST, DriveConnector, EMAIL, EMAIL_DELETE,
        EMAIL_SEND, EmailConnector, MockCalendar, MockDrive, MockEmail, OutboundEmail, SendReceipt,
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

    fn list_files<C: DriveConnector>(connector: &C) -> Result<Vec<super::DriveFile>, C::Error> {
        connector.list()
    }

    fn list_events<C: CalendarConnector>(
        connector: &C,
    ) -> Result<Vec<super::CalendarEvent>, C::Error> {
        connector.list()
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
            (DRIVE, DRIVE_DELETE),
            (CALENDAR, CALENDAR_DELETE),
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

    #[test]
    fn classification_does_not_list() {
        let registry = ConnectorRegistry::phase3();
        let mut drive = MockDrive::default();
        let file = drive.insert("notes");
        assert!(registry.invoke(DRIVE, DRIVE_LIST).is_err());
        assert_eq!(drive.list(), vec![file.clone()]);
        assert!(registry.authorize_confirmed(DRIVE, DRIVE_LIST).is_ok());
        assert_eq!(drive.list(), vec![file.clone()]);
        assert_eq!(
            registry.authorize_confirmed(DRIVE, DRIVE_DELETE),
            Err(ConnectorError::Denied {
                connector: DRIVE.to_owned(),
                action: DRIVE_DELETE.to_owned(),
            })
        );
        assert_eq!(drive.list(), vec![file]);

        let mut calendar = MockCalendar::default();
        let event = calendar.insert("stand-up", "Monday");
        assert!(registry.invoke(CALENDAR, CALENDAR_LIST).is_err());
        assert_eq!(calendar.list(), vec![event.clone()]);
        assert!(
            registry
                .authorize_confirmed(CALENDAR, CALENDAR_LIST)
                .is_ok()
        );
        assert_eq!(calendar.list(), vec![event.clone()]);
        assert_eq!(
            registry.authorize_confirmed(CALENDAR, CALENDAR_DELETE),
            Err(ConnectorError::Denied {
                connector: CALENDAR.to_owned(),
                action: CALENDAR_DELETE.to_owned(),
            })
        );
        assert_eq!(calendar.list(), vec![event]);
    }

    #[test]
    fn mock_drive_satisfies_drive_connector() {
        let mut drive = MockDrive::default();
        let stored = drive.insert("notes");
        let listed = list_files(&drive).expect("mock list");
        assert_eq!(listed, vec![stored]);
    }

    #[test]
    fn mock_calendar_satisfies_calendar_connector() {
        let mut calendar = MockCalendar::default();
        let stored = calendar.insert("stand-up", "Monday");
        let listed = list_events(&calendar).expect("mock list");
        assert_eq!(listed, vec![stored]);
    }
}
