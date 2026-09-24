//! Local phrase table for the phase-1 spike.
//!
//! Matching is a case-folded substring check on one UTF-8 window. It is not an
//! on-device wake-word model. ADR 0002 records that limit.

use crate::PhraseHit;

/// Rejected phrase-table configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PhraseTableError {
    /// A phrase was empty or only whitespace.
    ///
    /// An empty phrase would match every window via `contains("")`.
    #[error("phrase must contain non-whitespace characters")]
    EmptyPhrase,
}

/// Configured wake and sleep phrases, stored trimmed and lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhraseTable {
    wake_phrases: Vec<String>,
    sleep_phrases: Vec<String>,
}

impl PhraseTable {
    /// Build a table.
    ///
    /// Each phrase is trimmed and stored in lowercase. Empty lists are
    /// allowed: that side never matches.
    ///
    /// # Errors
    ///
    /// Returns [`PhraseTableError::EmptyPhrase`] when any phrase is empty
    /// after trimming.
    pub fn new<W, S, T, U>(wake_phrases: W, sleep_phrases: S) -> Result<Self, PhraseTableError>
    where
        W: IntoIterator<Item = T>,
        S: IntoIterator<Item = U>,
        T: AsRef<str>,
        U: AsRef<str>,
    {
        Ok(Self {
            wake_phrases: normalize(wake_phrases)?,
            sleep_phrases: normalize(sleep_phrases)?,
        })
    }

    /// Configured wake phrases, in order, already lowercase.
    #[must_use]
    pub fn wake_phrases(&self) -> &[String] {
        &self.wake_phrases
    }

    /// Configured sleep phrases, in order, already lowercase.
    #[must_use]
    pub fn sleep_phrases(&self) -> &[String] {
        &self.sleep_phrases
    }

    /// First wake phrase. The daemon demo submits this text for `wake`.
    #[must_use]
    pub fn primary_wake_phrase(&self) -> Option<&str> {
        self.wake_phrases.first().map(String::as_str)
    }

    /// First sleep phrase. The daemon demo submits this text for `sleep`.
    #[must_use]
    pub fn primary_sleep_phrase(&self) -> Option<&str> {
        self.sleep_phrases.first().map(String::as_str)
    }

    /// Score one UTF-8 window.
    ///
    /// The window is trimmed and lowercased. A phrase hits when the window
    /// contains it. The longest hit wins. When a wake phrase and a sleep
    /// phrase tie on length, the result is [`PhraseHit::Sleep`], so a sleep
    /// phrase is not swallowed by an equal wake phrase.
    #[must_use]
    pub fn score(&self, window: &str) -> PhraseHit {
        let folded = window.trim().to_lowercase();
        let wake_len = longest_hit(&self.wake_phrases, &folded);
        let sleep_len = longest_hit(&self.sleep_phrases, &folded);
        match (wake_len, sleep_len) {
            (None, None) => PhraseHit::None,
            (Some(wake), Some(sleep)) => {
                if wake > sleep {
                    PhraseHit::Wake
                } else {
                    PhraseHit::Sleep
                }
            }
            (Some(_), None) => PhraseHit::Wake,
            (None, Some(_)) => PhraseHit::Sleep,
        }
    }
}

impl Default for PhraseTable {
    /// Wake phrases `hey softwake` and `softwake`.
    /// Sleep phrases `softwake sleep` and `go to sleep`.
    ///
    /// # Panics
    ///
    /// Panics if those built-in phrases fail validation. They are non-empty.
    fn default() -> Self {
        Self::new(
            ["hey softwake", "softwake"],
            ["softwake sleep", "go to sleep"],
        )
        .expect("built-in phrases are non-empty")
    }
}

/// Scores UTF-8 text windows with a [`PhraseTable`].
///
/// Each [`TextWakeDetector::push_text`] call is independent. This spike does
/// not keep a rolling audio buffer and does not implement [`crate::WakeDetector`]:
/// PCM samples belong to the on-device engine chosen in ADR 0006.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `TextWakeDetector` is the public name of the text spike.
pub struct TextWakeDetector {
    table: PhraseTable,
}

impl TextWakeDetector {
    /// Score later windows with `table`.
    #[must_use]
    pub const fn new(table: PhraseTable) -> Self {
        Self { table }
    }

    /// Phrase table this detector scores against.
    #[must_use]
    pub const fn table(&self) -> &PhraseTable {
        &self.table
    }

    /// Score one UTF-8 window. See [`PhraseTable::score`].
    #[must_use]
    pub fn push_text(&self, window: &str) -> PhraseHit {
        self.table.score(window)
    }
}

fn normalize<I, S>(phrases: I) -> Result<Vec<String>, PhraseTableError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut normalized = Vec::new();
    for phrase in phrases {
        let trimmed = phrase.as_ref().trim();
        if trimmed.is_empty() {
            return Err(PhraseTableError::EmptyPhrase);
        }
        normalized.push(trimmed.to_lowercase());
    }
    Ok(normalized)
}

