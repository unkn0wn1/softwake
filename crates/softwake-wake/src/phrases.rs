//! Wake and sleep phrase lists driven by the active agent / profile name.

/// Built-in Softwake identity used when a profile name is blank.
pub const DEFAULT_AGENT_NAME: &str = "Softwake";

/// Always-on Softwake wake fallbacks (spoken product name).
const PRODUCT_WAKE: &[&str] = &["hey softwake", "softwake"];

/// Always-on Softwake sleep fallbacks.
const PRODUCT_SLEEP: &[&str] = &["go to sleep", "goodnight softwake", "softwake sleep"];

/// Build wake and sleep phrase lists for `agent_name`.
///
/// The active profile **name** is the primary wake keyword. Softwake product
/// phrases stay as fallbacks so "hey Softwake" still works after a rename.
/// Blank names fall back to [`DEFAULT_AGENT_NAME`]. Phrases are lowercase,
/// trimmed, and de-duplicated while preserving first-seen order.
#[must_use]
pub fn phrases_for_agent(agent_name: &str) -> (Vec<String>, Vec<String>) {
    let name = normalize_name(agent_name);
    let mut wake = Vec::new();
    push_unique(&mut wake, &name);
    push_unique(&mut wake, &format!("hey {name}"));
    for phrase in PRODUCT_WAKE {
        push_unique(&mut wake, phrase);
    }

    let mut sleep = Vec::new();
    push_unique(&mut sleep, "go to sleep");
    push_unique(&mut sleep, &format!("goodnight {name}"));
    push_unique(&mut sleep, &format!("{name} sleep"));
    for phrase in PRODUCT_SLEEP {
        push_unique(&mut sleep, phrase);
    }
    (wake, sleep)
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

/// Map a decoded keyword (or `@tag`) onto wake / sleep with the text-table rule:
/// longest match wins; equal length prefers sleep.
#[must_use]
pub fn hit_from_keyword(keyword: &str, wake: &[String], sleep: &[String]) -> crate::PhraseHit {
    let folded = keyword
        .trim()
        .trim_start_matches('@')
        .replace('_', " ")
        .to_ascii_lowercase();
    if folded.is_empty() {
        return crate::PhraseHit::None;
    }
    let wake_len = longest_contained(wake, &folded);
    let sleep_len = longest_contained(sleep, &folded);
    match (wake_len, sleep_len) {
        (None, None) => crate::PhraseHit::None,
        (Some(_), None) => crate::PhraseHit::Wake,
        (None, Some(_)) => crate::PhraseHit::Sleep,
        (Some(wake), Some(sleep)) => {
            if wake > sleep {
                crate::PhraseHit::Wake
            } else {
                crate::PhraseHit::Sleep
            }
        }
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
    fn default_agent_includes_product_wake_and_sleep() {
        let (wake, sleep) = phrases_for_agent(DEFAULT_AGENT_NAME);
        assert!(wake.iter().any(|p| p == "softwake"));
        assert!(wake.iter().any(|p| p == "hey softwake"));
        assert!(sleep.iter().any(|p| p == "go to sleep"));
        assert!(sleep.iter().any(|p| p == "goodnight softwake"));
    }

    #[test]
    fn profile_name_drives_primary_wake() {
        let (wake, sleep) = phrases_for_agent("Ada");
        assert_eq!(wake[0], "ada");
        assert!(wake.iter().any(|p| p == "hey ada"));
        assert!(wake.iter().any(|p| p == "hey softwake"));
        assert!(sleep.iter().any(|p| p == "goodnight ada"));
        assert!(sleep.iter().any(|p| p == "ada sleep"));
        assert!(sleep.iter().any(|p| p == "go to sleep"));
    }

    #[test]
    fn blank_name_falls_back_to_softwake() {
        let (wake, _) = phrases_for_agent("   ");
        assert_eq!(wake[0], "softwake");
    }

    #[test]
    fn keyword_tag_maps_with_sleep_tie_break() {
        let wake = vec!["softwake".to_owned()];
        let sleep = vec!["softwake sleep".to_owned()];
        assert_eq!(
            hit_from_keyword("softwake_sleep", &wake, &sleep),
            PhraseHit::Sleep
        );
        assert_eq!(
            hit_from_keyword("@softwake", &wake, &sleep),
            PhraseHit::Wake
        );
        assert_eq!(hit_from_keyword("", &wake, &sleep), PhraseHit::None);
    }
}
