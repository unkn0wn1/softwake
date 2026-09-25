//! Calendar event types and the [`CalendarConnector`] trait.

/// One event stored on a single calendar value.
///
/// The strings are stored unchanged. Nothing in this crate parses a title,
/// trims the fields, or treats [`CalendarEvent::start`] as a timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    /// `1` for the first insert on that value, then `2`, and so on.
    pub id: u64,
    /// Title. Stored unchanged. Not validated.
    pub title: String,
    /// When-text. Stored unchanged. Not parsed as a date or a timezone.
    pub start: String,
}

/// List events through a backend.
///
/// The registry decides whether a caller may list. This trait only returns
/// the events already stored. It does not open a socket by itself; a backend
/// that talks to a remote account would, and the default mock does not.
pub trait CalendarConnector {
    /// Backend failure while listing events.
    type Error: std::error::Error;

    /// Events on this backend, oldest first.
    ///
    /// An empty backend returns an empty vec. Listing does not remove events.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the events cannot be read.
    fn list(&self) -> Result<Vec<CalendarEvent>, Self::Error>;
}
