//! Feature-gated sherpa-onnx TTS stub.
//!
//! Compiles only with `--features sherpa-tts`. Does not link a synthesizer and
//! does not read weights. [`SherpaTts::speak`] succeeds and records nothing
//! audible until a later change loads assets from [`crate::tts_model_dir`].

use std::convert::Infallible;
use std::path::PathBuf;

use crate::{TextToSpeech, tts_model_dir};

/// TTS stand-in named for a possible sherpa-onnx (or espeak) path in ADR 0007.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SherpaTts {
    model_dir: PathBuf,
    /// Always false until a later change loads assets.
    pub weights_loaded: bool,
    /// Utterances accepted while the stub is wired (still silent).
    spoken: Vec<String>,
}

impl SherpaTts {
    /// Remember where assets will load. Does not create or read the directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            model_dir: tts_model_dir(),
            weights_loaded: false,
            spoken: Vec::new(),
        }
    }

    /// Configured model directory (not verified to exist).
    #[must_use]
    pub fn model_dir(&self) -> &PathBuf {
        &self.model_dir
    }

    /// Strings passed to [`TextToSpeech::speak`] on this stub.
    #[must_use]
    pub fn spoken(&self) -> &[String] {
        &self.spoken
    }
}

impl Default for SherpaTts {
    fn default() -> Self {
        Self::new()
    }
}

impl TextToSpeech for SherpaTts {
    type Error = Infallible;

    fn speak(&mut self, text: &str) -> Result<(), Self::Error> {
        // No audio device and no weights. Record the request for tests.
        self.spoken.push(text.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SherpaTts;
    use crate::TextToSpeech;

    #[test]
    fn stub_records_without_weights() {
        let mut tts = SherpaTts::new();
        assert!(!tts.weights_loaded);
        tts.speak("ping").unwrap();
        assert_eq!(tts.spoken(), &["ping".to_owned()]);
    }
}
