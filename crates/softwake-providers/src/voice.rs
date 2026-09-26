//! Optional xAI speech-to-text and text-to-speech.
//!
//! CI uses [`crate::MockTransport`]. Live HTTPS is `live-http` only.
//! Voice id `eve` is an xAI TTS id and is not sent to other families.
//! See [ADR 0007](../../docs/ADR-0007-awake-stt-tts.md).

use serde_json::Value;

use crate::constants::{XAI_TTS_VOICE_EVE, XAI_TTS_VOICES, XAI_VOICE_SEED};
use crate::ids::ProviderId;
use crate::registry::{ProviderFamily, provider_definition};
use crate::transport::{HttpBytes, MultipartField, Transport, TransportError};

/// Language sent on xAI STT and TTS in this slice.
pub const VOICE_LANGUAGE: &str = "en";

/// Hard cap on TTS input. The xAI API allows more; this keeps one HUD reply short.
pub const TTS_MAX_CHARS: usize = 4_000;

/// STT or TTS failure. Display text omits the body, the bearer, and the URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VoiceHttpError {
    /// The transport could not complete the request.
    #[error("Could not reach the voice service.")]
    Unreachable,
    /// HTTP 401 or 403.
    #[error("Voice service rejected the credentials.")]
    Rejected,
    /// Any other non-2xx status.
    #[error("Voice request failed ({status}).")]
    Failed {
        /// HTTP status code.
        status: u16,
    },
    /// 2xx STT content was empty after trim.
    #[error("The voice service returned an empty transcript.")]
    Empty,
    /// 2xx STT body was not a JSON string at `text`.
    #[error("The transcript could not be read.")]
    Unparseable,
    /// 2xx TTS body had no audio bytes.
    #[error("The voice service returned no audio.")]
    NoAudio,
    /// This provider family has no xAI voice connector.
    #[error("Speech playback is available for xAI providers. Eve is an xAI voice.")]
    NotXai,
    /// TTS text was blank.
    #[error("Nothing to speak.")]
    EmptyText,
}

/// Whether `provider` uses the xAI voice APIs (`/v1/stt`, `/v1/tts`).
#[must_use]
pub fn family_speaks_xai(provider: ProviderId) -> bool {
    provider_definition(provider).family == ProviderFamily::Xai
}

/// Built-in xAI TTS voice ids. Empty for every other family.
#[must_use]
pub fn tts_voice_roster(provider: ProviderId) -> &'static [&'static str] {
    if family_speaks_xai(provider) {
        XAI_TTS_VOICES
    } else {
        &[]
    }
}

/// Voice id to send on xAI TTS.
///
/// Empty Settings means [`XAI_TTS_VOICE_EVE`]. A saved id is used only when it
/// is in [`XAI_TTS_VOICES`]. Other families return [`None`].
#[must_use]
pub fn resolve_tts_voice(provider: ProviderId, selected: &str) -> Option<&'static str> {
    if !family_speaks_xai(provider) {
        return None;
    }
    let trimmed = selected.trim();
    if trimmed.is_empty() {
        return Some(XAI_TTS_VOICE_EVE);
    }
    XAI_TTS_VOICES
        .iter()
        .find(|id| id.eq_ignore_ascii_case(trimmed))
        .copied()
}

/// STT model id. Empty Settings means [`XAI_VOICE_SEED`] for xAI.
#[must_use]
pub fn resolve_stt_model(provider: ProviderId, selected: &str) -> String {
    let trimmed = selected.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }
    if family_speaks_xai(provider) {
        XAI_VOICE_SEED.to_owned()
    } else {
        String::new()
    }
}

/// `POST {api_base}/stt` with multipart `model` then `file`.
///
/// # Errors
///
/// [`VoiceHttpError`] when the transport fails, the status is not success, or
/// `text` is missing or empty. The error text does not include the body.
pub fn stt_transcribe<T: Transport>(
    transport: &T,
    api_base: &str,
    bearer: &str,
    model: &str,
    wav_bytes: &[u8],
) -> Result<String, VoiceHttpError> {
    let url = format!("{}/stt", api_base.trim_end_matches('/'));
    let fields = [MultipartField {
        name: "model".to_owned(),
        value: model.to_owned(),
    }];
    let response = transport
        .post_multipart_bearer(
            &url,
            bearer,
            &fields,
            "utterance.wav",
            wav_bytes,
            "audio/wav",
        )
        .map_err(|_error: TransportError| VoiceHttpError::Unreachable)?;
    parse_stt_body(response.status, &response.body)
}

