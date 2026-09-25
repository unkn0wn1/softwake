//! Alias map parsed from `glossary.md`, plus confirm-echo readback.
//!
//! Expansion is one pass over whole tokens. It does not expand `~`,
//! environment variables, or a target that is itself an alias. It does not
//! touch the filesystem. Echo text does not authorize a tool.

#![allow(
    clippy::module_name_repetitions,
    reason = "Glossary is the public name of this module"
)]

/// A parsed alias map, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glossary {
    entries: Vec<(String, String)>,
}

/// `glossary.md` passed the file checks and failed the map checks.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GlossaryError {
    /// The same alias appears on two rows.
    #[error("duplicate alias {alias}")]
    DuplicateAlias {
        /// The repeated alias.
        alias: String,
    },

    /// The left side is not one alias token, or the target contains another arrow.
    #[error("bad alias {alias}")]
    BadAlias {
        /// The text that failed, or a short note naming a bad target.
        alias: String,
    },

    /// The alias is valid and the target is empty.
    #[error("empty target for {alias}")]
    EmptyTarget {
        /// Alias whose target was missing.
        alias: String,
    },
}

/// Original command text, the one-pass expansion, and a stable readback.
///
/// [`ConfirmEcho::requires_readback`] is false for a quiet safe read. The
/// readback string is still filled in. This value does not run anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmEcho {
    /// Command text passed to [`Glossary::confirm_echo`], unchanged.
    pub original: String,
    /// [`Glossary::expand`] of `original`.
    pub expanded: String,
    /// Three-line readback, including the trailing newline.
    pub readback: String,
    /// Whether a later runner should show `readback` before it starts.
    pub requires_readback: bool,
}

/// How an operator answered a confirm-echo readback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EchoReply {
    /// Trimmed reply was `yes`, `execute`, or `continue`.
    Accept,
    /// Any other non-empty reply. The string is the trimmed correction.
    Correction(String),
}

/// The reply was empty after trimming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EchoReplyError {
    /// Blank, or only whitespace.
    #[error("empty reply")]
    Empty,
}

const MUTATING_FIRST: &[&str] = &[
    "rm", "mv", "cp", "mkdir", "rmdir", "touch", "chmod", "chown", "ln", "dd", "truncate",
    "unlink", "delete", "send", "write",
];

/// Remote or privileged first tokens — always require confirm-echo readback for shell.
const REMOTE_FIRST: &[&str] = &["ssh", "scp", "rsync", "sudo"];

impl Glossary {
    /// Parse alias rows from `glossary.md`.
    ///
    /// Blank lines, and lines whose first non-whitespace character is `#`,
    /// are prose. Any other line without `→` or `->` is prose. A heading
    /// and no aliases is a valid empty map.
    ///
    /// An alias line may start with `- ` or `* `. The left side is one
    /// token matching `[A-Za-z_][A-Za-z0-9_-]{0,31}`. The right side is the
    /// rest of the line, trimmed, non-empty, and must not contain another
    /// arrow. Match is case-sensitive.
    ///
    /// # Errors
    ///
    /// Returns [`GlossaryError`] on a duplicate alias, a bad left side, an
    /// empty target, or a target that contains a second arrow.
    pub fn parse(text: &str) -> Result<Self, GlossaryError> {
        let mut glossary = Self {
            entries: Vec::new(),
        };
        for line in text.lines() {
            glossary.accept_line(line)?;
        }
        Ok(glossary)
    }

    /// Target for `alias`, if the map has that exact token.
    #[must_use]
    pub fn get(&self, alias: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(name, _)| name == alias)
            .map(|(_, target)| target.as_str())
    }

    /// No alias rows. Prose-only text parses as empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replace whole tokens that equal an alias. One pass. Rejoined with one ASCII space.
    ///
    /// `docs2`, `mydocs`, and `docs/file` do not expand. Quotes are not special.
    /// A target is inserted as written, including spaces. A target that is
    /// itself an alias is not expanded again.
    #[must_use]
    pub fn expand(&self, command: &str) -> String {
        let mut expanded = String::new();
        for (index, token) in command.split_whitespace().enumerate() {
            if index > 0 {
                expanded.push(' ');
            }
            expanded.push_str(self.get(token).unwrap_or(token));
        }
        expanded
    }

    /// Expand `command` and build the readback. Does not run the command.
    #[must_use]
    pub fn confirm_echo(&self, command: &str) -> ConfirmEcho {
        let expanded = self.expand(command);
        let requires_readback = requires_readback(self, command, &expanded);
        let readback = format!(
            "readback: {expanded}\nwas: {command}\nreply yes, execute, or continue; any other text is a correction\n"
        );
        ConfirmEcho {
            original: command.to_owned(),
            expanded,
            readback,
            requires_readback,
        }
    }

    fn accept_line(&mut self, line: &str) -> Result<(), GlossaryError> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Ok(());
        }
        let body = strip_bullet(trimmed);
        let Some((left, right)) = split_first_arrow(body) else {
            return Ok(());
        };
        let alias = left.trim();
        if !is_alias_token(alias) {
            return Err(GlossaryError::BadAlias {
                alias: alias.to_owned(),
            });
        }
        let target = right.trim();
        if target.is_empty() {
            return Err(GlossaryError::EmptyTarget {
                alias: alias.to_owned(),
            });
        }
        if contains_arrow(target) {
            return Err(GlossaryError::BadAlias {
                alias: format!("{alias} (target contains another arrow)"),
            });
        }
        if self.get(alias).is_some() {
            return Err(GlossaryError::DuplicateAlias {
                alias: alias.to_owned(),
            });
        }
        self.entries.push((alias.to_owned(), target.to_owned()));
        Ok(())
    }
}

