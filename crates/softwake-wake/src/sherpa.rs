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
use crate::{PhraseHit, WakeDetector};

/// PCM detector for sherpa-onnx keyword spotting.
#[allow(clippy::module_name_repetitions)] // Public engine name.
pub struct SherpaKwsDetector {
    model_dir: PathBuf,
    wake_phrases: Vec<String>,
    sleep_phrases: Vec<String>,
    /// Phrases that encoded into `keywords_buf` (config ∩ sherpa).
    registered_phrases: Vec<String>,
    /// Phrases skipped because BPE encode failed.
    skipped_phrases: Vec<String>,
    engine: Option<LoadedEngine>,
    /// Accepted audio since the last `KeywordSpotter::reset`.
    stream_budget: crate::stream_budget::StreamBudget,
}

struct LoadedEngine {
    spotter: KeywordSpotter,
    stream: OnlineStream,
}

impl std::fmt::Debug for SherpaKwsDetector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SherpaKwsDetector")
            .field("model_dir", &self.model_dir)
            .field("wake_phrases", &self.wake_phrases)
            .field("sleep_phrases", &self.sleep_phrases)
            .field("registered_phrases", &self.registered_phrases)
            .field("skipped_phrases", &self.skipped_phrases)
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
    pub fn new<W, S, T, U>(model_dir: impl Into<PathBuf>, wake_phrases: W, sleep_phrases: S) -> Self
    where
        W: IntoIterator<Item = T>,
        S: IntoIterator<Item = U>,
        T: AsRef<str>,
        U: AsRef<str>,
    {
        let model_dir = model_dir.into();
        let wake_phrases = normalize(wake_phrases);
        let sleep_phrases = normalize(sleep_phrases);
        let (engine, registered_phrases, skipped_phrases) =
            load_engine(&model_dir, &wake_phrases, &sleep_phrases);
        Self {
            model_dir,
            wake_phrases,
            sleep_phrases,
            registered_phrases,
            skipped_phrases,
            engine,
            stream_budget: crate::stream_budget::StreamBudget::new(),
        }
    }

    /// Detector for the active agent / profile name and [`Self::default_model_dir`].
    #[must_use]
    pub fn for_agent(agent_name: &str) -> Self {
        let (wake, sleep) = phrases_for_agent(agent_name);
        Self::new(Self::default_model_dir(), wake, sleep)
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
    /// silence with no decode, [`SpotDetail::keyword`] stays `None`.
    #[must_use]
    pub fn push_samples_detailed(&mut self, samples: &[i16]) -> crate::SpotDetail {
        let Some(engine) = self.engine.as_mut() else {
            return crate::SpotDetail {
                hit: PhraseHit::None,
                keyword: None,
            };
        };
        if samples.is_empty() {
            return crate::SpotDetail {
                hit: PhraseHit::None,
                keyword: None,
            };
        }
        // sherpa-onnx auto-resets only after ~1.5 s of trailing blanks. Awake
        // speech never builds that run, and a keyword is the only other reset,
        // so a long awake session stops emitting keywords. Reset before this
        // window once the sample budget is spent. `begin_window` / `finish_window`
        // match `stream_budget` tests (CI does not link sherpa).
        if self.stream_budget.begin_window() {
            engine.spotter.reset(&engine.stream);
        }
        let float_samples: Vec<f32> = samples
            .iter()
            .map(|sample| f32::from(*sample) / f32::from(i16::MAX))
            .collect();
        engine.stream.accept_waveform(16_000, &float_samples);
        let mut last_keyword = None;
        let mut last_hit = PhraseHit::None;
        let mut keyword_reset = false;
        while engine.spotter.is_ready(&engine.stream) {
            engine.spotter.decode(&engine.stream);
            if let Some(result) = engine.spotter.get_result(&engine.stream) {
                if !result.keyword.is_empty() {
                    let hit =
                        hit_from_keyword(&result.keyword, &self.wake_phrases, &self.sleep_phrases);
                    engine.spotter.reset(&engine.stream);
                    keyword_reset = true;
                    last_keyword = Some(result.keyword);
                    last_hit = hit;
                    if hit != PhraseHit::None {
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
        }
    }
}

fn load_engine(
    model_dir: &Path,
    wake_phrases: &[String],
    sleep_phrases: &[String],
) -> (Option<LoadedEngine>, Vec<String>, Vec<String>) {
    let Some(paths) = ModelPaths::discover(model_dir) else {
        return (None, Vec::new(), Vec::new());
    };
    let Some(pieces) = load_token_pieces(&paths.tokens) else {
        return (None, Vec::new(), Vec::new());
    };
    let built = build_keywords_buf(&pieces, wake_phrases, sleep_phrases);
    let registered = built.registered.clone();
    let skipped = built.skipped.clone();
    let Some(keywords_buf) = built.buf else {
        return (None, registered, skipped);
    };
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
    config.keywords_threshold = 0.25;
    config.keywords_score = 1.0;
    let Some(spotter) = KeywordSpotter::create(&config) else {
        return (None, registered, skipped);
    };
    let stream = spotter.create_stream();
    (Some(LoadedEngine { spotter, stream }), registered, skipped)
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
) -> KeywordBuild {
    let mut lines = Vec::new();
    let mut registered = Vec::new();
    let mut skipped = Vec::new();
    for phrase in wake_phrases.iter().chain(sleep_phrases.iter()) {
        // Skip phrases the BPE table cannot encode instead of failing the
        // whole keyword list (one bad name must not idle voice wake).
        if let Some(line) = encode_keyword_line(pieces, phrase) {
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

fn encode_keyword_line(pieces: &[String], phrase: &str) -> Option<String> {
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
    let mut line = format!("{} @{tag}", encoded.join(" "));
    // Short single-word names (e.g. "sally") are harder to spot at the
    // default 0.25 threshold. sherpa per-keyword `#threshold` lowers the
    // trigger bar for that line only (docs: lower = easier). Multi-word
    // "hey <name>" stays on the global default and is usually more reliable.
    if is_short_single_word(phrase) {
        line.push_str(" #0.15");
    }
    Some(line)
}

/// True for a single whitespace-free word of at most 8 letters.
fn is_short_single_word(phrase: &str) -> bool {
    let trimmed = phrase.trim();
    !trimmed.is_empty() && !trimmed.chars().any(char::is_whitespace) && trimmed.chars().count() <= 8
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

    use super::{
        SherpaKwsDetector, encode_keyword_line, is_short_single_word, load_token_pieces,
        model_dir_from,
    };
    use crate::{PhraseHit, WakeDetector};

    #[test]
    fn silence_scores_as_none_without_weights() {
        let mut detector = SherpaKwsDetector::new(
            PathBuf::from("/tmp/softwake-kws-missing"),
            ["hey softwake"],
            ["go to sleep"],
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
    }

    #[test]
    fn greedy_tokens_match_known_softwake_encoding() {
        let tokens = PathBuf::from(
            "/tmp/kws-extract/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01/tokens.txt",
        );
        if !tokens.is_file() {
            return;
        }
        let pieces = load_token_pieces(tokens.to_str().expect("utf8")).expect("pieces");
        let line = encode_keyword_line(&pieces, "hey softwake").expect("encode");
        assert!(line.ends_with("@hey_softwake"));
        assert!(line.starts_with("▁HE Y ▁SO F T W A KE "));
        let sally = encode_keyword_line(&pieces, "sally").expect("sally encodes");
        assert!(sally.contains("@sally"), "{sally}");
        assert!(
            sally.contains("#0.15"),
            "short name gets lower threshold: {sally}"
        );
        let hey_sally = encode_keyword_line(&pieces, "hey sally").expect("hey sally");
        assert!(hey_sally.contains("@hey_sally"), "{hey_sally}");
        assert!(
            !hey_sally.contains('#'),
            "multi-word stays on global threshold: {hey_sally}"
        );
    }

    #[test]
    fn short_single_word_helper() {
        assert!(is_short_single_word("sally"));
        assert!(is_short_single_word("ada"));
        assert!(!is_short_single_word("hey sally"));
        assert!(is_short_single_word("softwake")); // 8 chars exactly counts as short
        assert!(!is_short_single_word("softwakes")); // 9+
    }
}
