//! Wake, sleep, and hibernate phrase lists driven by the active agent name.

use crate::PhraseHit;

/// Built-in Softwake identity used when a profile name is blank.
pub const DEFAULT_AGENT_NAME: &str = "Softwake";

/// Always-on Softwake wake fallbacks (spoken product name).
const PRODUCT_WAKE: &[&str] = &["hey softwake", "softwake"];

/// Always-on Softwake sleep fallbacks.
const PRODUCT_SLEEP: &[&str] = &["go to sleep", "goodnight softwake", "softwake sleep"];

/// Voice entry to hibernate. Not a way out of hibernate.
const HIBERNATE_PHRASE: &str = "deep sleep";

/// Wake, sleep, and hibernate phrases for one agent name.
///
/// Order is first-seen. Callers that treat index 0 as the primary wake phrase
/// still see the profile name, not bare `hi`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPhrases {
    /// Wake phrases, lowercase.
    pub wake: Vec<String>,
    /// Sleep phrases, lowercase.
    pub sleep: Vec<String>,
    /// Hibernate phrases, lowercase. Today this is only `deep sleep`.
    pub hibernate: Vec<String>,
}

/// Build wake, sleep, and hibernate phrase lists for `agent_name`.
///
/// The active profile **name** is the primary wake keyword. Softwake product
/// phrases stay as fallbacks so "hey Softwake" still works after a rename.
/// Bare `hi` and `sleep` are appended. `deep sleep` is the only hibernate
/// phrase. Blank names fall back to [`DEFAULT_AGENT_NAME`]. Phrases are
/// lowercase, trimmed, and de-duplicated while preserving first-seen order.
#[must_use]
pub fn phrases_for_agent(agent_name: &str) -> AgentPhrases {
    let name = normalize_name(agent_name);
    let mut wake = Vec::new();
    push_unique(&mut wake, &name);
    push_unique(&mut wake, &format!("hey {name}"));
    for phrase in PRODUCT_WAKE {
        push_unique(&mut wake, phrase);
    }
    push_unique(&mut wake, "hi");

    let mut sleep = Vec::new();
    push_unique(&mut sleep, "go to sleep");
    push_unique(&mut sleep, &format!("goodnight {name}"));
    push_unique(&mut sleep, &format!("{name} sleep"));
    for phrase in PRODUCT_SLEEP {
        push_unique(&mut sleep, phrase);
    }
    push_unique(&mut sleep, "sleep");

    let mut hibernate = Vec::new();
    push_unique(&mut hibernate, HIBERNATE_PHRASE);
    AgentPhrases {
        wake,
        sleep,
        hibernate,
    }
}

fn normalize_name(agent_name: &str) -> String {
    let trimmed = agent_name.trim();
    if trimmed.is_empty() {
        DEFAULT_AGENT_NAME.to_ascii_lowercase()
    } else {
        trimmed.to_ascii_lowercase()
    }
}

fn push_unique(out: &mut Vec<String>, phrase: &str) {
    let folded = phrase.trim().to_ascii_lowercase();
    if folded.is_empty() {
        return;
    }
    if !out.iter().any(|existing| existing == &folded) {
        out.push(folded);
    }
}

/// Map a decoded keyword (or `@tag`) onto wake / sleep / hibernate.
///
/// Longest contained phrase wins. Equal length prefers hibernate, then sleep,
/// then wake, so `deep sleep` is not swallowed by `sleep` and a sleep phrase
/// is not swallowed by an equal-length wake phrase.
///
/// A profile whose name is exactly `sleep` or `hi` ties with that bare word
/// and loses this tie-break. `hey <name>` still wakes. This is not special-cased.
#[must_use]
pub fn hit_from_keyword(
    keyword: &str,
    wake: &[String],
    sleep: &[String],
    hibernate: &[String],
) -> PhraseHit {
    let folded = keyword
        .trim()
        .trim_start_matches('@')
        .replace('_', " ")
        .to_ascii_lowercase();
    if folded.is_empty() {
        return PhraseHit::None;
    }
    let best = [
        (longest_contained(wake, &folded), PhraseHit::Wake),
        (longest_contained(sleep, &folded), PhraseHit::Sleep),
        (longest_contained(hibernate, &folded), PhraseHit::Hibernate),
    ]
    .into_iter()
    .filter_map(|(len, hit)| len.map(|len| (len, hit)))
    .max_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| hit_rank(left.1).cmp(&hit_rank(right.1)))
    });
    best.map_or(PhraseHit::None, |(_, hit)| hit)
}

/// Higher wins an equal-length tie: hibernate, then sleep, then wake.
fn hit_rank(hit: PhraseHit) -> u8 {
    match hit {
        PhraseHit::Hibernate => 3,
        PhraseHit::Sleep => 2,
        PhraseHit::Wake => 1,
        PhraseHit::None => 0,
    }
}

