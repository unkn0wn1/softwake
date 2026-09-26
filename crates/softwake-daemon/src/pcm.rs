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
    /// Build the PCM detector for `agent_name` (near-miss probe off).
    ///
    /// When `sherpa-kws` is enabled and weights load, returns the sherpa
    /// engine configured with profile-driven wake/sleep phrases. Otherwise
    /// returns [`NullDetector`]. Runtime prefers [`Self::for_agent_with_verbosity`].
    #[must_use]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn for_agent(agent_name: &str) -> Self {
        Self::for_agent_with_verbosity(agent_name, 0)
    }

    /// Build the PCM detector for `agent_name`, enabling the near-miss probe at `-vv`.
    #[must_use]
    pub(crate) fn for_agent_with_verbosity(agent_name: &str, verbosity: u8) -> Self {
        #[cfg(feature = "sherpa-kws")]
        {
            let thresholds = resolve_kws_thresholds();
            let mut detector =
                softwake_wake::SherpaKwsDetector::for_agent_with_thresholds(agent_name, thresholds);
            detector.set_near_miss_probe(verbosity >= 2);
            if detector.weights_loaded() {
                eprintln!(
                    "softwaked: KWS profile={agent_name} weights loaded from {}",
                    detector.model_dir().display()
                );
                return Self::Sherpa(detector);
            }
            eprintln!(
                "softwaked: KWS profile={agent_name} weights missing under {} — voice wake idle; run scripts/install-kws-weights.sh (see README)",
                softwake_wake::SherpaKwsDetector::default_model_dir().display()
            );
        }
        let _ = (agent_name, verbosity);
        Self::Null(NullDetector)
    }

    /// Active thresholds when sherpa is loaded.
    #[must_use]
    pub(crate) fn thresholds_display(&self) -> Option<String> {
        #[cfg(feature = "sherpa-kws")]
        {
            if let Self::Sherpa(detector) = self {
                let t = detector.thresholds();
                return Some(format!(
                    "global={:.2} short={:.2} probe={:.2}",
                    t.global, t.short, t.probe
                ));
            }
        }
        let _ = self;
        None
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

    /// Force-reset the sherpa online streams after a mode transition or unmute.
    ///
    /// No-op for null / scripted engines.
    #[allow(clippy::unused_self)] // Only the sherpa variant holds resettable state.
    pub(crate) fn rearm(&mut self) {
        #[cfg(feature = "sherpa-kws")]
        {
            if let Self::Sherpa(detector) = self {
                detector.rearm();
            }
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

    /// Configured hibernate phrases when sherpa is loaded; empty for null.
    #[must_use]
    pub(crate) fn hibernate_phrases(&self) -> &[String] {
        match self {
            Self::Null(_) => &[],
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.hibernate_phrases(),
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
                near_miss: None,
            },
            #[cfg(feature = "sherpa-kws")]
            Self::Sherpa(detector) => detector.push_samples_detailed(samples),
            #[cfg(test)]
            Self::Scripted(detector) => {
                let (hit, keyword) = detector.pop_detailed();
                let keyword = keyword.or_else(|| match hit {
                    PhraseHit::Wake => Some("scripted-wake".to_owned()),
                    PhraseHit::Sleep => Some("scripted-sleep".to_owned()),
                    PhraseHit::Hibernate => Some("scripted-hibernate".to_owned()),
                    PhraseHit::None => None,
                });
                SpotDetail {
                    hit,
                    keyword,
                    near_miss: None,
                }
            }
        }
    }

    /// Install a scripted hit queue (tests only).
    #[cfg(test)]
    pub(crate) fn scripted(hits: impl IntoIterator<Item = PhraseHit>) -> Self {
        Self::Scripted(softwake_wake::ScriptedDetector::new(hits))
    }

    /// Install scripted hits with the raw keyword tag (tests only).
    #[cfg(test)]
    pub(crate) fn scripted_keywords<S>(hits: impl IntoIterator<Item = (PhraseHit, S)>) -> Self
    where
        S: Into<String>,
    {
        Self::Scripted(softwake_wake::ScriptedDetector::with_keywords(hits))
    }

    /// One-shot startup summary (profile phrases, backend, weights).
    ///
    /// `profile` is already `profile=<name>` or `profile=<name> id=<id>`.
    pub(crate) fn log_startup(&self, profile: &str, verbosity: u8) {
        let feature = if cfg!(feature = "sherpa-kws") {
            "sherpa-kws"
        } else {
            "off"
        };
        eprintln!(
            "softwaked: KWS {profile} startup backend={} feature={feature} weights_loaded={}",
            self.backend_name(),
            self.weights_loaded()
        );
        if !self.model_dir_display().is_empty() {
            eprintln!("softwaked: KWS model_dir={}", self.model_dir_display());
        }
        if verbosity >= 1 || self.weights_loaded() {
            let wake = self.wake_phrases();
            let sleep = self.sleep_phrases();
            let hibernate = self.hibernate_phrases();
            if wake.is_empty() && sleep.is_empty() && hibernate.is_empty() {
                eprintln!(
                    "softwaked: KWS {profile} phrases=(none — null detector; say-configured wake phrases will not match)"
                );
            } else {
                eprintln!(
                    "softwaked: KWS {profile} wake_phrases=[{}] sleep_phrases=[{}] hibernate_phrases=[{}]",
                    wake.join(", "),
                    sleep.join(", "),
                    hibernate.join(", ")
                );
            }
            self.log_registered_keywords(profile, verbosity);
        }
        if let Some(thresholds) = self.thresholds_display() {
            eprintln!("softwaked: KWS {profile} thresholds {thresholds}");
        }
        if verbosity >= 1 {
            eprintln!(
                "softwaked: KWS {profile} verbose={verbosity} (-v logs keyword hear/match; -vv also logs mic energy / near-miss while sleeping)"
            );
        }
    }

    /// Log which phrases actually entered sherpa `keywords_buf` vs encode skips.
    fn log_registered_keywords(&self, profile: &str, verbosity: u8) {
        #[cfg(feature = "sherpa-kws")]
        {
            if let Self::Sherpa(detector) = self {
                let registered = detector.registered_phrases();
                let skipped = detector.skipped_phrases();
                if verbosity >= 1 || !skipped.is_empty() || detector.weights_loaded() {
                    eprintln!(
                        "softwaked: KWS {profile} keywords registered=[{}] skipped=[{}]",
                        registered.join(", "),
                        skipped.join(", ")
                    );
                }
                for phrase in skipped {
                    eprintln!(
                        "softwaked: KWS {profile} skipped unencodable phrase=`{phrase}` (not in sherpa keywords_buf)"
                    );
                }
            }
        }
        #[cfg(not(feature = "sherpa-kws"))]
        {
            let _ = (self, profile, verbosity);
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

/// Resolve KWS thresholds from `softwake.json`, then env overrides.
///
/// Precedence: env (`SOFTWAKE_KWS_THRESHOLD` / `SOFTWAKE_KWS_SHORT_THRESHOLD`
/// floats, e.g. `0.12`) **wins over** file (`kws_*_milli` in softwake.json).
/// Missing config uses product defaults (0.15 / 0.10). Settings → General
/// writes the milli keys and then IPC `ReloadKws` rebuilds via this helper, so
/// an env override still wins on every live rebuild until it is unset.
#[cfg(feature = "sherpa-kws")]
#[must_use]
pub(crate) fn resolve_kws_thresholds() -> softwake_wake::KwsThresholds {
    use softwake_wake::{
        DEFAULT_GLOBAL_THRESHOLD, DEFAULT_PROBE_THRESHOLD, DEFAULT_SHORT_THRESHOLD, KwsThresholds,
    };

    let mut global = DEFAULT_GLOBAL_THRESHOLD;
    let mut short = DEFAULT_SHORT_THRESHOLD;

    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if let Ok(config_dir) = softwake_soul::resolve_config_dir(xdg.as_deref(), home.as_deref()) {
        if let Ok(app) = softwake_soul::load_app_config(&config_dir) {
            global = f32::from(app.kws_threshold_milli) / 1000.0;
            short = f32::from(app.kws_short_threshold_milli) / 1000.0;
        }
    }
    if let Ok(raw) = std::env::var("SOFTWAKE_KWS_THRESHOLD") {
        if let Ok(value) = raw.parse::<f32>() {
            global = value;
        }
    }
    if let Ok(raw) = std::env::var("SOFTWAKE_KWS_SHORT_THRESHOLD") {
        if let Ok(value) = raw.parse::<f32>() {
            short = value;
        }
    }
    KwsThresholds::clamped(global, short, DEFAULT_PROBE_THRESHOLD)
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
    fn rearm_is_noop_on_null() {
        let mut engine = PcmEngine::for_agent("Sally");
        engine.rearm();
        engine.rearm();
    }

    #[test]
    fn score_detailed_null_has_no_keyword() {
        let mut engine = PcmEngine::for_agent("Ada");
        let detail = engine.score_detailed(&[0; 160]);
        assert_eq!(detail.hit, PhraseHit::None);
        assert!(detail.keyword.is_none());
    }
}
