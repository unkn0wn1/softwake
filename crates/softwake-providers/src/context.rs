//! Context-window limits and char-based token estimates for awake chat.
//!
//! See [ADR 0021](../../docs/ADR-0021-multi-turn-compact.md).

/// Fallback context window when Settings has no override and the model id is unknown.
pub const DEFAULT_CONTEXT_LIMIT_TOKENS: u32 = 128_000;

/// Default compaction trigger as a percent of the resolved context limit.
pub const DEFAULT_COMPACT_AT_PERCENT: u8 = 70;

/// Default number of recent message entries kept raw after compaction.
pub const DEFAULT_KEEP_RECENT_TURNS: u32 = 8;

/// v1 estimator: Unicode scalar count / 4, rounded up.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    u32::try_from(chars.div_ceil(4)).unwrap_or(u32::MAX)
}

/// Sum of [`estimate_tokens`] over each string.
#[must_use]
pub fn estimate_tokens_parts<'a, I>(parts: I) -> u32
where
    I: IntoIterator<Item = &'a str>,
{
    parts
        .into_iter()
        .map(estimate_tokens)
        .fold(0_u32, u32::saturating_add)
}

/// Built-in context limits for known Grok / OpenAI-family model ids.
///
/// Matching is case-insensitive. Exact id matches win over prefixes.
const BUILTIN_EXACT_LIMITS: &[(&str, u32)] = &[
    ("grok-4.5", 131_072),
    ("grok-4", 131_072),
    ("grok-3-mini", 131_072),
    ("grok-3", 131_072),
    ("grok-2", 131_072),
    ("gpt-4.1-mini", 1_047_576),
    ("gpt-4.1", 1_047_576),
    ("gpt-4o-mini", 128_000),
    ("gpt-4o", 128_000),
    ("gpt-4-turbo", 128_000),
    ("chatgpt-4o-latest", 128_000),
    ("o3-mini", 200_000),
    ("o3", 200_000),
    ("o1-mini", 128_000),
    ("o1", 200_000),
    ("o4-mini", 200_000),
];

#[must_use]
pub fn builtin_context_limit(model_id: &str) -> Option<u32> {
    let lower = model_id.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    for (id, limit) in BUILTIN_EXACT_LIMITS {
        if lower == *id {
            return Some(*limit);
        }
    }
    if lower.starts_with("grok-") {
        return Some(131_072);
    }
    if lower.starts_with("gpt-4.1") || lower.contains("/gpt-4.1") {
        return Some(1_047_576);
    }
    if lower.starts_with("gpt-4o") || lower.contains("/gpt-4o") {
        return Some(128_000);
    }
    if lower.starts_with("o1") || lower.starts_with("o3") || lower.starts_with("o4") {
        return Some(200_000);
    }
    if lower.contains("/grok-") {
        return Some(131_072);
    }
    None
}

/// Resolve the context window for a model id and optional Settings override.
///
/// `override_tokens` of `0` means unset (use map / fallback).
#[must_use]
pub fn resolve_context_limit(model_id: &str, override_tokens: u32) -> u32 {
    if override_tokens > 0 {
        return override_tokens;
    }
    builtin_context_limit(model_id).unwrap_or(DEFAULT_CONTEXT_LIMIT_TOKENS)
}

/// Clamp Settings `compact_at_percent` into 1..=100 (default 70 when 0).
#[must_use]
pub fn resolve_compact_at_percent(raw: u8) -> u8 {
    if raw == 0 {
        DEFAULT_COMPACT_AT_PERCENT
    } else {
        raw.min(100)
    }
}

/// Clamp Settings `keep_recent_turns` (default 8 when 0).
#[must_use]
pub fn resolve_keep_recent_turns(raw: u32) -> usize {
    let n = if raw == 0 {
        DEFAULT_KEEP_RECENT_TURNS
    } else {
        raw
    };
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Whether estimated usage meets or exceeds the compaction threshold.
#[must_use]
pub fn should_compact(estimated_tokens: u32, limit: u32, compact_at_percent: u8) -> bool {
    if limit == 0 {
        return false;
    }
    let percent = u32::from(resolve_compact_at_percent(compact_at_percent));
    let threshold = limit.saturating_mul(percent) / 100;
    estimated_tokens >= threshold
}

/// Percent of limit used, capped at 100 for display.
#[must_use]
pub fn usage_percent(used: u32, limit: u32) -> u8 {
    if limit == 0 {
        return 0;
    }
    let pct = u64::from(used).saturating_mul(100) / u64::from(limit);
    u8::try_from(pct.min(100)).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_COMPACT_AT_PERCENT, DEFAULT_CONTEXT_LIMIT_TOKENS, builtin_context_limit,
        estimate_tokens, resolve_compact_at_percent, resolve_context_limit, should_compact,
        usage_percent,
    };

    #[test]
    fn estimate_tokens_is_chars_div_4_ceil() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens("你好"), 1);
    }

    #[test]
    fn builtin_map_covers_grok_and_openai_family() {
        assert_eq!(builtin_context_limit("grok-4.5"), Some(131_072));
        assert_eq!(builtin_context_limit("grok-4-latest"), Some(131_072));
        assert_eq!(builtin_context_limit("gpt-4o-2024-08-06"), Some(128_000));
        assert_eq!(
            builtin_context_limit("openai/gpt-4.1-mini"),
            Some(1_047_576)
        );
        assert_eq!(builtin_context_limit("mystery-model"), None);
    }

    #[test]
    fn resolve_prefers_override_then_map_then_fallback() {
        assert_eq!(resolve_context_limit("mystery", 4096), 4096);
        assert_eq!(resolve_context_limit("grok-4.5", 0), 131_072);
        assert_eq!(
            resolve_context_limit("mystery", 0),
            DEFAULT_CONTEXT_LIMIT_TOKENS
        );
    }

    #[test]
    fn compact_threshold_uses_percent_of_limit() {
        assert!(!should_compact(69, 100, 70));
        assert!(should_compact(70, 100, 70));
        assert_eq!(resolve_compact_at_percent(0), DEFAULT_COMPACT_AT_PERCENT);
        assert_eq!(usage_percent(70, 100), 70);
    }
}
