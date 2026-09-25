//! In-memory capture for tests and the daemon demo.
//!
//! [`MockAudioCapture::stop`] clears any queued frames and [`MockAudioCapture::push_frame`]
//! drops audio while stopped, so hibernate cannot observe a late frame.

use std::collections::VecDeque;

use crate::{AudioCapture, AudioFormat, AudioFrame};

/// Microphone stand-in. It starts stopped.
///
/// `start` and `stop` cannot fail. The [`AudioCapture::Error`] type is
/// [`std::convert::Infallible`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `MockAudioCapture` is the public name of this backend.
pub struct MockAudioCapture {
    running: bool,
    pending: VecDeque<AudioFrame>,
}

impl MockAudioCapture {
    /// Whether `start` was called and `stop` has not.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.running
    }

    /// Queue one frame while running.
    ///
    /// Returns `false` and keeps the queue unchanged when capture is stopped.
    #[must_use]
    pub fn push_frame(&mut self, samples: &[i16]) -> bool {
        if !self.running {
            return false;
        }
        self.pending
            .push_back(AudioFrame::from_samples(samples.to_vec()));
        true
    }

    /// Queue one short listening tone while running (16 kHz mono).
    ///
    /// Amplitude breathes with `phase_secs` so a `GetStatus` poll can feed HUD
    /// particles from real PCM without a microphone. Returns `false` when
    /// capture is stopped.
    #[must_use]
    pub fn push_listening_tone(&mut self, phase_secs: f32) -> bool {
        const RATE: f32 = 16_000.0;
        const HZ: f32 = 440.0;
        const N: usize = 160; // 10 ms
        let breath = 0.18 + 0.42 * (0.5 + 0.5 * (phase_secs * 1.7).sin());
        let mut samples = [0_i16; N];
        for (i, sample) in samples.iter_mut().enumerate() {
            #[allow(
                clippy::cast_precision_loss,
                reason = "tone index 0..160 is exact in f32"
            )]
            let phase = (i as f32) * 2.0 * std::f32::consts::PI * HZ / RATE;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "intentional PCM quantization for the mock tone"
            )]
            {
                *sample = (breath * phase.sin() * f32::from(i16::MAX)) as i16;
            }
        }
        self.push_frame(&samples)
    }

    /// Next queued frame, and only while running.
    ///
    /// After `stop`, this returns [`None`] and the queue stays empty.
    /// [`AudioCapture::poll_frame`] is the same queue behind `Result`, so a
    /// caller that only has the trait can pull PCM the same way.
    #[must_use]
    pub fn poll_frame(&mut self) -> Option<AudioFrame> {
        if !self.running {
            self.pending.clear();
            return None;
        }
        self.pending.pop_front()
    }
}

impl AudioCapture for MockAudioCapture {
    type Error = std::convert::Infallible;

    fn start(&mut self) -> Result<(), Self::Error> {
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) -> Result<(), Self::Error> {
        self.running = false;
        self.pending.clear();
        Ok(())
    }

    fn poll_frame(&mut self) -> Result<Option<AudioFrame>, Self::Error> {
        Ok(MockAudioCapture::poll_frame(self))
    }

    fn format(&self) -> AudioFormat {
        AudioFormat::WAKE
    }
}

#[cfg(test)]
mod tests {
    use super::MockAudioCapture;
    use crate::AudioCapture;

    #[test]
    fn starts_stopped_and_stop_clears_the_running_flag() {
        let mut capture = MockAudioCapture::default();
        assert!(!capture.is_running());
        assert!(capture.poll_frame().is_none());

        capture.start().expect("mock start");
        assert!(capture.is_running());

        capture.stop().expect("mock stop");
        assert!(!capture.is_running());
        assert!(capture.poll_frame().is_none());
    }

    #[test]
    fn frames_are_delivered_in_order_only_while_running() {
        let mut capture = MockAudioCapture::default();
        assert!(!capture.push_frame(&[9]));
        assert!(capture.poll_frame().is_none());

        capture.start().expect("start");
        assert!(capture.push_frame(&[1, -1]));
        assert!(capture.push_frame(&[2]));
        assert_eq!(capture.poll_frame().expect("first").samples(), &[1, -1]);
        assert_eq!(capture.poll_frame().expect("second").samples(), &[2]);
        assert!(capture.poll_frame().is_none());
    }

    #[test]
    fn stop_ends_frame_delivery() {
        let mut capture = MockAudioCapture::default();
        capture.start().expect("start");
        assert!(capture.push_frame(&[1, 2, 3]));

        capture.stop().expect("stop");
        assert!(!capture.is_running());
        assert!(capture.poll_frame().is_none());
        assert!(!capture.push_frame(&[4]));
        assert!(capture.poll_frame().is_none());

        capture.start().expect("restart");
        assert!(capture.poll_frame().is_none(), "stop dropped the old queue");
        assert!(capture.push_frame(&[5]));
        assert_eq!(capture.poll_frame().expect("new frame").samples(), &[5]);
    }

    #[test]
    fn start_while_running_keeps_the_queue() {
        let mut capture = MockAudioCapture::default();
        capture.start().expect("start");
        assert!(capture.push_frame(&[7]));
        capture.start().expect("start again");
        assert!(capture.is_running());
        assert_eq!(capture.poll_frame().expect("kept").samples(), &[7]);
    }

    #[test]
    fn listening_tone_queues_pcm_only_while_running() {
        let mut capture = MockAudioCapture::default();
        assert!(!capture.push_listening_tone(0.0));
        capture.start().expect("start");
        assert!(capture.push_listening_tone(0.3));
        let frame = capture.poll_frame().expect("tone frame");
        assert_eq!(frame.samples().len(), 160);
        assert!(frame.samples().iter().any(|s| *s != 0));
    }

    #[test]
    fn stop_while_stopped_stays_stopped() {
        let mut capture = MockAudioCapture::default();
        capture.stop().expect("stop");
        assert!(!capture.is_running());
        assert!(capture.poll_frame().is_none());
    }
}
