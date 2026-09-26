//! One-shot state announcements.
//!
//! A transition speaks a short line in the active profile voice. The line is
//! a single completion: soul instructions as system, one fixed instruction as
//! the only user turn, then the existing TTS path. It is not [`crate::chat::perform_ask`]
//! and it is not appended to the awake session.
//!
//! See [ADR 0022](../../docs/ADR-0022-voice-modes.md).

#[cfg(not(test))]
use std::sync::{Mutex, OnceLock};
#[cfg(not(test))]
use std::thread;

use softwake_providers::ChatMessage;
#[cfg(not(test))]
use softwake_providers::{family_speaks_xai, resolve_tts_voice};
use softwake_state::VoiceState;

/// Awake announcement instruction. Spoken text comes back from the model.
pub(crate) const AWAKE_PROMPT: &str =
    "Tell the user that you are now awake and listening. Be very concise. Keep it short.";

/// Sleep announcement instruction (listening for a wake phrase).
pub(crate) const SLEEP_PROMPT: &str = "Tell the user that you are now asleep and only listening for a wake phrase. Be very concise. Keep it short.";

/// Hibernate / deep-sleep announcement instruction.
pub(crate) const HIBERNATE_PROMPT: &str = "Tell the user that you are going into deep sleep and will not listen until they resume from the UI. Be very concise. Keep it short.";

/// Fixed speech when the one-shot completion cannot run.
#[cfg(not(test))]
const AWAKE_FALLBACK: &str = "I'm awake.";
/// Fixed speech when the one-shot completion cannot run.
#[cfg(not(test))]
const SLEEP_FALLBACK: &str = "Listening for wake.";
/// Fixed speech when the one-shot completion cannot run.
#[cfg(not(test))]
const HIBERNATE_FALLBACK: &str = "Deep sleep.";

/// Instruction for the state just entered.
#[must_use]
pub(crate) const fn prompt_for(state: VoiceState) -> &'static str {
    match state {
        VoiceState::Awake => AWAKE_PROMPT,
        VoiceState::Sleep => SLEEP_PROMPT,
        VoiceState::Hibernate => HIBERNATE_PROMPT,
    }
}

/// System text plus exactly one user turn. No session history.
#[must_use]
pub(crate) fn oneshot_messages(system: &str, prompt: &str) -> (String, Vec<ChatMessage>) {
    (system.to_owned(), vec![ChatMessage::user(prompt)])
}

#[cfg(not(test))]
fn fallback_line(state: VoiceState) -> &'static str {
    match state {
        VoiceState::Awake => AWAKE_FALLBACK,
        VoiceState::Sleep => SLEEP_FALLBACK,
        VoiceState::Hibernate => HIBERNATE_FALLBACK,
    }
}

#[cfg(not(test))]
fn speak_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Speak the state line off the runtime lock.
///
/// Tests record the prompt instead of calling this. A missing provider, a
/// non-xAI family, or no TTS voice logs at `verbosity >= 1` and returns.
/// The bearer is never logged.
#[cfg(not(test))]
pub(crate) fn spawn_announcement(
    system: String,
    state: VoiceState,
    profile: String,
    verbosity: u8,
) {
    let prompt = prompt_for(state).to_owned();
    let _ = thread::Builder::new()
        .name("softwake-state-voice".to_owned())
        .spawn(move || {
            let _guard = speak_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            speak_now(&system, state, &prompt, &profile, verbosity);
        });
}

#[cfg(not(test))]
fn speak_now(system: &str, state: VoiceState, prompt: &str, profile: &str, verbosity: u8) {
    let ready = match crate::chat::load_disk_chat() {
        Ok(ready) => ready,
        Err(message) => {
            log_skip(
                verbosity,
                profile,
                &format!("state voice skipped: {message}"),
            );
            return;
        }
    };
    if !family_speaks_xai(ready.prepared.provider) {
        log_skip(
            verbosity,
            profile,
            "state voice skipped: provider has no TTS",
        );
        return;
    }
    if resolve_tts_voice(ready.prepared.provider, ready.prepared_tts_voice()).is_none() {
        log_skip(verbosity, profile, "state voice skipped: no TTS voice");
        return;
    }
    let line = if system.trim().is_empty() {
        fallback_line(state).to_owned()
    } else {
        match crate::chat::complete_oneshot(system, prompt, |system, messages| {
            let turns: Vec<softwake_session::SessionMessage> = messages
                .iter()
                .map(|message| softwake_session::SessionMessage::user(message.content.clone()))
                .collect();
            crate::chat::finish_prepared_chat(&ready.prepared, &ready.bearer, system, &turns)
        }) {
            Ok(text) if !text.trim().is_empty() => text,
            _ => fallback_line(state).to_owned(),
        }
    };
    if let Err(message) = crate::talk::speak_reply(&ready, &line) {
        let hint = if message.contains("rejected the credentials") {
            format!(
                "state voice skipped: {message} Re-run Providers Test or re-sign in (xAI OAuth)."
            )
        } else if message.contains("Could not reach the voice service") {
            format!(
                "state voice skipped: {message} Check network, or re-run Providers Test if OAuth expired."
            )
        } else {
            format!("state voice skipped: {message}")
        };
        log_skip(verbosity, profile, &hint);
    }
}

#[cfg(not(test))]
fn log_skip(verbosity: u8, profile: &str, message: &str) {
    if verbosity >= 1 {
        eprintln!("softwaked: {profile} {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::{AWAKE_PROMPT, HIBERNATE_PROMPT, SLEEP_PROMPT, oneshot_messages, prompt_for};
    use softwake_providers::ChatRole;
    use softwake_state::VoiceState;

    #[test]
    fn prompts_match_the_three_states() {
        assert_eq!(prompt_for(VoiceState::Awake), AWAKE_PROMPT);
        assert_eq!(
            AWAKE_PROMPT,
            "Tell the user that you are now awake and listening. Be very concise. Keep it short."
        );
        assert_eq!(prompt_for(VoiceState::Sleep), SLEEP_PROMPT);
        assert_eq!(prompt_for(VoiceState::Hibernate), HIBERNATE_PROMPT);
        assert_ne!(SLEEP_PROMPT, AWAKE_PROMPT);
        assert_ne!(HIBERNATE_PROMPT, SLEEP_PROMPT);
    }

    #[test]
    fn oneshot_is_system_plus_one_user_turn() {
        let (system, messages) = oneshot_messages("soul pack", AWAKE_PROMPT);
        assert_eq!(system, "soul pack");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, ChatRole::User);
        assert_eq!(messages[0].content, AWAKE_PROMPT);
    }

    #[test]
    fn complete_oneshot_does_not_need_a_session() {
        let spoken = crate::chat::complete_oneshot("soul", SLEEP_PROMPT, |system, messages| {
            assert_eq!(system, "soul");
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].content, SLEEP_PROMPT);
            Ok("Listening.".to_owned())
        })
        .expect("stub");
        assert_eq!(spoken, "Listening.");
    }
}
