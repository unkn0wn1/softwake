//! CI-safe mock STT and TTS.
//!
//! [`MockStt`] never opens a microphone. Tests and the typed demo inject
//! transcript text. [`MockTts`] records strings instead of synthesizing audio.

use std::collections::VecDeque;
use std::convert::Infallible;

use crate::{SpeechToText, TextToSpeech, TranscriptEvent};

/// Speech-to-text stand-in that accepts injected transcript events.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MockStt {
    pending: VecDeque<TranscriptEvent>,
}

impl MockStt {
    /// Queue a partial transcript for later [`MockStt::pop`] or drain.
    pub fn inject_partial(&mut self, text: impl Into<String>) {
        self.pending
            .push_back(TranscriptEvent::Partial { text: text.into() });
    }

    /// Queue a final transcript for later [`MockStt::pop`] or drain.
    pub fn inject_final(&mut self, text: impl Into<String>) {
        self.pending
            .push_back(TranscriptEvent::Final { text: text.into() });
    }

    /// Take the next queued event, if any.
    pub fn pop(&mut self) -> Option<TranscriptEvent> {
        self.pending.pop_front()
    }

    /// Drain every queued event in order.
    pub fn drain(&mut self) -> Vec<TranscriptEvent> {
        self.pending.drain(..).collect()
    }

    /// Whether any injected event is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

impl SpeechToText for MockStt {
    type Error = Infallible;

    fn push_samples(&mut self, _samples: &[i16]) -> Result<Option<TranscriptEvent>, Self::Error> {
        // The mock has no acoustic model. PCM is ignored; use inject_*.
        Ok(None)
    }
}

/// Text-to-speech stand-in that records spoken strings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MockTts {
    spoken: Vec<String>,
}

impl MockTts {
    /// Strings passed to [`TextToSpeech::speak`], in order.
    #[must_use]
    pub fn spoken(&self) -> &[String] {
        &self.spoken
    }

    /// Clear the recorded utterances.
    pub fn clear(&mut self) {
        self.spoken.clear();
    }
}

impl TextToSpeech for MockTts {
    type Error = Infallible;

    fn speak(&mut self, text: &str) -> Result<(), Self::Error> {
        self.spoken.push(text.to_owned());
        Ok(())
    }
}
