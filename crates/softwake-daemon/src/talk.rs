//! Press-to-talk: buffer PCM, cloud STT, then the existing ask path.
//!
//! Cloud STT and TTS run only when this crate is built with `live-http`.
//! Default tests inject PCM and a transcript without a microphone or a socket.
//! See [ADR 0007](../../docs/ADR-0007-awake-stt-tts.md).

use softwake_providers::{ProviderId, family_speaks_xai, resolve_stt_model, resolve_tts_voice};

#[cfg(feature = "live-http")]
use softwake_providers::{stt_transcribe, tts_synthesize, wav_from_pcm16};
use softwake_voice::TalkBuffer;
#[cfg(feature = "live-http")]
use softwake_voice::{PLAYBACK_TIMEOUT, PlaybackMode, play_audio};

use crate::chat::DiskChat;
#[cfg(not(feature = "live-http"))]
use crate::chat::LIVE_HTTP_DISABLED;

/// Shown when the held clip is shorter than the minimum.
pub(crate) const TALK_TOO_SHORT: &str = "hold the mic a little longer";

/// Shown when talk is used outside awake after the sleep-to-wake attempt.
pub(crate) fn talk_while(state: &str) -> String {
    format!("talk while {state} (speech acts only while awake)")
}

/// Hibernate refuses before any buffer work.
pub(crate) const TALK_HIBERNATING: &str =
    "Softwake is hibernating — leave hibernate from Settings (Wake) or the tray first";

/// Utterance captured while the mic button is held.
#[derive(Debug, Default)]
pub(crate) struct TalkSession {
    buffer: TalkBuffer,
}

impl TalkSession {
    pub(crate) fn is_armed(&self) -> bool {
        self.buffer.is_armed()
    }

    pub(crate) fn arm(&mut self) {
        self.buffer.arm();
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.clear();
    }

    pub(crate) fn push(&mut self, samples: &[i16]) {
        self.buffer.push(samples);
    }

    /// Stop and return PCM when the clip is long enough.
    pub(crate) fn finish(&mut self) -> Result<Vec<i16>, String> {
        self.buffer
            .take_if_long_enough()
            .ok_or_else(|| TALK_TOO_SHORT.to_owned())
    }
}

/// Cloud STT of one WAV clip.
///
/// # Errors
///
/// The live-HTTP sentence when the feature is off, or a voice-service sentence.
pub(crate) fn transcribe_pcm(
    ready: &DiskChat,
    model: &str,
    samples: &[i16],
) -> Result<String, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (ready, model, samples);
        Err(LIVE_HTTP_DISABLED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let wav = wav_from_pcm16(samples);
        let transport = softwake_providers::live::LiveTransport::bounded(crate::chat::CHAT_TIMEOUT);
        stt_transcribe(
            &transport,
            &ready.prepared.api_base,
            &ready.bearer,
            model,
            &wav,
        )
        .map_err(|error| error.to_string())
    }
}

/// Speak `text` with the xAI TTS voice when the selected provider is xAI.
///
/// Other families and a missing voice return [`Ok`] (no TTS for that provider).
/// A build without `live-http` returns [`Err`] so the HUD can show why Eve is
/// silent. Playback errors are returned the same way.
///
/// # Errors
///
/// A voice, feature, or player sentence. The bearer is not included.
#[cfg_attr(test, allow(dead_code))] // called only from `#[cfg(not(test))]` speak path
pub(crate) fn speak_reply(ready: &DiskChat, text: &str) -> Result<(), String> {
    let provider = ready.prepared.provider;
    if !family_speaks_xai(provider) {
        return Ok(());
    }
    let Some(voice) = resolve_tts_voice(provider, ready.prepared_tts_voice()) else {
        return Ok(());
    };
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (text, voice);
        Err(LIVE_HTTP_DISABLED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let transport = softwake_providers::live::LiveTransport::bounded(crate::chat::CHAT_TIMEOUT);
        let audio = tts_synthesize(
            &transport,
            provider,
            &ready.prepared.api_base,
            &ready.bearer,
            text,
            voice,
        )
        .map_err(|error| error.to_string())?;
        let mut record = None;
        play_audio(
            PlaybackMode::Spawn,
            &audio,
            "mp3",
            PLAYBACK_TIMEOUT,
            &mut record,
        )
    }
}

/// Model id for one STT call. xAI falls back to the registry seed.
pub(crate) fn stt_model_for(provider: ProviderId, selected: &str) -> Result<String, String> {
    let model = resolve_stt_model(provider, selected);
    if model.is_empty() {
        return Err(
            "No speech-to-text model is selected. Choose a voice model in Settings after Test."
                .to_owned(),
        );
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::{TALK_TOO_SHORT, TalkSession};
    use softwake_voice::TALK_MIN_SAMPLES;

    #[test]
    fn short_hold_is_refused_and_a_long_hold_returns_pcm() {
        let mut talk = TalkSession::default();
        talk.arm();
        talk.push(&[0; 10]);
        assert_eq!(talk.finish().expect_err("short"), TALK_TOO_SHORT);
        talk.arm();
        talk.push(&vec![1; TALK_MIN_SAMPLES]);
        assert_eq!(talk.finish().expect("pcm").len(), TALK_MIN_SAMPLES);
    }

    #[test]
    fn clear_drops_an_armed_buffer() {
        let mut talk = TalkSession::default();
        talk.arm();
        talk.push(&[1, 2, 3]);
        assert!(talk.is_armed());
        talk.clear();
        assert!(!talk.is_armed());
        assert_eq!(talk.finish().expect_err("empty"), TALK_TOO_SHORT);
    }
}
