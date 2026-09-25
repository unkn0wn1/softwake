//! One capture window into the PCM wake detector.
//!
//! Typed wake and sleep still use [`softwake_wake::TextWakeDetector`]. This
//! function is the microphone path: a frame's samples are
//! [`softwake_wake::WakeDetector::push_samples`].
//!
//! With `--features sherpa-kws` and weights under `$XDG_DATA_HOME/softwake/kws`
//! (or `~/.local/share/softwake/kws`), the daemon scores through
//! [`softwake_wake::SherpaKwsDetector`]. Otherwise [`softwake_wake::NullDetector`]
//! returns [`softwake_wake::PhraseHit::None`] for every window, including
//! silence. CI does not enable `sherpa-kws`.
//!
//! Capture frames are [`softwake_audio::AudioFormat::WAKE`]: 16 kHz, mono,
//! `i16`.

use softwake_audio::AudioFrame;
use softwake_wake::{NullDetector, PhraseHit, WakeDetector};

/// PCM engine used by [`crate::runtime::Runtime`].
pub(crate) enum PcmEngine {
    /// Default / CI stand-in. Never matches.
    Null(NullDetector),
    /// Real sherpa-onnx KWS when the feature is on and weights loaded.
    #[cfg(feature = "sherpa-kws")]
    Sherpa(softwake_wake::SherpaKwsDetector),
}

impl WakeDetector for PcmEngine {
    fn push_samples(&mut self, samples: &[i16]) -> PhraseHit {
        match self {
            Self::Null(detector) => detector.push_samples(samples),
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.push_samples(samples),
        }
    }
}

impl PcmEngine {
    /// Build the PCM detector for `agent_name`.
    ///
    /// When `sherpa-kws` is enabled and weights load, returns the sherpa
    /// engine configured with profile-driven wake/sleep phrases. Otherwise
    /// returns [`NullDetector`].
    #[must_use]
    pub(crate) fn for_agent(agent_name: &str) -> Self {
        #[cfg(feature = "sherpa-kws")]
        {
            let detector = softwake_wake::SherpaKwsDetector::for_agent(agent_name);
            if detector.weights_loaded() {
                eprintln!(
                    "softwaked: KWS weights loaded from {} (agent `{agent_name}`)",
                    detector.model_dir().display()
                );
                return Self::Sherpa(detector);
            }
            eprintln!(
                "softwaked: sherpa-kws built but weights missing under {} — voice wake idle; run scripts/install-kws-weights.sh (see README)",
                softwake_wake::SherpaKwsDetector::default_model_dir().display()
            );
        }
        let _ = agent_name;
        Self::Null(NullDetector)
    }

    /// `true` when real KWS weights are loaded.
    #[must_use]
    pub(crate) fn weights_loaded(&self) -> bool {
        match self {
            Self::Null(_) => false,
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.weights_loaded(),
        }
    }
}

/// Score one captured window.
#[must_use]
pub(crate) fn score_frame(detector: &mut impl WakeDetector, frame: &AudioFrame) -> PhraseHit {
    detector.push_samples(frame.samples())
}

#[cfg(test)]
mod tests {
    use softwake_audio::{AudioCapture, MockAudioCapture};
    use softwake_wake::{NullDetector, PhraseHit};

    use super::{PcmEngine, score_frame};

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
        let frame = take_frame(&[0; 160]);
        let mut detector = NullDetector;
        assert_eq!(score_frame(&mut detector, &frame), PhraseHit::None);
    }

    #[test]
    fn empty_window_scores_as_none() {
        let frame = take_frame(&[]);
        let mut detector = NullDetector;
        assert_eq!(score_frame(&mut detector, &frame), PhraseHit::None);
    }

    #[test]
    fn for_agent_without_weights_is_null() {
        let engine = PcmEngine::for_agent("Ada");
        assert!(!engine.weights_loaded());
    }
}
