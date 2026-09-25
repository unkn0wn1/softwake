//! Model catalog helpers.

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
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{filter_chat_models, is_chat_model, parse_model_ids};
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
    }
}
