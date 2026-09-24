//! Feature-gated sherpa-onnx streaming ASR stub.
//!
//! Compiles only with `--features sherpa-asr`. Does not link the ONNX runtime
//! and does not read weights. [`SherpaAsr::push_samples`] always returns
//! `Ok(None)` until a later change loads a checkpoint from
//! [`crate::asr_model_dir`].

use std::convert::Infallible;
use std::path::PathBuf;

use crate::{SpeechToText, TranscriptEvent, asr_model_dir};

/// Streaming ASR stand-in named for the production engine in ADR 0007.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SherpaAsr {
    model_dir: PathBuf,
    /// Always false until a later change loads weights.
    pub weights_loaded: bool,
}

impl SherpaAsr {
    /// Remember where weights will load. Does not create or read the directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            model_dir: asr_model_dir(),
            weights_loaded: false,
        }
    }

    /// Configured model directory (not verified to exist).
    #[must_use]
    pub fn model_dir(&self) -> &PathBuf {
        &self.model_dir
    }
}

impl Default for SherpaAsr {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechToText for SherpaAsr {
    type Error = Infallible;

    fn push_samples(&mut self, _samples: &[i16]) -> Result<Option<TranscriptEvent>, Self::Error> {
        // Weights are not loaded. Silence and speech both yield nothing.
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::SherpaAsr;
    use crate::SpeechToText;

    #[test]
    fn stub_never_emits_without_weights() {
        let mut asr = SherpaAsr::new();
        assert!(!asr.weights_loaded);
        assert!(
            asr.model_dir().ends_with("softwake/asr") || asr.model_dir().ends_with("softwake\\asr")
        );
        assert!(asr.push_samples(&[0; 160]).unwrap().is_none());
    }
}
