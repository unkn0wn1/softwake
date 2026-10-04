//! xAI Voice Agent / Speech-to-Speech realtime WebSocket.
//!
//! URL + `session.update` JSON + event parse are always available for unit
//! tests. The live tungstenite client is compiled only with `live-http`.
//!
//! Spike-validated (2026-10-02): `wss://api.x.ai/v1/realtime?model=grok-voice-latest`
//! accepts Softwake's xAI bearer, nested `audio` PCM 16 kHz in / 24 kHz out,
//! `turn_detection.server_vad`, and emits `response.output_audio.delta` (base64
//! PCM16) plus `response.output_audio_transcript.delta`.
//!
//! See [ADR 0050](../../docs/ADR-0050-voice-agent-s2s.md).

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};

use crate::voice::VoiceHttpError;

/// Default Voice Agent model alias (tracks xAI's current flagship voice model).
pub const VOICE_AGENT_MODEL_DEFAULT: &str = "grok-voice-latest";

/// Softwake capture rate — must match [`softwake_audio`] wake format (16 kHz).
pub const VOICE_AGENT_INPUT_RATE_HZ: u32 = 16_000;

/// Preferred assistant PCM rate from the spike (24 kHz linear16).
pub const VOICE_AGENT_OUTPUT_RATE_HZ: u32 = 24_000;

/// Hard cap on `instructions` characters sent on `session.update`.
pub const VOICE_AGENT_INSTRUCTIONS_MAX_CHARS: usize = 8_000;

/// Parsed server event from the realtime socket (text frames).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceAgentEvent {
    /// Base64-decoded PCM16 (or raw when transport is binary — not used in v1 JSON path).
    OutputAudioDelta(Vec<u8>),
    /// Assistant finished emitting audio for this response.
    OutputAudioDone,
    /// Incremental transcript of what the assistant is saying.
    TranscriptDelta(String),
    /// Final transcript for the assistant utterance.
    TranscriptDone(String),
    /// Server VAD heard the user start speaking (barge-in signal).
    SpeechStarted,
    /// Final user transcript from the same realtime socket.
    ///
    /// `conversation.item.input_audio_transcription.completed`. Partial
    /// `.updated` snapshots stay [`VoiceAgentEvent::Ignored`].
    InputTranscript(String),
    /// Server VAD decided the user stopped.
    SpeechStopped,
    /// Model response finished.
    ResponseDone,
    /// Session accepted our update (informational).
    SessionUpdated,
    /// Server error payload (display string; session may still be open).
    Error(String),
    /// Ping / unknown / ignored control events.
    Ignored,
}

/// Build `wss://…/v1/realtime?model=…` from an HTTPS/HTTP API base.
///
/// # Errors
///
/// [`VoiceHttpError::Unreachable`] when the base scheme is unsupported or the
/// model id is hostile; [`VoiceHttpError::EmptyText`] when model is blank.
pub fn voice_agent_realtime_url(api_base: &str, model: &str) -> Result<String, VoiceHttpError> {
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
    let model = model.trim();
    if model.is_empty() {
        return Err(VoiceHttpError::EmptyText);
    }
    if !model
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(VoiceHttpError::Unreachable);
    }
    Ok(format!("{scheme}://{rest}/realtime?model={model}"))
}

/// Truncate instructions for `session.update` (char boundary safe).
#[must_use]
pub fn clip_voice_agent_instructions(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    trimmed.chars().take(max_chars).collect()
}

/// Build the `session.update` client event JSON.
///
/// `web_search` adds the server-side Voice Agent tool only (not Softwake Hands).
#[must_use]
pub fn voice_agent_session_update_json(
    voice: &str,
    instructions: &str,
    input_rate_hz: u32,
    output_rate_hz: u32,
    speed: f64,
    web_search: bool,
) -> String {
    let voice = {
        let v = voice.trim();
        if v.is_empty() { "eve" } else { v }
    };
    let instructions =
        clip_voice_agent_instructions(instructions, VOICE_AGENT_INSTRUCTIONS_MAX_CHARS);
    let mut session = json!({
        "voice": voice,
        "instructions": instructions,
        "turn_detection": { "type": "server_vad" },
        "reasoning": { "effort": "none" },
        "audio": {
            "input": {
                "format": { "type": "audio/pcm", "rate": input_rate_hz },
                "transport": "json"
            },
            "output": {
                "format": { "type": "audio/pcm", "rate": output_rate_hz },
                "transport": "json",
                "speed": crate::voice::clamp_tts_speed(speed)
            }
        }
    });
    if web_search {
        session["tools"] = json!([{ "type": "web_search" }]);
    }
    json!({ "type": "session.update", "session": session }).to_string()
}

