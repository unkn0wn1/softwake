//! Best-effort gate for bare short keywords.
//!
//! sherpa-onnx keyword spotting has no grammar and no word boundary. A short
//! registered word (`hi`, `sleep`, a short profile name) can fire inside a
//! longer utterance. This module does not parse sentences. It only says when
//! a keyword surface is a single short word, and when that hit should be
//! dropped because speech has already been buffering for a while.

/// Samples of continuous speech (16 kHz) after which a short keyword is ignored.
///
/// One and a half seconds is longer than an isolated `hi` or `sleep`, and
/// shorter than a typical awake sentence. Multi-word phrases are never
/// suppressed by this gate.
pub const SHORT_SUPPRESS_AFTER_SAMPLES: u64 = 16_000 * 3 / 2;

/// True for one whitespace-free word of at most 8 characters.
#[must_use]
pub fn is_short_single_word(phrase: &str) -> bool {
    let trimmed = phrase.trim();
    !trimmed.is_empty() && !trimmed.chars().any(char::is_whitespace) && trimmed.chars().count() <= 8
}

/// Whether `keyword` is a short single word that should be ignored.
///
/// `speech_samples` is how much speech is already buffered (free speech or
/// press-to-talk). Below [`SHORT_SUPPRESS_AFTER_SAMPLES`] the hit stands.
/// A `@tag`, a `word_word` tag, or a sherpa line that contains `@tag` is
/// reduced to that tag before the length check, so `deep_sleep` and
/// `hey_sally` stay multi-word.
#[must_use]
pub fn suppress_short_keyword(keyword: &str, speech_samples: u64) -> bool {
    if speech_samples < SHORT_SUPPRESS_AFTER_SAMPLES {
        return false;
    }
    is_short_single_word(&keyword_surface(keyword))
}

/// Keyword tag sherpa would report, folded to lowercase words.
fn keyword_surface(keyword: &str) -> String {
    let folded = keyword.trim().to_ascii_lowercase();
    let tag = folded
        .split_whitespace()
        .find(|part| part.starts_with('@'))
        .unwrap_or(folded.as_str());
    tag.trim_start_matches('@').replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::{SHORT_SUPPRESS_AFTER_SAMPLES, is_short_single_word, suppress_short_keyword};

    #[test]
    fn one_and_a_half_seconds_at_16khz() {
        assert_eq!(SHORT_SUPPRESS_AFTER_SAMPLES, 24_000);
        assert_eq!(SHORT_SUPPRESS_AFTER_SAMPLES, 16_000 * 3 / 2);
    }

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

    #[test]
    fn short_keyword_is_suppressed_only_after_the_buffer_is_long() {
        assert!(!suppress_short_keyword(
            "hi",
            SHORT_SUPPRESS_AFTER_SAMPLES - 1
        ));
        assert!(suppress_short_keyword(
            "sleep",
            SHORT_SUPPRESS_AFTER_SAMPLES
        ));
        assert!(suppress_short_keyword("@hi", SHORT_SUPPRESS_AFTER_SAMPLES));
        assert!(suppress_short_keyword(
            "▁H I @sleep #0.15",
            SHORT_SUPPRESS_AFTER_SAMPLES
        ));
        assert!(!suppress_short_keyword(
            "@deep_sleep",
            SHORT_SUPPRESS_AFTER_SAMPLES
        ));
        assert!(!suppress_short_keyword(
            "@hey_sally",
            SHORT_SUPPRESS_AFTER_SAMPLES
        ));
        assert!(!suppress_short_keyword(
            "go to sleep",
            SHORT_SUPPRESS_AFTER_SAMPLES
        ));
        assert!(!suppress_short_keyword(
            "scripted-wake",
            SHORT_SUPPRESS_AFTER_SAMPLES
        ));
    }
}
