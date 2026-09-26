//! Heuristic `schedule` proposals from typed/spoken ask lines.
//!
//! Does not write a schedule. Hands confirm-gates `schedule`.

/// Propose `schedule` argv from a clear reminder line.
#[must_use]
pub(crate) fn propose_schedule(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    // "remind me daily at 07:30 …"
    if let Some(rest) = strip_prefix_ci(&lower, "remind me daily at ") {
        return daily_from(rest, trimmed);
    }
    if let Some(rest) = strip_prefix_ci(&lower, "set a daily reminder at ") {
        return daily_from(rest, trimmed);
    }
    if let Some(rest) = strip_prefix_ci(&lower, "daily reminder at ") {
        return daily_from(rest, trimmed);
    }

    // "remind me at 2026-09-27T07:30 …" or "remind me at 07:30 …" (daily if HH:MM only)
    for prefix in ["remind me at ", "set a reminder at ", "reminder at "] {
        if let Some(rest) = strip_prefix_ci(&lower, prefix) {
            return once_or_daily_from(rest, trimmed, prefix.len());
        }
    }

    // "schedule cron 0 9 * * 1-5 …"
    if let Some(rest) = strip_prefix_ci(&lower, "schedule cron ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() < 6 {
            return None;
        }
        let cron = parts[..5].join(" ");
        // Message is everything after the five cron tokens in the original line.
        let after_cron = trimmed
            .split_whitespace()
            .skip(2 + 5) // "schedule" "cron" + 5 fields
            .collect::<Vec<_>>()
            .join(" ");
        if after_cron.is_empty() {
            return None;
        }
        return Some(vec!["create".into(), "cron".into(), cron, after_cron]);
    }

    None
}

fn strip_prefix_ci<'a>(lower: &'a str, prefix: &str) -> Option<&'a str> {
    lower.strip_prefix(prefix)
}

fn daily_from(rest_lower: &str, original: &str) -> Option<Vec<String>> {
    let mut parts = rest_lower.split_whitespace();
    let when = parts.next()?.to_owned();
    if !looks_like_hhmm(&when) {
        return None;
    }
    // message from original after the time token
    let idx = original
        .to_ascii_lowercase()
        .find(&when)
        .map_or(original.len(), |i| i + when.len());
    let message = original[idx..].trim();
    if message.is_empty() {
        return None;
    }
    Some(vec![
        "create".into(),
        "daily".into(),
        when,
        message.to_owned(),
    ])
}

fn once_or_daily_from(rest_lower: &str, original: &str, _prefix_len: usize) -> Option<Vec<String>> {
    let mut parts = rest_lower.split_whitespace();
    let when_lower = parts.next()?.to_owned();
    let start = original.to_ascii_lowercase().find(&when_lower)?;
    let end = start + when_lower.len();
    let when = original.get(start..end)?.to_owned();
    let message = original[end..].trim();
    if message.is_empty() {
        return None;
    }
    if looks_like_datetime(&when_lower) {
        return Some(vec![
            "create".into(),
            "once".into(),
            when,
            message.to_owned(),
        ]);
    }
    if looks_like_hhmm(&when_lower) {
        return Some(vec![
            "create".into(),
            "daily".into(),
            when,
            message.to_owned(),
        ]);
    }
    None
}

fn looks_like_hhmm(text: &str) -> bool {
    let b = text.as_bytes();
    matches!(b, [h1, h2, b':', m1, m2] if h1.is_ascii_digit()
        && h2.is_ascii_digit()
        && m1.is_ascii_digit()
        && m2.is_ascii_digit())
}

fn looks_like_datetime(text: &str) -> bool {
    (text.contains('T') || text.contains('t')) && text.contains('-') && text.contains(':')
}

#[cfg(test)]
mod tests {
    use super::propose_schedule;

    #[test]
    fn daily_remind() {
        let args = propose_schedule("remind me daily at 07:30 take meds").expect("args");
        assert_eq!(args[0], "create");
        assert_eq!(args[1], "daily");
        assert_eq!(args[2], "07:30");
        assert!(args[3].contains("meds"));
    }

    #[test]
    fn once_remind() {
        let args = propose_schedule("remind me at 2026-09-27T09:00 call bank").expect("args");
        assert_eq!(args[1], "once");
        assert_eq!(args[2], "2026-09-27T09:00");
    }
}
