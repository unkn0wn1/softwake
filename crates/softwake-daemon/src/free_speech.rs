//! Resolve free-speech end-of-utterance silence.
//!
//! Precedence: env `SOFTWAKE_FREE_SPEECH_END_SILENCE_MS` (integer milliseconds)
//! wins over `free_speech_end_silence_ms` in `softwake.json`. Missing both uses
//! 2000 ms (200 frames at 10 ms). Settings writes the file, then IPC
//! `ReloadUtterance` applies the resolved frame count without rebuilding KWS.

use softwake_soul::FREE_SPEECH_END_SILENCE_MS_DEFAULT;
use softwake_voice::silence_frames_from_ms;

/// Frames of silence that end a free-speech utterance (env > file > default).
#[must_use]
pub(crate) fn resolve_free_speech_end_silence_frames() -> u32 {
    resolve_free_speech_end_silence_frames_from(
        std::env::var("SOFTWAKE_FREE_SPEECH_END_SILENCE_MS")
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
        .map(|app| app.free_speech_end_silence_ms)
}

/// Same precedence as [`resolve_free_speech_end_silence_frames`], with inputs injected.
///
/// An env string that is not an integer is ignored so a bad override falls
/// through to the file (or the product default).
#[must_use]
pub(crate) fn resolve_free_speech_end_silence_frames_from(
    env_raw: Option<&str>,
    file_ms: Option<u32>,
) -> u32 {
    let mut ms = file_ms.unwrap_or(FREE_SPEECH_END_SILENCE_MS_DEFAULT);
    if let Some(raw) = env_raw {
        if let Ok(value) = raw.parse::<u32>() {
            ms = value;
        }
    }
    silence_frames_from_ms(ms)
}

#[cfg(test)]
mod tests {
    use super::resolve_free_speech_end_silence_frames_from;

    #[test]
    fn env_wins_over_file_and_invalid_env_falls_through() {
        assert_eq!(resolve_free_speech_end_silence_frames_from(None, None), 200);
        assert_eq!(
            resolve_free_speech_end_silence_frames_from(None, Some(1500)),
            150
        );
        assert_eq!(
            resolve_free_speech_end_silence_frames_from(Some("800"), Some(1500)),
            80
        );
        assert_eq!(
            resolve_free_speech_end_silence_frames_from(Some("nope"), Some(1500)),
            150
        );
        assert_eq!(
            resolve_free_speech_end_silence_frames_from(Some(""), Some(2500)),
            250
        );
        assert_eq!(
            resolve_free_speech_end_silence_frames_from(Some("100"), None),
            50
        );
        assert_eq!(
            resolve_free_speech_end_silence_frames_from(Some("9000"), None),
            400
        );
        assert_eq!(
            resolve_free_speech_end_silence_frames_from(None, Some(0)),
            50
        );
    }

    #[test]
    fn live_resolve_stays_inside_frame_bounds() {
        let frames = super::resolve_free_speech_end_silence_frames();
        assert!(
            (50..=400).contains(&frames),
            "resolved frames {frames} outside 50..=400"
        );
    }
}
