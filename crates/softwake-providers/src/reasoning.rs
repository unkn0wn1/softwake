//! Chat Completions `reasoning_effort` (xAI / OpenAI-compatible).
//!
//! Softwake stores the operator choice in `providers.json` and forwards it on
//! each chat request when set. Empty means omit the field (provider default).

use serde_json::Value;

/// Supported effort modes (xAI Grok 4.x / compatible APIs).
pub const REASONING_EFFORT_MODES: &[&str] = &["low", "medium", "high", "xhigh"];

/// Normalize slash / tool input to a stored value.
///
/// `default`, `off`, `none`, or empty clear the override (omit on the wire).
/// Known modes are lowercased. Anything else is an error.
///
/// # Errors
///
/// Unknown spelling.
pub fn normalize_reasoning_effort(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(lower.as_str(), "default" | "off" | "none") {
        return Ok(String::new());
    }
    REASONING_EFFORT_MODES
        .iter()
        .find(|mode| **mode == lower)
        .map(|mode| (*mode).to_owned())
        .ok_or_else(|| {
            format!(
                "unknown reasoning mode `{trimmed}`; modes: {}",
                REASONING_EFFORT_MODES.join(", ")
            )
        })
}

/// Insert `reasoning_effort` when `effort` is non-empty.
pub fn insert_reasoning_effort(body: &mut Value, effort: &str) {
    let trimmed = effort.trim();
    if trimmed.is_empty() {
        return;
    }
    body["reasoning_effort"] = Value::String(trimmed.to_owned());
}

/// Operator-facing label for a stored value.
#[must_use]
pub fn reasoning_effort_label(stored: &str) -> &str {
    let trimmed = stored.trim();
    if trimmed.is_empty() {
        "default"
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_modes_and_clear() {
        assert_eq!(normalize_reasoning_effort("HIGH").unwrap(), "high");
        assert_eq!(normalize_reasoning_effort("xhigh").unwrap(), "xhigh");
        assert_eq!(normalize_reasoning_effort("default").unwrap(), "");
        assert_eq!(normalize_reasoning_effort("off").unwrap(), "");
        assert!(normalize_reasoning_effort("turbo").is_err());
    }

    #[test]
    fn insert_omits_when_empty() {
        let mut body = json!({"model": "grok"});
        insert_reasoning_effort(&mut body, "");
        assert!(body.get("reasoning_effort").is_none());
        insert_reasoning_effort(&mut body, "low");
        assert_eq!(body["reasoning_effort"], "low");
    }
}
