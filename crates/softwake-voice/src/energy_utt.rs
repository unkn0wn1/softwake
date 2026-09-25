//! Energy / silence-gated utterance detector for listen-while-awake.
//!
//! Softwake feeds 16 kHz mono `i16` PCM while awake and not on press-to-talk.
//! Speech energy starts a buffer; sustained silence ends it. No sherpa. CI stays
//! mic-free — unit tests push synthetic samples.

use crate::{TALK_MAX_SAMPLES, TALK_MIN_SAMPLES};

/// RMS (peak-normalized) above which a frame counts as speech.
pub const START_RMS: f32 = 0.04;
/// RMS below which a frame counts as silence while buffering.
pub const SILENCE_RMS: f32 = 0.02;
/// Contiguous silence frames that end an utterance (~400 ms at 20 ms frames).
pub const SILENCE_FRAMES_END: u32 = 20;
/// Contiguous speech frames that start an utterance (~100 ms).
pub const START_FRAMES: u32 = 5;

/// One energy-gated utterance collector.
#[derive(Debug, Default, Clone)]
pub struct EnergyUtterance {
    buffering: bool,
    samples: Vec<i16>,
    speech_run: u32,
    silence_run: u32,
}

impl EnergyUtterance {
    /// Empty detector, not buffering.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether PCM is being stored after speech onset.
    #[must_use]
    pub const fn is_buffering(&self) -> bool {
        self.buffering
    }

    /// Drop any partial utterance (PTT took priority, sleep, or cooldown).
    pub fn reset(&mut self) {
        self.buffering = false;
        self.samples.clear();
        self.speech_run = 0;
        self.silence_run = 0;
    }

    /// Push one capture frame. Returns finished PCM when silence ends a long enough utterance.
    pub fn push_frame(&mut self, samples: &[i16], rms: f32) -> Option<Vec<i16>> {
        if samples.is_empty() {
            return None;
        }
        if !self.buffering {
            if rms >= START_RMS {
                self.speech_run = self.speech_run.saturating_add(1);
            } else {
                self.speech_run = 0;
            }
            if self.speech_run < START_FRAMES {
                return None;
            }
            self.buffering = true;
            self.silence_run = 0;
            self.samples.clear();
        }

        let room = TALK_MAX_SAMPLES.saturating_sub(self.samples.len());
        if room == 0 {
            return self.take_if_long_enough();
        }
        let take = samples.len().min(room);
        self.samples.extend_from_slice(&samples[..take]);

        if self.samples.len() >= TALK_MAX_SAMPLES {
            return self.take_if_long_enough();
        }

        if rms < SILENCE_RMS {
            self.silence_run = self.silence_run.saturating_add(1);
        } else {
            self.silence_run = 0;
        }
        if self.silence_run >= SILENCE_FRAMES_END {
            return self.take_if_long_enough();
        }
        None
    }

    fn take_if_long_enough(&mut self) -> Option<Vec<i16>> {
        self.buffering = false;
        self.speech_run = 0;
        self.silence_run = 0;
        if self.samples.len() < TALK_MIN_SAMPLES {
            self.samples.clear();
            return None;
        }
        Some(std::mem::take(&mut self.samples))
    }
}

#[cfg(test)]
mod tests {
    use super::{EnergyUtterance, SILENCE_FRAMES_END, SILENCE_RMS, START_FRAMES, START_RMS};
    use crate::TALK_MIN_SAMPLES;

    fn tone(n: usize, amp: f32) -> Vec<i16> {
        (0..n)
            .map(|i| {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "synthetic test tone; index stays tiny"
                )]
                let phase = (i as f32) * 0.1;
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "intentional PCM quantization for tests"
                )]
                {
                    (phase.sin() * amp * f32::from(i16::MAX)) as i16
                }
            })
            .collect()
    }

    #[test]
    fn silence_is_ignored() {
        let mut gate = EnergyUtterance::new();
        for _ in 0..50 {
            assert!(gate.push_frame(&tone(320, 0.0), 0.0).is_none());
        }
        assert!(!gate.is_buffering());
    }

    #[test]
    fn speech_then_silence_yields_pcm() {
        let mut gate = EnergyUtterance::new();
        let loud = tone(320, 0.2);
        for _ in 0..START_FRAMES {
            assert!(gate.push_frame(&loud, START_RMS + 0.01).is_none());
        }
        assert!(gate.is_buffering());
        // Pad to min length while still "speech".
        while gate.samples_len_for_test() < TALK_MIN_SAMPLES {
            assert!(gate.push_frame(&loud, START_RMS + 0.01).is_none());
        }
        let mut finished = None;
        for _ in 0..SILENCE_FRAMES_END {
            finished = gate.push_frame(&tone(320, 0.0), SILENCE_RMS / 2.0);
            if finished.is_some() {
                break;
            }
        }
        let pcm = finished.expect("utterance");
        assert!(pcm.len() >= TALK_MIN_SAMPLES);
        assert!(!gate.is_buffering());
    }

    #[test]
    fn short_burst_is_discarded() {
        let mut gate = EnergyUtterance::new();
        let loud = tone(160, 0.2);
        for _ in 0..START_FRAMES {
            let _ = gate.push_frame(&loud, START_RMS + 0.01);
        }
        for _ in 0..SILENCE_FRAMES_END {
            assert!(
                gate.push_frame(&tone(160, 0.0), SILENCE_RMS / 2.0)
                    .is_none()
            );
        }
        assert!(!gate.is_buffering());
    }

    impl EnergyUtterance {
        fn samples_len_for_test(&self) -> usize {
            self.samples.len()
        }
    }
}