/// Classify a reply to a confirm-echo readback.
///
/// Trimmed, ASCII case-insensitive exact match of `yes`, `execute`, or
/// `continue` is [`EchoReply::Accept`]. Any other non-empty reply is a
/// correction of that trimmed text. This does not run a command.
///
/// # Errors
///
/// Returns [`EchoReplyError::Empty`] when `reply` is empty or only whitespace.
pub fn classify_echo_reply(reply: &str) -> Result<EchoReply, EchoReplyError> {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return Err(EchoReplyError::Empty);
    }
    if is_accept(trimmed) {
        Ok(EchoReply::Accept)
    } else {
        Ok(EchoReply::Correction(trimmed.to_owned()))
    }
}

fn is_accept(trimmed: &str) -> bool {
    trimmed.eq_ignore_ascii_case("yes")
        || trimmed.eq_ignore_ascii_case("execute")
        || trimmed.eq_ignore_ascii_case("continue")
}

fn strip_bullet(trimmed: &str) -> &str {
    trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .unwrap_or(trimmed)
}

fn split_first_arrow(line: &str) -> Option<(&str, &str)> {
    let unicode_at = line.find('\u{2192}');
    let ascii_at = line.find("->");
    let unicode_first = match (unicode_at, ascii_at) {
        (Some(unicode_at), Some(ascii_at)) => unicode_at <= ascii_at,
        (Some(_), None) => true,
        (None, _) => false,
    };
    if unicode_first {
        let at = unicode_at?;
        let rest = at + '\u{2192}'.len_utf8();
        return Some((&line[..at], &line[rest..]));
    }
    let at = ascii_at?;
    Some((&line[..at], &line[at + 2..]))
}

fn contains_arrow(text: &str) -> bool {
    text.contains('\u{2192}') || text.contains("->")
}

