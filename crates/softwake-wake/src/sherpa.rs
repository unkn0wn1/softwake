//! Stub PCM detector for the sherpa-onnx keyword spotter.
//!
//! ADR 0006 names this type as the production wake engine. This module does
//! not depend on the `sherpa-onnx` crate and does not read model files.
//!
//! When weights are added, `push_samples` will convert 16 kHz mono `i16` to
//! `f32`, call `OnlineStream::accept_waveform`, and map the decoded keyword
//! onto [`crate::PhraseHit`] with the same longest-match rule as the text
//! table (a tie prefers sleep). Until then every window is [`crate::PhraseHit::None`].

use std::path::{Path, PathBuf};

use crate::{PhraseHit, WakeDetector};

/// PCM detector slot for sherpa-onnx keyword spotting.
///
/// Weights are not loaded. [`WakeDetector::push_samples`] returns
/// [`PhraseHit::None`] for every window, including silence.
///
/// Model files belong in [`SherpaKwsDetector::model_dir`]:
/// `$XDG_DATA_HOME/softwake/kws`, or `$HOME/.local/share/softwake/kws` when
/// `XDG_DATA_HOME` is unset. The intended download is an English Zipformer
/// keyword-spotting model such as
/// `sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01`. Do not commit those
/// files. The `sherpa-onnx` crate (`KeywordSpotter`, `KeywordSpotterConfig`,
/// `OnlineStream`) is the later wiring; it is not a dependency of this feature.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::module_name_repetitions)] // `SherpaKwsDetector` is the public name of this engine slot.
pub struct SherpaKwsDetector {
    model_dir: PathBuf,
    wake_phrases: Vec<String>,
    sleep_phrases: Vec<String>,
}

impl SherpaKwsDetector {
    /// Remember where weights would load and which phrases map to wake or sleep.
    ///
    /// Phrases are trimmed and stored in lowercase. Empty phrases are dropped
    /// so a blank entry cannot become a keyword later. The directory is not
    /// created and is not read.
    #[must_use]
    pub fn new<W, S, T, U>(model_dir: impl Into<PathBuf>, wake_phrases: W, sleep_phrases: S) -> Self
    where
        W: IntoIterator<Item = T>,
        S: IntoIterator<Item = U>,
        T: AsRef<str>,
        U: AsRef<str>,
    {
        Self {
            model_dir: model_dir.into(),
            wake_phrases: normalize(wake_phrases),
            sleep_phrases: normalize(sleep_phrases),
        }
    }

    /// Detector aimed at [`Self::default_model_dir`] and the built-in phrases.
    ///
    /// Wake phrases are `hey softwake` and `softwake`. Sleep phrases are
    /// `softwake sleep` and `go to sleep`. Same strings as the text spike.
    #[must_use]
    pub fn with_default_phrases() -> Self {
        Self::new(
            Self::default_model_dir(),
            ["hey softwake", "softwake"],
            ["softwake sleep", "go to sleep"],
        )
    }

    /// `$XDG_DATA_HOME/softwake/kws`, else `$HOME/.local/share/softwake/kws`.
    ///
    /// When both variables are unset, the path is the relative
    /// `.local/share/softwake/kws`.
    #[must_use]
    pub fn default_model_dir() -> PathBuf {
        model_dir_from(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
    }

    /// Directory a later build will read for the ONNX keyword-spotting files.
    #[must_use]
    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    /// Wake phrases, lowercase, in the order they were given.
    #[must_use]
    pub fn wake_phrases(&self) -> &[String] {
        &self.wake_phrases
    }

    /// Sleep phrases, lowercase, in the order they were given.
    #[must_use]
    pub fn sleep_phrases(&self) -> &[String] {
        &self.sleep_phrases
    }

    /// `false` until a build actually loads weights from [`Self::model_dir`].
    #[must_use]
    #[allow(clippy::unused_self)] // No weights are loaded, so the flag does not read state.
    pub const fn weights_loaded(&self) -> bool {
        false
    }
}

impl WakeDetector for SherpaKwsDetector {
    #[allow(clippy::unused_self)] // Weights are not linked, so there is no stream to read.
    fn push_samples(&mut self, _samples: &[i16]) -> PhraseHit {
        PhraseHit::None
    }
}

fn normalize<I, S>(phrases: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    phrases
        .into_iter()
        .map(|phrase| phrase.as_ref().trim().to_lowercase())
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

    use super::{SherpaKwsDetector, model_dir_from};
    use crate::{PhraseHit, WakeDetector};

    #[test]
    fn silence_and_noise_score_as_none_without_weights() {
        let mut detector = SherpaKwsDetector::with_default_phrases();
        assert!(!detector.weights_loaded());
        assert_eq!(detector.push_samples(&[]), PhraseHit::None);
        assert_eq!(detector.push_samples(&[0; 160]), PhraseHit::None);
        assert_eq!(detector.push_samples(&[0, 1, -1, 32_000]), PhraseHit::None);
        assert_eq!(
            detector.wake_phrases(),
            ["hey softwake".to_owned(), "softwake".to_owned()]
        );
        assert_eq!(
            detector.sleep_phrases(),
            ["softwake sleep".to_owned(), "go to sleep".to_owned()]
        );
    }

    #[test]
    fn blank_phrases_are_dropped_and_the_directory_is_not_opened() {
        let dir = PathBuf::from("kws-models");
        let detector = SherpaKwsDetector::new(&dir, ["  hey softwake  ", " "], ["GO TO SLEEP"]);
        assert_eq!(detector.model_dir(), dir.as_path());
        assert_eq!(detector.wake_phrases(), ["hey softwake".to_owned()]);
        assert_eq!(detector.sleep_phrases(), ["go to sleep".to_owned()]);
        assert!(!dir.exists());
    }

    #[test]
    fn model_dir_prefers_xdg_data_home() {
        let xdg = OsString::from("/tmp/softwake-data");
        let home = OsString::from("/tmp/softwake-home");
        assert_eq!(
            model_dir_from(Some(xdg), Some(home)),
            PathBuf::from("/tmp/softwake-data/softwake/kws")
        );
        assert_eq!(
            model_dir_from(None, Some(OsString::from("/tmp/softwake-home"))),
            PathBuf::from("/tmp/softwake-home/.local/share/softwake/kws")
        );
        assert_eq!(
            model_dir_from(None, None),
            PathBuf::from(".local/share/softwake/kws")
        );
    }
}
