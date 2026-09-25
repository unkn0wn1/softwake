//! Heuristic shell proposals from typed/spoken ask lines.
//!
//! Does not run a command. The Hands path expands glossary aliases and confirm-gates.

use softwake_soul::Glossary;

/// Map a casual remote phrase to a small remote command.
fn remote_phrase(rest: &str) -> String {
    let lower = rest.trim().to_ascii_lowercase();
    match lower.as_str() {
        "check swap" | "show swap" | "swap" => "free -h && swapon --show".to_owned(),
        "check disk" | "show disk" | "disk" => "df -h".to_owned(),
        "uptime" | "check uptime" => "uptime".to_owned(),
        _ => rest.trim().to_owned(),
    }
}

/// Extract a shell command proposal from ask text.
///
/// Patterns (case-insensitive lead-in):
/// - `run <cmd>` / `shell <cmd>`
/// - `ssh …` (whole line kept, then glossary-expanded by caller)
/// - `ssh to <alias> and <rest>` → `{expand(alias)} {remote}` when `alias` is in the glossary;
///   otherwise `ssh <alias> <remote>`
#[must_use]
pub(crate) fn propose_shell_command(text: &str, glossary: &Glossary) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    for prefix in ["run ", "shell "] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let start = prefix.len();
            let original_rest = trimmed[start..].trim();
            if rest.trim().is_empty() || original_rest.is_empty() {
                return None;
            }
            return Some(glossary.expand(original_rest));
        }
    }

    if let Some(command) = parse_ssh_to_and(trimmed, glossary) {
        return Some(command);
    }

    if lower.starts_with("ssh ") || lower == "ssh" {
        return Some(glossary.expand(trimmed));
    }

    None
}

fn parse_ssh_to_and(text: &str, glossary: &Glossary) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let after = lower.strip_prefix("ssh to ")?;
    let and_at = after.find(" and ")?;
    let host_lower = after[..and_at].trim();
    if host_lower.is_empty() {
        return None;
    }
    // Map lengths back onto original text for host / rest casing.
    let prefix_len = "ssh to ".len();
    let host_original = text[prefix_len..prefix_len + host_lower.len()].trim();
    let rest_start = prefix_len + and_at + " and ".len();
    if rest_start > text.len() {
        return None;
    }
    let rest_original = text[rest_start..].trim();
    if rest_original.is_empty() {
        return None;
    }
    let remote = remote_phrase(rest_original);
    if glossary.get(host_original).is_some() {
        let expanded = glossary.expand(host_original);
        return Some(format!("{expanded} {remote}"));
    }
    Some(glossary.expand(&format!("ssh {host_original} {remote}")))
}

#[cfg(test)]
mod tests {
    use super::propose_shell_command;
    use softwake_soul::Glossary;

    fn glossary() -> Glossary {
        Glossary::parse("aau → ssh -l root aau\n").expect("glossary")
    }

    #[test]
    fn run_prefix_expands_alias_tokens() {
        let glossary = glossary();
        assert_eq!(
            propose_shell_command("run echo hi", &glossary).as_deref(),
            Some("echo hi")
        );
        assert_eq!(
            propose_shell_command("shell aau free -h", &glossary).as_deref(),
            Some("ssh -l root aau free -h")
        );
    }

    #[test]
    fn ssh_to_alias_and_check_swap() {
        let glossary = glossary();
        assert_eq!(
            propose_shell_command("ssh to aau and check swap", &glossary).as_deref(),
            Some("ssh -l root aau free -h && swapon --show")
        );
    }

    #[test]
    fn plain_chat_is_ignored() {
        let glossary = glossary();
        assert_eq!(propose_shell_command("what is swap?", &glossary), None);
        assert_eq!(propose_shell_command("run", &glossary), None);
    }
}
