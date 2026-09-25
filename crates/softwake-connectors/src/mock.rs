//! In-memory connector backends for tests.
//!
//! [`MockEmail::send`] appends one message to an outbox on this value.
//! [`MockDrive::list`] and [`MockCalendar::list`] return entries stored on
//! that value. None of them open a socket, read credentials, or share state
//! with another mock.

use crate::{
    CalendarConnector, CalendarEvent, DriveConnector, DriveFile, EmailConnector, OutboundEmail,
    SendReceipt,
};

/// Mailbox stand-in. The outbox starts empty.
///
/// Send ids start at 1 for each value. Two mocks do not share a counter.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `MockEmail` is the public name of this backend.
pub struct MockEmail {
    next_id: u64,
    outbox: Vec<OutboundEmail>,
}

impl MockEmail {
    /// Messages accepted by [`MockEmail::send`], oldest first.
    #[must_use]
    pub fn outbox(&self) -> &[OutboundEmail] {
        &self.outbox
    }

    /// Append `message` and return its id.
    ///
    /// The id is `1` for the first send on this value and increases by 1.
    /// The stored message is a clone of `message`, including empty fields.
    #[must_use]
    pub fn send(&mut self, message: &OutboundEmail) -> SendReceipt {
        self.next_id += 1;
        self.outbox.push(message.clone());
        SendReceipt { id: self.next_id }
    }
}

impl EmailConnector for MockEmail {
    type Error = std::convert::Infallible;

    fn send(&mut self, message: &OutboundEmail) -> Result<SendReceipt, Self::Error> {
        Ok(MockEmail::send(self, message))
    }
}

/// Drive stand-in. The file list starts empty.
///
/// File ids start at 1 for each value. Two mocks do not share a counter.
/// [`MockDrive::insert`] is test setup on this value. It is not a registry action.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `MockDrive` is the public name of this backend.
pub struct MockDrive {
    next_id: u64,
    files: Vec<DriveFile>,
}

impl MockDrive {
    /// Append one file and return it.
    ///
    /// The id is `1` for the first insert on this value and increases by 1.
    /// The stored name is `name` unchanged, including an empty name.
    #[must_use]
    pub fn insert(&mut self, name: &str) -> DriveFile {
        self.next_id += 1;
        let file = DriveFile {
            id: self.next_id,
            name: name.to_owned(),
        };
        self.files.push(file.clone());
        file
    }

    /// Files accepted by [`MockDrive::insert`], oldest first.
    ///
    /// This clones the stored files. It does not increment the id counter.
    #[must_use]
    pub fn list(&self) -> Vec<DriveFile> {
        self.files.clone()
    }
}

impl DriveConnector for MockDrive {
    type Error = std::convert::Infallible;

    fn list(&self) -> Result<Vec<DriveFile>, Self::Error> {
        Ok(MockDrive::list(self))
    }
}

/// Calendar stand-in. The event list starts empty.
///
/// Event ids start at 1 for each value. Two mocks do not share a counter.
/// [`MockCalendar::insert`] is test setup on this value. It is not a registry action.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `MockCalendar` is the public name of this backend.
pub struct MockCalendar {
    next_id: u64,
    events: Vec<CalendarEvent>,
}

impl MockCalendar {
    /// Append one event and return it.
    ///
    /// The id is `1` for the first insert on this value and increases by 1.
    /// Title and start text are stored unchanged, including empty strings.
    /// Start text is not parsed as a date.
    #[must_use]
    pub fn insert(&mut self, title: &str, start: &str) -> CalendarEvent {
        self.next_id += 1;
        let event = CalendarEvent {
            id: self.next_id,
            title: title.to_owned(),
            start: start.to_owned(),
        };
        self.events.push(event.clone());
        event
    }

    /// Events accepted by [`MockCalendar::insert`], oldest first.
    ///
    /// This clones the stored events. It does not increment the id counter.
    #[must_use]
    pub fn list(&self) -> Vec<CalendarEvent> {
        self.events.clone()
    }
}