fn longest_contained(phrases: &[String], haystack: &str) -> Option<usize> {
    phrases
        .iter()
        .filter(|phrase| haystack.contains(phrase.as_str()) || phrase.as_str() == haystack)
        .map(String::len)
        .max()
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_AGENT_NAME, hit_from_keyword, phrases_for_agent};
    use crate::PhraseHit;

    #[test]
    fn default_agent_keeps_product_phrases_and_adds_short_words() {
        let phrases = phrases_for_agent(DEFAULT_AGENT_NAME);
        assert!(phrases.wake.iter().any(|phrase| phrase == "softwake"));
        assert!(phrases.wake.iter().any(|phrase| phrase == "hey softwake"));
        assert!(phrases.wake.iter().any(|phrase| phrase == "hi"));
        assert_ne!(phrases.wake[0], "hi");
        assert!(phrases.sleep.iter().any(|phrase| phrase == "go to sleep"));
        assert!(
            phrases
                .sleep
                .iter()
                .any(|phrase| phrase == "goodnight softwake")
        );
        assert!(phrases.sleep.iter().any(|phrase| phrase == "sleep"));
        assert_ne!(phrases.sleep[0], "sleep");
        assert_eq!(phrases.hibernate, ["deep sleep".to_owned()]);
    }

    #[test]
    fn profile_name_drives_primary_wake() {
        let phrases = phrases_for_agent("Sally");
        assert_eq!(phrases.wake[0], "sally");
        assert!(phrases.wake.iter().any(|phrase| phrase == "hey sally"));
        assert!(phrases.wake.iter().any(|phrase| phrase == "hey softwake"));
        assert!(phrases.wake.iter().any(|phrase| phrase == "softwake"));
        assert!(phrases.wake.iter().any(|phrase| phrase == "hi"));
        assert_eq!(
            phrases.sleep[0], "go to sleep",
            "bare sleep stays after the longer phrases"
        );
        assert!(
            phrases
                .sleep
                .iter()
                .any(|phrase| phrase == "goodnight sally")
        );
        assert!(phrases.sleep.iter().any(|phrase| phrase == "sally sleep"));
        assert!(
            phrases
                .sleep
                .iter()
                .any(|phrase| phrase == "goodnight softwake")
        );
        assert!(
            phrases
                .sleep
                .iter()
                .any(|phrase| phrase == "softwake sleep")
        );
        assert!(phrases.sleep.iter().any(|phrase| phrase == "sleep"));
        assert_eq!(phrases.hibernate, ["deep sleep".to_owned()]);
    }

    #[test]
    fn blank_name_falls_back_to_softwake() {
        let phrases = phrases_for_agent("   ");
        assert_eq!(phrases.wake[0], "softwake");
        assert!(phrases.wake.iter().any(|phrase| phrase == "hi"));
        assert!(phrases.sleep.iter().any(|phrase| phrase == "sleep"));
        assert_eq!(phrases.hibernate, ["deep sleep".to_owned()]);
    }

    #[test]
    fn keyword_tag_maps_with_longest_match_and_sleep_tie_break() {
        let wake = vec!["softwake".to_owned()];
        let sleep = vec!["softwake sleep".to_owned()];
        let hibernate = vec!["deep sleep".to_owned()];
        assert_eq!(
            hit_from_keyword("softwake_sleep", &wake, &sleep, &hibernate),
            PhraseHit::Sleep
        );
        assert_eq!(
            hit_from_keyword("@softwake", &wake, &sleep, &hibernate),
            PhraseHit::Wake
        );
        assert_eq!(
            hit_from_keyword("", &wake, &sleep, &hibernate),
            PhraseHit::None
        );
    }

    #[test]
    fn bare_words_and_deep_sleep_map_without_swallowing_longer_phrases() {
        let phrases = phrases_for_agent("Sally");
        let wake = &phrases.wake;
        let sleep = &phrases.sleep;
        let hibernate = &phrases.hibernate;
        assert_eq!(
            hit_from_keyword("hi", wake, sleep, hibernate),
            PhraseHit::Wake
        );
        assert_eq!(
            hit_from_keyword("@hi", wake, sleep, hibernate),
            PhraseHit::Wake
        );
        assert_eq!(
            hit_from_keyword("sleep", wake, sleep, hibernate),
            PhraseHit::Sleep
        );
        assert_eq!(
            hit_from_keyword("deep sleep", wake, sleep, hibernate),
            PhraseHit::Hibernate
        );
        assert_eq!(
            hit_from_keyword("deep_sleep", wake, sleep, hibernate),
            PhraseHit::Hibernate
        );
        assert_eq!(
            hit_from_keyword("@deep_sleep", wake, sleep, hibernate),
            PhraseHit::Hibernate
        );
        assert_eq!(
            hit_from_keyword("go to sleep", wake, sleep, hibernate),
            PhraseHit::Sleep
        );
        assert_eq!(
            hit_from_keyword("hey sally", wake, sleep, hibernate),
            PhraseHit::Wake
        );
        assert_eq!(
            hit_from_keyword("sally sleep", wake, sleep, hibernate),
            PhraseHit::Sleep
        );
        assert_eq!(
            hit_from_keyword("", wake, sleep, hibernate),
            PhraseHit::None
        );
    }

    #[test]
    fn equal_length_prefers_hibernate_then_sleep() {
        let tied = vec!["hi".to_owned()];
        assert_eq!(
            hit_from_keyword("hi", &tied, &tied, &tied),
            PhraseHit::Hibernate
        );
        let empty = Vec::new();
        assert_eq!(
            hit_from_keyword("abcd", &tied_word("abcd"), &tied_word("abcd"), &empty),
            PhraseHit::Sleep
        );
        assert_eq!(
            hit_from_keyword("abcdef", &tied_word("abcdef"), &tied_word("ab"), &empty),
            PhraseHit::Wake
        );
    }

    fn tied_word(word: &str) -> Vec<String> {
        vec![word.to_owned()]
    }
}
