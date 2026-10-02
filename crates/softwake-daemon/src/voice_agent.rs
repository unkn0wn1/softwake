//! Opt-in xAI Voice Agent continuous speech-to-speech (ADR-0050).
//!
//! When enabled, awake mic PCM is streamed to the realtime WebSocket and
//! assistant PCM plays locally. Softwake Hands / tool-loop / full soul session
//! stay on the text ask path. Feature `live-http` required for the socket.

#![allow(
    dead_code,
    reason = "Voice Agent bridge is exercised under live-http; default CI builds keep the types"
)]
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;

#[cfg(feature = "live-http")]
use std::sync::mpsc;
#[cfg(feature = "live-http")]
use std::thread;
#[cfg(feature = "live-http")]
use std::time::Duration;

use softwake_soul::{load_app_config, resolve_config_dir};

/// Env override (wins over softwake.json when set).
pub(crate) const VOICE_AGENT_S2S_ENV: &str = "SOFTWAKE_VOICE_AGENT_S2S";

/// Events the background session posts back to the runtime.
#[derive(Debug)]
pub(crate) enum BridgeEvent {
    TranscriptDelta(String),
    TranscriptDone(String),
    AudioDelta(Vec<u8>),
    SpeechStarted,
    Error(String),
    /// Background thread exited (socket closed or fatal).
    Stopped,
}

enum BridgeCmd {
    Pcm(Vec<i16>),
    Cancel,
    Stop,
}

/// Background Voice Agent session owned by [`Runtime`].
pub(crate) struct VoiceAgentBridge {
    cmd_tx: Sender<BridgeCmd>,
    event_rx: Receiver<BridgeEvent>,
    join: Option<JoinHandle<()>>,
    /// Accumulated assistant transcript for the current utterance.
    pub(crate) live_transcript: String,
    player: Option<softwake_voice::PcmPipePlayer>,
}

