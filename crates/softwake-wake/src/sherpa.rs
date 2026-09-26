//! Sherpa-onnx keyword spotting PCM detector.
//!
//! Compiles only with `--features sherpa-kws`. Loads an English Zipformer KWS
//! checkpoint from [`SherpaKwsDetector::model_dir`] when the ONNX files and
//! `tokens.txt` are present. Keywords are BPE-encoded with a longest-piece
//! matcher over `tokens.txt` (pieces sorted by length descending; no
//! `SentencePiece` link — that duplicates protobuf symbols with
//! `sherpa-onnx-sys`). Without weights, [`WakeDetector::push_samples`]
//! returns [`PhraseHit::None`]. CI does not enable this feature.

use std::fs;
use std::path::{Path, PathBuf};

use sherpa_onnx::{
    KeywordSpotter, KeywordSpotterConfig, OnlineModelConfig, OnlineStream,
    OnlineTransducerModelConfig,
};

use crate::phrases::{hit_from_keyword, phrases_for_agent};
use crate::short_word::is_short_single_word;
use crate::thresholds::KwsThresholds;
use crate::{PhraseHit, WakeDetector};

/// PCM detector for sherpa-onnx keyword spotting.
#[allow(clippy::module_name_repetitions)] // Public engine name.
pub struct SherpaKwsDetector {
    model_dir: PathBuf,
    wake_phrases: Vec<String>,
    sleep_phrases: Vec<String>,
    hibernate_phrases: Vec<String>,
    /// Phrases that encoded into `keywords_buf` (config ∩ sherpa).
    registered_phrases: Vec<String>,
    /// Phrases skipped because BPE encode failed.
    skipped_phrases: Vec<String>,
    thresholds: KwsThresholds,
    /// When true, feed a low-threshold probe stream for `-vv` near-miss logs.
    near_miss_probe: bool,
    engine: Option<LoadedEngine>,
    /// Accepted audio since the last `KeywordSpotter::reset`.
    stream_budget: crate::stream_budget::StreamBudget,
}

struct LoadedEngine {
    spotter: KeywordSpotter,
    stream: OnlineStream,
    /// Low-threshold stream for near-miss diagnostics. Same phrases, easier fire.
    probe_stream: OnlineStream,
}

impl std::fmt::Debug for SherpaKwsDetector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SherpaKwsDetector")
            .field("model_dir", &self.model_dir)
            .field("wake_phrases", &self.wake_phrases)
            .field("sleep_phrases", &self.sleep_phrases)
            .field("hibernate_phrases", &self.hibernate_phrases)
            .field("registered_phrases", &self.registered_phrases)
            .field("skipped_phrases", &self.skipped_phrases)
            .field("thresholds", &self.thresholds)
            .field("near_miss_probe", &self.near_miss_probe)
            .field("weights_loaded", &self.engine.is_some())
            .field(
                "samples_since_reset",
                &self.stream_budget.samples_since_reset(),
            )
            .finish()
    }
}

impl SherpaKwsDetector {
    /// Remember phrases and try to load weights from `model_dir`.
    #[must_use]
    pub fn new<W, S, H, T, U, V>(
        model_dir: impl Into<PathBuf>,
        wake_phrases: W,
        sleep_phrases: S,
        hibernate_phrases: H,
    ) -> Self
    where
        W: IntoIterator<Item = T>,
        S: IntoIterator<Item = U>,
        H: IntoIterator<Item = V>,
        T: AsRef<str>,
        U: AsRef<str>,
        V: AsRef<str>,
    {
        Self::with_thresholds(
            model_dir,
            wake_phrases,
            sleep_phrases,
            hibernate_phrases,
            KwsThresholds::default(),
        )
    }