/// `POST {api_base}/tts` with JSON `{text, voice_id, language}` and return mp3 bytes.
///
/// `voice_id` must already be an xAI id (see [`resolve_tts_voice`]).
///
/// # Errors
///
/// [`VoiceHttpError`] when the family is not xAI, the text is blank, the
/// transport fails, or the body is empty.
pub fn tts_synthesize<T: Transport>(
    transport: &T,
    provider: ProviderId,
    api_base: &str,
    bearer: &str,
    text: &str,
    voice_id: &str,
) -> Result<Vec<u8>, VoiceHttpError> {
    if !family_speaks_xai(provider) {
        return Err(VoiceHttpError::NotXai);
    }
    let text = text.trim();
    if text.is_empty() {
        return Err(VoiceHttpError::EmptyText);
    }
    let text = clip_chars(text, TTS_MAX_CHARS);
    let url = format!("{}/tts", api_base.trim_end_matches('/'));
    let body = serde_json::json!({
        "text": text,
        "voice_id": voice_id,
        "language": VOICE_LANGUAGE,
    })
    .to_string();
    let response = transport
        .post_json_bearer_bytes(&url, bearer, &body)
        .map_err(|_error: TransportError| VoiceHttpError::Unreachable)?;
    parse_tts_bytes(response)
}

fn parse_stt_body(status: u16, body: &str) -> Result<String, VoiceHttpError> {
    if status == 401 || status == 403 {
        return Err(VoiceHttpError::Rejected);
    }
    if !(200..300).contains(&status) {
        return Err(VoiceHttpError::Failed { status });
    }
    let Ok(payload) = serde_json::from_str::<Value>(body) else {
        return Err(VoiceHttpError::Unparseable);
    };
    let Some(text) = payload.get("text").and_then(Value::as_str) else {
        return Err(VoiceHttpError::Unparseable);
    };
    let text = text.trim();
    if text.is_empty() {
        return Err(VoiceHttpError::Empty);
    }
    Ok(text.to_owned())
}

fn parse_tts_bytes(response: HttpBytes) -> Result<Vec<u8>, VoiceHttpError> {
    if response.status == 401 || response.status == 403 {
        return Err(VoiceHttpError::Rejected);
    }
    if !(200..300).contains(&response.status) {
        return Err(VoiceHttpError::Failed {
            status: response.status,
        });
    }
    if response.body.is_empty() {
        return Err(VoiceHttpError::NoAudio);
    }
    Ok(response.body)
}

fn clip_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            break;
        }
        out.push(ch);
    }
    out
}

