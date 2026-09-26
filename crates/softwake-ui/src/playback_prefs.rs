//! Settings → General TTS playback reaper control.
//!
//! Persist `tts_playback_timeout_ms` in `softwake.json`, then IPC
//! `reload_playback` so the running daemon can confirm the deadline.
//! The next speak reads the file again. An in-flight clip keeps the
//! deadline it started with.
//!
//! Precedence on the daemon: env `SOFTWAKE_TTS_PLAYBACK_TIMEOUT_MS` > file.
//! Unset that env for this Settings knob to stick on every speak.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use softwake_ipc::{Client, resolve_socket_path};
use softwake_soul::{
    TTS_PLAYBACK_TIMEOUT_MS_DEFAULT, TTS_PLAYBACK_TIMEOUT_MS_MAX, TTS_PLAYBACK_TIMEOUT_MS_MIN,
    clamp_tts_playback_timeout_ms, load_app_config, resolve_config_dir,
    set_tts_playback_timeout_ms,
};

/// Snapshot for the General pane (integer seconds, 30–300).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TtsPlaybackTimeoutSnapshot {
    /// Reaper deadline in seconds (e.g. 60).
    pub seconds: u32,
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

fn ms_to_seconds(ms: u32) -> u32 {
    ms / 1000
}

fn seconds_to_ms(seconds: f64) -> u32 {
    if !seconds.is_finite() {
        return TTS_PLAYBACK_TIMEOUT_MS_DEFAULT;
    }
    let rounded = seconds.round().clamp(
        f64::from(TTS_PLAYBACK_TIMEOUT_MS_MIN) / 1000.0,
        f64::from(TTS_PLAYBACK_TIMEOUT_MS_MAX) / 1000.0,
    );
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value clamped into 30..=300 before cast"
    )]
    let seconds_u32 = rounded as u32;
    clamp_tts_playback_timeout_ms(seconds_u32.saturating_mul(1000))
}

fn snapshot_from(ms: u32, live_applied: bool, message: String) -> TtsPlaybackTimeoutSnapshot {
    TtsPlaybackTimeoutSnapshot {
        seconds: ms_to_seconds(ms),
        live_applied,
        message,
    }
}

/// Load the current playback deadline from softwake.json (default when missing).
#[tauri::command]
pub fn tts_playback_timeout_snapshot() -> Result<TtsPlaybackTimeoutSnapshot, String> {
    let dir = config_dir()?;
    let app = load_app_config(&dir).unwrap_or_default();
    Ok(snapshot_from(
        app.tts_playback_timeout_ms,
        false,
        format!(
            "Max spoken reply {} s",
            ms_to_seconds(app.tts_playback_timeout_ms)
        ),
    ))
}

/// Persist seconds as milliseconds and ask the daemon to re-read the deadline.
#[tauri::command]
pub fn tts_playback_timeout_set(seconds: f64) -> Result<TtsPlaybackTimeoutSnapshot, String> {
    let ms = seconds_to_ms(seconds);
    let dir = config_dir()?;
    let app = set_tts_playback_timeout_ms(&dir, ms).map_err(|error| error.to_string())?;

    let mut live_applied = false;
    let mut message = format!(
        "Saved max spoken reply {} s",
        ms_to_seconds(app.tts_playback_timeout_ms)
    );
    match (|| -> Result<(), String> {
        let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
        let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
        client
            .reload_playback()
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
        app.tts_playback_timeout_ms,
        live_applied,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::seconds_to_ms;

    #[test]
    fn seconds_to_ms_rounds_and_clamps() {
        assert_eq!(seconds_to_ms(60.0), 60_000);
        assert_eq!(seconds_to_ms(30.0), 30_000);
        assert_eq!(seconds_to_ms(300.0), 300_000);
        assert_eq!(seconds_to_ms(60.4), 60_000);
        assert_eq!(seconds_to_ms(60.6), 61_000);
        assert_eq!(seconds_to_ms(10.0), 30_000);
        assert_eq!(seconds_to_ms(999.0), 300_000);
        assert_eq!(seconds_to_ms(f64::NAN), 60_000);
    }
}
