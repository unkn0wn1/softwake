//! [`Memory`] and the values it stores.
//!
//! Opt-in belongs to the backend value. This module does not read a config
//! file and does not open a socket.

/// Longest snippet [`Memory::remember`] will store, in UTF-8 bytes.
///
/// A snippet is attached beside instructions, so the cap is smaller than a
/// soul file. Longer text is rejected before it is stored.
pub const MAX_TEXT_BYTES: usize = 8 * 1024;

/// One stored snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// Id from the [`Memory::remember`] that stored [`Self::text`].
    pub id: MemoryId,
    /// Text stored unchanged.
    pub text: String,
}

/// Identifier for one snippet on a single memory value.
///
/// The first successful remember on that value is 1, then 2. Ids are not
/// reused after forget or disable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryId(u64);

impl MemoryId {
    /// Rebuild an id from the number a previous remember returned.
    ///
    /// This does not check that a store issued `raw`.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Numeric id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for MemoryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Failure from [`crate::MockMemory`].
///
/// A later backend uses its own error type on [`Memory::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[allow(clippy::module_name_repetitions)] // `MemoryError` is the public name of this failure.
pub enum MemoryError {
    /// The value is off. No snippet was read or written.
    #[error("memory is disabled")]
    Disabled,

    /// Remember was given empty or whitespace-only text.
    #[error("memory text is empty")]
    Empty,

    /// Remember was given text longer than [`MAX_TEXT_BYTES`].
    #[error("memory text is {len} bytes; max is {max}")]
    TooLong {
        /// UTF-8 byte length of the rejected text.
        len: usize,
        /// Cap that was exceeded.
        max: usize,
    },

    /// Forget named an id this value did not hold.
    #[error("memory id {id} is missing")]
    Missing {
        /// Id that was not in the store.
        id: MemoryId,
    },
}

/// Store, search, and drop text snippets.
///
/// The backend value decides whether memory is on. This trait does not take
/// a path, does not open a socket, and does not embed a vector index.
#[allow(clippy::module_name_repetitions)] // `Memory` is the public name of this trait.
pub trait Memory {
    /// Failure from this backend.
    type Error: std::error::Error;

    /// Store `text` and return its id on this value.
    ///
    /// # Errors
    ///
    /// Returns the backend error when `text` is rejected or the backend is off.
    fn remember(&mut self, text: &str) -> Result<MemoryId, Self::Error>;

    /// Return snippets whose text contains `query`.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the backend is off.
    fn recall(&self, query: &str) -> Result<Vec<Snippet>, Self::Error>;

    /// Drop the snippet identified by `id`.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the backend is off or `id` is absent.
    fn forget(&mut self, id: MemoryId) -> Result<(), Self::Error>;
}
