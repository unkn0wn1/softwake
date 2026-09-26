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
mod text;

use std::fmt;

pub use phrases::{DEFAULT_AGENT_NAME, hit_from_keyword, phrases_for_agent};

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
}

impl PhraseHit {
    /// Stable log spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Wake => "wake",
            Self::Sleep => "sleep",
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
    /// Mapped wake / sleep / none for the configured phrase lists.
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

#[cfg(test)]
mod tests {
    use super::{NullDetector, PhraseHit, WakeDetector};

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
    }
}
