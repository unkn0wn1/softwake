//! Press-to-talk: buffer PCM, cloud STT, then the existing ask path.
//!
//! Cloud STT and TTS run only when this crate is built with `live-http`.
//! Default tests inject PCM and a transcript without a microphone or a socket.
//! See [ADR 0007](../../docs/ADR-0007-awake-stt-tts.md).

use softwake_providers::{ProviderId, family_speaks_xai, resolve_stt_model, resolve_tts_voice};

#[cfg(feature = "live-http")]
use softwake_providers::{stt_transcribe, tts_synthesize, wav_from_pcm16};
use softwake_voice::TalkBuffer;
#[cfg(feature = "live-http")]
use softwake_voice::{PlaybackMode, play_audio_with_interrupt};

use crate::chat::DiskChat;
#[cfg(not(feature = "live-http"))]
use crate::chat::LIVE_HTTP_DISABLED;

/// Shown when the held clip is shorter than the minimum.
pub(crate) const TALK_TOO_SHORT: &str = "hold the mic a little longer";

/// Shown when talk is used outside awake after the sleep-to-wake attempt.
pub(crate) fn talk_while(state: &str) -> String {
    format!("talk while {state} (speech acts only while awake)")
}

/// Hibernate refuses before any buffer work.
pub(crate) const TALK_HIBERNATING: &str =
    "Softwake is hibernating — leave hibernate from Settings (Resume) first";

/// Utterance captured while the mic button is held.
#[derive(Debug, Default)]
pub(crate) struct TalkSession {
    buffer: TalkBuffer,
}

impl TalkSession {
    pub(crate) fn is_armed(&self) -> bool {
        self.buffer.is_armed()
    }

    pub(crate) fn arm(&mut self) {
        self.buffer.arm();
    }

    pub(crate) fn clear(&mut self) {
        self.buffer.clear();
    }

    pub(crate) fn push(&mut self, samples: &[i16]) {
        self.buffer.push(samples);
    }

    /// Stop and return PCM when the clip is long enough.
    pub(crate) fn finish(&mut self) -> Result<Vec<i16>, String> {
        self.buffer
            .take_if_long_enough()
            .ok_or_else(|| TALK_TOO_SHORT.to_owned())
    }
}

/// Cloud STT of one WAV clip.
///
/// # Errors
///
/// The live-HTTP sentence when the feature is off, or a voice-service sentence.
pub(crate) fn transcribe_pcm(
    ready: &DiskChat,
    model: &str,
    samples: &[i16],
) -> Result<String, String> {
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (ready, model, samples);
        Err(LIVE_HTTP_DISABLED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let wav = wav_from_pcm16(samples);
        let transport = softwake_providers::live::LiveTransport::bounded(crate::chat::CHAT_TIMEOUT);
        stt_transcribe(
            &transport,
            &ready.prepared.api_base,
            &ready.bearer,
            model,
            &wav,
        )
        .map_err(|error| error.to_string())
    }
}

/// Non-S2S voice for one speaker.
///
/// A non-empty profile id that [`resolve_tts_voice`] accepts wins. Otherwise
/// the Settings id if it is accepted. Otherwise the provider default (`eve`
/// on xAI). An empty profile voice is unset, so it does not become eve before
/// Settings is tried. Non-xAI returns [`None`].
#[must_use]
pub(crate) fn resolve_spoken_voice(
    provider: ProviderId,
    profile_voice: &str,
    settings_voice: &str,
) -> Option<&'static str> {
    let profile = profile_voice.trim();
    if !profile.is_empty()
        && let Some(voice) = resolve_tts_voice(provider, profile)
    {
        return Some(voice);
    }
    let settings = settings_voice.trim();
    if !settings.is_empty()
        && let Some(voice) = resolve_tts_voice(provider, settings)
    {
        return Some(voice);
    }
    resolve_tts_voice(provider, "")
}

/// Speak `text` with the xAI TTS voice when the selected provider is xAI.
///
/// `profile_voice` is that speaker's `profile.json` `tts_voice`. Other families
/// return [`Ok`] (no TTS). A build without `live-http` returns [`Err`] so the
/// HUD can show why speech is silent. Playback errors are returned the same way.
///
/// # Errors
///
/// A voice, feature, or player sentence. The bearer is not included.
#[cfg_attr(test, allow(dead_code))] // called only from `#[cfg(not(test))]` speak path
pub(crate) fn speak_reply(ready: &DiskChat, text: &str, profile_voice: &str) -> Result<(), String> {
    speak_reply_with_interrupt(ready, text, profile_voice, true)
}

