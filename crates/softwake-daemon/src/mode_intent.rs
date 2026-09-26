//! Heuristic sleep, hibernate, yes/no, and fuzzy-wake classification.
//!
//! Keyword spotting stays one-shot. This module only labels text that already
//! arrived on the ask path, or a probe keyword that did not fire.

use softwake_wake::{PhraseHit, hit_from_keyword};

/// Awake ask text that means sleep or hibernate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModeIntent {
    /// Enter sleep after a yes.
    Sleep,
    /// Enter hibernate after a yes.
    Hibernate,
}

/// Reply while a mode confirm is waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmReply {
    /// Apply the pending transition.
    Yes,
    /// Stay in the current state.
    No,
    /// Not a clear yes or no.
    Unclear,
}

const QUESTION_STEMS: &[&str] = &["who", "what", "why", "how", "when", "where", "which"];

const NEGATIONS: &[&str] = &["not", "dont", "never"];

/// Dropped before the remaining words must match a mode pattern.
///
/// `thats` is here so "that's enough" collapses to `enough`.
const FILLERS: &[&str] = &[
    "please", "now", "just", "okay", "ok", "um", "uh", "hey", "the", "a", "an", "to", "into",
    "for", "me", "my", "myself", "yourself", "you", "your", "can", "could", "would", "will",
    "want", "i", "im", "am", "oh", "so", "well", "and", "then", "thats",
];

const SLEEP_PATTERNS: &[&str] = &[
    "sleep",
    "go sleep",
    "put sleep",
    "bed",
    "go bed",
    "goodnight",
    "done",
    "done bit",
    "enough",
    "time sleep",
    "going sleep",
];

/// Trailing soft closers / duration words stripped after fillers, so
/// "go to sleep for a little while" collapses to `go sleep`.
const DURATION_TAILS: &[&str] = &[
    "little", "while", "bit", "awhile", "moment", "second", "seconds", "minute", "minutes", "hour",
    "hours", "nap", "break", "rest", "few",
];

const YES_LINES: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "ya",
    "sure",
    "ok",
    "okay",
    "please",
    "please do",
    "do it",
    "go ahead",
    "confirm",
    "correct",
    "right",
    "absolutely",
    "fine",
];

const NO_LINES: &[&str] = &[
    "no",
    "nope",
    "nah",
    "no thanks",
    "cancel",
    "stop",
    "dont",
    "never mind",
    "nevermind",
    "stay awake",
    "stay asleep",
    "stay sleeping",
    "negative",
];

const FUZZY_LINES: &[&str] = &[
    "hey",
    "hey there",
    "wake",
    "wake up",
    "you there",
    "are you there",
    "are you awake",
];

/// Sleep or hibernate, when the line is only that request.
#[must_use]
pub(crate) fn classify_mode(text: &str, agent_name: &str) -> Option<ModeIntent> {
    let words = normalized_words(text);
    if words.is_empty() || is_question(&words) || has_negation(&words) {
        return None;
    }
    let muted = drop_fillers(&words, agent_name);
    let line = strip_duration_tails(&muted).join(" ");
    if line == "deep sleep" || line == "hibernate" {
        return Some(ModeIntent::Hibernate);
    }
    if SLEEP_PATTERNS.contains(&line.as_str()) {
        Some(ModeIntent::Sleep)
    } else {
        None
    }
}

/// Yes, no, or unclear. Wake phrases count as yes only for fuzzy wake.
#[must_use]
pub(crate) fn classify_reply(
    text: &str,
    allow_wake_phrase: bool,
    wake_phrases: &[String],
) -> ConfirmReply {
    let Some(line) = reply_line(text) else {
        return ConfirmReply::Unclear;
    };
    if YES_LINES.contains(&line.as_str()) {
        return ConfirmReply::Yes;
    }
    if NO_LINES.contains(&line.as_str()) {
        return ConfirmReply::No;
    }
    if allow_wake_phrase
        && (line == "wake"
            || line == "wake up"
            || wake_phrases.iter().any(|phrase| phrase == &line))
    {
        return ConfirmReply::Yes;
    }
    ConfirmReply::Unclear
}

/// Typed or transcribed text while asleep that looks like a wake attempt.
#[must_use]
pub(crate) fn is_fuzzy_wake_text(text: &str, wake_phrases: &[String], agent_name: &str) -> bool {
    let words = normalized_words(text);
    if words.is_empty() || is_question(&words) || has_negation(&words) {
        return false;
    }
    let line = words.join(" ");
    if wake_phrases
        .iter()
        .any(|phrase| contains_phrase(&line, phrase))
    {
        return true;
    }
    if FUZZY_LINES.contains(&line.as_str()) {
        return true;
    }
    let name = agent_name.trim().to_ascii_lowercase();
    if name.chars().count() < 4 {
        return false;
    }
    words.iter().any(|word| {
        let token = word.replace('\'', "");
        token.chars().count() >= 3 && edit_distance(&token, &name) == 1
    })
}

