//! Model catalog helpers.
//!
//! After Test, Softwake splits `GET …/models` into **chat** and **voice (STT)**
//! lists:
//!
//! - **Voice / STT:** lowercase id contains `whisper`, `transcribe`, or `stt`,
//!   and does **not** contain `tts` or `realtime`. This is the speech-to-text
//!   catalog, not TTS speaker names.
//! - **Chat:** excludes embed / moderation / tts / whisper / transcribe /
//!   realtime / image / audio / video / dall-e / davinci / babbage / ada- /
//!   search, then applies family prefixes (xAI `grok-`, `OpenAI` `gpt-` /
//!   `chatgpt-` / `oN`, `OpenRouter` and OpenAI-compatible: anything not
//!   excluded). Voice ids therefore never appear in the chat list.
//!
//! Unmatched ids are dropped (no “dump raw into both”). Empty voice after a
//! passing Test uses the registry voice seed when the family has one.

use serde_json::Value;

use crate::registry::ProviderFamily;

/// Classify a model id as a chat candidate for Softwake's acting session.
#[must_use]
pub fn is_chat_model(family: ProviderFamily, id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if lower.contains("embed")
        || lower.contains("moderation")
        || lower.contains("tts")
        || lower.contains("whisper")
        || lower.contains("transcribe")
        || lower.contains("realtime")
        || lower.contains("image")
        || lower.contains("audio")
        || lower.contains("video")
        || lower.contains("dall-e")
        || lower.contains("davinci")
        || lower.contains("babbage")
        || lower.contains("ada-")
        || lower.contains("search")
        || lower.contains("stt")
    {
        return false;
    }
    match family {
        ProviderFamily::Xai => lower.starts_with("grok-"),
        ProviderFamily::Openai => {
            lower.starts_with("gpt-")
                || lower.starts_with("chatgpt-")
                || (lower.starts_with('o')
                    && lower.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        }
        // OpenRouter and compatible catalogs mix vendors; keep anything not excluded above.
        ProviderFamily::Openrouter | ProviderFamily::OpenaiCompatible => true,
    }
}

/// Classify a model id as a voice / STT (speech-to-text) candidate.
///
/// Does not include TTS speaker names. Ids containing `tts` or `realtime` are
/// excluded even when they also mention transcription.
#[must_use]
pub fn is_voice_model(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if lower.contains("tts") || lower.contains("realtime") {
        return false;
    }
    lower.contains("whisper") || lower.contains("transcribe") || lower.contains("stt")
}

/// Pull model ids from a `GET /v1/models` JSON body.
#[must_use]
pub fn parse_model_ids(payload: &Value) -> Vec<String> {
    let Some(data) = payload.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for item in data {
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            let trimmed = id.trim();
            if !trimmed.is_empty() {
                ids.push(trimmed.to_owned());
            }
        }
    }
    ids
}

/// Keep chat ids for `family`, preserving catalog order and dropping duplicates.
#[must_use]
pub fn filter_chat_models(family: ProviderFamily, ids: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        if is_chat_model(family, id) && !out.iter().any(|kept: &String| kept == id) {
            out.push(id.clone());
        }
    }
    out
}

/// Keep voice / STT ids, preserving catalog order and dropping duplicates.
#[must_use]
pub fn filter_voice_models(ids: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for id in ids {
        if is_voice_model(id) && !out.iter().any(|kept: &String| kept == id) {
            out.push(id.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        filter_chat_models, filter_voice_models, is_chat_model, is_voice_model, parse_model_ids,
    };
    use crate::registry::ProviderFamily;

    #[test]
    fn classifies_chat_ids() {
        assert!(is_chat_model(ProviderFamily::Xai, "grok-4.5"));
        assert!(!is_chat_model(
            ProviderFamily::Xai,
            "grok-voice-transcribe-2.0"
        ));
        assert!(is_chat_model(ProviderFamily::Openai, "gpt-4.1-mini"));
        assert!(is_chat_model(ProviderFamily::Openai, "o3-mini"));
        assert!(!is_chat_model(ProviderFamily::Openai, "whisper-1"));
        assert!(is_chat_model(
            ProviderFamily::Openrouter,
            "anthropic/claude-sonnet-4"
        ));
        assert!(!is_chat_model(
            ProviderFamily::Openrouter,
            "openai/text-embedding-3-small"
        ));
        assert!(is_chat_model(
            ProviderFamily::OpenaiCompatible,
            "local-llama-3"
        ));
    }

    #[test]
    fn classifies_voice_ids() {
        assert!(is_voice_model("whisper-1"));
        assert!(is_voice_model("gpt-4o-transcribe-diarize"));
        assert!(is_voice_model("grok-voice-transcribe-2.0"));
        assert!(is_voice_model("vendor/stt-large"));
        assert!(!is_voice_model("tts-1"));
        assert!(!is_voice_model("gpt-4o-realtime-preview"));
        assert!(!is_voice_model("grok-4.5"));
        assert!(!is_voice_model("grok-voice-latest"));
    }

    #[test]
    fn parses_and_filters_catalog() {
        let ids = parse_model_ids(&json!({
            "data": [
                {"id": "grok-4.5"},
                {"id": "grok-voice-transcribe-2.0"},
                {"id": "grok-4.5"}
            ]
        }));
        assert_eq!(
            filter_chat_models(ProviderFamily::Xai, &ids),
            vec!["grok-4.5".to_owned()]
        );
        assert_eq!(
            filter_voice_models(&ids),
            vec!["grok-voice-transcribe-2.0".to_owned()]
        );
    }

    #[test]
    fn splits_openai_catalog() {
        let ids = [
            "gpt-4o-transcribe-diarize",
            "whisper-1",
            "gpt-4.1-mini",
            "o4-mini",
            "text-embedding-3-small",
            "dall-e-3",
            "tts-1",
            "gpt-4o-realtime-preview",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        assert_eq!(
            filter_voice_models(&ids),
            vec![
                "gpt-4o-transcribe-diarize".to_owned(),
                "whisper-1".to_owned()
            ]
        );
        assert_eq!(
            filter_chat_models(ProviderFamily::Openai, &ids),
            vec!["gpt-4.1-mini".to_owned(), "o4-mini".to_owned()]
        );
    }
}
