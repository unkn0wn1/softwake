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
/// Default contiguous silence frames that end an utterance (~2.0 s at 10 ms frames).
///
/// PipeWire/WASAPI wake capture queues 160 samples @ 16 kHz (10 ms). Free speech
/// needs a longer hangover so a mid-thought pause does not cut the utterance.
/// Settings and `SOFTWAKE_FREE_SPEECH_END_SILENCE_MS` override this per detector.
pub const SILENCE_FRAMES_END: u32 = 200;
/// Capture hop used to convert milliseconds into frames.
pub const CAPTURE_FRAME_MS: u32 = 10;
/// Shortest end-of-utterance hangover, in frames (0.5 s).
pub const SILENCE_FRAMES_END_MIN: u32 = 50;
/// Longest end-of-utterance hangover, in frames (4.0 s).
pub const SILENCE_FRAMES_END_MAX: u32 = 400;
/// Contiguous speech frames that start an utterance (~100 ms).
pub const START_FRAMES: u32 = 5;

/// Milliseconds → silence frames at [`CAPTURE_FRAME_MS`], clamped to 0.5–4.0 s.
#[must_use]
pub fn silence_frames_from_ms(ms: u32) -> u32 {
    let frames = ms / CAPTURE_FRAME_MS;
    frames.clamp(SILENCE_FRAMES_END_MIN, SILENCE_FRAMES_END_MAX)
}

/// One energy-gated utterance collector.
#[derive(Debug, Clone)]
pub struct EnergyUtterance {
    buffering: bool,
    samples: Vec<i16>,
    speech_run: u32,
    silence_run: u32,
    silence_frames_end: u32,
}

impl Default for EnergyUtterance {
    fn default() -> Self {
        Self {
            buffering: false,
            samples: Vec::new(),
            speech_run: 0,
            silence_run: 0,
            silence_frames_end: SILENCE_FRAMES_END,
        }
    }
}

impl EnergyUtterance {
    /// Empty detector, not buffering, with the default hangover.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Empty detector whose end-of-utterance hangover is `frames` (clamped).
    #[must_use]
    pub fn with_silence_frames_end(frames: u32) -> Self {
        let mut gate = Self::new();
        gate.set_silence_frames_end(frames);
        gate
    }

    /// Contiguous silence frames required to end the current utterance.
    #[must_use]
    pub const fn silence_frames_end(&self) -> u32 {
        self.silence_frames_end
    }

    /// Update the hangover without dropping a partial utterance.
    ///
    /// Values outside 50..=400 frames (0.5–4.0 s) are clamped. An in-flight
    /// buffer, speech run, and silence run stay as they are. The next
    /// [`Self::push_frame`] uses the new threshold.
    pub fn set_silence_frames_end(&mut self, frames: u32) {
        self.silence_frames_end = frames.clamp(SILENCE_FRAMES_END_MIN, SILENCE_FRAMES_END_MAX);
    }

    /// Whether PCM is being stored after speech onset.
    #[must_use]
    pub const fn is_buffering(&self) -> bool {
        self.buffering
    }

    /// Samples stored in the current utterance. Zero when not buffering.
    #[must_use]
    pub fn buffered_samples(&self) -> usize {
        self.samples.len()
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
        if self.silence_run >= self.silence_frames_end {
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
    use super::{
        EnergyUtterance, SILENCE_FRAMES_END, SILENCE_FRAMES_END_MAX, SILENCE_FRAMES_END_MIN,
        SILENCE_RMS, START_FRAMES, START_RMS, silence_frames_from_ms,
    };
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
        // Tiny frames so even a long silence hangover stays under TALK_MIN_SAMPLES.
        let loud = tone(10, 0.2);
        for _ in 0..START_FRAMES {
            let _ = gate.push_frame(&loud, START_RMS + 0.01);
        }
        for _ in 0..SILENCE_FRAMES_END {
            assert!(gate.push_frame(&tone(10, 0.0), SILENCE_RMS / 2.0).is_none());
        }
        assert!(!gate.is_buffering());
    }

    impl EnergyUtterance {
        fn samples_len_for_test(&self) -> usize {
            self.samples.len()
        }
    }

    #[test]
    fn silence_frames_from_ms_maps_and_clamps() {
        assert_eq!(silence_frames_from_ms(2000), SILENCE_FRAMES_END);
        assert_eq!(silence_frames_from_ms(500), SILENCE_FRAMES_END_MIN);
        assert_eq!(silence_frames_from_ms(4000), SILENCE_FRAMES_END_MAX);
        assert_eq!(silence_frames_from_ms(0), SILENCE_FRAMES_END_MIN);
        assert_eq!(silence_frames_from_ms(9_000), SILENCE_FRAMES_END_MAX);
        assert_eq!(silence_frames_from_ms(2500), 250);
    }

    #[test]
    fn custom_silence_frames_end_is_honored() {
        let mut gate = EnergyUtterance::with_silence_frames_end(50);
        assert_eq!(gate.silence_frames_end(), 50);
        let loud = tone(320, 0.2);
        for _ in 0..START_FRAMES {
            assert!(gate.push_frame(&loud, START_RMS + 0.01).is_none());
        }
        while gate.samples_len_for_test() < TALK_MIN_SAMPLES {
            assert!(gate.push_frame(&loud, START_RMS + 0.01).is_none());
        }
        let mut finished = None;
        for index in 0..50 {
            finished = gate.push_frame(&tone(320, 0.0), SILENCE_RMS / 2.0);
            if finished.is_some() {
                assert_eq!(index + 1, 50, "ended before the custom hangover");
                break;
            }
        }
        assert!(
            finished.is_some(),
            "custom hangover should end the utterance"
        );
        assert!(!gate.is_buffering());
    }

    #[test]
    fn set_silence_frames_end_clamps_and_keeps_the_buffer() {
        let mut gate = EnergyUtterance::new();
        assert_eq!(gate.silence_frames_end(), SILENCE_FRAMES_END);
        gate.set_silence_frames_end(1);
        assert_eq!(gate.silence_frames_end(), SILENCE_FRAMES_END_MIN);
        gate.set_silence_frames_end(9_999);
        assert_eq!(gate.silence_frames_end(), SILENCE_FRAMES_END_MAX);
        let loud = tone(320, 0.2);
        for _ in 0..START_FRAMES {
            let _ = gate.push_frame(&loud, START_RMS + 0.01);
        }
        assert!(gate.is_buffering());
        let stored = gate.samples_len_for_test();
        gate.set_silence_frames_end(80);
        assert!(gate.is_buffering());
        assert_eq!(gate.samples_len_for_test(), stored);
        assert_eq!(gate.silence_frames_end(), 80);
        gate.reset();
        assert!(!gate.is_buffering());
        assert_eq!(gate.samples_len_for_test(), 0);
        assert_eq!(gate.silence_frames_end(), 80);
    }
}