impl VoiceAgentBridge {
    /// Spawn the realtime worker. Caller supplies credentials + instructions.
    #[cfg(feature = "live-http")]
    pub(crate) fn start(
        api_base: String,
        bearer: String,
        voice: String,
        instructions: String,
    ) -> Result<Self, String> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<BridgeCmd>();
        let (event_tx, event_rx) = mpsc::channel::<BridgeEvent>();
        let join = thread::Builder::new()
            .name("softwake-voice-agent".to_owned())
            .spawn(move || {
                worker_loop(api_base, bearer, voice, instructions, cmd_rx, event_tx);
            })
            .map_err(|error| format!("could not start voice agent thread: {error}"))?;
        Ok(Self {
            cmd_tx,
            event_rx,
            join: Some(join),
            live_transcript: String::new(),
            player: None,
        })
    }

    #[cfg(not(feature = "live-http"))]
    pub(crate) fn start(
        _api_base: String,
        _bearer: String,
        _voice: String,
        _instructions: String,
    ) -> Result<Self, String> {
        Err(crate::chat::LIVE_HTTP_DISABLED.to_owned())
    }

    /// Queue one capture frame (non-blocking; drops when the worker is slow).
    pub(crate) fn push_pcm(&self, samples: &[i16]) {
        if samples.is_empty() {
            return;
        }
        let _ = self.cmd_tx.send(BridgeCmd::Pcm(samples.to_vec()));
    }

    /// Cancel in-flight assistant audio on the server and locally.
    pub(crate) fn cancel(&mut self) {
        let _ = self.cmd_tx.send(BridgeCmd::Cancel);
        if let Some(player) = self.player.take() {
            player.abort();
        }
        softwake_voice::interrupt_playback();
        softwake_voice::force_clear_input_mute();
        self.live_transcript.clear();
    }

    /// Drain server events; play PCM; update transcript. Returns status hint.
    pub(crate) fn pump(&mut self) -> Option<String> {
        let mut status = None;
        loop {
            match self.event_rx.try_recv() {
                Ok(BridgeEvent::AudioDelta(bytes)) => {
                    if self.player.is_none() {
                        match softwake_voice::play_pcm_pipe_start(
                            softwake_providers::VOICE_AGENT_OUTPUT_RATE_HZ,
                            softwake_voice::PLAYBACK_TIMEOUT,
                            false, // do not kill unrelated players mid-stream chunk
                            false, // keep mic open for server_vad barge
                        ) {
                            Ok(player) => self.player = Some(player),
                            Err(error) => status = Some(error),
                        }
                    }
                    if let Some(player) = self.player.as_mut() {
                        if let Err(error) = player.write_chunk(&bytes) {
                            status = Some(error);
                            if let Some(dead) = self.player.take() {
                                dead.abort();
                            }
                        }
                    }
                }
                Ok(BridgeEvent::TranscriptDelta(delta)) => {
                    self.live_transcript.push_str(&delta);
                    status = Some(self.live_transcript.clone());
                }
                Ok(BridgeEvent::TranscriptDone(text)) => {
                    if !text.is_empty() {
                        self.live_transcript = text;
                    }
                    status = Some(self.live_transcript.clone());
                    if let Some(player) = self.player.take() {
                        player.finish();
                    }
                }
                Ok(BridgeEvent::SpeechStarted) => {
                    // Server VAD barge: cut local playback immediately.
                    if let Some(player) = self.player.take() {
                        player.abort();
                    } else {
                        softwake_voice::interrupt_playback();
                    }
                    self.live_transcript.clear();
                    let _ = self.cmd_tx.send(BridgeCmd::Cancel);
                    status = Some("listening…".to_owned());
                }
                Ok(BridgeEvent::Error(message)) => {
                    status = Some(format!("Voice Agent: {message}"));
                    if let Some(player) = self.player.take() {
                        player.abort();
                    }
                }
                Ok(BridgeEvent::Stopped) => {
                    if let Some(player) = self.player.take() {
                        player.abort();
                    }
                    status = Some("Voice Agent session ended".to_owned());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if let Some(player) = self.player.take() {
                        player.abort();
                    }
                    status = Some("Voice Agent session ended".to_owned());
                    break;
                }
            }
        }
        status
    }

    /// True while the worker thread is still running.
    pub(crate) fn alive(&self) -> bool {
        self.join.as_ref().is_some_and(|join| !join.is_finished())
    }

    /// Shut down the worker and local player.
    pub(crate) fn stop(mut self) {
        let _ = self.cmd_tx.send(BridgeCmd::Stop);
        if let Some(player) = self.player.take() {
            player.abort();
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for VoiceAgentBridge {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(BridgeCmd::Stop);
        if let Some(player) = self.player.take() {
            player.abort();
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Resolve whether Voice Agent S2S mode is on (env wins over softwake.json).
#[must_use]
pub(crate) fn resolve_voice_agent_s2s_enabled() -> bool {
    resolve_voice_agent_s2s_enabled_from(
        std::env::var(VOICE_AGENT_S2S_ENV).ok().as_deref(),
        load_file_flag(),
    )
}

fn load_file_flag() -> bool {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let Ok(dir) = resolve_config_dir(xdg.as_deref(), home.as_deref()) else {
        return false;
    };
    load_app_config(&dir).is_ok_and(|app| app.voice_agent_s2s)
}

/// Same precedence as [`resolve_voice_agent_s2s_enabled`], with inputs injected.
#[must_use]
pub(crate) fn resolve_voice_agent_s2s_enabled_from(
    env_raw: Option<&str>,
    file_enabled: bool,
) -> bool {
    if let Some(raw) = env_raw {
        let lower = raw.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return file_enabled;
        }
        return matches!(lower.as_str(), "1" | "true" | "yes" | "on");
    }
    file_enabled
}

/// Build truncated instructions from the applied soul pack text.
#[must_use]
pub(crate) fn instructions_from_soul(applied: Option<&str>, agent_name: &str) -> String {
    let name = if agent_name.trim().is_empty() {
        "Softwake"
    } else {
        agent_name.trim()
    };
    let header = format!(
        "You are {name}, a desktop voice companion. Speak briefly and naturally. Softwake Hands tools are not available on this voice path — answer from knowledge and web search only.\n\n"
    );
    let body = applied.unwrap_or("Be helpful, clear, and kind.");
    softwake_providers::clip_voice_agent_instructions(
        &format!("{header}{body}"),
        softwake_providers::VOICE_AGENT_INSTRUCTIONS_MAX_CHARS,
    )
}

#[cfg(feature = "live-http")]
fn worker_loop(
    api_base: String,
    bearer: String,
    voice: String,
    instructions: String,
    cmd_rx: Receiver<BridgeCmd>,
    event_tx: Sender<BridgeEvent>,
) {
    use softwake_providers::{VOICE_AGENT_MODEL_DEFAULT, VoiceAgentEvent, VoiceAgentSession};

    let mut session =
        match VoiceAgentSession::connect(&api_base, &bearer, VOICE_AGENT_MODEL_DEFAULT) {
            Ok(session) => session,
            Err(_) => {
                let _ = event_tx.send(BridgeEvent::Error(
                    "could not connect to Voice Agent realtime".to_owned(),
                ));
                let _ = event_tx.send(BridgeEvent::Stopped);
                return;
            }
        };
    if session
        .send_session_update(&voice, &instructions, true)
        .is_err()
    {
        let _ = event_tx.send(BridgeEvent::Error(
            "Voice Agent session.update failed".to_owned(),
        ));
        let _ = event_tx.send(BridgeEvent::Stopped);
        return;
    }

    loop {
        // Pump outbound PCM / control first.
        loop {
            match cmd_rx.try_recv() {
                Ok(BridgeCmd::Pcm(samples)) => {
                    if session.append_pcm16(&samples).is_err() {
                        let _ = event_tx.send(BridgeEvent::Error(
                            "Voice Agent audio upload failed".to_owned(),
                        ));
                        let _ = event_tx.send(BridgeEvent::Stopped);
                        session.close();
                        return;
                    }
                }
                Ok(BridgeCmd::Cancel) => {
                    session.cancel_response();
                }
                Ok(BridgeCmd::Stop) => {
                    session.cancel_response();
                    session.close();
                    let _ = event_tx.send(BridgeEvent::Stopped);
                    return;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    session.close();
                    let _ = event_tx.send(BridgeEvent::Stopped);
                    return;
                }
            }
        }

        match session.try_read_event() {
            Ok(None) => {
                // Idle briefly so we do not spin.
                thread::sleep(Duration::from_millis(5));
            }
            Ok(Some(VoiceAgentEvent::OutputAudioDelta(bytes))) => {
                let _ = event_tx.send(BridgeEvent::AudioDelta(bytes));
            }
            Ok(Some(VoiceAgentEvent::TranscriptDelta(text))) => {
                let _ = event_tx.send(BridgeEvent::TranscriptDelta(text));
            }
            Ok(Some(VoiceAgentEvent::TranscriptDone(text))) => {
                let _ = event_tx.send(BridgeEvent::TranscriptDone(text));
            }
            Ok(Some(VoiceAgentEvent::SpeechStarted)) => {
                let _ = event_tx.send(BridgeEvent::SpeechStarted);
            }
            Ok(Some(VoiceAgentEvent::Error(message))) => {
                let _ = event_tx.send(BridgeEvent::Error(message));
            }
            Ok(Some(VoiceAgentEvent::OutputAudioDone | VoiceAgentEvent::ResponseDone)) => {
                // Playback finishes on TranscriptDone or naturally via player.
            }
            Ok(Some(
                VoiceAgentEvent::SpeechStopped
                | VoiceAgentEvent::SessionUpdated
                | VoiceAgentEvent::Ignored,
            )) => {}
            Err(_) => {
                let _ = event_tx.send(BridgeEvent::Error("Voice Agent socket closed".to_owned()));
                let _ = event_tx.send(BridgeEvent::Stopped);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_wins_over_file() {
        assert!(resolve_voice_agent_s2s_enabled_from(Some("1"), false));
        assert!(!resolve_voice_agent_s2s_enabled_from(Some("0"), true));
        assert!(resolve_voice_agent_s2s_enabled_from(None, true));
        assert!(!resolve_voice_agent_s2s_enabled_from(None, false));
        assert!(resolve_voice_agent_s2s_enabled_from(Some("YES"), false));
    }

    #[test]
    fn instructions_include_agent_and_residual_note() {
        let text = instructions_from_soul(Some("Be witty."), "Sally");
        assert!(text.contains("Sally"));
        assert!(text.contains("Hands tools are not available"));
        assert!(text.contains("Be witty."));
    }
}
