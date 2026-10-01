//! Sentence-boundary early TTS during chat streams (ADR-0048 stream-feel).
//!
//! Speaks committed prefixes off the SSE reader via [`crate::announce::spawn_fixed_line`].
//! Tracks how many chars were already handed to TTS so the final speak only does the remainder.

/// Minimum chars in a sentence before we start TTS early.
const MIN_SENTENCE_CHARS: usize = 12;
/// Prefer waiting for at least this much total text before first early speak (avoids "Hi.").
const MIN_TOTAL_BEFORE_FIRST: usize = 24;

/// Return the next speakable end index (char count) after `already_spoken`, if any.
///
/// A boundary is `.`, `!`, or `?` followed by whitespace or end-of-string.
#[must_use]
pub(crate) fn next_speakable_end(text: &str, already_spoken: usize) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    if already_spoken >= len {
        return None;
    }
    if already_spoken == 0 && len < MIN_TOTAL_BEFORE_FIRST {
        return None;
    }
    let mut i = already_spoken;
    while i < len {
        let ch = chars[i];
        if matches!(ch, '.' | '!' | '?') {
            let sentence_len = i + 1 - already_spoken;
            let at_end = i + 1 == len;
            let followed_by_space = chars.get(i + 1).is_some_and(|c| c.is_whitespace());
            if sentence_len >= MIN_SENTENCE_CHARS && (at_end || followed_by_space) {
                let mut end = i + 1;
                while end < len && chars[end].is_whitespace() {
                    end += 1;
                }
                return Some(end);
            }
        }
        i += 1;
    }
    None
}

/// Slice `text` from char offset `from` (inclusive) to `to` (exclusive).
#[must_use]
pub(crate) fn slice_chars(text: &str, from: usize, to: usize) -> String {
    text.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect()
}

/// Drain all newly speakable sentence prefixes; returns updated spoken char count
/// and the joined text that should be spoken now (may be multiple sentences).
#[must_use]
pub(crate) fn take_new_speech(text: &str, already_spoken: usize) -> (usize, Option<String>) {
    let mut spoken = already_spoken;
    let mut chunks = Vec::new();
    while let Some(end) = next_speakable_end(text, spoken) {
        let piece = slice_chars(text, spoken, end);
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            chunks.push(trimmed.to_owned());
        }
        spoken = end;
    }
    if chunks.is_empty() {
        (already_spoken, None)
    } else {
        (spoken, Some(chunks.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::{next_speakable_end, take_new_speech};

    #[test]
    fn waits_for_min_total_before_first_sentence() {
        assert_eq!(next_speakable_end("Hi there.", 0), None);
        let long = "Hello there, this is Softwake speaking.";
        assert_eq!(next_speakable_end(long, 0), Some(long.chars().count()));
    }

    #[test]
    fn drains_two_sentences() {
        let text = "First sentence is long enough. Second sentence is also long enough.";
        let (spoken, chunk) = take_new_speech(text, 0);
        assert!(chunk.is_some());
        assert_eq!(spoken, text.chars().count());
        let (spoken2, chunk2) = take_new_speech(text, spoken);
        assert!(chunk2.is_none());
        assert_eq!(spoken2, spoken);
    }
}