/// Build a 16 kHz mono PCM WAV (RIFF) from little-endian `i16` samples.
#[must_use]
pub fn wav_from_pcm16(samples: &[i16]) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const CHANNELS: u16 = 1;
    const BITS: u16 = 16;
    let data_len = u32::try_from(samples.len().saturating_mul(2)).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36u32.saturating_add(data_len)).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&CHANNELS.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS / 8);
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = CHANNELS * (BITS / 8);
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&BITS.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::{
        TTS_MAX_CHARS, VoiceHttpError, family_speaks_xai, resolve_stt_model, resolve_tts_voice,
        stt_transcribe, tts_synthesize, tts_voice_roster, wav_from_pcm16,
    };
    use crate::constants::{XAI_TTS_VOICE_EVE, XAI_VOICE_SEED};
    use crate::ids::ProviderId;
    use crate::transport::{HttpBytes, HttpResponse, MockTransport};

    #[test]
    fn eve_is_the_xai_default_and_other_families_have_no_roster() {
        assert!(family_speaks_xai(ProviderId::XaiKey));
        assert!(family_speaks_xai(ProviderId::XaiOauth));
        assert!(!family_speaks_xai(ProviderId::Openai));
        assert!(tts_voice_roster(ProviderId::XaiKey).contains(&XAI_TTS_VOICE_EVE));
        assert!(tts_voice_roster(ProviderId::Openai).is_empty());
        assert_eq!(
            resolve_tts_voice(ProviderId::XaiKey, ""),
            Some(XAI_TTS_VOICE_EVE)
        );
        assert_eq!(
            resolve_tts_voice(ProviderId::XaiOauth, "EVE"),
            Some(XAI_TTS_VOICE_EVE)
        );
        assert_eq!(resolve_tts_voice(ProviderId::XaiKey, "ara"), Some("ara"));
        assert_eq!(resolve_tts_voice(ProviderId::XaiKey, "not-a-voice"), None);
        assert_eq!(resolve_tts_voice(ProviderId::Openai, "eve"), None);
        assert_eq!(resolve_stt_model(ProviderId::XaiKey, ""), XAI_VOICE_SEED);
        assert_eq!(
            resolve_stt_model(ProviderId::XaiKey, " grok-voice-transcribe-1.0 "),
            "grok-voice-transcribe-1.0"
        );
        assert!(resolve_stt_model(ProviderId::Openai, "").is_empty());
    }

    #[test]
    fn stt_posts_model_then_wav_and_reads_text() {
        let wav = wav_from_pcm16(&[0, 1, -1]);
        assert!(wav.starts_with(b"RIFF"));
        assert!(wav.windows(4).any(|window| window == b"data"));
        let transport = MockTransport::new().with_multipart(
            "https://api.x.ai/v1/stt",
            HttpResponse {
                status: 200,
                body: r#"{"text":" hello there "}"#.to_owned(),
            },
        );
        let text = stt_transcribe(
            &transport,
            "https://api.x.ai/v1",
            "bearer-token",
            XAI_VOICE_SEED,
            &wav,
        )
        .expect("stt");
        assert_eq!(text, "hello there");
        let recorded = transport.last_multipart().expect("recorded");
        assert_eq!(recorded.url, "https://api.x.ai/v1/stt");
        assert_eq!(recorded.bearer, "bearer-token");
        assert_eq!(recorded.fields.len(), 1);
        assert_eq!(recorded.fields[0].name, "model");
        assert_eq!(recorded.fields[0].value, XAI_VOICE_SEED);
        assert_eq!(recorded.file_name, "utterance.wav");
        assert_eq!(recorded.file_content_type, "audio/wav");
        assert_eq!(recorded.file_bytes, wav);
    }

    #[test]
    fn stt_maps_auth_and_empty_transcript() {
        let rejected = MockTransport::new().with_multipart(
            "https://api.x.ai/v1/stt",
            HttpResponse {
                status: 401,
                body: "no".to_owned(),
            },
        );
        assert_eq!(
            stt_transcribe(&rejected, "https://api.x.ai/v1", "b", "m", b"wav").expect_err("401"),
            VoiceHttpError::Rejected
        );
        let empty = MockTransport::new().with_multipart(
            "https://api.x.ai/v1/stt",
            HttpResponse {
                status: 200,
                body: r#"{"text":"  "}"#.to_owned(),
            },
        );
        assert_eq!(
            stt_transcribe(&empty, "https://api.x.ai/v1", "b", "m", b"wav").expect_err("empty"),
            VoiceHttpError::Empty
        );
        let missing = MockTransport::new();
        assert_eq!(
            stt_transcribe(&missing, "https://api.x.ai/v1", "b", "m", b"wav").expect_err("route"),
            VoiceHttpError::Unreachable
        );
    }

    #[test]
    fn tts_posts_eve_json_and_returns_mp3_bytes() {
        let mp3 = b"ID3fake-mp3".to_vec();
        let transport = MockTransport::new().with_post_bytes(
            "https://api.x.ai/v1/tts",
            HttpBytes {
                status: 200,
                body: mp3.clone(),
            },
        );
        let audio = tts_synthesize(
            &transport,
            ProviderId::XaiKey,
            "https://api.x.ai/v1/",
            "bearer-token",
            "  she replies  ",
            XAI_TTS_VOICE_EVE,
        )
        .expect("tts");
        assert_eq!(audio, mp3);
        let recorded = transport.last_byte_post().expect("recorded");
        assert_eq!(recorded.bearer, "bearer-token");
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("json");
        assert_eq!(body["text"], "she replies");
        assert_eq!(body["voice_id"], "eve");
        assert_eq!(body["language"], "en");
    }

    #[test]
    fn tts_refuses_non_xai_and_blank_text() {
        let transport = MockTransport::new();
        assert_eq!(
            tts_synthesize(
                &transport,
                ProviderId::Openai,
                "https://api.openai.com/v1",
                "b",
                "hello",
                "eve"
            )
            .expect_err("openai"),
            VoiceHttpError::NotXai
        );
        assert!(transport.last_byte_post().is_none());
        assert_eq!(
            tts_synthesize(
                &transport,
                ProviderId::XaiOauth,
                "https://api.x.ai/v1",
                "b",
                "   ",
                "eve"
            )
            .expect_err("blank"),
            VoiceHttpError::EmptyText
        );
    }

    #[test]
    fn tts_maps_http_403_to_rejected_not_unreachable() {
        let transport = MockTransport::new().with_post_bytes(
            "https://api.x.ai/v1/tts",
            HttpBytes {
                status: 403,
                body: b"forbidden".to_vec(),
            },
        );
        let error = tts_synthesize(
            &transport,
            ProviderId::XaiOauth,
            "https://api.x.ai/v1",
            "expired-token",
            "I'm awake.",
            "eve",
        )
        .expect_err("403");
        assert_eq!(error, VoiceHttpError::Rejected);
        assert_ne!(error, VoiceHttpError::Unreachable);
    }

    #[test]
    fn tts_clips_long_text_and_maps_no_audio() {
        let transport = MockTransport::new().with_post_bytes(
            "https://api.x.ai/v1/tts",
            HttpBytes {
                status: 200,
                body: Vec::new(),
            },
        );
        let long = "a".repeat(TTS_MAX_CHARS + 20);
        let error = tts_synthesize(
            &transport,
            ProviderId::XaiKey,
            "https://api.x.ai/v1",
            "b",
            &long,
            "eve",
        )
        .expect_err("empty audio");
        assert_eq!(error, VoiceHttpError::NoAudio);
        let recorded = transport.last_byte_post().expect("posted");
        let body: serde_json::Value = serde_json::from_str(&recorded.body).expect("json");
        assert_eq!(
            body["text"].as_str().expect("text").chars().count(),
            TTS_MAX_CHARS
        );
    }
}
