//! Local playback of one TTS audio file.
//!
//! Tries `ffplay`, then `mpv`, then `paplay` for WAV. Tests use
//! [`PlaybackMode::Record`] and do not spawn a player. A missing player is an
//! error string the HUD can show.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// Join deadline for one spawned player. Longer than a short reply, shorter
/// than leaving the serve thread blocked overnight.
pub const PLAYBACK_TIMEOUT: Duration = Duration::from_secs(60);

/// How [`play_audio`] delivers bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackMode {
    /// Write a temp file and spawn a player. Used by the serve binary.
    Spawn,
    /// Remember the bytes and return success. Used by unit tests.
    Record,
}

/// Last clip recorded by [`PlaybackMode::Record`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayedClip {
    /// Suggested file suffix (`mp3` or `wav`).
    pub suffix: String,
    /// Audio bytes.
    pub bytes: Vec<u8>,
}

/// Play `bytes` or record them.
///
/// Spawn waits at most `timeout` so a stuck player cannot hold the caller
/// forever. The player runs in a short-lived child; this function joins that
/// wait.
///
/// # Errors
///
/// A sentence when no player is installed, the temp file cannot be written,
/// or the player exits non-zero.
pub fn play_audio(
    mode: PlaybackMode,
    bytes: &[u8],
    suffix: &str,
    timeout: Duration,
    record: &mut Option<PlayedClip>,
) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("The voice service returned no audio.".to_owned());
    }
    match mode {
        PlaybackMode::Record => {
            *record = Some(PlayedClip {
                suffix: suffix.to_owned(),
                bytes: bytes.to_vec(),
            });
            Ok(())
        }
        PlaybackMode::Spawn => spawn_player(bytes, suffix, timeout),
    }
}

fn spawn_player(bytes: &[u8], suffix: &str, timeout: Duration) -> Result<(), String> {
    let path = write_temp(bytes, suffix)?;
    let players = player_commands(&path, suffix);
    let mut missing = Vec::new();
    for (program, args) in players {
        match Command::new(program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                let result = wait_child(&mut child, timeout);
                let _ = std::fs::remove_file(&path);
                return result;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(program);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                return Err(format!("could not start {program}: {error}"));
            }
        }
    }
    let _ = std::fs::remove_file(&path);
    Err(format!(
        "no audio player found (tried {}). Install ffplay or mpv to hear replies.",
        missing.join(", ")
    ))
}

fn player_commands(path: &Path, suffix: &str) -> Vec<(&'static str, Vec<String>)> {
    let display = path.display().to_string();
    let mut commands = vec![
        (
            "ffplay",
            vec![
                "-nodisp".to_owned(),
                "-autoexit".to_owned(),
                "-loglevel".to_owned(),
                "quiet".to_owned(),
                display.clone(),
            ],
        ),
        (
            "mpv",
            vec![
                "--no-video".to_owned(),
                "--really-quiet".to_owned(),
                display.clone(),
            ],
        ),
    ];
    if suffix.eq_ignore_ascii_case("wav") {
        commands.push(("paplay", vec![display]));
    }
    commands
}

fn wait_child(child: &mut std::process::Child, timeout: Duration) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("audio player exited {status}")),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("audio playback timed out".to_owned());
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(format!("audio player failed: {error}")),
        }
    }
}

fn write_temp(bytes: &[u8], suffix: &str) -> Result<std::path::PathBuf, String> {
    let path = std::env::temp_dir().join(format!(
        "softwake-tts-{}-{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis()),
        suffix
    ));
    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("could not write speech audio: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("could not write speech audio: {error}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{PlaybackMode, play_audio};

    #[test]
    fn record_mode_keeps_bytes_and_does_not_spawn() {
        let mut record = None;
        play_audio(
            PlaybackMode::Record,
            b"mp3",
            "mp3",
            Duration::from_secs(1),
            &mut record,
        )
        .expect("record");
        let clip = record.expect("clip");
        assert_eq!(clip.suffix, "mp3");
        assert_eq!(clip.bytes, b"mp3");
    }

    #[test]
    fn empty_audio_is_an_error() {
        let mut record = None;
        let error = play_audio(
            PlaybackMode::Record,
            b"",
            "mp3",
            Duration::from_secs(1),
            &mut record,
        )
        .expect_err("empty");
        assert!(error.contains("no audio"));
    }
}
