//! Sentence-boundary early TTS during chat streams (ADR-0048 stream-feel).
//!
//! Speaks committed prefixes off the SSE reader via [`crate::announce::spawn_fixed_line`].
//! Tracks how many chars were already handed to TTS so the final speak only does the remainder.

/// Minimum chars in a sentence before we start TTS early.
const MIN_SENTENCE_CHARS: usize = 12;
/// Prefer waiting for at least this much total text before first early speak (avoids "Hi.").
const MIN_TOTAL_BEFORE_FIRST: usize = 24;
/// Words this short before `.` are treated as abbreviations (Dr./Mr./Ms./…).
const ABBREV_MAX_WORD_CHARS: usize = 3;

/// Return the next speakable end index (char count) after `already_spoken`, if any.
///
/// A boundary is `.`, `!`, or `?` followed by whitespace (confirmed separator).
/// Bare end-of-string after `.` is **not** enough mid-stream — that leftover is
/// spoken as remainder in [`crate::runtime`] finish. `!`/`?` at end-of-string
/// still commit (exclamation/question usually complete).
///
/// For `.`, the next non-whitespace must be uppercase (or absent after spaces),
/// and the word immediately before `.` must be longer than [`ABBREV_MAX_WORD_CHARS`]
/// so "Dr. Smith" / "U.S. " do not fire mid-clause.
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
            if sentence_len < MIN_SENTENCE_CHARS {
                i += 1;
                continue;
            }
            let is_boundary = match ch {
                '!' | '?' => followed_by_space || at_end,
                '.' => {
                    if !followed_by_space {
                        // Never commit a period at bare EOS mid-stream.
                        false
                    } else if abbrev_word_before(&chars, i) {
                        false
                    } else {
                        // Next non-ws must be uppercase or there is no more text.
                        let mut j = i + 1;
                        while j < len && chars[j].is_whitespace() {
                            j += 1;
                        }
                        j >= len || chars[j].is_uppercase()
                    }
                }
                _ => false,
            };
            if is_boundary {
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

/// True when the alphabetic run immediately before `dot_idx` is a short abbreviation.
fn abbrev_word_before(chars: &[char], dot_idx: usize) -> bool {
    let mut start = dot_idx;
    while start > 0 {
        let prev = chars[start - 1];
        if prev.is_alphabetic() {
            start -= 1;
        } else {
            break;
        }
    }
    let word_len = dot_idx.saturating_sub(start);
    word_len > 0 && word_len <= ABBREV_MAX_WORD_CHARS
}

/// Slice `text` from char offset `from` (inclusive) to `to` (exclusive).
#[must_use]
pub(crate) fn slice_chars(text: &str, from: usize, to: usize) -> String {
    text.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect()
}

/// When a new tool-loop round starts, `partial` resets and may be shorter than
/// the previous round's spoken offset — clamp so we never skip the new reply.
#[must_use]
pub(crate) fn clamp_spoken_to_text(text: &str, already_spoken: usize) -> usize {
    let len = text.chars().count();
    if already_spoken > len {
        0
    } else {
        already_spoken
    }
}

/// Drain all newly speakable sentence prefixes; returns updated spoken char count
/// and the joined text that should be spoken now (may be multiple sentences).
#[must_use]
pub(crate) fn take_new_speech(text: &str, already_spoken: usize) -> (usize, Option<String>) {
    let mut spoken = clamp_spoken_to_text(text, already_spoken);
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
        (clamp_spoken_to_text(text, already_spoken), None)
    } else {
        (spoken, Some(chunks.join(" ")))
    }
}

/// Remainder after early TTS (`spoken` char offset), trimmed. Empty when done.
#[must_use]
pub(crate) fn remainder_after(text: &str, spoken: usize) -> String {
    let spoken = clamp_spoken_to_text(text, spoken);
    slice_chars(text, spoken, text.chars().count())
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{clamp_spoken_to_text, next_speakable_end, remainder_after, take_new_speech};

    #[test]
    fn waits_for_min_total_before_first_sentence() {
        assert_eq!(next_speakable_end("Hi there.", 0), None);
        let long = "Hello there, this is Softwake speaking. ";
        assert_eq!(next_speakable_end(long, 0), Some(long.chars().count()));
    }

    #[test]
    fn does_not_commit_period_at_bare_end() {
        let text = "Hello there, this is Softwake speaking.";
        assert_eq!(next_speakable_end(text, 0), None);
        assert_eq!(remainder_after(text, 0), text);
    }

    #[test]
    fn skips_short_abbreviation_before_name() {
        let text = "Good morning Dr. Smith is visiting today. ";
        // Must not end after "Dr."
        let end = next_speakable_end(text, 0).expect("full sentence");
        assert_eq!(end, text.chars().count());
        let spoken = slice_for_assert(text, 0, end);
        assert!(spoken.contains("Smith"));
    }

    #[test]
    fn drains_two_sentences_with_spaces() {
        let text = "First sentence is long enough. Second sentence is also long enough. ";
        let (spoken, chunk) = take_new_speech(text, 0);
        assert!(chunk.is_some());
        assert_eq!(spoken, text.chars().count());
        let (spoken2, chunk2) = take_new_speech(text, spoken);
        assert!(chunk2.is_none());
        assert_eq!(spoken2, spoken);
    }

    #[test]
    fn clamp_resets_when_new_round_partial_is_shorter() {
        assert_eq!(clamp_spoken_to_text("Hi", 40), 0);
        assert_eq!(clamp_spoken_to_text("Hello world", 5), 5);
        assert_eq!(
            remainder_after("Final answer here", 40),
            "Final answer here"
        );
    }

    #[test]
    fn question_at_end_still_commits() {
        let text = "Would you like me to continue with that?";
        assert!(text.chars().count() >= 24);
        assert_eq!(next_speakable_end(text, 0), Some(text.chars().count()));
    }

    fn slice_for_assert(text: &str, from: usize, to: usize) -> String {
        text.chars().skip(from).take(to - from).collect()
    }
}
