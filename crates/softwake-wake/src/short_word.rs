//! Threshold hint for bare short keywords.
//!
//! sherpa-onnx keyword spotting has no grammar and no word boundary. A short
//! registered word (`hi`, `sleep`, a short profile name) is harder to spot at
//! the global 0.25 threshold. [`is_short_single_word`] marks those phrases so
//! the encoder can append `#0.15` on that line only.
//!
//! Softwake does not drop a short hit because speech has been buffering.
//! A ~1.5 s suppress did that and cut real `sleep` commands during awake
//! chat, so it is gone. `#0.15` is not lowered further for `hi`.

/// True for one whitespace-free word of at most 8 characters.
#[must_use]
pub fn is_short_single_word(phrase: &str) -> bool {
    let trimmed = phrase.trim();
    !trimmed.is_empty() && !trimmed.chars().any(char::is_whitespace) && trimmed.chars().count() <= 8
}

#[cfg(test)]
mod tests {
    use super::is_short_single_word;

    #[test]
    fn short_words_are_bare_hi_sleep_and_names_not_phrases() {
        assert!(is_short_single_word("hi"));
        assert!(is_short_single_word("sleep"));
        assert!(is_short_single_word("sally"));
        assert!(is_short_single_word("softwake"));
        assert!(!is_short_single_word("deep sleep"));
        assert!(!is_short_single_word("hey sally"));
        assert!(!is_short_single_word("go to sleep"));
        assert!(!is_short_single_word("softwakes"));
        assert!(!is_short_single_word(""));
    }

    /// Length only selects the `#0.15` encode hint. It does not change at the
    /// old 1.5 s suppress point (`24_000` samples) or across a long awake turn
    /// (8 s). The daemon observe path must not turn these into `None`.
    #[test]
    fn short_sleep_stays_a_short_word_at_conversational_lengths() {
        // 1.5 s was the withdrawn suppress point. 8 s is a long awake turn.
        let conversational = [16_000_u64 * 3 / 2, 16_000 * 8];
        assert_eq!(conversational, [24_000, 128_000]);
        for speech_samples in conversational {
            assert!(is_short_single_word("sleep"), "{speech_samples}");
            assert!(is_short_single_word("hi"), "{speech_samples}");
            assert!(!is_short_single_word("go to sleep"), "{speech_samples}");
            assert!(!is_short_single_word("deep sleep"), "{speech_samples}");
            assert!(!is_short_single_word("hey sally"), "{speech_samples}");
        }
    }
}