    /// Like [`Self::new`] with explicit trigger thresholds.
    #[must_use]
    pub fn with_thresholds<W, S, H, T, U, V>(
        model_dir: impl Into<PathBuf>,
        wake_phrases: W,
        sleep_phrases: S,
        hibernate_phrases: H,
        thresholds: KwsThresholds,
    ) -> Self
    where
        W: IntoIterator<Item = T>,
        S: IntoIterator<Item = U>,
        H: IntoIterator<Item = V>,
        T: AsRef<str>,
        U: AsRef<str>,
        V: AsRef<str>,
    {
        let model_dir = model_dir.into();
        let wake_phrases = normalize(wake_phrases);
        let sleep_phrases = normalize(sleep_phrases);
        let hibernate_phrases = normalize(hibernate_phrases);
        let thresholds =
            KwsThresholds::clamped(thresholds.global, thresholds.short, thresholds.probe);
        let (engine, registered_phrases, skipped_phrases) = load_engine(
            &model_dir,
            &wake_phrases,
            &sleep_phrases,
            &hibernate_phrases,
            thresholds,
        );
        Self {
            model_dir,
            wake_phrases,
            sleep_phrases,
            hibernate_phrases,
            registered_phrases,
            skipped_phrases,
            thresholds,
            near_miss_probe: false,
            engine,
            stream_budget: crate::stream_budget::StreamBudget::new(),
        }
    }

    /// Detector for the active agent / profile name and [`Self::default_model_dir`].
    #[must_use]
    pub fn for_agent(agent_name: &str) -> Self {
        Self::for_agent_with_thresholds(agent_name, KwsThresholds::default())
    }

    /// [`Self::for_agent`] with explicit thresholds.
    #[must_use]
    pub fn for_agent_with_thresholds(agent_name: &str, thresholds: KwsThresholds) -> Self {
        let phrases = phrases_for_agent(agent_name);
        Self::with_thresholds(
            Self::default_model_dir(),
            phrases.wake,
            phrases.sleep,
            phrases.hibernate,
            thresholds,
        )
    }

    /// Softwake defaults under [`Self::default_model_dir`].
    #[must_use]
    pub fn with_default_phrases() -> Self {
        Self::for_agent(crate::phrases::DEFAULT_AGENT_NAME)
    }

    /// `$XDG_DATA_HOME/softwake/kws`, else `$HOME/.local/share/softwake/kws`.
    #[must_use]
    pub fn default_model_dir() -> PathBuf {
        model_dir_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
    }

    /// Configured model directory.
    #[must_use]
    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    /// Wake phrases, lowercase, in order.
    #[must_use]
    pub fn wake_phrases(&self) -> &[String] {
        &self.wake_phrases
    }

    /// Sleep phrases, lowercase, in order.
    #[must_use]
    pub fn sleep_phrases(&self) -> &[String] {
        &self.sleep_phrases
    }

    /// Hibernate phrases, lowercase, in order.
    #[must_use]
    pub fn hibernate_phrases(&self) -> &[String] {
        &self.hibernate_phrases
    }

    /// Phrases that made it into the sherpa `keywords_buf`.
    #[must_use]
    pub fn registered_phrases(&self) -> &[String] {
        &self.registered_phrases
    }

    /// Phrases skipped because the BPE table could not encode them.
    #[must_use]
    pub fn skipped_phrases(&self) -> &[String] {
        &self.skipped_phrases
    }

    /// Active trigger thresholds.
    #[must_use]
    pub const fn thresholds(&self) -> KwsThresholds {
        self.thresholds
    }

    /// Enable or disable the low-threshold near-miss probe stream.
    ///
    /// The daemon turns this on at `-vv` so operators can see keywords that
    /// almost fired. Probe hits never change voice state.
    pub fn set_near_miss_probe(&mut self, enabled: bool) {
        self.near_miss_probe = enabled;
    }

    /// `true` when the near-miss probe stream is fed.
    #[must_use]
    pub const fn near_miss_probe(&self) -> bool {
        self.near_miss_probe
    }

    /// Force-reset both online streams and the sample budget.
    ///
    /// Call after an applied wake/sleep/hibernate transition and when
    /// half-duplex mute lifts. Mute drops mic frames without feeding the
    /// spotter; without a rearm the OnlineStream can stay silent across gaps
    /// (no main hit and no near-miss) until a hard budget lucks through.
    pub fn rearm(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.spotter.reset(&engine.stream);
            engine.spotter.reset(&engine.probe_stream);
        }
        self.stream_budget.reset();
    }

    /// `true` when ONNX + tokens loaded and a stream is open.
    #[must_use]
    pub fn weights_loaded(&self) -> bool {
        self.engine.is_some()
    }
}

impl WakeDetector for SherpaKwsDetector {
    fn push_samples(&mut self, samples: &[i16]) -> PhraseHit {
        self.push_samples_detailed(samples).hit
    }
}

