//! Elevated-energy barge-in while TTS is playing (soft duplex).
//!
//! Softwake keeps half-duplex mute for free-speech / KWS so Eve does not hear
//! herself, but still watches RMS. Sustained loud energy cancels playback so
//! the user can cut in without Escape. This is **not** acoustic echo
//! cancellation — speaker bleed can false-trigger; thresholds stay elevated.

/// Peak-normalized RMS that counts as barge speech (≈3× free-speech start).
pub const BARGE_RMS: f32 = 0.12;
/// Contiguous frames above [`BARGE_RMS`] before barge fires (~120 ms @ 10 ms).
pub const BARGE_FRAMES: u32 = 12;

/// Counts elevated-energy frames while Eve is speaking.
#[derive(Debug, Clone, Default)]
pub struct BargeDetector {
    speech_run: u32,
}

impl BargeDetector {
    /// Record one capture-frame RMS. Returns `true` when barge should fire.
    pub fn note(&mut self, rms: f32) -> bool {
        if rms >= BARGE_RMS {
            self.speech_run = self.speech_run.saturating_add(1);
        } else {
            self.speech_run = 0;
        }
        self.speech_run >= BARGE_FRAMES
    }

    /// Clear the run after a barge or when TTS mute lifts.
    pub fn reset(&mut self) {
        self.speech_run = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{BARGE_FRAMES, BARGE_RMS, BargeDetector};

    #[test]
    fn needs_sustained_elevated_energy() {
        let mut d = BargeDetector::default();
        for _ in 0..(BARGE_FRAMES - 1) {
            assert!(!d.note(BARGE_RMS));
        }
        assert!(d.note(BARGE_RMS));
        d.reset();
        assert!(!d.note(BARGE_RMS));
    }

    #[test]
    fn quiet_or_free_speech_level_does_not_barge() {
        let mut d = BargeDetector::default();
        for _ in 0..40 {
            assert!(!d.note(0.05));
        }
        // One loud frame after quiet resets the run.
        assert!(!d.note(BARGE_RMS));
        assert!(!d.note(0.01));
        // After a quiet frame the run is cleared — another loud frame alone must not fire.
        assert!(!d.note(BARGE_RMS));
    }
}
