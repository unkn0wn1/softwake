//! In-memory snippet store.
//!
//! [`MockMemory`] keeps records on this value. It does not open a socket,
//! read credentials, share state with another value, or write a directory.

use crate::{MAX_TEXT_BYTES, Memory, MemoryError, MemoryId, Snippet};

/// Snippet stand-in. A new value is disabled and empty.
///
/// Ids start at 1 for each value and are not reused. Two mocks do not share
/// a counter or a record list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `MockMemory` is the public name of this backend.
pub struct MockMemory {
    enabled: bool,
    next_id: u64,
    records: Vec<Snippet>,
}

impl MockMemory {
    /// An opted-in value with no snippets.
    ///
    /// The first [`Self::remember`] returns id 1. [`Self::default`] is off.
    #[must_use]
    pub const fn enabled() -> Self {
        Self {
            enabled: true,
            next_id: 0,
            records: Vec::new(),
        }
    }

    /// Opt this value in.
    ///
    /// Existing snippets stay. The id counter is not reset. [`Self::disable`]
    /// is what clears snippets.
    pub const fn enable(&mut self) {
        self.enabled = true;
    }

    /// Turn this value off and drop its snippets.
    ///
    /// The id counter stays put, so a later [`Self::enable`] does not reuse
    /// an id this value already issued.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.records.clear();
    }

    /// Whether [`Self::remember`] can store text on this value.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Snippets accepted by [`Self::remember`], oldest first.
    ///
    /// Empty while this value is disabled, because [`Self::disable`] clears
    /// them and [`Self::default`] starts empty.
    #[must_use]
    pub fn records(&self) -> &[Snippet] {
        &self.records
    }

    /// Store `text` and return its id.
    ///
    /// Blank text is rejected before the length cap. Disabled is rejected
    /// before either check, so an off value does not describe the text.
    /// Accepted text is stored unchanged, including a duplicate of an
    /// existing snippet. The new copy gets a new id.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::Disabled`] when this value is off.
    /// Returns [`MemoryError::Empty`] when `text` is empty or whitespace.
    /// Returns [`MemoryError::TooLong`] when `text` exceeds [`MAX_TEXT_BYTES`].
    pub fn remember(&mut self, text: &str) -> Result<MemoryId, MemoryError> {
        self.require_enabled()?;
        if text.trim().is_empty() {
            return Err(MemoryError::Empty);
        }
        let len = text.len();
        if len > MAX_TEXT_BYTES {
            return Err(MemoryError::TooLong {
                len,
                max: MAX_TEXT_BYTES,
            });
        }
        self.next_id += 1;
        let id = MemoryId::from_raw(self.next_id);
        self.records.push(Snippet {
            id,
            text: text.to_owned(),
        });
        Ok(id)
    }

    /// Return snippets whose text contains `query`, oldest first.
    ///
    /// The match is case-sensitive and is not a regular expression. An empty
    /// `query` matches nothing: [`str::contains`] would accept every snippet
    /// for `""`, and a later session attach must not dump the store on a
    /// blank query.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::Disabled`] when this value is off.
    pub fn recall(&self, query: &str) -> Result<Vec<Snippet>, MemoryError> {
        self.require_enabled()?;
        if query.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .records
            .iter()
            .filter(|snippet| snippet.text.contains(query))
            .cloned()
            .collect())
    }

    /// Drop the snippet with `id`.
    ///
    /// The remaining snippets stay in insertion order.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::Disabled`] when this value is off, including
    /// when `id` was never issued. Returns [`MemoryError::Missing`] when
    /// this value is on and does not hold `id`.
    pub fn forget(&mut self, id: MemoryId) -> Result<(), MemoryError> {
        self.require_enabled()?;
        let Some(index) = self.records.iter().position(|snippet| snippet.id == id) else {
            return Err(MemoryError::Missing { id });
        };
        self.records.remove(index);
        Ok(())
    }

    /// Disabled wins over empty, too-long, and missing.
    fn require_enabled(&self) -> Result<(), MemoryError> {
        if self.enabled {
            Ok(())
        } else {
            Err(MemoryError::Disabled)
        }
    }
}

impl Memory for MockMemory {
    type Error = MemoryError;

    fn remember(&mut self, text: &str) -> Result<MemoryId, Self::Error> {
        MockMemory::remember(self, text)
    }

    fn recall(&self, query: &str) -> Result<Vec<Snippet>, Self::Error> {
        MockMemory::recall(self, query)
    }

    fn forget(&mut self, id: MemoryId) -> Result<(), Self::Error> {
        MockMemory::forget(self, id)
    }
}

#[cfg(test)]
mod tests {
    use super::MockMemory;
    use crate::{MAX_TEXT_BYTES, MemoryError, MemoryId, Snippet};

    #[test]
    fn default_is_disabled_and_stores_nothing() {
        let mut memory = MockMemory::default();
        let id = MemoryId::from_raw(1);
        assert!(!memory.is_enabled());
        assert_eq!(memory.remember("hi"), Err(MemoryError::Disabled));
        assert_eq!(memory.recall("hi"), Err(MemoryError::Disabled));
        assert_eq!(memory.forget(id), Err(MemoryError::Disabled));
        assert!(memory.records().is_empty());
        assert_eq!(MemoryError::Disabled.to_string(), "memory is disabled");
    }