fn is_alias_token(alias: &str) -> bool {
    let bytes = alias.as_bytes();
    if !(1..=32).contains(&bytes.len()) {
        return false;
    }
    let first = bytes[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-')
}

fn requires_readback(glossary: &Glossary, command: &str, expanded: &str) -> bool {
    let mut original = command.split_whitespace();
    let first = original.next();
    let first_is_sensitive = first.is_some_and(|token| {
        MUTATING_FIRST
            .iter()
            .chain(REMOTE_FIRST.iter())
            .any(|name| token.eq_ignore_ascii_case(name))
    });
    if first_is_sensitive {
        return true;
    }
    if command
        .split_whitespace()
        .any(|token| glossary.get(token).is_some())
    {
        return true;
    }
    expanded
        .split_whitespace()
        .any(|token| token == "~" || token.starts_with("~/") || token.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::{
        ConfirmEcho, EchoReply, EchoReplyError, Glossary, GlossaryError, classify_echo_reply,
    };

    fn sample() -> Glossary {
        Glossary::parse("docs → /path/to/docs\nnotes → /path/to/notes\n").expect("glossary")
    }

    #[test]
    fn duplicate_alias_is_refused() {
        let error = Glossary::parse("docs → /path/to/docs\ndocs → /path/to/other\n")
            .expect_err("duplicate");
        assert_eq!(
            error,
            GlossaryError::DuplicateAlias {
                alias: "docs".to_owned(),
            }
        );
        assert_eq!(error.to_string(), "duplicate alias docs");
    }

    #[test]
    fn bad_alias_empty_target_and_second_arrow() {
        let bad = Glossary::parse("my docs → /path/to/docs\n").expect_err("bad");
        assert_eq!(
            bad,
            GlossaryError::BadAlias {
                alias: "my docs".to_owned(),
            }
        );

        let empty = Glossary::parse("docs →\n").expect_err("empty");
        assert_eq!(
            empty,
            GlossaryError::EmptyTarget {
                alias: "docs".to_owned(),
            }
        );
        let spaced = Glossary::parse("docs → \n").expect_err("spaces");
        assert_eq!(
            spaced,
            GlossaryError::EmptyTarget {
                alias: "docs".to_owned(),
            }
        );

        let second = Glossary::parse("docs → /path/to/docs → /other\n").expect_err("second");
        match &second {
            GlossaryError::BadAlias { alias } => {
                assert!(alias.contains("another arrow"), "{alias}");
                assert!(alias.contains("docs"), "{alias}");
            }
            other => panic!("expected bad alias, got {other:?}"),
        }
        assert!(second.to_string().contains("another arrow"));
    }

    #[test]
    fn heading_and_prose_with_no_arrows_is_empty() {
        let glossary = Glossary::parse("# heading\n\nA prose line.\n").expect("prose");
        assert!(glossary.is_empty());
        assert_eq!(glossary.get("heading"), None);
    }

    #[test]
    fn arrows_and_bullets_parse() {
        let glossary = Glossary::parse(
            "\
# heading
plain prose
docs → /path/to/docs
notes -> /path/to/notes
- logs → /path/to/logs
* extra → /path/to/extra
",
        )
        .expect("parse");
        assert_eq!(glossary.get("docs"), Some("/path/to/docs"));
        assert_eq!(glossary.get("notes"), Some("/path/to/notes"));
        assert_eq!(glossary.get("logs"), Some("/path/to/logs"));
        assert_eq!(glossary.get("extra"), Some("/path/to/extra"));
        assert_eq!(glossary.get("Docs"), None);
    }

    #[test]
    fn expand_replaces_whole_tokens_once() {
        let glossary = sample();
        let echo = glossary.confirm_echo("list docs");
        assert_eq!(echo.expanded, "list /path/to/docs");
        assert!(echo.requires_readback);
        assert_eq!(
            echo.readback,
            "readback: list /path/to/docs\nwas: list docs\nreply yes, execute, or continue; any other text is a correction\n"
        );
        assert_eq!(echo.original, "list docs");

        assert_eq!(glossary.expand("docs2"), "docs2");
        assert_eq!(glossary.expand("mydocs"), "mydocs");
        assert_eq!(glossary.expand("docs/file"), "docs/file");
        assert_eq!(glossary.expand("Docs"), "Docs");
        assert_eq!(glossary.expand("list \"docs\""), "list \"docs\"");

        let chained = Glossary::parse("a → b\nb → /path/to/b\n").expect("chain");
        assert_eq!(chained.expand("a"), "b");

        let spaced = Glossary::parse("docs → /path/to/my docs\n").expect("spaces");
        assert_eq!(spaced.expand("list docs"), "list /path/to/my docs");
    }

    #[test]
    fn readback_follows_mutating_alias_and_path_tokens() {
        let glossary = sample();
        assert!(!glossary.confirm_echo("cat README.md").requires_readback);
        assert!(!glossary.confirm_echo("status").requires_readback);
        assert!(glossary.confirm_echo("cat /path/to/docs").requires_readback);
        assert!(glossary.confirm_echo("ls ~/notes").requires_readback);
        assert!(glossary.confirm_echo("rm notes").requires_readback);
        assert!(glossary.confirm_echo("RM file").requires_readback);
        assert!(glossary.confirm_echo("send docs").requires_readback);
        assert!(glossary.confirm_echo("ssh aau").requires_readback);
        assert!(glossary.confirm_echo("SSH host").requires_readback);
        assert!(glossary.confirm_echo("sudo id").requires_readback);
        assert!(glossary.confirm_echo("scp a b").requires_readback);
        assert!(glossary.confirm_echo("rsync a b").requires_readback);

        let quiet = glossary.confirm_echo("cat README.md");
        assert!(quiet.readback.starts_with("readback: cat README.md\n"));
        assert!(quiet.readback.ends_with('\n'));
    }

    #[test]
    fn classify_accepts_three_words_and_keeps_corrections() {
        assert_eq!(classify_echo_reply("yes"), Ok(EchoReply::Accept));
        assert_eq!(classify_echo_reply("YES"), Ok(EchoReply::Accept));
        assert_eq!(classify_echo_reply(" execute "), Ok(EchoReply::Accept));
        assert_eq!(classify_echo_reply("continue"), Ok(EchoReply::Accept));
        assert_eq!(
            classify_echo_reply("I said /path/to/other"),
            Ok(EchoReply::Correction("I said /path/to/other".to_owned()))
        );
        assert_eq!(
            classify_echo_reply("no"),
            Ok(EchoReply::Correction("no".to_owned()))
        );
        assert_eq!(
            classify_echo_reply("cancel"),
            Ok(EchoReply::Correction("cancel".to_owned()))
        );
        assert_eq!(
            classify_echo_reply("yes."),
            Ok(EchoReply::Correction("yes.".to_owned()))
        );
        assert_eq!(classify_echo_reply(""), Err(EchoReplyError::Empty));
        assert_eq!(classify_echo_reply("  "), Err(EchoReplyError::Empty));
    }

    #[test]
    fn echo_type_is_the_value_callers_compare() {
        let echo = sample().confirm_echo("status");
        let ConfirmEcho {
            original,
            expanded,
            requires_readback,
            ..
        } = echo;
        assert_eq!(original, "status");
        assert_eq!(expanded, "status");
        assert!(!requires_readback);
    }
}
