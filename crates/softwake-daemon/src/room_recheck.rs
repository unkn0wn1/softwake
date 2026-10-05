//! One revise-or-keep check for a room reply that is still waiting to play.
//!
//! The check is not a new room turn. The model returns KEEP, `NO_REPLY`, a
//! private note, or a replacement line. The room log appends the final public
//! line when that clip is about to play
//! ([ADR 0053](../../docs/ADR-0053-room-queued-reply-recheck.md)).

use std::time::Duration;

/// How long playback may wait for a re-check that has already started.
///
/// Past this, the original draft plays. The room does not stall on the model.
pub(crate) const ROOM_REQUEUE_RECHECK_WAIT: Duration = Duration::from_millis(1500);

/// What the short re-check model call decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecheckDecision {
    /// Append the draft at play time and speak it.
    Keep,
    /// Do not log this reply and do not play it.
    Drop,
    /// Append this public line once at play time, then speak it.
    Revise(String),
    /// Send a private note at play time and do not speak a public line.
    Private { to: String, text: String },
}

/// One peer line already logged ahead of a waiting reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AheadLine {
    /// Speaker display name.
    pub name: String,
    /// Current text. A revision replaces the draft before a later check reads it.
    pub text: String,
}

/// One public reply queued in this fan-out, in finish order.
///
/// `text` is the draft until the play-time decision stores the final line.
/// The room log does not have the row until that decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FanoutSay {
    /// Profile id of the speaker.
    pub profile_id: String,
    /// Display name stored on the log row.
    pub name: String,
    /// Draft until play time, then the final line.
    pub text: String,
}

/// True when the whole body is the quiet token the fan-out already uses.
#[must_use]
pub(crate) fn is_quiet_reply(text: &str) -> bool {
    let lower = text
        .trim()
        .trim_matches(|c: char| c == '.' || c == '!' || c == '"')
        .to_ascii_lowercase();
    lower.is_empty() || lower == "no_reply" || lower == "no reply"
}

/// `PRIVATE <name>: <message>` is a note to one member, not a room line.
#[must_use]
pub(crate) fn looks_like_private_note(reply: &str) -> bool {
    private_note_body(reply).is_some()
}

/// Recipient token and note body. `None` when this is not a private note.
#[must_use]
pub(crate) fn parse_private_note(reply: &str) -> Option<(String, String)> {
    let rest = private_note_body(reply)?;
    let (to, text) = rest.split_once(':')?;
    let to = to.trim();
    let text = text.trim();
    if to.is_empty() || text.is_empty() || to.contains('\n') || to.contains(',') {
        return None;
    }
    Some((to.to_owned(), text.to_owned()))
}

fn private_note_body(reply: &str) -> Option<&str> {
    let trimmed = reply.trim();
    let prefix = "PRIVATE";
    let rest = trimmed.get(prefix.len()..)?;
    if !trimmed[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return None;
    }
    if !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
        return None;
    }
    Some(rest.trim_start())
}

fn is_keep_token(text: &str) -> bool {
    let lower = text
        .trim()
        .trim_matches(|c: char| c == '.' || c == '!' || c == '"')
        .to_ascii_lowercase();
    lower == "keep"
}

/// Parse one re-check reply. The whole trimmed body decides.
///
/// `KEEP` or `NO_REPLY` with any later non-empty line is a revision of the
/// whole body. `KEEP: still shipping` is a revision. A body that is only
/// `PRIVATE Name: ...` stays a private note.
#[must_use]
pub(crate) fn parse_recheck_decision(raw: &str) -> RecheckDecision {
    let trimmed = raw.trim();
    if is_quiet_reply(trimmed) {
        return RecheckDecision::Drop;
    }
    if is_keep_token(trimmed) {
        return RecheckDecision::Keep;
    }
    if let Some((to, text)) = parse_private_note(trimmed) {
        return RecheckDecision::Private { to, text };
    }
    RecheckDecision::Revise(trimmed.to_owned())
}

/// Map a model result onto a decision. A transport error keeps the draft.
#[must_use]
pub(crate) fn decision_from_oneshot(result: Result<String, String>) -> RecheckDecision {
    match result {
        Ok(raw) => parse_recheck_decision(&raw),
        Err(_) => RecheckDecision::Keep,
    }
}

/// Peer lines logged before `speaker_index`, still present, in order.
#[must_use]
pub(crate) fn ahead_of(says: &[FanoutSay], speaker_index: usize) -> Vec<AheadLine> {
    says.get(..speaker_index)
        .unwrap_or(&[])
        .iter()
        .map(|say| AheadLine {
            name: say.name.clone(),
            text: say.text.clone(),
        })
        .collect()
}

/// Ahead lines for the say whose profile and current text match.
///
/// Empty when this speaker is first or the row was already removed.
#[must_use]
pub(crate) fn ahead_for(says: &[FanoutSay], profile_id: &str, draft: &str) -> Vec<AheadLine> {
    let Some(index) = says
        .iter()
        .position(|say| say.profile_id == profile_id && say.text == draft)
    else {
        return Vec::new();
    };
    ahead_of(says, index)
}

/// True when a re-check should call the model.
///
/// Off skips every clip. On still skips a clip with nothing logged ahead of it.
#[must_use]
pub(crate) fn recheck_should_run(enabled: bool, ahead: &[AheadLine]) -> bool {
    enabled && !ahead.is_empty()
}