/// Probe keyword that should ask "were you trying to wake me?".
///
/// Bare `hi` is too common at the probe floor. A real `hi` fire still wakes.
#[must_use]
pub(crate) fn probe_is_fuzzy_wake(
    keyword: &str,
    wake: &[String],
    sleep: &[String],
    hibernate: &[String],
) -> bool {
    let folded = keyword
        .trim()
        .trim_start_matches('@')
        .replace('_', " ")
        .to_ascii_lowercase();
    if folded.is_empty() || folded == "hi" {
        return false;
    }
    hit_from_keyword(&folded, wake, sleep, hibernate) == PhraseHit::Wake
}

fn normalized_words(text: &str) -> Vec<String> {
    let mut raw = String::new();
    for ch in text.chars() {
        let ch = if matches!(ch, '\u{2019}' | '\u{2018}') {
            '\''
        } else {
            ch
        };
        if ch.is_ascii_alphanumeric() || ch.is_ascii_whitespace() || ch == '\'' {
            raw.push(ch.to_ascii_lowercase());
        } else {
            raw.push(' ');
        }
    }
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let folded = collapsed
        .replace("i'm", "im")
        .replace("that's", "thats")
        .replace("good night", "goodnight");
    folded.split_whitespace().map(str::to_owned).collect()
}

fn reply_line(text: &str) -> Option<String> {
    let words: Vec<String> = normalized_words(text)
        .into_iter()
        .map(|word| word.replace('\'', ""))
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() || words.len() > 5 {
        return None;
    }
    Some(words.join(" "))
}

fn is_question(words: &[String]) -> bool {
    let Some(first) = words.first() else {
        return false;
    };
    let stem = first.split('\'').next().unwrap_or(first.as_str());
    QUESTION_STEMS.contains(&stem)
}

fn has_negation(words: &[String]) -> bool {
    words.iter().any(|word| {
        let bare = word.replace('\'', "");
        NEGATIONS.contains(&bare.as_str())
    })
}

fn drop_fillers(words: &[String], agent_name: &str) -> Vec<String> {
    let name = agent_name.trim().to_ascii_lowercase();
    words
        .iter()
        .filter_map(|word| {
            let bare = word.replace('\'', "");
            if bare.is_empty()
                || FILLERS.contains(&bare.as_str())
                || bare == "softwake"
                || (!name.is_empty() && bare == name)
            {
                None
            } else {
                Some(bare)
            }
        })
        .collect()
}

fn strip_duration_tails(words: &[String]) -> Vec<String> {
    let mut out = words.to_vec();
    while out
        .last()
        .is_some_and(|word| DURATION_TAILS.contains(&word.as_str()))
    {
        out.pop();
    }
    out
}

fn contains_phrase(haystack: &str, phrase: &str) -> bool {
    let phrase = phrase.trim();
    if phrase.is_empty() || haystack.is_empty() {
        return false;
    }
    let padded = format!(" {haystack} ");
    let needle = format!(" {phrase} ");
    padded.contains(&needle)
}

