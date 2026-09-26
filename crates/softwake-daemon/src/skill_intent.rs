//! Heuristic `skill_save` proposals from typed/spoken ask lines.
//!
//! Does not write a skill. Hands confirm-gates `skill_save`.

/// Propose `skill_save` args: title, procedure, pitfalls, verify.
#[must_use]
pub(crate) fn propose_skill_save(text: &str, last_assistant: Option<&str>) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    for prefix in [
        "make a skill from this",
        "save as skill from this",
        "create a skill from this",
    ] {
        if lower == prefix || lower.starts_with(&format!("{prefix} ")) {
            let title = "from-conversation";
            let procedure = last_assistant
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Capture the last successful workflow here.")
                .to_owned();
            return Some(vec![
                title.to_owned(),
                procedure,
                String::new(),
                String::new(),
            ]);
        }
    }

    for prefix in [
        "make a skill ",
        "create a skill ",
        "save as skill ",
        "save skill ",
    ] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            if rest.trim().is_empty() {
                return None;
            }
            let start = prefix.len();
            let original = trimmed[start..].trim();
            if original.is_empty() {
                return None;
            }
            // Optional "TITLE: procedure…"
            let (title, procedure) = if let Some((t, p)) = original.split_once(':') {
                (t.trim(), p.trim())
            } else {
                (original, "Fill in the procedure for this skill.")
            };
            if title.is_empty() {
                return None;
            }
            return Some(vec![
                title.to_owned(),
                procedure.to_owned(),
                String::new(),
                String::new(),
            ]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::propose_skill_save;

    #[test]
    fn make_a_skill_title() {
        let args = propose_skill_save("make a skill Check Swap", None).expect("args");
        assert_eq!(args[0], "Check Swap");
        assert!(!args[1].is_empty());
    }

    #[test]
    fn from_this_uses_last_assistant() {
        let args = propose_skill_save("make a skill from this", Some("Run free -h")).expect("args");
        assert_eq!(args[0], "from-conversation");
        assert_eq!(args[1], "Run free -h");
    }

    #[test]
    fn unrelated_is_none() {
        assert!(propose_skill_save("what is swap?", None).is_none());
    }
}