impl SherpaKwsDetector {
    /// Score samples and return the last decoded keyword (if any) with the hit.
    ///
    /// Used by the daemon for `-v` / `-vv` hear logs. Without weights, or on
    /// silence with no decode, [`SpotDetail::keyword`] stays `None`. When the
    /// near-miss probe is on and fires without a main hit, [`SpotDetail::near_miss`]
    /// carries that tag.
    #[must_use]
    pub fn push_samples_detailed(&mut self, samples: &[i16]) -> crate::SpotDetail {
        if self.engine.is_none() {
            return crate::SpotDetail::default();
        }
        if samples.is_empty() {
            return crate::SpotDetail::default();
        }
        let speech_energy = window_has_speech_energy(samples);
        // Soft budget waits for quiet so slow phrases are not chopped. Hard
        // budget still recovers a long awake stream (#56).
        if self.stream_budget.begin_window(speech_energy)
            && let Some(engine) = self.engine.as_mut()
        {
            engine.spotter.reset(&engine.stream);
            engine.spotter.reset(&engine.probe_stream);
        }
        let Some(engine) = self.engine.as_mut() else {
            return crate::SpotDetail::default();
        };
        let float_samples: Vec<f32> = samples
            .iter()
            .map(|sample| f32::from(*sample) / f32::from(i16::MAX))
            .collect();
        engine.stream.accept_waveform(16_000, &float_samples);
        if self.near_miss_probe {
            engine.probe_stream.accept_waveform(16_000, &float_samples);
        }
        let mut last_keyword = None;
        let mut last_hit = PhraseHit::None;
        let mut keyword_reset = false;
        while engine.spotter.is_ready(&engine.stream) {
            engine.spotter.decode(&engine.stream);
            if let Some(result) = engine.spotter.get_result(&engine.stream) {
                if !result.keyword.is_empty() {
                    let hit = hit_from_keyword(
                        &result.keyword,
                        &self.wake_phrases,
                        &self.sleep_phrases,
                        &self.hibernate_phrases,
                    );
                    engine.spotter.reset(&engine.stream);
                    engine.spotter.reset(&engine.probe_stream);
                    keyword_reset = true;
                    last_keyword = Some(result.keyword);
                    last_hit = hit;
                    if hit != PhraseHit::None {
                        break;
                    }
                }
            }
        }
        let mut near_miss = None;
        if self.near_miss_probe && last_keyword.is_none() {
            while engine.spotter.is_ready(&engine.probe_stream) {
                engine.spotter.decode(&engine.probe_stream);
                if let Some(result) = engine.spotter.get_result(&engine.probe_stream) {
                    if !result.keyword.is_empty() {
                        engine.spotter.reset(&engine.probe_stream);
                        near_miss = Some(result.keyword);
                        break;
                    }
                }
            }
        }
        self.stream_budget
            .finish_window(samples.len(), keyword_reset);
        crate::SpotDetail {
            hit: last_hit,
            keyword: last_keyword,
            near_miss,
        }
    }
}

/// Peak-normalized RMS energy floor matching the daemon capture energy log.
fn window_has_speech_energy(samples: &[i16]) -> bool {
    if samples.is_empty() {
        return false;
    }
    // Capture windows are tens of milliseconds (≪ u16::MAX samples).
    let n = u16::try_from(samples.len()).unwrap_or(u16::MAX);
    let sum_sq: f32 = samples
        .iter()
        .map(|sample| {
            let x = f32::from(*sample) / f32::from(i16::MAX);
            x * x
        })
        .sum();
    let rms = (sum_sq / f32::from(n)).sqrt();
    rms >= 0.02
}