/// One conversation item so the next voice turn can see text already spoken elsewhere.
///
/// Does not include `response.create`. The session should not speak this line back.
#[must_use]
#[cfg(any(feature = "live-http", test))]
pub fn voice_agent_context_item_json(role: &str, text: &str) -> String {
    let role = if role.eq_ignore_ascii_case("assistant") {
        "assistant"
    } else {
        "user"
    };
    let content_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    json!({
        "type": "conversation.item.create",
        "item": {
            "type": "message",
            "role": role,
            "content": [{ "type": content_type, "text": text }]
        }
    })
    .to_string()
}

/// Build `input_audio_buffer.append` for one PCM16 little-endian clip.
#[must_use]
pub fn voice_agent_append_pcm_json(pcm16_le: &[u8]) -> String {
    json!({
        "type": "input_audio_buffer.append",
        "audio": B64.encode(pcm16_le)
    })
    .to_string()
}

/// Encode `i16` mono samples as little-endian bytes for append.
#[must_use]
pub fn pcm16_le_bytes(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// Best-effort `response.cancel` client event.
#[must_use]
pub fn voice_agent_cancel_json() -> String {
    json!({ "type": "response.cancel" }).to_string()
}

/// Decode one WebSocket text payload into a [`VoiceAgentEvent`].
///
/// # Errors
///
/// [`VoiceHttpError::Unparseable`] for non-JSON; error events still return
/// `Ok(VoiceAgentEvent::Error(_))` so the session can stay up.
pub fn parse_voice_agent_event(payload: &str) -> Result<VoiceAgentEvent, VoiceHttpError> {
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return Err(VoiceHttpError::Unparseable);
    };
    let Some(kind) = value.get("type").and_then(Value::as_str) else {
        return Err(VoiceHttpError::Unparseable);
    };
    Ok(match kind {
        "response.output_audio.delta" | "response.audio.delta" => {
            let Some(delta) = value.get("delta").and_then(Value::as_str) else {
                return Err(VoiceHttpError::Unparseable);
            };
            if delta.is_empty() {
                return Ok(VoiceAgentEvent::Ignored);
            }
            let bytes = B64
                .decode(delta.trim())
                .map_err(|_| VoiceHttpError::Unparseable)?;
            if bytes.is_empty() {
                VoiceAgentEvent::Ignored
            } else {
                VoiceAgentEvent::OutputAudioDelta(bytes)
            }
        }
        "response.output_audio.done" | "response.audio.done" => VoiceAgentEvent::OutputAudioDone,
        "response.output_audio_transcript.delta" | "response.audio_transcript.delta" => {
            let text = value
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if text.is_empty() {
                VoiceAgentEvent::Ignored
            } else {
                VoiceAgentEvent::TranscriptDelta(text)
            }
        }
        "response.output_audio_transcript.done" | "response.audio_transcript.done" => {
            let text = value
                .get("transcript")
                .or_else(|| value.get("delta"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            VoiceAgentEvent::TranscriptDone(text)
        }
        "input_audio_buffer.speech_started" => VoiceAgentEvent::SpeechStarted,
        "input_audio_buffer.speech_stopped" => VoiceAgentEvent::SpeechStopped,
        "conversation.item.input_audio_transcription.completed" => {
            let text = value
                .get("transcript")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned();
            if text.is_empty() {
                VoiceAgentEvent::Ignored
            } else {
                VoiceAgentEvent::InputTranscript(text)
            }
        }
        "response.done" => VoiceAgentEvent::ResponseDone,
        "session.updated" => VoiceAgentEvent::SessionUpdated,
        "error" => {
            let message = value
                .get("error")
                .and_then(|err| {
                    err.get("message")
                        .and_then(Value::as_str)
                        .or_else(|| err.as_str())
                })
                .or_else(|| value.get("message").and_then(Value::as_str))
                .unwrap_or("voice agent error")
                .to_owned();
            VoiceAgentEvent::Error(message)
        }
        _ => VoiceAgentEvent::Ignored,
    })
}

/// Live Voice Agent session (sync tungstenite). Feature `live-http` only.
#[cfg(feature = "live-http")]
pub struct VoiceAgentSession {
    socket: tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>,
}

#[cfg(feature = "live-http")]
impl VoiceAgentSession {
    /// Connect with `Authorization: Bearer …`.
    ///
    /// # Errors
    ///
    /// [`VoiceHttpError`] when the URL/auth/socket fails.
    pub fn connect(api_base: &str, bearer: &str, model: &str) -> Result<Self, VoiceHttpError> {
        use tungstenite::client::IntoClientRequest;
        use tungstenite::connect;
        use tungstenite::http::header::{AUTHORIZATION, HeaderValue};

        let url = voice_agent_realtime_url(api_base, model)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| VoiceHttpError::Unreachable)?;
        let auth = format!("Bearer {bearer}");
        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth).map_err(|_| VoiceHttpError::Rejected)?,
        );
        let (socket, _response) = connect(request).map_err(|_| VoiceHttpError::Unreachable)?;
        let mut session = Self { socket };
        session.set_read_timeout_ms(15);
        Ok(session)
    }

    /// Send `session.update`.
    ///
    /// # Errors
    ///
    /// Socket send failures.
    pub fn send_session_update(
        &mut self,
        voice: &str,
        instructions: &str,
        speed: f64,
        web_search: bool,
    ) -> Result<(), VoiceHttpError> {
        let payload = voice_agent_session_update_json(
            voice,
            instructions,
            VOICE_AGENT_INPUT_RATE_HZ,
            VOICE_AGENT_OUTPUT_RATE_HZ,
            speed,
            web_search,
        );
        self.send_text(&payload)
    }

    /// Append one mono PCM16 frame (Softwake capture samples).
    ///
    /// # Errors
    ///
    /// Socket send failures.
    pub fn append_pcm16(&mut self, samples: &[i16]) -> Result<(), VoiceHttpError> {
        if samples.is_empty() {
            return Ok(());
        }
        let bytes = pcm16_le_bytes(samples);
        let payload = voice_agent_append_pcm_json(&bytes);
        self.send_text(&payload)
    }

    /// Best-effort cancel of the in-flight assistant response.
    pub fn cancel_response(&mut self) {
        let _ = self.send_text(&voice_agent_cancel_json());
    }

    /// Insert a text turn. Does not ask the model to reply.
    ///
    /// # Errors
    ///
    /// Socket send failures.
    pub fn send_context_item(&mut self, role: &str, text: &str) -> Result<(), VoiceHttpError> {
        let payload = voice_agent_context_item_json(role, text);
        self.send_text(&payload)
    }

    /// Arm a short TCP read timeout (call once after connect).
    pub fn set_read_timeout_ms(&mut self, ms: u64) {
        use std::time::Duration;
        use tungstenite::stream::MaybeTlsStream;
        let dur = Some(Duration::from_millis(ms));
        match self.socket.get_ref() {
            MaybeTlsStream::Plain(stream) => {
                let _ = stream.set_read_timeout(dur);
            }
            MaybeTlsStream::Rustls(stream) => {
                let _ = stream.get_ref().set_read_timeout(dur);
            }
            _ => {}
        }
    }

    /// Read the next event. Returns `Ok(None)` on timeout / would-block.
    ///
    /// # Errors
    ///
    /// Hard socket failures or unparseable payloads that are not timeouts.
    pub fn try_read_event(&mut self) -> Result<Option<VoiceAgentEvent>, VoiceHttpError> {
        use tungstenite::Message;

        match self.socket.read() {
            Ok(Message::Text(payload)) => Ok(Some(parse_voice_agent_event(&payload)?)),
            Ok(Message::Binary(bytes)) if !bytes.is_empty() => {
                Ok(Some(VoiceAgentEvent::OutputAudioDelta(bytes)))
            }
            Ok(Message::Ping(data)) => {
                let _ = self.socket.send(Message::Pong(data));
                Ok(Some(VoiceAgentEvent::Ignored))
            }
            Ok(Message::Close(_)) => Err(VoiceHttpError::Unreachable),
            Ok(_) => Ok(Some(VoiceAgentEvent::Ignored)),
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                Ok(None)
            }
            Err(_) => Err(VoiceHttpError::Unreachable),
        }
    }

    /// Close the WebSocket.
    pub fn close(mut self) {
        let _ = self.socket.close(None);
    }

    fn send_text(&mut self, payload: &str) -> Result<(), VoiceHttpError> {
        use tungstenite::Message;
        self.socket
            .send(Message::Text(payload.to_owned()))
            .map_err(|_| VoiceHttpError::Unreachable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::VoiceHttpError;

    #[test]
    fn https_api_base_becomes_wss_realtime() {
        let url = voice_agent_realtime_url("https://api.x.ai/v1/", VOICE_AGENT_MODEL_DEFAULT)
            .expect("url");
        assert_eq!(url, "wss://api.x.ai/v1/realtime?model=grok-voice-latest");
    }

    #[test]
    fn rejects_empty_or_hostile_model() {
        assert_eq!(
            voice_agent_realtime_url("https://api.x.ai/v1", "").expect_err("empty"),
            VoiceHttpError::EmptyText
        );
        assert_eq!(
            voice_agent_realtime_url("https://api.x.ai/v1", "grok voice").expect_err("space"),
            VoiceHttpError::Unreachable
        );
    }

    #[test]
    fn context_item_does_not_request_a_reply() {
        let raw = voice_agent_context_item_json("user", "[room spncxrchat] Operator: promoted");
        let value: Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(value["type"], "conversation.item.create");
        assert_eq!(value["item"]["role"], "user");
        assert_eq!(value["item"]["content"][0]["type"], "input_text");
        assert!(raw.contains("spncxrchat"));
        assert!(!raw.contains("response.create"));
        let assistant = voice_agent_context_item_json("assistant", "Sorted.");
        assert!(assistant.contains("output_text"));
        assert!(!assistant.contains("response.create"));
    }

    #[test]
    fn session_update_json_has_vad_and_rates() {
        let raw = voice_agent_session_update_json(
            "carina",
            "You are Softwake.",
            16_000,
            24_000,
            1.25,
            true,
        );
        let value: Value = serde_json::from_str(&raw).expect("json");
        assert_eq!(value["type"], "session.update");
        let session = &value["session"];
        assert_eq!(session["voice"], "carina");
        assert_eq!(session["turn_detection"]["type"], "server_vad");
        assert_eq!(session["audio"]["input"]["format"]["rate"], 16_000);
        assert_eq!(session["audio"]["output"]["format"]["rate"], 24_000);
        assert_eq!(session["audio"]["output"]["speed"], 1.25);
        assert_eq!(session["tools"][0]["type"], "web_search");
        assert_eq!(session["reasoning"]["effort"], "none");
    }

    #[test]
    fn parses_output_audio_and_transcript() {
        let pcm = [0_u8, 1, 2, 3];
        let payload = format!(
            r#"{{"type":"response.output_audio.delta","delta":"{}"}}"#,
            B64.encode(pcm)
        );
        match parse_voice_agent_event(&payload).expect("parse") {
            VoiceAgentEvent::OutputAudioDelta(bytes) => assert_eq!(bytes, pcm),
            other => panic!("unexpected {other:?}"),
        }
        match parse_voice_agent_event(
            r#"{"type":"response.output_audio_transcript.delta","delta":"Hi"}"#,
        )
        .expect("tr")
        {
            VoiceAgentEvent::TranscriptDelta(t) => assert_eq!(t, "Hi"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            parse_voice_agent_event(r#"{"type":"input_audio_buffer.speech_started"}"#)
                .expect("speech"),
            VoiceAgentEvent::SpeechStarted
        );
        match parse_voice_agent_event(
            r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"hello room"}"#,
        )
        .expect("user transcript")
        {
            VoiceAgentEvent::InputTranscript(text) => assert_eq!(text, "hello room"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            parse_voice_agent_event(
                r#"{"type":"conversation.item.input_audio_transcription.updated","transcript":"hel"}"#
            )
            .expect("partial"),
            VoiceAgentEvent::Ignored
        );
        assert_eq!(
            parse_voice_agent_event(
                r#"{"type":"conversation.item.input_audio_transcription.completed","transcript":"  "}"#
            )
            .expect("blank"),
            VoiceAgentEvent::Ignored
        );
        match parse_voice_agent_event(r#"{"type":"error","error":{"message":"nope"}}"#)
            .expect("err")
        {
            VoiceAgentEvent::Error(msg) => assert!(msg.contains("nope")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn pcm16_le_round_bytes() {
        assert_eq!(
            pcm16_le_bytes(&[0x0102_i16, -2]),
            vec![0x02, 0x01, 0xfe, 0xff]
        );
    }

    #[test]
    fn clips_instructions() {
        let long = "a".repeat(50);
        assert_eq!(clip_voice_agent_instructions(&long, 10).len(), 10);
    }
}
