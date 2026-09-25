//! Drive file types and the [`DriveConnector`] trait.

/// One file stored on a single drive value.
///
/// The strings are stored unchanged. Nothing in this crate parses a name,
/// trims it, or treats it as a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveFile {
    /// `1` for the first insert on that value, then `2`, and so on.
    pub id: u64,
    /// Display name. Stored unchanged. Not a path and not validated.
    pub name: String,
}

/// List files through a backend.
///
/// The registry decides whether a caller may list. This trait only returns
/// the files already stored. It does not open a socket by itself; a backend
/// that talks to a remote account would, and the default mock does not.
pub trait DriveConnector {
    /// Backend failure while listing files.
    type Error: std::error::Error;

    /// Files on this backend, oldest first.
    ///
    /// An empty backend returns an empty vec. Listing does not remove files.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the files cannot be read.
    fn list(&self) -> Result<Vec<DriveFile>, Self::Error>;
}
