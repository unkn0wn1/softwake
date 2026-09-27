//! Connector boundary.
//!
//! World I/O is a separate surface from the tool registry. [`ConnectorRegistry`]
//! classifies an action as confirm or deny. There is no safe world action.
//! [`invoke`](ConnectorRegistry::invoke) never sends or lists.
//! [`authorize_confirmed`](ConnectorRegistry::authorize_confirmed) does not send
//! or list either: it only reports that a confirm action may proceed.
//! [`MockEmail`] stores outbound messages in memory. [`LiveEmail`] is an opt-in
//! scaffold (off by default) that can store drafts after confirm and does not
//! open a socket. OAuth send builders are network-free; [`LiveEmail`] SMTP send
//! stays unwired. [`MockDrive`] and [`MockCalendar`] list entries stored on that
//! value. Default builds do not read credentials for connectors; the live email
//! password lives in the provider secret bag and is only checked as a bool here.

mod calendar;
mod calendar_api;
mod calendar_write_api;
mod drive;
mod drive_api;
mod email;
mod email_send_api;
mod email_settings;
mod inbox_api;
mod live_email;
mod mock;
mod registry;

pub use calendar::{CalendarConnector, CalendarEvent};
pub use calendar_api::{
    DEFAULT_CALENDAR_DAYS, DEFAULT_CALENDAR_MAX, LiveCalendarEvent, MAX_CALENDAR_MAX,
    clamp_calendar_days, clamp_calendar_max, format_calendar_event, format_calendar_list,
    google_event_get_url, google_events_url, graph_calendar_view_url, graph_event_get_url,
    parse_google_event, parse_google_events, parse_graph_event, parse_graph_events,
};
pub use calendar_write_api::{
    CalendarEventWrite, format_calendar_created, format_calendar_deleted, format_calendar_updated,
    google_event_create_url, google_event_delete_url, google_event_patch_url,
    google_event_write_body, graph_event_create_url, graph_event_delete_url, graph_event_patch_url,
    graph_event_write_body, parse_written_event_id,
};
pub use drive::{DriveConnector, DriveFile};
pub use drive_api::{
    DEFAULT_DRIVE_MAX, LiveDriveFile, MAX_DRIVE_MAX, MAX_DRIVE_TEXT_BYTES, clamp_drive_max,
    format_drive_file, format_drive_list, google_drive_export_text_url, google_drive_get_url,
    google_drive_list_url, google_drive_media_url, graph_approot_children_url,
    graph_approot_search_url, graph_drive_content_url, graph_drive_item_url, is_cheap_text_mime,
    parse_google_drive_file, parse_google_drive_list, parse_graph_drive_file,
    parse_graph_drive_list, truncate_drive_text,
};
pub use email::{EmailConnector, OutboundEmail, SendReceipt};
pub use email_send_api::{
    gmail_raw_rfc2822, gmail_send_body, gmail_send_url, graph_send_mail_body, graph_send_mail_url,
    parse_gmail_send_id,
};
pub use email_settings::{
    DEFAULT_SMTP_PORT, EMAIL_FILE_NAME, EmailSettings, EmailSettingsError, FileEmailSettings,
    LiveEmailMode, MAX_EMAIL_SETTINGS_BYTES, parse_live_email_mode, resolve_email_file,
    resolve_email_file_from,
};
pub use inbox_api::{
    DEFAULT_INBOX_MAX, InboxMessage, MAX_INBOX_MAX, clamp_inbox_max, format_inbox_list,
    format_inbox_message, gmail_get_url, gmail_list_url, graph_get_url, graph_list_url,
    parse_gmail_list, parse_gmail_message, parse_graph_list, parse_graph_message,
};
pub use live_email::{
    EmailBackend, LIVE_EMAIL_DISABLED, LIVE_EMAIL_NOT_CONFIGURED, LIVE_EMAIL_TEST_DRAFT_OK,
    LIVE_EMAIL_TEST_SEND_SCAFFOLD, LIVE_EMAIL_TRANSPORT_NOT_WIRED, LiveEmail, LiveEmailError,
};
pub use mock::{MockCalendar, MockDrive, MockEmail};
pub use registry::{
    CALENDAR, CALENDAR_CREATE, CALENDAR_DELETE, CALENDAR_GET, CALENDAR_LIST, CALENDAR_UPDATE,
    ConnectorError, ConnectorMeta, ConnectorRegistry, ConnectorRisk, DRIVE, DRIVE_DELETE,
    DRIVE_GET, DRIVE_LIST, DRIVE_SEARCH, EMAIL, EMAIL_DELETE, EMAIL_GET, EMAIL_LIST, EMAIL_SEARCH,
    EMAIL_SEND,
};

#[cfg(test)]
mod tests {
    use super::{
        CALENDAR, CALENDAR_CREATE, CALENDAR_DELETE, CALENDAR_LIST, CALENDAR_UPDATE,
        CalendarConnector, ConnectorError, ConnectorRegistry, DRIVE, DRIVE_DELETE, DRIVE_LIST,
        DriveConnector, EMAIL, EMAIL_DELETE, EMAIL_SEND, EmailConnector, MockCalendar, MockDrive,
        MockEmail, OutboundEmail, SendReceipt,
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
        for (connector, action) in [(EMAIL, EMAIL_DELETE), (DRIVE, DRIVE_DELETE)] {
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
        assert!(
            registry
                .authorize_confirmed(CALENDAR, CALENDAR_CREATE)
                .is_ok()
        );
        assert!(
            registry
                .authorize_confirmed(CALENDAR, CALENDAR_UPDATE)
                .is_ok()
        );
        assert!(
            registry
                .authorize_confirmed(CALENDAR, CALENDAR_DELETE)
                .is_ok()
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