    #[test]
    fn enabled_starts_empty() {
        let mut memory = MockMemory::enabled();
        let id = MemoryId::from_raw(1);
        assert!(memory.is_enabled());
        assert!(memory.recall("hi").expect("recall").is_empty());
        assert_eq!(memory.forget(id), Err(MemoryError::Missing { id }));
        assert!(memory.records().is_empty());
        assert_eq!(
            MemoryError::Missing { id }.to_string(),
            "memory id 1 is missing"
        );
    }

    #[test]
    fn remember_recall_forget_round_trip() {
        let mut memory = MockMemory::enabled();
        let first = memory.remember("alpha note").expect("first");
        let second = memory.remember("beta note").expect("second");
        assert_eq!(first, MemoryId::from_raw(1));
        assert_eq!(second, MemoryId::from_raw(2));
        assert_eq!(
            memory.recall("note").expect("both"),
            vec![
                Snippet {
                    id: first,
                    text: "alpha note".to_owned(),
                },
                Snippet {
                    id: second,
                    text: "beta note".to_owned(),
                },
            ]
        );
        assert_eq!(
            memory.recall("alpha").expect("alpha"),
            vec![Snippet {
                id: first,
                text: "alpha note".to_owned(),
            }]
        );
        assert!(memory.recall("nope").expect("miss").is_empty());
        assert!(memory.recall("").expect("blank query").is_empty());
        memory.forget(first).expect("forget first");
        assert_eq!(
            memory.recall("note").expect("remaining"),
            vec![Snippet {
                id: second,
                text: "beta note".to_owned(),
            }]
        );
        assert_eq!(
            memory.forget(first),
            Err(MemoryError::Missing { id: first })
        );
        assert_eq!(
            memory.records(),
            &[Snippet {
                id: second,
                text: "beta note".to_owned(),
            }]
        );
    }

    #[test]
    fn remember_rejects_blank_and_overlong() {
        let mut memory = MockMemory::enabled();
        let blank = " \n\t";
        let over = "a".repeat(MAX_TEXT_BYTES + 1);
        let exact = "a".repeat(MAX_TEXT_BYTES);
        assert_eq!(memory.remember(""), Err(MemoryError::Empty));
        assert_eq!(memory.remember(blank), Err(MemoryError::Empty));
        assert_eq!(MemoryError::Empty.to_string(), "memory text is empty");
        assert_eq!(
            memory.remember(&over),
            Err(MemoryError::TooLong {
                len: MAX_TEXT_BYTES + 1,
                max: MAX_TEXT_BYTES,
            })
        );
        assert_eq!(
            MemoryError::TooLong {
                len: MAX_TEXT_BYTES + 1,
                max: MAX_TEXT_BYTES,
            }
            .to_string(),
            format!(
                "memory text is {} bytes; max is {}",
                MAX_TEXT_BYTES + 1,
                MAX_TEXT_BYTES
            )
        );
        assert!(memory.records().is_empty());
        assert_eq!(
            memory.remember(&exact).expect("exact cap"),
            MemoryId::from_raw(1)
        );
        assert_eq!(memory.records().len(), 1);
        assert_eq!(memory.records()[0].text.len(), MAX_TEXT_BYTES);
    }

    #[test]
    fn text_stored_unchanged() {
        let mut memory = MockMemory::enabled();
        let text = "  café  ";
        let id = memory.remember(text).expect("remember");
        assert_eq!(
            memory.records(),
            &[Snippet {
                id,
                text: text.to_owned(),
            }]
        );
        assert_eq!(memory.recall("café").expect("hit").len(), 1);
        assert!(memory.recall("Café").expect("case").is_empty());
    }

    #[test]
    fn two_mocks_do_not_share_records() {
        let mut first = MockMemory::enabled();
        let mut second = MockMemory::enabled();
        let text = "local";
        assert_eq!(first.remember(text).expect("first"), MemoryId::from_raw(1));
        assert!(second.records().is_empty());
        assert_eq!(
            second.remember(text).expect("second"),
            MemoryId::from_raw(1)
        );
        assert_eq!(
            first.records(),
            &[Snippet {
                id: MemoryId::from_raw(1),
                text: text.to_owned(),
            }]
        );
        assert_eq!(second.records(), first.records());
    }

    #[test]
    fn disable_clears_and_ids_do_not_restart() {
        let mut memory = MockMemory::enabled();
        let id = memory.remember("kept").expect("remember");
        assert_eq!(id, MemoryId::from_raw(1));
        memory.disable();
        assert!(!memory.is_enabled());
        assert!(memory.records().is_empty());
        assert_eq!(memory.remember("next"), Err(MemoryError::Disabled));
        memory.enable();
        assert!(memory.is_enabled());
        assert!(memory.records().is_empty());
        assert_eq!(
            memory.remember("next").expect("re-enabled"),
            MemoryId::from_raw(2)
        );
        assert_eq!(memory.forget(id), Err(MemoryError::Missing { id }));
    }
}
