//! Awake operator slash / clear-typed commands (ADR 0028).
//!
//! Context commands (`/clear` `/halve` `/compact`) stay in [`crate::chat`];
//! this module owns the broader operator surface and disk helpers.

use softwake_providers::{
    FileProviderSettings, REASONING_EFFORT_MODES, normalize_reasoning_effort,
    reasoning_effort_label, resolve_providers_file, resolve_tts_voice, tts_voice_roster,
};
use std::path::PathBuf;

use softwake_soul::{list_profiles, load_app_config, resolve_config_dir, set_active_profile};

/// Operator command parsed from an awake ask line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlashCommand {
    Help,
    Status,
    Clear,
    Compact,
    Halve,
    ModelList,
    ModelAi(String),
    ModelVoice(String),
    VoiceList,
    VoiceSet(String),
    NewSession,
    ProfileList,
    ProfileSet(String),
    Sleep,
    Hibernate,
    Resume,
    /// Full live-config refresh (profile soul + chat + MCP). ADR-0031.
    Refresh,
    /// `/reasoning` or `/reasoning list`.
    ReasoningList,
    /// `/reasoning <mode>`.
    ReasoningSet(String),
    /// Leading `/` with an unknown verb — show a short hint, do not chat.
    Unknown(String),
}

/// Parse slash / clear-typed operator commands.
///
/// Returns `None` for ordinary chat text (including unknown bare words).
/// A leading `/` with an unrecognized verb returns [`SlashCommand::Unknown`].
#[must_use]
#[allow(
    clippy::unnested_or_patterns,
    reason = "slash verbs differ by arity; nested | on slices is harder to read"
)]
pub(crate) fn parse_slash_command(text: &str) -> Option<SlashCommand> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Reuse ADR 0027 context parser first.
    if let Some(ctx) = crate::chat::parse_context_command(trimmed) {
        return Some(match ctx {
            crate::chat::ContextCommand::Clear => SlashCommand::Clear,
            crate::chat::ContextCommand::Compact => SlashCommand::Compact,
            crate::chat::ContextCommand::Halve => SlashCommand::Halve,
        });
    }
    let lower = trimmed.to_ascii_lowercase();
    let (had_slash, body) = match lower.strip_prefix('/') {
        Some(rest) => (true, rest.trim()),
        None => (false, lower.as_str()),
    };
    if body.is_empty() {
        return had_slash.then_some(SlashCommand::Help);
    }
    let mut parts = body.split_whitespace();
    let verb = parts.next().unwrap_or("");
    let rest: Vec<&str> = parts.collect();

    match (verb, rest.as_slice(), had_slash) {
        ("help", [], _) | ("list", ["commands"], false) => Some(SlashCommand::Help),
        ("status", [], _) => Some(SlashCommand::Status),
        ("model", [], _) => Some(SlashCommand::ModelList),
        ("model", ["ai"], true) | ("model", ["voice"], true) => {
            Some(SlashCommand::Unknown(body.to_owned()))
        }
        ("model", ["ai", name, ..], true) => Some(SlashCommand::ModelAi((*name).to_owned())),
        ("model", ["voice", name, ..], true) => Some(SlashCommand::ModelVoice((*name).to_owned())),
        ("voice", [], true) | ("voice", ["list"], true) => Some(SlashCommand::VoiceList),
        ("voice", [name, ..], true) => Some(SlashCommand::VoiceSet((*name).to_owned())),
        ("new", [], _) | ("new", ["session"], _) | ("fresh", ["session"], _) => {
            Some(SlashCommand::NewSession)
        }
        ("profile", [], _) => Some(SlashCommand::ProfileList),
        ("profile", [name, ..], _) => Some(SlashCommand::ProfileSet((*name).to_owned())),
        ("sleep", [], true) => Some(SlashCommand::Sleep),
        ("hibernate", [], true) => Some(SlashCommand::Hibernate),
        ("resume", [], true) => Some(SlashCommand::Resume),
        ("refresh", [], _) => Some(SlashCommand::Refresh),
        ("reasoning", [], true) | ("reasoning", ["list"], true) => {
            Some(SlashCommand::ReasoningList)
        }
        ("reasoning", [mode, ..], true) => Some(SlashCommand::ReasoningSet((*mode).to_owned())),
        (_, _, true) => Some(SlashCommand::Unknown(body.to_owned())),
        _ => None,
    }
}

/// Help text for `/help`.
#[must_use]
pub(crate) fn help_text() -> String {
    "Commands: /help /status /clear /halve|/reduce /compact /model [ai|voice <id>] /voice [list|<id>] /reasoning [list|<mode>] /new /profile [<name>] /sleep /hibernate /resume /refresh"
        .to_owned()
}

/// Load providers.json for slash model/voice commands.
pub(crate) fn open_provider_store() -> Result<FileProviderSettings, String> {
    let path = resolve_providers_file().map_err(|e| e.to_string())?;
    FileProviderSettings::new(path).map_err(|e| e.to_string())
}