fn edit_distance(left: &str, right: &str) -> usize {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return a.len().abs_diff(b.len());
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::{
        ConfirmReply, ModeIntent, classify_mode, classify_reply, is_fuzzy_wake_text,
        probe_is_fuzzy_wake,
    };
    use softwake_wake::phrases_for_agent;

    fn sally() -> softwake_wake::AgentPhrases {
        phrases_for_agent("Sally")
    }

    #[test]
    fn hibernate_and_sleep_lines_match_only_the_request() {
        let hibernate = [
            "hibernate",
            "deep sleep now",
            "please hibernate",
            "can you hibernate",
            "deep sleep",
        ];
        for line in hibernate {
            assert_eq!(
                classify_mode(line, "Sally"),
                Some(ModeIntent::Hibernate),
                "{line}"
            );
        }
        let sleep = [
            "put yourself to sleep",
            "go to sleep",
            "go to sleep for a little while",
            "go to sleep for a bit",
            "hey sally go to sleep for a while",
            "I'm going to sleep",
            "sleep for a few minutes",
            "I'm done for a bit",
            "I'm done",
            "goodnight Sally",
            "good night sally",
            "sleep",
            "please sleep",
            "time to sleep",
            "go to bed",
            "that's enough",
            "hey sally go to sleep",
        ];
        for line in sleep {
            assert_eq!(
                classify_mode(line, "Sally"),
                Some(ModeIntent::Sleep),
                "{line}"
            );
        }
    }

    #[test]
    fn questions_negation_and_extra_words_are_not_mode_commands() {
        for line in [
            "what is hibernate",
            "why deep sleep",
            "don't go to sleep",
            "do not sleep",
            "I'm done with the dishes",
            "tell me about deep sleep",
            "hello",
            "",
        ] {
            assert_eq!(classify_mode(line, "Sally"), None, "{line}");
        }
    }

    #[test]
    fn deep_sleep_is_hibernate_not_sleep() {
        assert_eq!(
            classify_mode("deep sleep", "Sally"),
            Some(ModeIntent::Hibernate)
        );
        assert_ne!(
            classify_mode("deep sleep", "Sally"),
            Some(ModeIntent::Sleep)
        );
    }

    #[test]
    fn replies_are_yes_no_or_unclear() {
        let wake = sally().wake;
        assert_eq!(classify_reply("Yes!", false, &wake), ConfirmReply::Yes);
        assert_eq!(classify_reply("please do", false, &wake), ConfirmReply::Yes);
        assert_eq!(classify_reply("no thanks", false, &wake), ConfirmReply::No);
        assert_eq!(classify_reply("cancel", false, &wake), ConfirmReply::No);
        assert_eq!(classify_reply("don't", false, &wake), ConfirmReply::No);
        assert_eq!(classify_reply("stay asleep", true, &wake), ConfirmReply::No);
        assert_eq!(classify_reply("sleep", false, &wake), ConfirmReply::Unclear);
        assert_eq!(
            classify_reply("yes I would like that now", false, &wake),
            ConfirmReply::Unclear
        );
        assert_eq!(
            classify_reply("wake up", false, &wake),
            ConfirmReply::Unclear
        );
        assert_eq!(classify_reply("wake up", true, &wake), ConfirmReply::Yes);
        assert_eq!(classify_reply("Sally", true, &wake), ConfirmReply::Yes);
        assert_eq!(classify_reply("hey sally", true, &wake), ConfirmReply::Yes);
        assert_eq!(classify_reply("Sally", false, &wake), ConfirmReply::Unclear);
    }

    #[test]
    fn typed_fuzzy_wake_catches_name_garble_and_not_chat() {
        let wake = sally().wake;
        assert!(is_fuzzy_wake_text("saly", &wake, "Sally"));
        assert!(is_fuzzy_wake_text("hey sally", &wake, "Sally"));
        assert!(is_fuzzy_wake_text("hi", &wake, "Sally"));
        assert!(is_fuzzy_wake_text("hey", &wake, "Sally"));
        assert!(is_fuzzy_wake_text("are you there", &wake, "Sally"));
        assert!(!is_fuzzy_wake_text("hello", &wake, "Sally"));
        assert!(!is_fuzzy_wake_text("what time is it", &wake, "Sally"));
        assert!(!is_fuzzy_wake_text(
            "the history of hibernation in mammals is fascinating",
            &wake,
            "Sally"
        ));
        assert!(!is_fuzzy_wake_text("don't wake", &wake, "Sally"));
    }

    #[test]
    fn probe_fuzzy_skips_bare_hi_and_non_wake_keywords() {
        let phrases = sally();
        assert!(probe_is_fuzzy_wake(
            "sally",
            &phrases.wake,
            &phrases.sleep,
            &phrases.hibernate
        ));
        assert!(probe_is_fuzzy_wake(
            "hey_softwake",
            &phrases.wake,
            &phrases.sleep,
            &phrases.hibernate
        ));
        assert!(probe_is_fuzzy_wake(
            "@sally",
            &phrases.wake,
            &phrases.sleep,
            &phrases.hibernate
        ));
        assert!(!probe_is_fuzzy_wake(
            "hi",
            &phrases.wake,
            &phrases.sleep,
            &phrases.hibernate
        ));
        assert!(!probe_is_fuzzy_wake(
            "HI",
            &phrases.wake,
            &phrases.sleep,
            &phrases.hibernate
        ));
        assert!(!probe_is_fuzzy_wake(
            "sleep",
            &phrases.wake,
            &phrases.sleep,
            &phrases.hibernate
        ));
        assert!(!probe_is_fuzzy_wake(
            "deep sleep",
            &phrases.wake,
            &phrases.sleep,
            &phrases.hibernate
        ));
    }
}
