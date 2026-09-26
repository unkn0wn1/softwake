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
use softwake_wake::{NullDetector, PhraseHit, SpotDetail, WakeDetector};

/// PCM engine used by [`crate::runtime::Runtime`].
pub(crate) enum PcmEngine {
    /// Default / CI stand-in. Never matches.
    Null(NullDetector),
    /// Real sherpa-onnx KWS when the feature is on and weights loaded.
    #[cfg(feature = "sherpa-kws")]
    Sherpa(softwake_wake::SherpaKwsDetector),
    /// Scripted hits for drain regression tests (no sherpa link).
    #[cfg(test)]
    Scripted(softwake_wake::ScriptedDetector),
}

impl WakeDetector for PcmEngine {
    fn push_samples(&mut self, samples: &[i16]) -> PhraseHit {
        self.score_detailed(samples).hit
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
            #[cfg(test)]
            Self::Scripted(_) => true,
        }
    }

    /// Stable backend label for startup / verbose logs.
    #[must_use]
    pub(crate) fn backend_name(&self) -> &'static str {
        match self {
            Self::Null(_) => "null",
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(_) => "sherpa-kws",
            #[cfg(test)]
            Self::Scripted(_) => "scripted",
        }
    }

    /// Configured wake phrases when sherpa is loaded; empty for null.
    #[must_use]
    pub(crate) fn wake_phrases(&self) -> &[String] {
        match self {
            Self::Null(_) => &[],
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.wake_phrases(),
            #[cfg(test)]
            Self::Scripted(_) => &[],
        }
    }

    /// Configured sleep phrases when sherpa is loaded; empty for null.
    #[must_use]
    pub(crate) fn sleep_phrases(&self) -> &[String] {
        match self {
            Self::Null(_) => &[],
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.sleep_phrases(),
            #[cfg(test)]
            Self::Scripted(_) => &[],
        }
    }

    /// Model directory when sherpa is in use.
    #[must_use]
    pub(crate) fn model_dir_display(&self) -> String {
        match self {
            Self::Null(_) => String::new(),
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.model_dir().display().to_string(),
            #[cfg(test)]
            Self::Scripted(_) => String::new(),
        }
    }

    /// Score one window and keep the raw keyword for verbose hear logs.
    #[must_use]
    pub(crate) fn score_detailed(&mut self, samples: &[i16]) -> SpotDetail {
        match self {
            Self::Null(detector) => SpotDetail {
                hit: detector.push_samples(samples),
                keyword: None,
            },
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.push_samples_detailed(samples),
            #[cfg(test)]
            Self::Scripted(detector) => {
                let hit = detector.push_samples(samples);
                let keyword = match hit {
                    PhraseHit::Wake => Some("scripted-wake".to_owned()),
                    PhraseHit::Sleep => Some("scripted-sleep".to_owned()),
                    PhraseHit::None => None,
                };
                SpotDetail { hit, keyword }
            }
        }
    }

    /// Install a scripted hit queue (tests only).
    #[cfg(test)]
    pub(crate) fn scripted(hits: impl IntoIterator<Item = PhraseHit>) -> Self {
        Self::Scripted(softwake_wake::ScriptedDetector::new(hits))
    }

    /// One-shot startup summary (profile phrases, backend, weights).
    pub(crate) fn log_startup(&self, agent_name: &str, verbosity: u8) {
        let feature = if cfg!(feature = "sherpa-kws") {
            "sherpa-kws"
        } else {
            "off"
        };
        eprintln!(
            "softwaked: KWS startup agent=`{agent_name}` backend={} feature={feature} weights_loaded={}",
            self.backend_name(),
            self.weights_loaded()
        );
        if !self.model_dir_display().is_empty() {
            eprintln!("softwaked: KWS model_dir={}", self.model_dir_display());
        }
        if verbosity >= 1 || self.weights_loaded() {
            let wake = self.wake_phrases();
            let sleep = self.sleep_phrases();
            if wake.is_empty() && sleep.is_empty() {
                eprintln!(
                    "softwaked: KWS phrases=(none — null detector; say-configured wake phrases will not match)"
                );
            } else {
                eprintln!(
                    "softwaked: KWS wake_phrases=[{}] sleep_phrases=[{}]",
                    wake.join(", "),
                    sleep.join(", ")
                );
            }
            self.log_registered_keywords(verbosity);
        }
        if verbosity >= 1 {
            eprintln!(
                "softwaked: KWS verbose={verbosity} (-v logs keyword hear/match; -vv also logs mic energy while sleeping)"
            );
        }
    }

    /// Log which phrases actually entered sherpa `keywords_buf` vs encode skips.
    fn log_registered_keywords(&self, verbosity: u8) {
        #[cfg(feature = "sherpa-kws")]
        {
            if let Self::Sherpa(detector) = self {
                let registered = detector.registered_phrases();
                let skipped = detector.skipped_phrases();
                if verbosity >= 1 || !skipped.is_empty() || detector.weights_loaded() {
                    eprintln!(
                        "softwaked: KWS keywords registered=[{}] skipped=[{}]",
                        registered.join(", "),
                        skipped.join(", ")
                    );
                }
                for phrase in skipped {
                    eprintln!(
                        "softwaked: KWS skipped unencodable phrase=`{phrase}` (not in sherpa keywords_buf)"
                    );
                }
            }
        }
        #[cfg(not(feature = "sherpa-kws"))]
        {
            let _ = (self, verbosity);
        }
    }
}

/// Score one captured window.
#[must_use]
pub(crate) fn score_frame(detector: &mut impl WakeDetector, frame: &AudioFrame) -> PhraseHit {
    detector.push_samples(frame.samples())
}

/// Score one captured window with keyword detail.
#[must_use]
pub(crate) fn score_frame_detailed(engine: &mut PcmEngine, frame: &AudioFrame) -> SpotDetail {
    engine.score_detailed(frame.samples())
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
        assert_eq!(engine.backend_name(), "null");
    }

    #[test]
    fn score_detailed_null_has_no_keyword() {
        let mut engine = PcmEngine::for_agent("Ada");
        let detail = engine.score_detailed(&[0; 160]);
        assert_eq!(detail.hit, PhraseHit::None);
        assert!(detail.keyword.is_none());
    }
}