#[allow(clippy::field_reassign_with_default)] // KeywordSpotterConfig has no builder.
fn load_engine(
    model_dir: &Path,
    wake_phrases: &[String],
    sleep_phrases: &[String],
    hibernate_phrases: &[String],
    thresholds: KwsThresholds,
) -> (Option<LoadedEngine>, Vec<String>, Vec<String>) {
    let Some(paths) = ModelPaths::discover(model_dir) else {
        return (None, Vec::new(), Vec::new());
    };
    let Some(pieces) = load_token_pieces(&paths.tokens) else {
        return (None, Vec::new(), Vec::new());
    };
    let built = build_keywords_buf(
        &pieces,
        wake_phrases,
        sleep_phrases,
        hibernate_phrases,
        thresholds,
    );
    let registered = built.registered.clone();
    let skipped = built.skipped.clone();
    let Some(keywords_buf) = built.buf else {
        return (None, registered, skipped);
    };
    let probe_buf = build_probe_keywords_buf(
        &pieces,
        wake_phrases,
        sleep_phrases,
        hibernate_phrases,
        thresholds,
    );
    let mut config = KeywordSpotterConfig::default();
    config.model_config = OnlineModelConfig {
        transducer: OnlineTransducerModelConfig {
            encoder: Some(paths.encoder),
            decoder: Some(paths.decoder),
            joiner: Some(paths.joiner),
        },
        tokens: Some(paths.tokens),
        num_threads: 1,
        provider: Some("cpu".to_owned()),
        debug: false,
        ..OnlineModelConfig::default()
    };
    config.keywords_buf = Some(keywords_buf);
    config.keywords_threshold = thresholds.global;
    config.keywords_score = 1.0;
    let Some(spotter) = KeywordSpotter::create(&config) else {
        return (None, registered, skipped);
    };
    let stream = spotter.create_stream();
    let probe_stream = match probe_buf.as_deref() {
        Some(buf) => spotter.create_stream_with_keywords(buf),
        None => spotter.create_stream(),
    };
    (
        Some(LoadedEngine {
            spotter,
            stream,
            probe_stream,
        }),
        registered,
        skipped,
    )
}

struct ModelPaths {
    encoder: String,
    decoder: String,
    joiner: String,
    tokens: String,
}

impl ModelPaths {
    fn discover(model_dir: &Path) -> Option<Self> {
        if !model_dir.is_dir() {
            return None;
        }
        let encoder = find_model_file(model_dir, "encoder")?;
        let decoder = find_model_file(model_dir, "decoder")?;
        let joiner = find_model_file(model_dir, "joiner")?;
        let tokens = first_existing(model_dir, &["tokens.txt"])?;
        Some(Self {
            encoder,
            decoder,
            joiner,
            tokens,
        })
    }
}

#[allow(clippy::case_sensitive_file_extension_comparisons)] // Model files ship lowercase .onnx.
fn find_model_file(dir: &Path, prefix: &str) -> Option<String> {
    let preferred = [
        format!("{prefix}-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
        format!("{prefix}.int8.onnx"),
        format!("{prefix}.onnx"),
    ];
    for name in &preferred {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    let mut matches: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".onnx"))
        })
        .collect();
    matches.sort_by_key(|path| {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        (!name.contains("int8"), name.to_owned())
    });
    matches
        .into_iter()
        .next()
        .map(|path| path.to_string_lossy().into_owned())
}

fn first_existing(dir: &Path, names: &[&str]) -> Option<String> {
    for name in names {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

fn load_token_pieces(tokens_path: &str) -> Option<Vec<String>> {
    let text = fs::read_to_string(tokens_path).ok()?;
    let mut pieces = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // `TOKEN id` — token may contain spaces only in theory; GigaSpeech uses none.
        let Some((token, _)) = line.rsplit_once(' ') else {
            continue;
        };
        if token.starts_with('<') && token.ends_with('>') {
            continue;
        }
        pieces.push(token.to_owned());
    }
    if pieces.is_empty() {
        return None;
    }
    pieces.sort_by_key(|piece| (std::cmp::Reverse(piece.len()), piece.clone()));
    Some(pieces)
}

/// Result of encoding configured phrases into a sherpa `keywords_buf`.
struct KeywordBuild {
    /// Joined keyword lines, or `None` when nothing encoded.
    buf: Option<String>,
    /// Phrases that produced a line (config ∩ sherpa).
    registered: Vec<String>,
    /// Phrases the BPE table could not encode.
    skipped: Vec<String>,
}

fn build_keywords_buf(
    pieces: &[String],
    wake_phrases: &[String],
    sleep_phrases: &[String],
    hibernate_phrases: &[String],
    thresholds: KwsThresholds,
) -> KeywordBuild {
    // Primary wake phrase is the active profile name (`phrases_for_agent`).
    let profile_name = wake_phrases.first().map(String::as_str).unwrap_or("");
    let mut lines = Vec::new();
    let mut registered = Vec::new();
    let mut skipped = Vec::new();
    for phrase in wake_phrases
        .iter()
        .chain(sleep_phrases.iter())
        .chain(hibernate_phrases.iter())
    {
        // Skip phrases the BPE table cannot encode instead of failing the
        // whole keyword list (one bad name must not idle voice wake).
        if let Some(line) = encode_keyword_line(pieces, phrase, profile_name, thresholds) {
            lines.push(line);
            registered.push(phrase.clone());
        } else {
            skipped.push(phrase.clone());
        }
    }
    KeywordBuild {
        buf: if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        },
        registered,
        skipped,
    }
}

