//! Settings → General free-speech end-silence control.
//!
//! Persist `free_speech_end_silence_ms` in `softwake.json`, then IPC
//! `reload_utterance` so the running daemon updates the energy gate without
//! rebuilding KWS or restarting.
//!
//! Precedence on the daemon: env `SOFTWAKE_FREE_SPEECH_END_SILENCE_MS` > file.
//! Unset that env for this Settings knob to stick on every live reload.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use softwake_ipc::{Client, resolve_socket_path};
use softwake_soul::{
    FREE_SPEECH_END_SILENCE_MS_DEFAULT, FREE_SPEECH_END_SILENCE_MS_MAX,
    FREE_SPEECH_END_SILENCE_MS_MIN, clamp_free_speech_end_silence_ms, load_app_config,
    resolve_config_dir, set_free_speech_end_silence_ms,
};

/// Snapshot for the General pane (seconds, 0.5–4.0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FreeSpeechSilenceSnapshot {
    /// Hangover in seconds (e.g. 2.0).
    pub seconds: f64,
    /// Whether the running daemon accepted a live reload.
    pub live_applied: bool,
    /// Short status line for the pane.
    pub message: String,
}

fn config_dir() -> Result<std::path::PathBuf, String> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    resolve_config_dir(xdg.as_deref(), home.as_deref()).map_err(|error| error.to_string())
}

fn ms_to_seconds(ms: u32) -> f64 {
    f64::from(ms) / 1000.0
}

fn seconds_to_ms(seconds: f64) -> u32 {
    if !seconds.is_finite() {
        return FREE_SPEECH_END_SILENCE_MS_DEFAULT;
    }
    let ms = (seconds * 1000.0).round().clamp(
        f64::from(FREE_SPEECH_END_SILENCE_MS_MIN),
        f64::from(FREE_SPEECH_END_SILENCE_MS_MAX),
    );
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value clamped into 500..=4000 before cast"
    )]
    let ms_u32 = ms as u32;
    clamp_free_speech_end_silence_ms(ms_u32)
}

fn snapshot_from(ms: u32, live_applied: bool, message: String) -> FreeSpeechSilenceSnapshot {
    FreeSpeechSilenceSnapshot {
        seconds: ms_to_seconds(ms),
        live_applied,
        message,
    }
}

/// Load the current hangover from softwake.json (default when missing).
#[tauri::command]
pub fn free_speech_silence_snapshot() -> Result<FreeSpeechSilenceSnapshot, String> {
    let dir = config_dir()?;
    let app = load_app_config(&dir).unwrap_or_default();
    Ok(snapshot_from(
        app.free_speech_end_silence_ms,
        false,
        format!(
            "Free-speech end silence {:.1} s",
            ms_to_seconds(app.free_speech_end_silence_ms)
        ),
    ))
}

/// Persist seconds as milliseconds and live-reload the daemon energy gate.
#[tauri::command]
pub fn free_speech_silence_set(seconds: f64) -> Result<FreeSpeechSilenceSnapshot, String> {
    let ms = seconds_to_ms(seconds);
    let dir = config_dir()?;
    let app = set_free_speech_end_silence_ms(&dir, ms).map_err(|error| error.to_string())?;

    let mut live_applied = false;
    let mut message = format!(
        "Saved free-speech end silence {:.1} s",
        ms_to_seconds(app.free_speech_end_silence_ms)
    );
    match (|| -> Result<(), String> {
        let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
        let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
        client
            .reload_utterance()
            .map_err(|error| error.to_string())?;
        Ok(())
    })() {
        Ok(()) => {
            live_applied = true;
            message.push_str(" — applied live");
        }
        Err(error) => {
            let _ = write!(message, " — saved; live apply skipped ({error})");
        }
    }

    Ok(snapshot_from(
        app.free_speech_end_silence_ms,
        live_applied,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::{ms_to_seconds, seconds_to_ms};

    #[test]
    fn seconds_ms_round_trip_default() {
        assert!((ms_to_seconds(2000) - 2.0).abs() < f64::EPSILON);
        assert_eq!(seconds_to_ms(2.0), 2000);
        assert_eq!(seconds_to_ms(0.5), 500);
        assert_eq!(seconds_to_ms(4.0), 4000);
        assert_eq!(seconds_to_ms(1.5), 1500);
    }

    #[test]
    fn seconds_to_ms_clamps_bounds() {
        assert_eq!(seconds_to_ms(0.1), 500);
        assert_eq!(seconds_to_ms(9.0), 4000);
        assert_eq!(seconds_to_ms(f64::NAN), 2000);
    }
}
