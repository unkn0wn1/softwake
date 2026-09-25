//! Awake speech-to-text and text-to-speech boundary.
//!
//! Softwake listens for wake phrases while asleep through
//! [`softwake_wake`](../softwake_wake). Once awake, a streaming STT path and a
//! TTS path may act. That choice is [ADR 0007](../../docs/ADR-0007-awake-stt-tts.md):
//! sherpa-onnx streaming ASR for local STT, with [`MockStt`] / [`MockTts`] as
//! the default CI-safe path. An optional xAI cloud STT/TTS connector lives in
//! `softwake-providers` behind `live-http`. Cloud-only is not the only path.
//!
//! Weights are not linked. The `sherpa-asr` and `sherpa-tts` features compile
//! stubs that document where models will load. Default `cargo test` does not
//! enable those features and does not download anything.

mod energy_utt;
mod mock;
mod pcm_wav;
mod playback;
#[cfg(feature = "sherpa-asr")]
mod sherpa_asr;
#[cfg(feature = "sherpa-tts")]
mod sherpa_tts;
mod xdg;

pub use energy_utt::{EnergyUtterance, SILENCE_FRAMES_END, SILENCE_RMS, START_FRAMES, START_RMS};
pub use mock::{MockStt, MockTts};
pub use pcm_wav::{TALK_MAX_SAMPLES, TALK_MIN_SAMPLES, TalkBuffer, wav_from_pcm16};
pub use playback::{PLAYBACK_TIMEOUT, PlaybackMode, PlayedClip, interrupt_playback, play_audio};
#[cfg(feature = "sherpa-asr")]
pub use sherpa_asr::SherpaAsr;
#[cfg(feature = "sherpa-tts")]
pub use sherpa_tts::SherpaTts;
pub use xdg::{asr_model_dir, tts_model_dir};

use std::fmt;

/// One streaming transcript update from STT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptEvent {
    /// Incremental text. May be replaced by a later partial or a final.
    Partial {
        /// Transcript text so far.
        text: String,
    },
    /// Utterance is complete.
    Final {
        /// Final transcript text.
        text: String,
    },
}

impl TranscriptEvent {
    /// Stable log spelling for the kind.
    #[must_use]
    pub const fn kind_as_str(&self) -> &'static str {
        match self {
            Self::Partial { .. } => "partial",
            Self::Final { .. } => "final",
        }
    }

    /// Transcript text carried by this event.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::Partial { text } | Self::Final { text } => text,
        }
    }
}

impl fmt::Display for TranscriptEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: \"{}\"", self.kind_as_str(), self.text())
    }
}

/// Streaming speech-to-text while the daemon is awake.
///
/// Implementations may produce zero or more [`TranscriptEvent`] values from
/// PCM, or from an inject path used by mocks and the typed demo. Sleep and
/// hibernate must not feed an acting STT channel.
pub trait SpeechToText {
    /// Backend failure while decoding or configuring the engine.
    type Error: std::error::Error;

    /// Feed one window of interleaved 16-bit mono PCM (wake format: 16 kHz).
    ///
    /// Returns the next transcript event when the engine has one, or `Ok(None)`
    /// when the window produced nothing yet.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the engine cannot accept samples.
    fn push_samples(&mut self, samples: &[i16]) -> Result<Option<TranscriptEvent>, Self::Error>;
}

/// Text-to-speech while the daemon is awake.
///
/// The default mock records strings instead of opening a speaker. Sleep and
/// hibernate prefer silence: do not call [`TextToSpeech::speak`] unless a
/// future status-beep policy explicitly allows it.
pub trait TextToSpeech {
    /// Backend failure while synthesizing or playing.
    type Error: std::error::Error;

    /// Speak one utterance, or record it in a mock sink.
    ///
    /// # Errors
    ///
    /// Returns the backend error when synthesis or playback fails.
    fn speak(&mut self, text: &str) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{
        MockStt, MockTts, SpeechToText, TextToSpeech, TranscriptEvent, asr_model_dir, tts_model_dir,
    };

    #[test]
    fn transcript_event_display_is_stable() {
        let partial = TranscriptEvent::Partial {
            text: "hello".to_owned(),
        };
        let final_event = TranscriptEvent::Final {
            text: "hello world".to_owned(),
        };
        assert_eq!(partial.kind_as_str(), "partial");
        assert_eq!(final_event.kind_as_str(), "final");
        assert_eq!(partial.to_string(), "partial: \"hello\"");
        assert_eq!(final_event.to_string(), "final: \"hello world\"");
    }

    #[test]
    fn mock_stt_injects_partial_then_final() {
        let mut stt = MockStt::default();
        assert!(stt.push_samples(&[0; 160]).unwrap().is_none());
        stt.inject_partial("hel");
        stt.inject_final("hello");
        assert_eq!(
            stt.pop(),
            Some(TranscriptEvent::Partial {
                text: "hel".to_owned()
            })
        );
        assert_eq!(
            stt.pop(),
            Some(TranscriptEvent::Final {
                text: "hello".to_owned()
            })
        );
        assert_eq!(stt.pop(), None);
    }

    #[test]
    fn mock_tts_records_spoken_strings() {
        let mut tts = MockTts::default();
        tts.speak("hi").unwrap();
        tts.speak("there").unwrap();
        assert_eq!(tts.spoken(), &["hi".to_owned(), "there".to_owned()]);
    }

    #[test]
    fn xdg_model_dirs_end_with_asr_and_tts() {
        let asr = asr_model_dir();
        let tts = tts_model_dir();
        assert!(asr.ends_with("softwake/asr") || asr.ends_with("softwake\\asr"));
        assert!(tts.ends_with("softwake/tts") || tts.ends_with("softwake\\tts"));
    }
}
