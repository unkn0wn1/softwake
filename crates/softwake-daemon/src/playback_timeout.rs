//! Resolve the TTS playback reaper deadline.
//!
//! Precedence: env `SOFTWAKE_TTS_PLAYBACK_TIMEOUT_MS` (integer milliseconds)
//! wins over `tts_playback_timeout_ms` in `softwake.json`. Missing both uses
//! `60_000` ms. `ReloadPlayback` re-reads the value for the status line.
//! `speak_reply` resolves again when a clip starts; that duration is what the
//! reaper uses. An in-flight player keeps the deadline it was spawned with.

use std::time::Duration;

use softwake_soul::{TTS_PLAYBACK_TIMEOUT_MS_DEFAULT, clamp_tts_playback_timeout_ms};

/// Deadline for one TTS playback reaper (env > file > default), clamped.
#[must_use]
pub(crate) fn resolve_tts_playback_timeout() -> Duration {
    resolve_tts_playback_timeout_from(
        std::env::var("SOFTWAKE_TTS_PLAYBACK_TIMEOUT_MS")
            .ok()
            .as_deref(),
        file_ms(),
    )
}

fn file_ms() -> Option<u32> {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let Ok(config_dir) = softwake_soul::resolve_config_dir(xdg.as_deref(), home.as_deref()) else {
        return None;
    };
    softwake_soul::load_app_config(&config_dir)
        .ok()
        .map(|app| app.tts_playback_timeout_ms)
}

/// Same precedence as [`resolve_tts_playback_timeout`], with inputs injected.
///
/// An env string that is not an integer is ignored so a bad override falls
/// through to the file (or the product default). Whitespace and floats are
/// not trimmed or parsed.
#[must_use]
pub(crate) fn resolve_tts_playback_timeout_from(
    env_raw: Option<&str>,
    file_ms: Option<u32>,
) -> Duration {
    let mut ms = file_ms.unwrap_or(TTS_PLAYBACK_TIMEOUT_MS_DEFAULT);
    if let Some(raw) = env_raw {
        if let Ok(value) = raw.parse::<u32>() {
            ms = value;
        }
    }
    Duration::from_millis(u64::from(clamp_tts_playback_timeout_ms(ms)))
}

#[cfg(test)]
mod tests {
    use super::{resolve_tts_playback_timeout, resolve_tts_playback_timeout_from};
    use softwake_soul::TTS_PLAYBACK_TIMEOUT_MS_DEFAULT;
    use softwake_voice::PLAYBACK_TIMEOUT;
    use std::time::Duration;

    fn millis(duration: Duration) -> u128 {
        duration.as_millis()
    }

    #[test]
    fn env_wins_over_file_and_invalid_env_falls_through() {
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(None, None)),
            60_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(None, Some(180_000))),
            180_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(
                Some("45000"),
                Some(180_000)
            )),
            45_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(
                Some("nope"),
                Some(180_000)
            )),
            180_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(Some(""), Some(90_000))),
            90_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(
                Some(" 45000"),
                Some(180_000)
            )),
            180_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(
                Some("180000.0"),
                Some(90_000)
            )),
            90_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(Some("1000"), None)),
            30_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(Some("9000000"), None)),
            300_000
        );
        assert_eq!(
            millis(resolve_tts_playback_timeout_from(None, Some(0))),
            30_000
        );
    }

    #[test]
    fn default_matches_playback_timeout_const() {
        assert_eq!(
            PLAYBACK_TIMEOUT,
            Duration::from_millis(u64::from(TTS_PLAYBACK_TIMEOUT_MS_DEFAULT))
        );
        assert_eq!(
            resolve_tts_playback_timeout_from(None, None),
            PLAYBACK_TIMEOUT
        );
    }

    #[test]
    fn live_resolve_stays_inside_bounds() {
        let timeout = resolve_tts_playback_timeout();
        let seconds = timeout.as_secs();
        assert!(
            (30..=300).contains(&seconds),
            "resolved timeout {timeout:?} outside 30..=300 s"
        );
    }
}