/// Speak `text`. When `interrupt` is false, do not kill a still-playing clip.
/// The caller must wait for idle first (early-TTS sentence queue, room lines).
#[cfg_attr(test, allow(dead_code))] // called from `#[cfg(not(test))]` announce path
pub(crate) fn speak_reply_with_interrupt(
    ready: &DiskChat,
    text: &str,
    profile_voice: &str,
    interrupt: bool,
) -> Result<(), String> {
    let provider = ready.prepared.provider;
    if !family_speaks_xai(provider) {
        return Ok(());
    }
    let Some(voice) = resolve_spoken_voice(provider, profile_voice, ready.prepared_tts_voice())
    else {
        return Ok(());
    };
    #[cfg(not(feature = "live-http"))]
    {
        let _ = (text, voice, interrupt);
        Err(LIVE_HTTP_DISABLED.to_owned())
    }
    #[cfg(feature = "live-http")]
    {
        let timeout = crate::playback_timeout::resolve_tts_playback_timeout();
        if speak_via_stream(
            &ready.prepared.api_base,
            &ready.bearer,
            text,
            voice,
            ready.tts_speed,
            timeout,
            interrupt,
        )
        .is_ok()
        {
            return Ok(());
        }
        let transport = softwake_providers::live::LiveTransport::bounded(crate::chat::CHAT_TIMEOUT);
        let audio = tts_synthesize(
            &transport,
            provider,
            &ready.prepared.api_base,
            &ready.bearer,
            text,
            voice,
            ready.tts_speed,
        )
        .map_err(|error| error.to_string())?;
        let mut record = None;
        play_audio_with_interrupt(
            PlaybackMode::Spawn,
            &audio,
            "mp3",
            timeout,
            &mut record,
            interrupt,
        )
    }
}

/// Prefer xAI streaming TTS into an MP3 stdin pipe (lower time-to-first-audio).
#[cfg(feature = "live-http")]
fn speak_via_stream(
    api_base: &str,
    bearer: &str,
    text: &str,
    voice: &str,
    speed: f64,
    timeout: std::time::Duration,
    interrupt: bool,
) -> Result<(), String> {
    use softwake_voice::{Mp3PipePlayer, play_mp3_pipe_start};

    let mut player: Option<Mp3PipePlayer> = None;
    let mut player_error: Option<String> = None;
    let stream = softwake_providers::tts_synthesize_streaming(
        api_base,
        bearer,
        text,
        voice,
        speed,
        |chunk| {
            if player.is_none() {
                match play_mp3_pipe_start(timeout, interrupt) {
                    Ok(started) => player = Some(started),
                    Err(message) => {
                        player_error = Some(message);
                        return Err(softwake_providers::VoiceHttpError::NoAudio);
                    }
                }
            }
            if let Some(active) = player.as_mut() {
                if let Err(message) = active.write_chunk(chunk) {
                    player_error = Some(message);
                    return Err(softwake_providers::VoiceHttpError::NoAudio);
                }
            }
            Ok(())
        },
    );
    match stream {
        Ok(()) => {
            if let Some(active) = player.take() {
                active.finish();
                Ok(())
            } else if let Some(message) = player_error {
                Err(message)
            } else {
                Err("The voice service returned no audio.".to_owned())
            }
        }
        Err(error) => {
            if let Some(active) = player.take() {
                active.abort();
            }
            Err(player_error.unwrap_or_else(|| error.to_string()))
        }
    }
}

/// How often a room line rechecks the speech stamp while waiting for idle.
#[cfg(not(test))]
const ROOM_LINE_IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(40);

/// Wait until playback is idle, the 120s budget ends, or `speech_gen` is stale.
///
/// Returns false when the stamp is stale so the caller does not sit on the
/// full idle timeout and does not start playback. Returns true when idle or
/// when the budget elapsed with the stamp still current.
#[cfg(not(test))]
fn wait_room_line_until_idle(speech_gen: u64, timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if !crate::team::room_speech_allows(speech_gen) {
            return false;
        }
        if softwake_voice::wait_for_playback_idle(ROOM_LINE_IDLE_POLL) {
            return true;
        }
        if start.elapsed() >= timeout {
            return true;
        }
    }
}

