//! Peer DM to another Softwake profile (ADR-0052).

use serde::Deserialize;

/// Tool bus name.
pub const AGENT_MESSAGE_TOOL: &str = "agent_message";

/// Parsed peer DM args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMessageArgs {
    /// Target profile id or name.
    pub to: String,
    /// Message body.
    pub text: String,
    /// Optional room id for logging.
    pub room_id: Option<String>,
}

/// Parse `agent_message` args (JSON object or `to=` / `text=` / `room_id=`).
///
/// # Errors
///
/// Missing to/text.
pub fn parse_agent_message_args(args: &[String]) -> Result<AgentMessageArgs, String> {
    let joined = args.join(" ").trim().to_owned();
    if joined.is_empty() {
        return Err("agent_message needs to and text".into());
    }
    if joined.starts_with('{') {
        #[derive(Deserialize)]
        struct Raw {
            to: String,
            text: String,
            #[serde(default)]
            room_id: Option<String>,
        }
        let raw: Raw =
            serde_json::from_str(&joined).map_err(|e| format!("agent_message JSON: {e}"))?;
        if raw.to.trim().is_empty() || raw.text.trim().is_empty() {
            return Err("agent_message needs to and text".into());
        }
        return Ok(AgentMessageArgs {
            to: raw.to.trim().to_owned(),
            text: raw.text.trim().to_owned(),
            room_id: raw.room_id.filter(|s| !s.is_empty()),
        });
    }
    if args.len() >= 2 && !args[0].contains('=') {
        return Ok(AgentMessageArgs {
            to: args[0].trim().to_owned(),
            text: args[1..].join(" ").trim().to_owned(),
            room_id: None,
        });
    }
    let mut to = String::new();
    let mut text = String::new();
    let mut room_id = None;
    for line in joined.lines().chain(joined.split("%;%")) {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("to=") {
            v.clone_into(&mut to);
        } else if let Some(v) = line.strip_prefix("text=") {
            v.clone_into(&mut text);
        } else if let Some(v) = line.strip_prefix("room_id=") {
            room_id = Some(v.to_owned());
        }
    }
    if to.is_empty() || text.is_empty() {
        return Err("agent_message needs to and text".into());
    }
    Ok(AgentMessageArgs {
        to,
        text,
        room_id: room_id.filter(|s| !s.is_empty()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_and_json() {
        let a = parse_agent_message_args(&["sally".into(), "ping".into()]).unwrap();
        assert_eq!(a.to, "sally");
        assert_eq!(a.text, "ping");
        let b = parse_agent_message_args(&[
            r#"{"to":"default","text":"hi","room_id":"standup"}"#.into()
        ])
        .unwrap();
        assert_eq!(b.room_id.as_deref(), Some("standup"));
    }
}
