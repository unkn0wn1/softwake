//! Local playback of one TTS audio file.
//!
//! Tries `ffplay`, then `mpv`, then `paplay` for WAV. Tests use
//! [`PlaybackMode::Record`] and do not spawn a player. A missing player is an
//! error string the HUD can show.
//!
//! Spawn mode starts the player and **returns immediately**. Softwake must not
//! block the daemon, Tauri commands, or the HUD bloom on Eve finishing. A short
//! reaper thread joins the child (with a timeout) and deletes the temp file.
//! [`interrupt_playback`] kills the last spawned player so a new ask can cut in.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// Join deadline for the background reaper. Longer than a short reply, shorter
/// than leaving a zombie player overnight.
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

/// Pid of the player started by the latest [`PlaybackMode::Spawn`] call.
static LAST_PLAYER: Mutex<Option<u32>> = Mutex::new(None);

/// Play `bytes` or record them.
///
/// Spawn starts a player and returns as soon as the child is running. It does
/// **not** wait for playback to finish. A stuck or long clip cannot freeze the
/// HUD. The reaper still bounds how long the child may live.
///
/// # Errors
///
/// A sentence when no player is installed, the temp file cannot be written,
/// or the player cannot be started.
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
        PlaybackMode::Spawn => spawn_player_detached(bytes, suffix, timeout),
    }
}

/// Stop the last spawned player, if Softwake still knows its pid.
///
/// Best-effort. Used before a new reply so Eve does not overlap herself.
pub fn interrupt_playback() {
    let pid = {
        let mut guard = LAST_PLAYER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.take()
    };
    if let Some(pid) = pid {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn spawn_player_detached(bytes: &[u8], suffix: &str, timeout: Duration) -> Result<(), String> {
    interrupt_playback();
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
            Ok(child) => {
                let pid = child.id();
                remember_pid(pid);
                let path_for_reaper = path.clone();
                let _ = thread::Builder::new()
                    .name("softwake-tts-reaper".to_owned())
                    .spawn(move || {
                        reap_player(child, timeout);
                        let _ = std::fs::remove_file(&path_for_reaper);
                        clear_pid_if(pid);
                    });
                // Fire-and-forget: caller returns while Eve is still speaking.
                return Ok(());
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

fn remember_pid(pid: u32) {
    let mut guard = LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = Some(pid);
}

fn clear_pid_if(pid: u32) {
    let mut guard = LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.as_ref() == Some(&pid) {
        *guard = None;
    }
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

fn reap_player(mut child: Child, timeout: Duration) {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Ok(Some(_)) | Err(_) => return,
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
    use super::{PLAYBACK_TIMEOUT, PlaybackMode, play_audio};
    use std::time::Instant;

    #[test]
    fn record_mode_keeps_bytes_and_rejects_empty() {
        let mut record = None;
        play_audio(
            PlaybackMode::Record,
            b"fake-mp3",
            "mp3",
            PLAYBACK_TIMEOUT,
            &mut record,
        )
        .expect("record");
        let clip = record.expect("clip");
        assert_eq!(clip.suffix, "mp3");
        assert_eq!(clip.bytes, b"fake-mp3");

        let error = play_audio(
            PlaybackMode::Record,
            b"",
            "mp3",
            PLAYBACK_TIMEOUT,
            &mut None,
        )
        .expect_err("empty");
        assert!(error.contains("no audio"), "{error}");
    }

    #[test]
    fn spawn_mode_returns_before_player_timeout_budget() {
        // Even when every player is missing, spawn must fail fast — never sleep
        // for PLAYBACK_TIMEOUT. When a player exists, it returns without waiting
        // for the clip to finish (covered operationally; CI stays player-free).
        let start = Instant::now();
        let result = play_audio(
            PlaybackMode::Spawn,
            b"not-real-audio",
            "mp3",
            PLAYBACK_TIMEOUT,
            &mut None,
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "spawn path blocked for {elapsed:?}: {result:?}"
        );
        // Missing player or a real spawn both finish under the budget.
        let _ = result;
    }
}
