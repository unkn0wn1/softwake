//! Softwake ctl tools — agent-callable mirrors of awake slash commands (ADR-0031).

use crate::ToolError;

/// Mirror `/status`.
pub const SOFTWAKE_STATUS_TOOL: &str = "softwake_status";
/// Mirror `/model` list.
pub const SOFTWAKE_LIST_MODELS_TOOL: &str = "softwake_list_models";
/// Mirror `/voice` list.
pub const SOFTWAKE_LIST_VOICES_TOOL: &str = "softwake_list_voices";
/// Mirror `/profile` list.
pub const SOFTWAKE_LIST_PROFILES_TOOL: &str = "softwake_list_profiles";
/// Mirror `/model ai|voice <id>`.
pub const SOFTWAKE_SET_MODEL_TOOL: &str = "softwake_set_model";
/// Mirror `/voice <id>`.
pub const SOFTWAKE_SET_VOICE_TOOL: &str = "softwake_set_voice";
/// Mirror `/profile <name>`.
pub const SOFTWAKE_SET_PROFILE_TOOL: &str = "softwake_set_profile";
/// Mirror `/sleep`.
pub const SOFTWAKE_SLEEP_TOOL: &str = "softwake_sleep";
/// Mirror `/hibernate`.
pub const SOFTWAKE_HIBERNATE_TOOL: &str = "softwake_hibernate";
/// Mirror `/resume`.
pub const SOFTWAKE_RESUME_TOOL: &str = "softwake_resume";
/// Mirror `/new`.
pub const SOFTWAKE_NEW_SESSION_TOOL: &str = "softwake_new_session";
/// Mirror `/refresh` — full profile + chat reload (ADR-0031).
pub const SOFTWAKE_REFRESH_TOOL: &str = "softwake_refresh";

/// Every Softwake ctl tool name, registration order.
pub const SOFTWAKE_CTL_TOOLS: &[&str] = &[
    SOFTWAKE_STATUS_TOOL,
    SOFTWAKE_LIST_MODELS_TOOL,
    SOFTWAKE_LIST_VOICES_TOOL,
    SOFTWAKE_LIST_PROFILES_TOOL,
    SOFTWAKE_SET_MODEL_TOOL,
    SOFTWAKE_SET_VOICE_TOOL,
    SOFTWAKE_SET_PROFILE_TOOL,
    SOFTWAKE_SLEEP_TOOL,
    SOFTWAKE_HIBERNATE_TOOL,
    SOFTWAKE_RESUME_TOOL,
    SOFTWAKE_NEW_SESSION_TOOL,
    SOFTWAKE_REFRESH_TOOL,
];

/// Whether `name` is a Softwake ctl tool.
#[must_use]
pub fn is_softwake_ctl(name: &str) -> bool {
    SOFTWAKE_CTL_TOOLS.contains(&name)
}

/// Whether this ctl tool only reads (Always allow default).
#[must_use]
pub fn softwake_ctl_is_list(name: &str) -> bool {
    matches!(
        name,
        SOFTWAKE_STATUS_TOOL
            | SOFTWAKE_LIST_MODELS_TOOL
            | SOFTWAKE_LIST_VOICES_TOOL
            | SOFTWAKE_LIST_PROFILES_TOOL
    )
}

/// Kind of runtime side effect for mutating ctl tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftwakeCtlEffect {
    /// Format `/status` on the runtime.
    Status,
    /// List models (disk) — usually handled without effect; kept for completeness.
    ListModels,
    /// List voices.
    ListVoices,
    /// List profiles.
    ListProfiles,
    /// Set chat or voice/STT model: ("ai"|"voice", id).
    SetModel {
        /// `ai` or `voice`.
        which: String,
        /// Model id.
        id: String,
    },
    /// Set TTS voice id.
    SetVoice {
        /// Voice id.
        id: String,
    },
    /// Switch active profile.
    SetProfile {
        /// Name or id.
        name: String,
    },
    /// Enter sleep.
    Sleep,
    /// Enter hibernate.
    Hibernate,
    /// Hibernate → sleep.
    Resume,
    /// `/new` — fresh session from soul pack.
    NewSession,
    /// Full refresh: reload active profile soul, clear model session, HUD reseed, MCP rediscover.
    Refresh,
}

/// Parse ctl tool + args into a runtime effect.
///
/// # Errors
///
/// Unknown name or invalid args.
pub fn parse_softwake_ctl(name: &str, args: &[String]) -> Result<SoftwakeCtlEffect, ToolError> {
    let invalid = || ToolError::InvalidArgs {
        name: name.to_owned(),
    };
    match name {
        SOFTWAKE_STATUS_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::Status)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_LIST_MODELS_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::ListModels)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_LIST_VOICES_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::ListVoices)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_LIST_PROFILES_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::ListProfiles)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_SET_MODEL_TOOL => match args {
            [which, id] if matches!(which.as_str(), "ai" | "voice") && !id.trim().is_empty() => {
                Ok(SoftwakeCtlEffect::SetModel {
                    which: which.clone(),
                    id: id.clone(),
                })
            }
            _ => Err(invalid()),
        },
        SOFTWAKE_SET_VOICE_TOOL => match args {
            [id] if !id.trim().is_empty() => Ok(SoftwakeCtlEffect::SetVoice { id: id.clone() }),
            _ => Err(invalid()),
        },
        SOFTWAKE_SET_PROFILE_TOOL => match args {
            [name] if !name.trim().is_empty() => {
                Ok(SoftwakeCtlEffect::SetProfile { name: name.clone() })
            }
            _ => Err(invalid()),
        },
        SOFTWAKE_SLEEP_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::Sleep)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_HIBERNATE_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::Hibernate)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_RESUME_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::Resume)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_NEW_SESSION_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::NewSession)
            } else {
                Err(invalid())
            }
        }
        SOFTWAKE_REFRESH_TOOL => {
            if args.is_empty() {
                Ok(SoftwakeCtlEffect::Refresh)
            } else {
                Err(invalid())
            }
        }
        _ => Err(ToolError::Unknown {
            name: name.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_set_model_and_refresh() {
        assert_eq!(
            parse_softwake_ctl(SOFTWAKE_SET_MODEL_TOOL, &["ai".into(), "grok-4".into()]).unwrap(),
            SoftwakeCtlEffect::SetModel {
                which: "ai".into(),
                id: "grok-4".into()
            }
        );
        assert_eq!(
            parse_softwake_ctl(SOFTWAKE_REFRESH_TOOL, &[]).unwrap(),
            SoftwakeCtlEffect::Refresh
        );
        assert!(is_softwake_ctl(SOFTWAKE_SLEEP_TOOL));
        assert!(softwake_ctl_is_list(SOFTWAKE_STATUS_TOOL));
        assert!(!softwake_ctl_is_list(SOFTWAKE_SLEEP_TOOL));
    }
}