impl CalendarConnector for MockCalendar {
    type Error = std::convert::Infallible;

    fn list(&self) -> Result<Vec<CalendarEvent>, Self::Error> {
        Ok(MockCalendar::list(self))
    }
}

#[cfg(test)]
mod tests {
    use super::{MockCalendar, MockDrive, MockEmail};
    use crate::OutboundEmail;

    fn message(to: &str, subject: &str, body: &str) -> OutboundEmail {
        OutboundEmail {
            to: to.to_owned(),
            subject: subject.to_owned(),
            body: body.to_owned(),
        }
    }

    #[test]
    fn send_appends_in_order_and_ids_start_at_one() {
        let mut email = MockEmail::default();
        let first = message("a@example.com", "one", "1");
        let second = message("b@example.com", "two", "2");
        assert_eq!(email.send(&first).id, 1);
        assert_eq!(email.send(&second).id, 2);
        assert_eq!(email.outbox(), &[first, second]);
    }

    #[test]
    fn send_stores_fields_unchanged() {
        let mut email = MockEmail::default();
        let stored = message("", "café", "line");
        assert_eq!(email.send(&stored).id, 1);
        assert_eq!(email.outbox(), &[stored]);
    }

    #[test]
    fn two_mocks_do_not_share_an_outbox() {
        let mut first = MockEmail::default();
        let mut second = MockEmail::default();
        let stored = message("a@example.com", "one", "1");
        assert_eq!(first.send(&stored).id, 1);
        assert!(second.outbox().is_empty());
        assert_eq!(second.send(&stored).id, 1);
        assert_eq!(first.outbox(), std::slice::from_ref(&stored));
        assert_eq!(second.outbox(), std::slice::from_ref(&stored));
    }

    #[test]
    fn fresh_mocks_list_nothing() {
        assert!(MockDrive::default().list().is_empty());
        assert!(MockCalendar::default().list().is_empty());
    }

    #[test]
    fn drive_insert_orders_ids_and_keeps_names() {
        let mut drive = MockDrive::default();
        let first = drive.insert("");
        let second = drive.insert("café");
        assert_eq!(first.id, 1);
        assert_eq!(first.name, "");
        assert_eq!(second.id, 2);
        assert_eq!(second.name, "café");
        assert_eq!(drive.list(), vec![first.clone(), second.clone()]);
        assert_eq!(drive.list(), vec![first, second]);
    }

    #[test]
    fn calendar_insert_orders_ids_and_keeps_title_and_start() {
        let mut calendar = MockCalendar::default();
        let first = calendar.insert("", "");
        let second = calendar.insert("stand-up", "not-a-date");
        assert_eq!(first.id, 1);
        assert_eq!(first.title, "");
        assert_eq!(first.start, "");
        assert_eq!(second.id, 2);
        assert_eq!(second.title, "stand-up");
        assert_eq!(second.start, "not-a-date");
        assert_eq!(calendar.list(), vec![first.clone(), second.clone()]);
        assert_eq!(calendar.list(), vec![first, second]);
    }

    #[test]
    fn two_drive_mocks_do_not_share_files() {
        let mut first = MockDrive::default();
        let mut second = MockDrive::default();
        let stored = first.insert("notes");
        assert!(second.list().is_empty());
        let other = second.insert("other");
        assert_eq!(other.id, 1);
        assert_eq!(other.name, "other");
        assert_eq!(first.list(), vec![stored]);
        assert_eq!(second.list(), vec![other]);
    }

    #[test]
    fn two_calendar_mocks_do_not_share_events() {
        let mut first = MockCalendar::default();
        let mut second = MockCalendar::default();
        let stored = first.insert("stand-up", "Monday");
        assert!(second.list().is_empty());
        let other = second.insert("other", "Tuesday");
        assert_eq!(other.id, 1);
        assert_eq!(other.title, "other");
        assert_eq!(first.list(), vec![stored]);
        assert_eq!(second.list(), vec![other]);
    }
}