#[cfg(not(test))]
fn room_member_playback_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    let lock = LOCK.get_or_init(|| std::sync::Mutex::new(()));
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Speak one room-member line in that member's voice, after the log write.
///
/// The fan-out has already enqueued this clip. Wait until it is the oldest
/// waiting line, then play it with `interrupt` false. One playback lock keeps
/// a single TTS player. This does not open a Voice Agent session.
///
/// `speech_gen` is the stamp taken when the operator line was committed. When
/// barge-in or Voice Agent speech-started has cleared the room speech queue,
/// this returns without playing, including while it would otherwise wait up to
/// 120s for idle.
#[cfg(not(test))]
pub(crate) fn speak_room_member_line(profile_voice: &str, text: &str, speech_gen: u64) {
    if !crate::team::room_speech_allows(speech_gen) {
        return;
    }
    let clip = crate::team::RoomSpeechClip {
        text: text.to_owned(),
        voice: profile_voice.to_owned(),
    };
    let idle = std::time::Duration::from_secs(120);
    let start = std::time::Instant::now();
    loop {
        if !crate::team::room_speech_allows(speech_gen) {
            return;
        }
        if !crate::team::room_speech_waiting_has(&clip) {
            return;
        }
        if start.elapsed() >= idle {
            return;
        }
        if !crate::team::room_speech_front_is(&clip) {
            std::thread::sleep(ROOM_LINE_IDLE_POLL);
            continue;
        }
        let _playback = room_member_playback_lock();
        if !crate::team::room_speech_allows(speech_gen) || !crate::team::room_speech_front_is(&clip)
        {
            continue;
        }
        if !wait_room_line_until_idle(speech_gen, idle) {
            return;
        }
        if !crate::team::claim_waiting_room_audio(&clip, speech_gen) {
            return;
        }
        if !crate::team::room_speech_allows(speech_gen) {
            return;
        }
        let Ok(ready) = crate::chat::load_disk_chat() else {
            return;
        };
        let _ = speak_reply_with_interrupt(&ready, text, profile_voice, false);
        if !crate::team::room_speech_allows(speech_gen) {
            softwake_voice::interrupt_playback();
            return;
        }
        let _ = wait_room_line_until_idle(speech_gen, idle);
        return;
    }
}

pub(crate) fn stt_model_for(provider: ProviderId, selected: &str) -> Result<String, String> {
    let model = resolve_stt_model(provider, selected);
    if model.is_empty() {
        return Err(
            "No speech-to-text model is selected. Choose a voice model in Settings after Test."
                .to_owned(),
        );
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::{TALK_TOO_SHORT, TalkSession, resolve_spoken_voice};
    use softwake_providers::ProviderId;
    use softwake_voice::TALK_MIN_SAMPLES;

    #[test]
    fn spoken_voice_prefers_profile_then_settings_then_eve() {
        let xai = ProviderId::XaiKey;
        assert_eq!(resolve_spoken_voice(xai, "ara", "eve"), Some("ara"));
        assert_eq!(resolve_spoken_voice(xai, "rex", "eve"), Some("rex"));
        assert_eq!(resolve_spoken_voice(xai, " Ara ", "eve"), Some("ara"));
        assert_eq!(resolve_spoken_voice(xai, "", "eve"), Some("eve"));
        assert_eq!(resolve_spoken_voice(xai, "   ", "rex"), Some("rex"));
        assert_eq!(resolve_spoken_voice(xai, "nope", "rex"), Some("rex"));
        assert_eq!(resolve_spoken_voice(xai, "", ""), Some("eve"));
        assert_eq!(resolve_spoken_voice(xai, "nope", "also-nope"), Some("eve"));
        assert_eq!(resolve_spoken_voice(ProviderId::Openai, "ara", "eve"), None);
    }

    #[test]
    fn room_line_voices_follow_each_member() {
        let settings = "eve";
        let voices = ["ara", "rex", ""]
            .map(|profile| resolve_spoken_voice(ProviderId::XaiKey, profile, settings).unwrap());
        assert_eq!(voices, ["ara", "rex", "eve"]);
    }

    #[test]
    fn short_hold_is_refused_and_a_long_hold_returns_pcm() {
        let mut talk = TalkSession::default();
        talk.arm();
        talk.push(&[0; 10]);
        assert_eq!(talk.finish().expect_err("short"), TALK_TOO_SHORT);
        talk.arm();
        talk.push(&vec![1; TALK_MIN_SAMPLES]);
        assert_eq!(talk.finish().expect("pcm").len(), TALK_MIN_SAMPLES);
    }

    #[test]
    fn clear_drops_an_armed_buffer() {
        let mut talk = TalkSession::default();
        talk.arm();
        talk.push(&[1, 2, 3]);
        assert!(talk.is_armed());
        talk.clear();
        assert!(!talk.is_armed());
        assert_eq!(talk.finish().expect_err("empty"), TALK_TOO_SHORT);
    }
}
