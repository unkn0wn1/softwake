//! Reset a keyword stream after a stretch of silence.
//!
//! sherpa-onnx keeps one online stream until a keyword or a long blank run.
//! [`crate::stream_budget`] already resets every three seconds of audio.
//! This gate is the pause side of that: after about 400 ms below the same
//! silence level the awake energy detector uses, the caller should reset the
//! stream so the next short word is closer to an isolated utterance.
//!
//! It is not a sentence parser. Silence does not mean "end of command".

/// Samples (16 kHz) of silence before the caller should reset.
pub(crate) const SILENCE_RESET_SAMPLES: u64 = 16_000 * 400 / 1000;

/// Peak-normalized RMS below which a window counts as silence.
///
/// Matches `softwake_voice::energy_utt::SILENCE_RMS` (0.02) so a pause that
/// ends a free-speech utterance also drops keyword context.
pub(crate) const SILENCE_RMS: f32 = 0.02;

/// Consecutive silent samples since the last speech window or reset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SilenceReset {
    run: u64,
}

impl SilenceReset {
    /// No silence accumulated.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self { run: 0 }
    }

    /// Record one window. `true` once, when the silent run reaches the limit.
    ///
    /// The run restarts after a reset, so a long silence trips about every
    /// 400 ms rather than on every frame. RMS at or above [`SILENCE_RMS`]
    /// clears the run and does not reset.
    #[must_use]
    pub(crate) fn observe(&mut self, n_samples: usize, rms: f32) -> bool {
        if rms >= SILENCE_RMS {
            self.run = 0;
            return false;
        }
        let added = u64::try_from(n_samples).unwrap_or(u64::MAX);
        self.run = self.run.saturating_add(added);
        if self.run >= SILENCE_RESET_SAMPLES {
            self.run = 0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SILENCE_RESET_SAMPLES, SILENCE_RMS, SilenceReset};

    #[test]
    fn four_hundred_milliseconds_at_16khz() {
        assert_eq!(SILENCE_RESET_SAMPLES, 6_400);
        assert!((SILENCE_RMS - 0.02).abs() < f32::EPSILON);
    }

    #[test]
    fn under_the_limit_does_not_reset() {
        let mut gate = SilenceReset::new();
        assert!(!gate.observe(
            usize::try_from(SILENCE_RESET_SAMPLES - 1).expect("fits"),
            0.0
        ));
    }

    #[test]
    fn crossing_the_limit_resets_once_then_needs_another_run() {
        let mut gate = SilenceReset::new();
        assert!(gate.observe(usize::try_from(SILENCE_RESET_SAMPLES).expect("fits"), 0.01));
        assert!(!gate.observe(100, 0.0));
    }

    #[test]
    fn speech_clears_a_partial_silent_run() {
        let mut gate = SilenceReset::new();
        assert!(!gate.observe(6_000, 0.0));
        assert!(!gate.observe(160, SILENCE_RMS));
        assert!(!gate.observe(
            usize::try_from(SILENCE_RESET_SAMPLES - 1).expect("fits"),
            0.0
        ));
    }
}