fn longest_hit(phrases: &[String], window: &str) -> Option<usize> {
    phrases
        .iter()
        .map(String::as_str)
        .filter(|phrase| !phrase.is_empty() && window.contains(phrase))
        .map(str::len)
        .max()
}

#[cfg(test)]
mod tests {
    use super::{PhraseTable, PhraseTableError, TextWakeDetector};
    use crate::PhraseHit;

    fn detector() -> TextWakeDetector {
        TextWakeDetector::new(PhraseTable::default())
    }

    #[test]
    fn wake_phrase_hits() {
        let detector = detector();
        assert_eq!(detector.push_text("hey softwake"), PhraseHit::Wake);
        assert_eq!(detector.push_text("please softwake now"), PhraseHit::Wake);
    }

    #[test]
    fn sleep_phrase_hits() {
        let detector = detector();
        assert_eq!(detector.push_text("softwake sleep"), PhraseHit::Sleep);
        assert_eq!(detector.push_text("go to sleep"), PhraseHit::Sleep);
    }

    #[test]
    fn unrelated_text_does_not_hit() {
        let detector = detector();
        assert_eq!(detector.push_text("hello there"), PhraseHit::None);
        assert_eq!(detector.push_text(""), PhraseHit::None);
        assert_eq!(detector.push_text("   "), PhraseHit::None);
        assert_eq!(detector.push_text("hey"), PhraseHit::None);
    }

    #[test]
    fn case_folding_matches_wake_and_sleep() {
        let detector = detector();
        assert_eq!(detector.push_text("HeY SoFtWaKe"), PhraseHit::Wake);
        assert_eq!(detector.push_text("  GO TO SLEEP "), PhraseHit::Sleep);
        assert_eq!(detector.push_text("SOFTWAKE SLEEP"), PhraseHit::Sleep);
    }

    #[test]
    fn configured_phrases_are_case_folded() {
        let table = PhraseTable::new(["HeY SoFtWaKe"], ["Go To Sleep"]).expect("phrases");
        let detector = TextWakeDetector::new(table);
        assert_eq!(detector.push_text("hey softwake"), PhraseHit::Wake);
        assert_eq!(detector.push_text("go to sleep"), PhraseHit::Sleep);
    }

    #[test]
    fn sleep_phrase_that_contains_a_wake_phrase_scores_as_sleep() {
        let detector = detector();
        assert_eq!(detector.push_text("softwake sleep"), PhraseHit::Sleep);
        assert_eq!(
            detector.push_text("please SOFTWAKE SLEEP now"),
            PhraseHit::Sleep
        );
    }

    #[test]
    fn longer_wake_phrase_beats_a_shorter_sleep_phrase() {
        let table = PhraseTable::new(["hello softwake"], ["softwake"]).expect("phrases");
        let detector = TextWakeDetector::new(table);
        assert_eq!(detector.push_text("hello softwake"), PhraseHit::Wake);
    }

    #[test]
    fn equal_length_tie_prefers_sleep() {
        let table = PhraseTable::new(["abcd"], ["wxyz"]).expect("phrases");
        let detector = TextWakeDetector::new(table);
        assert_eq!(detector.push_text("abcd wxyz"), PhraseHit::Sleep);
    }

    #[test]
    fn empty_phrase_is_rejected() {
        let error = PhraseTable::new(["hey softwake", " "], ["go to sleep"]).expect_err("blank");
        assert_eq!(error, PhraseTableError::EmptyPhrase);
        assert_eq!(
            error.to_string(),
            "phrase must contain non-whitespace characters"
        );
    }

    #[test]
    fn empty_lists_never_match() {
        let table = PhraseTable::new(std::iter::empty::<&str>(), std::iter::empty::<&str>())
            .expect("empty lists");
        assert_eq!(table.primary_wake_phrase(), None);
        assert_eq!(table.primary_sleep_phrase(), None);
        let detector = TextWakeDetector::new(table);
        assert_eq!(detector.push_text("hey softwake"), PhraseHit::None);
    }

    #[test]
    fn default_phrases_match_the_voice_state_examples() {
        let table = PhraseTable::default();
        assert_eq!(
            table.wake_phrases(),
            ["hey softwake".to_owned(), "softwake".to_owned()]
        );
        assert_eq!(
            table.sleep_phrases(),
            ["softwake sleep".to_owned(), "go to sleep".to_owned()]
        );
        assert_eq!(table.primary_wake_phrase(), Some("hey softwake"));
        assert_eq!(table.primary_sleep_phrase(), Some("softwake sleep"));
    }

    #[test]
    fn windows_are_independent() {
        let detector = detector();
        assert_eq!(detector.push_text("hey softwake"), PhraseHit::Wake);
        assert_eq!(detector.push_text("hello"), PhraseHit::None);
        assert_eq!(detector.push_text("go to sleep"), PhraseHit::Sleep);
        assert_eq!(detector.push_text("hey softwake"), PhraseHit::Wake);
    }
}
