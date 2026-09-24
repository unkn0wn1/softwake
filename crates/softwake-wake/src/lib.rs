//! Wake and sleep phrase boundary.
//!
//! No wake-word engine is linked. [`NullDetector`] never matches, which keeps
//! tests and CI independent of a microphone or a model.

/// What a detector heard in one window of samples.
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
}
