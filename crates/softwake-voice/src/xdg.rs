//! Portable model directory paths for awake STT and TTS weights.
//!
//! Matches the wake KWS layout from ADR 0006: prefer `XDG_DATA_HOME`, else
//! `$HOME/.local/share`. Softwake does not create these directories and does
//! not download weights.

use std::path::PathBuf;

fn data_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(home).join(".local").join("share")
}

/// Directory for sherpa-onnx streaming ASR checkpoints.
///
/// `$XDG_DATA_HOME/softwake/asr` when `XDG_DATA_HOME` is set, otherwise
/// `$HOME/.local/share/softwake/asr`.
#[must_use]
pub fn asr_model_dir() -> PathBuf {
    data_home().join("softwake").join("asr")
}

/// Directory for sherpa-onnx (or later espeak) TTS assets.
///
/// `$XDG_DATA_HOME/softwake/tts` when `XDG_DATA_HOME` is set, otherwise
/// `$HOME/.local/share/softwake/tts`.
#[must_use]
pub fn tts_model_dir() -> PathBuf {
    data_home().join("softwake").join("tts")
}
