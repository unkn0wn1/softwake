//! One capture window into the PCM wake detector.
//!
//! Typed wake and sleep still use [`softwake_wake::TextWakeDetector`]. This
//! function is the microphone path: a frame's samples are
//! [`softwake_wake::WakeDetector::push_samples`]. [`softwake_wake::NullDetector`]
//! returns [`softwake_wake::PhraseHit::None`] for every window, including
//! silence, until sherpa-onnx weights from ADR 0006 are loaded.
//!
//! Capture frames are [`softwake_audio::AudioFormat::WAKE`]: 16 kHz, mono,
//! `i16`.

use softwake_audio::AudioFrame;
use softwake_wake::{PhraseHit, WakeDetector};

/// Score one captured window.
///
/// The detector sees [`AudioFrame::samples`] and nothing else. An empty window
/// and a silent window both reach `push_samples`.
#[must_use]
pub(crate) fn score_frame(detector: &mut impl WakeDetector, frame: &AudioFrame) -> PhraseHit {
    detector.push_samples(frame.samples())
}

#[cfg(test)]
mod tests {
    use softwake_audio::{AudioCapture, MockAudioCapture};
    use softwake_wake::{NullDetector, PhraseHit};

    use super::score_frame;

    fn take_frame(samples: &[i16]) -> softwake_audio::AudioFrame {
        let mut capture = MockAudioCapture::default();
        capture.start().expect("mock start");
        assert!(capture.push_frame(samples));
        AudioCapture::poll_frame(&mut capture)
            .expect("poll")
            .expect("frame")
    }

    #[test]
    fn silence_from_mock_capture_scores_as_none() {
        let capture = MockAudioCapture::default();
        assert_eq!(capture.format().sample_rate_hz(), 16_000);
        assert_eq!(capture.format().channels(), 1);
        let frame = take_frame(&[0; 160]);
        assert_eq!(frame.samples(), &[0; 160]);

        let mut detector = NullDetector;
        assert_eq!(score_frame(&mut detector, &frame), PhraseHit::None);
    }

    #[test]
    fn empty_window_scores_as_none() {
        let frame = take_frame(&[]);
        let mut detector = NullDetector;
        assert_eq!(score_frame(&mut detector, &frame), PhraseHit::None);
    }
}