fn build_probe_keywords_buf(
    pieces: &[String],
    wake_phrases: &[String],
    sleep_phrases: &[String],
    hibernate_phrases: &[String],
    thresholds: KwsThresholds,
) -> Option<String> {
    let mut lines = Vec::new();
    for phrase in wake_phrases
        .iter()
        .chain(sleep_phrases.iter())
        .chain(hibernate_phrases.iter())
    {
        if let Some(line) = encode_probe_keyword_line(pieces, phrase, thresholds) {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn encode_keyword_line(
    pieces: &[String],
    phrase: &str,
    profile_name: &str,
    thresholds: KwsThresholds,
) -> Option<String> {
    let mut line = encode_keyword_tokens(pieces, phrase)?;
    // Profile name / hey <name> get an extra ease vs product short words —
    // "sally" under-fires relative to "softwake" at the same short threshold.
    // Other short singles (`sleep`, `hi`, product `softwake` when not the
    // active name) keep the short suffix. Multi-word non-name phrases use global.
    if is_profile_wake_phrase(phrase, profile_name) {
        line.push_str(&thresholds.name_suffix());
    } else if is_short_single_word(phrase) {
        line.push_str(&thresholds.short_suffix());
    }
    Some(line)
}

/// Bare profile name or `hey <name>` (case already folded by callers).
fn is_profile_wake_phrase(phrase: &str, profile_name: &str) -> bool {
    let name = profile_name.trim();
    if name.is_empty() {
        return false;
    }
    phrase == name || phrase == format!("hey {name}")
}

fn encode_probe_keyword_line(
    pieces: &[String],
    phrase: &str,
    thresholds: KwsThresholds,
) -> Option<String> {
    let mut line = encode_keyword_tokens(pieces, phrase)?;
    line.push_str(&thresholds.probe_suffix());
    Some(line)
}

fn encode_keyword_tokens(pieces: &[String], phrase: &str) -> Option<String> {
    let upper = phrase.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return None;
    }
    // SentencePiece: leading ▁ and spaces become ▁.
    let mut surface = String::from("▁");
    for (index, word) in upper.split_whitespace().enumerate() {
        if index > 0 {
            surface.push('▁');
        }
        surface.push_str(word);
    }
    let mut encoded = Vec::new();
    let mut index = 0;
    let chars: Vec<char> = surface.chars().collect();
    while index < chars.len() {
        let rest: String = chars[index..].iter().collect();
        // `pieces` is sorted longest-first in [`load_token_pieces`], so the
        // first prefix match is the longest BPE piece (not first-id greedy).
        let piece = pieces
            .iter()
            .find(|candidate| rest.starts_with(candidate.as_str()))?;
        encoded.push(piece.as_str());
        index += piece.chars().count();
    }
    let tag = phrase.trim().to_ascii_lowercase().replace(' ', "_");
    Some(format!("{} @{tag}", encoded.join(" ")))
}

fn normalize<I, S>(phrases: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    phrases
        .into_iter()
        .map(|phrase| phrase.as_ref().trim().to_ascii_lowercase())
        .filter(|phrase| !phrase.is_empty())
        .collect()
}

fn model_dir_from(
    xdg_data_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(data_home) = xdg_data_home {
        return PathBuf::from(data_home).join("softwake/kws");
    }
    match home {
        Some(home) => PathBuf::from(home).join(".local/share/softwake/kws"),
        None => PathBuf::from(".local/share/softwake/kws"),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{SherpaKwsDetector, encode_keyword_line, load_token_pieces, model_dir_from};
    use crate::thresholds::KwsThresholds;
    use crate::{PhraseHit, WakeDetector};

    #[test]
    fn silence_scores_as_none_without_weights() {
        let mut detector = SherpaKwsDetector::new(
            PathBuf::from("/tmp/softwake-kws-missing"),
            ["hey softwake"],
            ["go to sleep"],
            ["deep sleep"],
        );
        assert!(!detector.weights_loaded());
        assert_eq!(detector.push_samples(&[]), PhraseHit::None);
        assert_eq!(detector.push_samples(&[0; 160]), PhraseHit::None);
    }

    #[test]
    fn blank_phrases_are_dropped() {
        let detector = SherpaKwsDetector::new(
            PathBuf::from("kws-models"),
            ["  hey softwake  ", " "],
            ["GO TO SLEEP"],
            [" deep sleep "],
        );
        assert_eq!(detector.wake_phrases(), ["hey softwake".to_owned()]);
        assert_eq!(detector.sleep_phrases(), ["go to sleep".to_owned()]);
    }

    #[test]
    fn model_dir_prefers_xdg_data_home() {
        assert_eq!(
            model_dir_from(
                Some(OsString::from("/tmp/softwake-data")),
                Some(OsString::from("/tmp/softwake-home"))
            ),
            PathBuf::from("/tmp/softwake-data/softwake/kws")
        );
        assert_eq!(
            model_dir_from(None, Some(OsString::from("/tmp/softwake-home"))),
            PathBuf::from("/tmp/softwake-home/.local/share/softwake/kws")
        );
    }

    #[test]
    fn agent_phrases_include_profile_name() {
        let detector = SherpaKwsDetector::for_agent("Ada");
        assert!(detector.wake_phrases().iter().any(|p| p == "ada"));
        assert!(detector.wake_phrases().iter().any(|p| p == "hey ada"));
        assert!(
            detector
                .sleep_phrases()
                .iter()
                .any(|p| p == "goodnight ada")
        );
        assert!(detector.wake_phrases().iter().any(|phrase| phrase == "hi"));
        assert!(
            detector
                .sleep_phrases()
                .iter()
                .any(|phrase| phrase == "sleep")
        );
        assert_eq!(detector.hibernate_phrases(), ["deep sleep".to_owned()]);
    }

    #[test]
    fn defaults_expose_looser_thresholds() {
        let detector = SherpaKwsDetector::for_agent("Sally");
        let t = detector.thresholds();
        assert!((t.global - 0.15).abs() < 0.000_1);
        assert!((t.short - 0.10).abs() < 0.000_1);
        assert!((t.probe - 0.05).abs() < 0.000_1);
    }

    #[test]
    fn rearm_is_safe_without_weights() {
        let mut detector = SherpaKwsDetector::with_thresholds(
            "/tmp/softwake-no-such-kws-dir",
            ["hey softwake"],
            ["sleep"],
            ["deep sleep"],
            crate::KwsThresholds::default(),
        );
        assert!(!detector.weights_loaded());
        detector.rearm();
        // Second call also fine.
        detector.rearm();
    }

    #[test]
    fn greedy_tokens_match_known_softwake_encoding() {
        let tokens = PathBuf::from(
            "/tmp/kws-extract/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01/tokens.txt",
        );
        let tokens = if tokens.is_file() {
            tokens
        } else {
            PathBuf::from("/home/spence/.local/share/softwake/kws/tokens.txt")
        };
        if !tokens.is_file() {
            return;
        }
        let pieces = load_token_pieces(tokens.to_str().expect("utf8")).expect("pieces");
        let thresholds = KwsThresholds::default();
        let line =
            encode_keyword_line(&pieces, "hey softwake", "sally", thresholds).expect("encode");
        assert!(line.ends_with("@hey_softwake"));
        assert!(line.starts_with("▁HE Y ▁SO F T W A KE "));
        // Product softwake while profile is sally → short suffix, not name ease.
        let softwake = encode_keyword_line(&pieces, "softwake", "sally", thresholds)
            .expect("softwake encodes");
        assert!(
            softwake.contains("#0.10"),
            "product short word keeps short threshold: {softwake}"
        );
        let sally =
            encode_keyword_line(&pieces, "sally", "sally", thresholds).expect("sally encodes");
        assert!(sally.contains("@sally"), "{sally}");
        assert!(
            sally.contains("#0.05"),
            "profile name gets name ease below short: {sally}"
        );
        let hey_sally =
            encode_keyword_line(&pieces, "hey sally", "sally", thresholds).expect("hey sally");
        assert!(hey_sally.contains("@hey_sally"), "{hey_sally}");
        assert!(
            hey_sally.contains("#0.05"),
            "hey <profile> also gets name ease: {hey_sally}"
        );
    }
}
