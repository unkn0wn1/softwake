//! Wake and sleep phrase boundary.
//!
//! [`WakeDetector`] is the PCM seam for the on-device engine chosen in
//! ADR 0006 (sherpa-onnx keyword spotting). [`NullDetector`] never matches
//! and is the default PCM stand-in: weights are not linked.
//! [`TextWakeDetector`] is the phase-1 spike: it scores UTF-8 windows against
//! a [`PhraseTable`]. ADR 0002 records why the spike is text instead of a
//! model. The `sherpa-kws` feature compiles `SherpaKwsDetector`, which
//! implements the same PCM trait and also returns no hit until weights load.

mod phrases;
#[cfg(feature = "sherpa-kws")]
mod sherpa;
mod short_word;
// The sample budget is a pure counter. Tests run it without ONNX weights.
// The sherpa detector is the only caller outside tests, so the budget stays
// behind that feature except in `cfg(test)`.
#[cfg(any(test, feature = "sherpa-kws"))]
mod stream_budget;
mod text;

use std::fmt;

pub use phrases::{AgentPhrases, DEFAULT_AGENT_NAME, hit_from_keyword, phrases_for_agent};
pub use short_word::is_short_single_word;

#[cfg(feature = "sherpa-kws")]
pub use sherpa::SherpaKwsDetector;
pub use text::{PhraseTable, PhraseTableError, TextWakeDetector};

/// What a detector heard in one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PhraseHit {
    /// No configured phrase in this window.
    #[default]
    None,
    /// Configured wake phrase matched.
    Wake,
    /// Configured sleep phrase matched.
    Sleep,
    /// Configured hibernate phrase matched (`deep sleep`).
    ///
    /// This enters hibernate. It is not a way out of hibernate.
    Hibernate,
}

impl PhraseHit {
    /// Stable log spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Wake => "wake",
            Self::Sleep => "sleep",
            Self::Hibernate => "hibernate",
        }
    }
}

impl fmt::Display for PhraseHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One KWS decode observation (keyword text + wake/sleep mapping).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpotDetail {
    /// Mapped wake / sleep / hibernate / none for the configured phrase lists.
    pub hit: PhraseHit,
    /// Raw keyword tag from the spotter (`@sally`, `sally`, …), when any.
    pub keyword: Option<String>,
}

/// Scores capture windows for the configured wake and sleep phrases.
pub trait WakeDetector {
    /// Feed one window of interleaved 16-bit samples.
    #[must_use]
    fn push_samples(&mut self, samples: &[i16]) -> PhraseHit;
}

/// Detector that never matches. Stand-in until a local engine is chosen.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NullDetector;

impl WakeDetector for NullDetector {
    #[allow(clippy::unused_self)] // No engine is linked, so there is no state to read.
    fn push_samples(&mut self, _samples: &[i16]) -> PhraseHit {
        PhraseHit::None
    }
}

/// Test / scripted detector that yields prepared hits in order.
///
/// Each [`WakeDetector::push_samples`] pops the next hit. When the queue is
/// empty, further windows score [`PhraseHit::None`]. Used by daemon drain
/// regression tests without linking sherpa-onnx.
#[derive(Debug, Default, Clone)]
pub struct ScriptedDetector {
    hits: std::collections::VecDeque<PhraseHit>,
    /// Raw keyword queued with each hit. `None` lets the caller pick a tag.
    keywords: std::collections::VecDeque<Option<String>>,
}

impl ScriptedDetector {
    /// Queue `hits` to return on successive windows.
    #[must_use]
    pub fn new(hits: impl IntoIterator<Item = PhraseHit>) -> Self {
        let hits: std::collections::VecDeque<PhraseHit> = hits.into_iter().collect();
        let keywords = (0..hits.len()).map(|_| None).collect();
        Self { hits, keywords }
    }

    /// Queue hits with the raw keyword tag a spotter would report.
    ///
    /// Daemon tests use this to feed bare `sleep` through the observe path
    /// while a long awake buffer is open. That hit must stand.
    #[must_use]
    pub fn with_keywords<S>(items: impl IntoIterator<Item = (PhraseHit, S)>) -> Self
    where
        S: Into<String>,
    {
        let mut hits = std::collections::VecDeque::new();
        let mut keywords = std::collections::VecDeque::new();
        for (hit, keyword) in items {
            hits.push_back(hit);
            keywords.push_back(Some(keyword.into()));
        }
        Self { hits, keywords }
    }

    /// Pop the next hit and its keyword, if one was queued.
    #[must_use]
    pub fn pop_detailed(&mut self) -> (PhraseHit, Option<String>) {
        let hit = self.hits.pop_front().unwrap_or(PhraseHit::None);
        let keyword = self.keywords.pop_front().flatten();
        (hit, keyword)
    }
}

impl WakeDetector for ScriptedDetector {
    fn push_samples(&mut self, _samples: &[i16]) -> PhraseHit {
        self.pop_detailed().0
    }
}

#[cfg(test)]
mod tests {
    use super::{NullDetector, PhraseHit, ScriptedDetector, WakeDetector};

    #[test]
    fn null_detector_never_matches() {
        let mut detector = NullDetector;
        assert_eq!(detector.push_samples(&[]), PhraseHit::None);
        assert_eq!(detector.push_samples(&[0, 1, -1]), PhraseHit::None);
    }

    #[test]
    fn phrase_hit_display_is_stable() {
        assert_eq!(PhraseHit::None.as_str(), "none");
        assert_eq!(PhraseHit::Wake.to_string(), "wake");
        assert_eq!(PhraseHit::Sleep.to_string(), "sleep");
        assert_eq!(PhraseHit::Hibernate.to_string(), "hibernate");
    }

    #[test]
    fn scripted_detector_yields_hits_in_order() {
        let mut detector =
            ScriptedDetector::new([PhraseHit::Wake, PhraseHit::None, PhraseHit::Sleep]);
        assert_eq!(detector.push_samples(&[]), PhraseHit::Wake);
        assert_eq!(detector.push_samples(&[0]), PhraseHit::None);
        assert_eq!(detector.push_samples(&[1, 2]), PhraseHit::Sleep);
        assert_eq!(detector.push_samples(&[]), PhraseHit::None);
    }

    #[test]
    fn scripted_keywords_round_trip_a_short_sleep_tag() {
        let mut detector = ScriptedDetector::with_keywords([(PhraseHit::Sleep, "sleep")]);
        assert_eq!(
            detector.pop_detailed(),
            (PhraseHit::Sleep, Some("sleep".to_owned()))
        );
        assert_eq!(detector.pop_detailed(), (PhraseHit::None, None));
    }
}
