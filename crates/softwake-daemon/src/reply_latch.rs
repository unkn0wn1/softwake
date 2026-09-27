//! Free-speech ambient reply latch: stop TV-noise loops by auto-sleeping.
//!
//! See [ADR 0028](../../../docs/ADR-0028-slash-hud-self-sleep.md).

use std::time::{Duration, Instant};

/// How many near-identical short free-speech replies trip the latch.
pub(crate) const LATCH_COUNT: usize = 3;
/// Window that those replies must fall inside.
pub(crate) const LATCH_WINDOW: Duration = Duration::from_secs(90);
/// Max Unicode scalars for a "short" dismissal-style reply.
pub(crate) const SHORT_MAX_CHARS: usize = 120;

/// Tracks recent free-speech assistant replies for the ambient latch.
#[derive(Debug, Default)]
pub(crate) struct ReplyLatch {
    /// Normalized short replies with when they landed.
    recent: Vec<(String, Instant)>,
}

impl ReplyLatch {
    /// Clear after sleep / hibernate / fresh session / wake.
    pub(crate) fn clear(&mut self) {
        self.recent.clear();
    }

    /// Whether recording `reply` would trip the latch (before mutating).
    #[must_use]
    pub(crate) fn would_trigger(&self, reply: &str, now: Instant) -> bool {
        let Some(norm) = normalize_short(reply) else {
            return false;
        };
        let kept = self.kept_matching(&norm, now);
        kept + 1 >= LATCH_COUNT
    }

    /// Record a free-speech assistant reply. Returns `true` when the latch trips
    /// (caller should sleep and clear).
    pub(crate) fn record(&mut self, reply: &str, now: Instant) -> bool {
        let Some(norm) = normalize_short(reply) else {
            // Long or empty replies break an ambient loop streak.
            self.recent.clear();
            return false;
        };
        self.recent
            .retain(|(_prev, at)| now.saturating_duration_since(*at) <= LATCH_WINDOW);
        let matching = self
            .recent
            .iter()
            .filter(|(prev, _)| near_identical(prev, &norm))
            .count();
        self.recent.push((norm, now));
        matching + 1 >= LATCH_COUNT
    }

    fn kept_matching(&self, norm: &str, now: Instant) -> usize {
        self.recent
            .iter()
            .filter(|(prev, at)| {
                now.saturating_duration_since(*at) <= LATCH_WINDOW && near_identical(prev, norm)
            })
            .count()
    }
}

/// Normalize and require short length. `None` = not a latch candidate.
#[must_use]
pub(crate) fn normalize_short(text: &str) -> Option<String> {
    let norm = normalize(text);
    if norm.is_empty() || norm.chars().count() > SHORT_MAX_CHARS {
        None
    } else {
        Some(norm)
    }
}

/// Lowercase, strip most punctuation, collapse whitespace.
#[must_use]
pub(crate) fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_space = false;
        } else if ch.is_whitespace() && !last_space && !out.is_empty() {
            out.push(' ');
            last_space = true;
        }
        // Drop other punctuation / symbols.
    }
    out.trim().to_owned()
}

#[must_use]
pub(crate) fn near_identical(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    if left.len().abs_diff(right.len()) > 2 {
        return false;
    }
    edit_distance(left, right) <= 2
}

/// True when a free-speech **assistant** reply is only a self-sleep marker.
///
/// Ambient "still the telly…" loops use [`ReplyLatch`], not this first-hit path.
#[must_use]
pub(crate) fn is_self_sleep_reply(text: &str) -> bool {
    let Some(norm) = normalize_short(text) else {
        return false;
    };
    matches!(
        norm.as_str(),
        "going to sleep"
            | "i am going to sleep"
            | "im going to sleep"
            | "i will sleep"
            | "ill sleep"
            | "sleeping now"
            | "mic off"
            | "mic off sleeping"
            | "going to sleep mic off"
            | "i will go to sleep"
            | "ill go to sleep"
    )
}

fn edit_distance(left: &str, right: &str) -> usize {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_punctuation_and_case() {
        assert_eq!(
            normalize("Still the telly. Not you. / Mic off."),
            "still the telly not you mic off"
        );
    }

    #[test]
    fn latch_trips_on_third_identical_short_reply() {
        let mut latch = ReplyLatch::default();
        let t0 = Instant::now();
        let line = "Still the telly. Not you. / Mic off.";
        assert!(!latch.record(line, t0));
        assert!(!latch.record(line, t0 + Duration::from_secs(5)));
        assert!(latch.would_trigger(line, t0 + Duration::from_secs(10)));
        assert!(latch.record(line, t0 + Duration::from_secs(10)));
    }

    #[test]
    fn latch_ignores_long_replies() {
        let mut latch = ReplyLatch::default();
        let long = "a".repeat(SHORT_MAX_CHARS + 1);
        assert!(!latch.record(&long, Instant::now()));
        assert!(latch.recent.is_empty());
    }

    #[test]
    fn self_sleep_markers_match() {
        assert!(is_self_sleep_reply("Going to sleep."));
        assert!(is_self_sleep_reply("Mic off."));
        assert!(!is_self_sleep_reply("Still the telly. Not you. / Mic off."));
        assert!(!is_self_sleep_reply("What time is it?"));
    }

    #[test]
    fn near_identical_allows_tiny_typos() {
        assert!(near_identical(
            "still the telly not you mic off",
            "still the telly not you mic of"
        ));
    }
}