/// Format `/model` list reply.
pub(crate) fn format_model_list() -> Result<String, String> {
    let store = open_provider_store()?;
    let settings = store.load().map_err(|e| e.to_string())?;
    let provider = settings.selected_provider;
    let models = settings.models_for(provider);
    let current = settings.selected_model.trim();
    let current = if current.is_empty() {
        "(none)"
    } else {
        current
    };
    if models.is_empty() {
        return Ok(format!(
            "Chat model: {current}. No Test catalog yet — run Test in Settings → Providers."
        ));
    }
    let list = models.join(", ");
    Ok(format!("Chat model: {current}. Catalog: {list}"))
}

/// Set chat model by id (case-insensitive match against Test catalog).
pub(crate) fn set_chat_model(name: &str) -> Result<String, String> {
    let store = open_provider_store()?;
    let mut settings = store.load().map_err(|e| e.to_string())?;
    let needle = name.trim();
    if needle.is_empty() {
        return Err("usage: /model ai <id>".to_owned());
    }
    let models = settings.models_for(settings.selected_provider);
    let Some(matched) = models
        .iter()
        .find(|id| id.eq_ignore_ascii_case(needle))
        .cloned()
    else {
        return Err(format!(
            "model `{needle}` is not in the Test catalog; run Test in Settings → Providers"
        ));
    };
    settings.selected_model.clone_from(&matched);
    store.save(&settings).map_err(|e| e.to_string())?;
    Ok(format!("Chat model set to {matched}"))
}

/// Set STT / voice model id.
pub(crate) fn set_voice_model(name: &str) -> Result<String, String> {
    let store = open_provider_store()?;
    let mut settings = store.load().map_err(|e| e.to_string())?;
    let needle = name.trim();
    if needle.is_empty() {
        return Err("usage: /model voice <id>".to_owned());
    }
    let models = settings.voice_models_for(settings.selected_provider);
    let Some(matched) = models
        .iter()
        .find(|id| id.eq_ignore_ascii_case(needle))
        .cloned()
    else {
        return Err(format!(
            "voice model `{needle}` is not in the Test catalog; run Test in Settings → Providers"
        ));
    };
    settings.selected_voice_model.clone_from(&matched);
    store.save(&settings).map_err(|e| e.to_string())?;
    Ok(format!("Voice/STT model set to {matched}"))
}

/// Format TTS voice list.
pub(crate) fn format_voice_list() -> Result<String, String> {
    let store = open_provider_store()?;
    let settings = store.load().map_err(|e| e.to_string())?;
    let provider = settings.selected_provider;
    let roster = tts_voice_roster(provider);
    let current = settings.selected_tts_voice.trim();
    let current = if current.is_empty() {
        resolve_tts_voice(provider, "").unwrap_or("(none)")
    } else {
        current
    };
    if roster.is_empty() {
        return Ok(format!(
            "TTS voice: {current}. Speech playback needs an xAI provider."
        ));
    }
    let list = roster.join(", ");
    Ok(format!("TTS voice: {current}. Roster: {list}"))
}

/// Set TTS voice id.
pub(crate) fn set_tts_voice(name: &str) -> Result<String, String> {
    let store = open_provider_store()?;
    let mut settings = store.load().map_err(|e| e.to_string())?;
    let Some(voice) = resolve_tts_voice(settings.selected_provider, name) else {
        return Err(format!(
            "`{name}` is not a built-in TTS voice for the selected provider"
        ));
    };
    voice.clone_into(&mut settings.selected_tts_voice);
    store.save(&settings).map_err(|e| e.to_string())?;
    Ok(format!("TTS voice set to {voice}"))
}

fn config_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|e| e.to_string())
}

/// Format `/reasoning` list reply.
pub(crate) fn format_reasoning_list() -> Result<String, String> {
    let store = open_provider_store()?;
    let settings = store.load().map_err(|e| e.to_string())?;
    let current = reasoning_effort_label(&settings.reasoning_effort);
    let modes = REASONING_EFFORT_MODES.join(", ");
    Ok(format!(
        "Reasoning effort: {current}. Modes: {modes} (or default to omit)."
    ))
}

/// Set chat `reasoning_effort` (or clear with default/off/none).
pub(crate) fn set_reasoning_effort(mode: &str) -> Result<String, String> {
    let store = open_provider_store()?;
    let mut settings = store.load().map_err(|e| e.to_string())?;
    let normalized = normalize_reasoning_effort(mode)?;
    settings.set_reasoning_effort(normalized.clone());
    store.save(&settings).map_err(|e| e.to_string())?;
    if normalized.is_empty() {
        Ok("Reasoning effort cleared (provider default)".to_owned())
    } else {
        Ok(format!("Reasoning effort set to {normalized}"))
    }
}

