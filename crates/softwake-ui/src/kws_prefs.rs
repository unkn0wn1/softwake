//! Settings → General KWS threshold controls.
//!
//! Persist to `softwake.json` milli keys, then IPC `reload_kws` so the running
//! daemon rebuilds the spotter without a process restart.
//!
//! Precedence on daemon rebuild: env `SOFTWAKE_KWS_*` > file. Unset env for
//! these Settings knobs to stick on every live reload.

use std::fmt::Write;

use serde::{Deserialize, Serialize};
use softwake_ipc::{Client, resolve_socket_path};
use softwake_soul::{
    KWS_THRESHOLD_MILLI_MAX, KWS_THRESHOLD_MILLI_MIN, clamp_kws_threshold_milli, load_app_config,
    resolve_config_dir, set_kws_thresholds,
};

/// Snapshot for the General pane (human floats 0.05–0.50).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KwsThresholdsSnapshot {
    /// Global multi-word threshold (e.g. 0.15).
    pub global: f64,
    /// Short-word threshold (e.g. 0.10).
    pub short: f64,
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

fn milli_to_display(milli: u16) -> f64 {
    f64::from(milli) / 1000.0
}

fn display_to_milli(value: f64) -> u16 {
    if !value.is_finite() {
        return 150;
    }
    let milli = (value * 1000.0).round().clamp(
        f64::from(KWS_THRESHOLD_MILLI_MIN),
        f64::from(KWS_THRESHOLD_MILLI_MAX),
    );
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value clamped into 50..=500 before cast"
    )]
    let milli_u16 = milli as u16;
    clamp_kws_threshold_milli(milli_u16)
}

fn snapshot_from(
    global_milli: u16,
    short_milli: u16,
    live_applied: bool,
    message: String,
) -> KwsThresholdsSnapshot {
    KwsThresholdsSnapshot {
        global: milli_to_display(global_milli),
        short: milli_to_display(short_milli),
        live_applied,
        message,
    }
}

/// Load current thresholds from softwake.json (defaults when missing).
#[tauri::command]
pub fn kws_thresholds_snapshot() -> Result<KwsThresholdsSnapshot, String> {
    let dir = config_dir()?;
    let app = load_app_config(&dir).unwrap_or_default();
    Ok(snapshot_from(
        app.kws_threshold_milli,
        app.kws_short_threshold_milli,
        false,
        format!(
            "Wake word {:.2} / short-word {:.2}",
            milli_to_display(app.kws_threshold_milli),
            milli_to_display(app.kws_short_threshold_milli)
        ),
    ))
}

/// Persist floats to milli keys and live-reload the daemon spotter.
#[tauri::command]
pub fn kws_thresholds_set(global: f64, short: f64) -> Result<KwsThresholdsSnapshot, String> {
    let global_milli = display_to_milli(global);
    let short_milli = display_to_milli(short);
    let dir = config_dir()?;
    let app =
        set_kws_thresholds(&dir, global_milli, short_milli).map_err(|error| error.to_string())?;

    let mut live_applied = false;
    let mut message = format!(
        "Saved wake word {:.2} / short-word {:.2}",
        milli_to_display(app.kws_threshold_milli),
        milli_to_display(app.kws_short_threshold_milli)
    );
    match (|| -> Result<(), String> {
        let path = resolve_socket_path(None).map_err(|error| error.to_string())?;
        let mut client = Client::connect(&path).map_err(|error| error.to_string())?;
        client.reload_kws().map_err(|error| error.to_string())?;
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
        app.kws_threshold_milli,
        app.kws_short_threshold_milli,
        live_applied,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::{display_to_milli, milli_to_display};

    #[test]
    fn display_milli_round_trip_defaults() {
        assert!((milli_to_display(150) - 0.15).abs() < f64::EPSILON);
        assert!((milli_to_display(100) - 0.10).abs() < f64::EPSILON);
        assert_eq!(display_to_milli(0.15), 150);
        assert_eq!(display_to_milli(0.10), 100);
    }

    #[test]
    fn display_to_milli_clamps_bounds() {
        assert_eq!(display_to_milli(0.01), 50);
        assert_eq!(display_to_milli(0.99), 500);
        assert_eq!(display_to_milli(f64::NAN), 150);
    }
}