/// Prompt for the one allowed re-check. No tool call, no extra turn.
#[must_use]
pub(crate) fn recheck_prompt(
    display: &str,
    member_id: &str,
    draft: &str,
    ahead: &[AheadLine],
) -> String {
    let peers = if ahead.is_empty() {
        "(none)".to_owned()
    } else {
        ahead
            .iter()
            .map(|line| {
                format!(
                    "{} (another agent in this room) said: {}",
                    line.name.trim(),
                    line.text.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "\
You are {display} (profile id `{member_id}`) in a Softwake room.
You drafted a reply that has not been spoken yet. Peer lines logged ahead of you:
{peers}

Your draft:
{draft}

Reply with exactly one of: KEEP, NO_REPLY, PRIVATE Name: message, or the replacement line alone.
You may address an earlier speaker by name or the operator.
Do not call tools. Do not explain."
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AheadLine, FanoutSay, RecheckDecision, ahead_for, ahead_of, decision_from_oneshot,
        is_quiet_reply, parse_private_note, parse_recheck_decision, recheck_prompt,
        recheck_should_run,
    };

    #[test]
    fn parser_keeps_drops_revises_and_accepts_private() {
        assert_eq!(parse_recheck_decision("KEEP"), RecheckDecision::Keep);
        assert_eq!(parse_recheck_decision("  keep. "), RecheckDecision::Keep);
        assert_eq!(parse_recheck_decision("KEEP!"), RecheckDecision::Keep);
        assert_eq!(parse_recheck_decision("NO_REPLY"), RecheckDecision::Drop);
        assert_eq!(parse_recheck_decision("no reply."), RecheckDecision::Drop);
        assert_eq!(parse_recheck_decision("   "), RecheckDecision::Drop);
        assert!(is_quiet_reply(""));
        assert_eq!(
            parse_recheck_decision("KEEP\nstill shipping"),
            RecheckDecision::Revise("KEEP\nstill shipping".to_owned())
        );
        assert_eq!(
            parse_recheck_decision("NO_REPLY but actually here is a thought"),
            RecheckDecision::Revise("NO_REPLY but actually here is a thought".to_owned())
        );
        assert_eq!(
            parse_recheck_decision("KEEP: still shipping"),
            RecheckDecision::Revise("KEEP: still shipping".to_owned())
        );
        assert_eq!(
            parse_recheck_decision("PRIVATE Sally: ping the notes"),
            RecheckDecision::Private {
                to: "Sally".to_owned(),
                text: "ping the notes".to_owned(),
            }
        );
        assert_eq!(
            parse_private_note("private Sally: hi").expect("note").0,
            "Sally"
        );
        assert_eq!(
            parse_recheck_decision("the cut moves to Friday"),
            RecheckDecision::Revise("the cut moves to Friday".to_owned())
        );
        assert_eq!(
            parse_recheck_decision("PRIVATE Sally, Joi: hi"),
            RecheckDecision::Revise("PRIVATE Sally, Joi: hi".to_owned())
        );
        assert!(matches!(
            parse_recheck_decision("not private, just a word"),
            RecheckDecision::Revise(text) if text.contains("private")
        ));
    }

    #[test]
    fn oneshot_error_keeps_the_draft() {
        assert_eq!(
            decision_from_oneshot(Err("offline".to_owned())),
            RecheckDecision::Keep
        );
        assert_eq!(
            decision_from_oneshot(Ok("NO_REPLY".to_owned())),
            RecheckDecision::Drop
        );
    }

    #[test]
    fn ahead_lines_follow_final_text_and_skip_drops() {
        let mut says = vec![
            FanoutSay {
                profile_id: "a".to_owned(),
                name: "Ann".to_owned(),
                text: "one".to_owned(),
            },
            FanoutSay {
                profile_id: "b".to_owned(),
                name: "Bea".to_owned(),
                text: "two".to_owned(),
            },
            FanoutSay {
                profile_id: "c".to_owned(),
                name: "Cy".to_owned(),
                text: "three".to_owned(),
            },
        ];
        assert!(ahead_of(&says, 0).is_empty());
        assert_eq!(ahead_of(&says, 1)[0].text, "one");
        assert!(!recheck_should_run(true, &ahead_of(&says, 0)));
        assert!(!recheck_should_run(false, &ahead_of(&says, 1)));
        assert!(recheck_should_run(true, &ahead_of(&says, 1)));
        says[1].text = "two revised".to_owned();
        let third = ahead_of(&says, 2);
        assert_eq!(third[0].text, "one");
        assert_eq!(third[1].text, "two revised");
        says.remove(1);
        let after_drop = ahead_for(&says, "c", "three");
        assert_eq!(after_drop.len(), 1);
        assert_eq!(after_drop[0].name, "Ann");
        assert_eq!(after_drop[0].text, "one");
        assert!(ahead_for(&says, "c", "missing").is_empty());
    }

    #[test]
    fn prompt_names_the_draft_and_the_peer() {
        let prompt = recheck_prompt(
            "Bea",
            "b",
            "two",
            &[AheadLine {
                name: "Ann".to_owned(),
                text: "one".to_owned(),
            }],
        );
        assert!(prompt.contains("Bea"));
        assert!(prompt.contains("`b`"));
        assert!(prompt.contains("Your draft:\ntwo"));
        assert!(prompt.contains("Ann (another agent in this room) said: one"));
        assert!(prompt.contains("KEEP"));
        assert!(prompt.contains("NO_REPLY"));
        assert!(prompt.contains("PRIVATE Name: message"));
        assert!(prompt.contains("Do not call tools."));
    }
}
