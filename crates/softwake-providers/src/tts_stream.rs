//! xAI streaming TTS WebSocket (`wss://…/v1/tts`).
//!
//! Unary `POST /v1/tts` stays the fallback in [`crate::voice`]. Event parsing
//! and URL helpers are always available for unit tests. The live socket client
//! is compiled only with `live-http`.
//!
//! Wire (xAI docs): client sends `text.delta` then `text.done`; server replies
//! with base64 `audio.delta` chunks and `audio.done`.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::Value;

use crate::voice::{VOICE_LANGUAGE, VoiceHttpError};

#[cfg(feature = "live-http")]
use crate::voice::{TTS_MAX_CHARS, clip_chars_for_stream};

/// Build the streaming TTS WebSocket URL from an HTTPS/HTTP API base.
///
/// `https://api.x.ai/v1` → `wss://api.x.ai/v1/tts?language=en&voice=eve&…`.
///
/// # Errors
///
/// [`VoiceHttpError::Unreachable`] when the base is not http(s)/ws(s) or the
/// voice id is hostile; [`VoiceHttpError::EmptyText`] when voice is blank.
pub fn tts_stream_url(api_base: &str, voice_id: &str) -> Result<String, VoiceHttpError> {
    let base = api_base.trim().trim_end_matches('/');
    let (scheme, rest) = if let Some(rest) = base.strip_prefix("https://") {
        ("wss", rest)
    } else if let Some(rest) = base.strip_prefix("http://") {
        ("ws", rest)
    } else if let Some(rest) = base.strip_prefix("wss://") {
        ("wss", rest)
    } else if let Some(rest) = base.strip_prefix("ws://") {
        ("ws", rest)
    } else {
        return Err(VoiceHttpError::Unreachable);
    };
    let voice = voice_id.trim();
    if voice.is_empty() {
        return Err(VoiceHttpError::EmptyText);
    }
    // Built-in voice ids are alphanumeric; reject anything that would break the query.
    if !voice
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(VoiceHttpError::Unreachable);
    }
    Ok(format!(
        "{scheme}://{rest}/tts?language={VOICE_LANGUAGE}&voice={voice}&codec=mp3&sample_rate=24000&bit_rate=128000&optimize_streaming_latency=1"
    ))
}

/// Decode one WebSocket text payload into optional MP3 bytes (`audio.delta`).
///
/// Returns `Ok(None)` for control events (`audio.done`, ignored kinds).
///
/// # Errors
///
/// [`VoiceHttpError::Unparseable`] for bad JSON or missing fields;
/// [`VoiceHttpError::Failed`] for a server `error` event.
pub fn parse_tts_stream_event(payload: &str) -> Result<Option<Vec<u8>>, VoiceHttpError> {
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return Err(VoiceHttpError::Unparseable);
    };
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Err(VoiceHttpError::Unparseable);
    };
    match kind {
        "audio.delta" => {
            let Some(delta) = value.get("delta").and_then(Value::as_str) else {
                return Err(VoiceHttpError::Unparseable);
            };
            if delta.is_empty() {
                return Ok(None);
            }
            let bytes = B64
                .decode(delta.trim())
                .map_err(|_| VoiceHttpError::Unparseable)?;
            if bytes.is_empty() {
                return Ok(None);
            }
            Ok(Some(bytes))
        }
        "error" => Err(VoiceHttpError::Failed { status: 500 }),
        _ => Ok(None),
    }
}

/// `true` when the payload is the terminal `audio.done` event.
#[must_use]
pub fn is_audio_done_event(payload: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(Value::as_str)
                .map(|kind| kind == "audio.done")
        })
        .unwrap_or(false)
}