/// List profiles for `/profile`.
pub(crate) fn format_profile_list() -> Result<String, String> {
    let config = config_dir()?;
    let active = load_app_config(&config)
        .map(|c| c.active_profile)
        .unwrap_or_default();
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    if profiles.is_empty() {
        return Ok("No profiles found.".to_owned());
    }
    let lines: Vec<String> = profiles
        .iter()
        .map(|p| {
            let mark = if p.id == active { " *" } else { "" };
            let name = if p.name.trim().is_empty() {
                "(unnamed)"
            } else {
                p.name.trim()
            };
            format!("{name} [{id}]{mark}", id = p.id)
        })
        .collect();
    Ok(format!("Profiles: {}", lines.join(", ")))
}

/// Resolve a profile name or id to an id.
pub(crate) fn resolve_profile_id(name_or_id: &str) -> Result<String, String> {
    let needle = name_or_id.trim();
    if needle.is_empty() {
        return Err("usage: /profile <name-or-id>".to_owned());
    }
    let config = config_dir()?;
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    if let Some(exact) = profiles.iter().find(|p| p.id == needle) {
        return Ok(exact.id.clone());
    }
    let matches: Vec<_> = profiles
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(needle) || p.id.eq_ignore_ascii_case(needle))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!("unknown profile `{needle}`")),
        many => Err(format!(
            "ambiguous profile `{needle}` ({})",
            many.iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Set active profile on disk. Caller retargets the loaded soul dir.
pub(crate) fn activate_profile(name_or_id: &str) -> Result<String, String> {
    let id = resolve_profile_id(name_or_id)?;
    let config = config_dir()?;
    set_active_profile(&config, &id).map_err(|e| e.to_string())?;
    let profiles = list_profiles(&config).map_err(|e| e.to_string())?;
    let name = profiles
        .iter()
        .find(|p| p.id == id)
        .map_or("", |p| p.name.as_str());
    let label = if name.trim().is_empty() {
        id.as_str()
    } else {
        name.trim()
    };
    Ok(format!("Active profile: {label} [{id}]"))
}

/// Whether the process soul path is locked by flag or `SOFTWAKE_SOUL_DIR`.
#[must_use]
pub(crate) fn soul_path_env_locked() -> bool {
    std::env::var_os("SOFTWAKE_SOUL_DIR").is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_help_status_and_context() {
        assert_eq!(parse_slash_command("/help"), Some(SlashCommand::Help));
        assert_eq!(parse_slash_command("help"), Some(SlashCommand::Help));
        assert_eq!(parse_slash_command("/status"), Some(SlashCommand::Status));
        assert_eq!(parse_slash_command("/clear"), Some(SlashCommand::Clear));
        assert_eq!(
            parse_slash_command("clear context"),
            Some(SlashCommand::Clear)
        );
        assert_eq!(parse_slash_command("/halve"), Some(SlashCommand::Halve));
        assert_eq!(parse_slash_command("/compact"), Some(SlashCommand::Compact));
    }

    #[test]
    fn parses_model_voice_profile_new_mode() {
        assert_eq!(parse_slash_command("/model"), Some(SlashCommand::ModelList));
        assert_eq!(
            parse_slash_command("/model ai grok-4"),
            Some(SlashCommand::ModelAi("grok-4".into()))
        );
        assert_eq!(
            parse_slash_command("/model voice whisper-1"),
            Some(SlashCommand::ModelVoice("whisper-1".into()))
        );
        assert_eq!(parse_slash_command("/voice"), Some(SlashCommand::VoiceList));
        assert_eq!(
            parse_slash_command("/voice eve"),
            Some(SlashCommand::VoiceSet("eve".into()))
        );
        assert_eq!(parse_slash_command("/new"), Some(SlashCommand::NewSession));
        assert_eq!(
            parse_slash_command("fresh session"),
            Some(SlashCommand::NewSession)
        );
        assert_eq!(
            parse_slash_command("/profile"),
            Some(SlashCommand::ProfileList)
        );
        assert_eq!(
            parse_slash_command("/profile sally"),
            Some(SlashCommand::ProfileSet("sally".into()))
        );
        assert_eq!(parse_slash_command("/sleep"), Some(SlashCommand::Sleep));
        assert_eq!(
            parse_slash_command("/hibernate"),
            Some(SlashCommand::Hibernate)
        );
        assert_eq!(parse_slash_command("/resume"), Some(SlashCommand::Resume));
        assert_eq!(parse_slash_command("/refresh"), Some(SlashCommand::Refresh));
        assert_eq!(parse_slash_command("refresh"), Some(SlashCommand::Refresh));
        assert_eq!(
            parse_slash_command("/reasoning"),
            Some(SlashCommand::ReasoningList)
        );
        assert_eq!(
            parse_slash_command("/reasoning list"),
            Some(SlashCommand::ReasoningList)
        );
        assert_eq!(
            parse_slash_command("/reasoning xhigh"),
            Some(SlashCommand::ReasoningSet("xhigh".into()))
        );
    }

    #[test]
    fn unknown_slash_is_hint_not_chat() {
        assert_eq!(
            parse_slash_command("/nope"),
            Some(SlashCommand::Unknown("nope".into()))
        );
        assert_eq!(parse_slash_command("hello there"), None);
        // Bare "sleep" is NL mode confirm, not a slash command.
        assert_eq!(parse_slash_command("sleep"), None);
    }
}