/// Stream TTS over WebSocket, invoking `on_chunk` for each MP3 delta.
///
/// # Errors
///
/// [`VoiceHttpError`] when the URL is bad, the socket fails, or no audio arrives.
#[cfg(feature = "live-http")]
pub fn tts_synthesize_streaming(
    api_base: &str,
    bearer: &str,
    text: &str,
    voice_id: &str,
    mut on_chunk: impl FnMut(&[u8]) -> Result<(), VoiceHttpError>,
) -> Result<(), VoiceHttpError> {
    use tungstenite::client::IntoClientRequest;
    use tungstenite::http::header::{AUTHORIZATION, HeaderValue};
    use tungstenite::{Message, connect};

    let text = text.trim();
    if text.is_empty() {
        return Err(VoiceHttpError::EmptyText);
    }
    let text = clip_chars_for_stream(text, TTS_MAX_CHARS);
    let url = tts_stream_url(api_base, voice_id)?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| VoiceHttpError::Unreachable)?;
    let auth = format!("Bearer {bearer}");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&auth).map_err(|_| VoiceHttpError::Rejected)?,
    );

    let (mut socket, _response) = connect(request).map_err(|_| VoiceHttpError::Unreachable)?;

    let delta = serde_json::json!({"type": "text.delta", "delta": text}).to_string();
    let done = serde_json::json!({"type": "text.done"}).to_string();
    socket
        .send(Message::Text(delta))
        .map_err(|_| VoiceHttpError::Unreachable)?;
    socket
        .send(Message::Text(done))
        .map_err(|_| VoiceHttpError::Unreachable)?;

    let mut got_audio = false;
    loop {
        let msg = socket.read().map_err(|_| VoiceHttpError::Unreachable)?;
        match msg {
            Message::Text(payload) => {
                let done = is_audio_done_event(&payload);
                if let Some(chunk) = parse_tts_stream_event(&payload)? {
                    got_audio = true;
                    on_chunk(&chunk)?;
                }
                if done {
                    break;
                }
            }
            Message::Binary(bytes) if !bytes.is_empty() => {
                got_audio = true;
                on_chunk(&bytes)?;
            }
            Message::Close(_) => break,
            Message::Ping(data) => {
                let _ = socket.send(Message::Pong(data));
            }
            _ => {}
        }
    }
    if got_audio {
        Ok(())
    } else {
        Err(VoiceHttpError::NoAudio)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_audio_done_event, parse_tts_stream_event, tts_stream_url};
    use crate::voice::VoiceHttpError;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;

    #[test]
    fn https_api_base_becomes_wss_tts() {
        let url = tts_stream_url("https://api.x.ai/v1/", "eve").expect("url");
        assert!(url.starts_with("wss://api.x.ai/v1/tts?"));
        assert!(url.contains("voice=eve"));
        assert!(url.contains("codec=mp3"));
        assert!(url.contains("optimize_streaming_latency=1"));
        assert!(url.contains("language=en"));
    }

    #[test]
    fn rejects_empty_or_hostile_voice() {
        assert_eq!(
            tts_stream_url("https://api.x.ai/v1", "").expect_err("empty"),
            VoiceHttpError::EmptyText
        );
        assert_eq!(
            tts_stream_url("https://api.x.ai/v1", "eve&x=1").expect_err("meta"),
            VoiceHttpError::Unreachable
        );
    }

    #[test]
    fn parses_audio_delta_and_ignores_done() {
        let raw = b"ID3stream";
        let payload = format!(r#"{{"type":"audio.delta","delta":"{}"}}"#, B64.encode(raw));
        let chunk = parse_tts_stream_event(&payload)
            .expect("parse")
            .expect("bytes");
        assert_eq!(chunk, raw);
        assert!(
            parse_tts_stream_event(r#"{"type":"audio.done"}"#)
                .expect("done")
                .is_none()
        );
        assert!(is_audio_done_event(r#"{"type":"audio.done"}"#));
        assert_eq!(
            parse_tts_stream_event(r#"{"type":"error","message":"nope"}"#).expect_err("err"),
            VoiceHttpError::Failed { status: 500 }
        );
    }
}
