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
//!
//! While a spawned player is alive (and for a short grace after it exits),
//! [`input_muted`] is true so the daemon can drop mic frames — half-duplex,
//! mute-while-speaking — and avoid Eve hearing herself over speakers. Early
//! returns (no TTS, missing player, Record mode) never arm mute.

use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Join deadline for the background reaper. Longer than a short reply, shorter
/// than leaving a zombie player overnight.
pub const PLAYBACK_TIMEOUT: Duration = Duration::from_secs(60);

/// Keep mic input gated briefly after the player exits (room reverb / latency).
pub const PLAYBACK_MUTE_GRACE: Duration = Duration::from_millis(200);

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

/// Generation of the active mute hold. Bumped on each arm so a stale reaper
/// cannot clear mute while a newer clip is still playing.
static MUTE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// True while a spawned player owns the mute hold (before grace).
static MUTE_HOLD: AtomicBool = AtomicBool::new(false);

/// Wall time until which [`input_muted`] stays true after a hold ends.
static MUTE_GRACE_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// True while TTS playback should suppress mic input (half-duplex).
///
/// Armed only when a Spawn player is running. Released when that player exits
/// (or fails), then held for [`PLAYBACK_MUTE_GRACE`]. Record mode, empty audio,
/// missing players, and synthesize failures never arm this — mute cannot stick
/// from a skipped or failed speak.
#[must_use]
pub fn input_muted() -> bool {
    if MUTE_HOLD.load(Ordering::Acquire) {
        return true;
    }
    let until = MUTE_GRACE_UNTIL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    until.is_some_and(|deadline| Instant::now() < deadline)
}

/// Begin mute-while-speaking. Returns a generation the reaper must pass to
/// [`end_input_mute`]. A newer arm invalidates older generations.
pub fn begin_input_mute() -> u64 {
    let generation = MUTE_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    MUTE_HOLD.store(true, Ordering::Release);
    let mut grace = MUTE_GRACE_UNTIL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *grace = None;
    generation
}

/// End the mute hold for `generation`, then apply [`PLAYBACK_MUTE_GRACE`].
///
/// No-op when a newer [`begin_input_mute`] has already taken over.
pub fn end_input_mute(generation: u64) {
    if MUTE_GENERATION.load(Ordering::Acquire) != generation {
        return;
    }
    MUTE_HOLD.store(false, Ordering::Release);
    let mut grace = MUTE_GRACE_UNTIL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *grace = Some(Instant::now() + PLAYBACK_MUTE_GRACE);
}

/// Drop any mute hold and grace immediately. For unit tests only — production
/// paths use [`end_input_mute`] so room reverb is still covered by grace.
pub fn clear_input_mute_for_test() {
    MUTE_GENERATION.fetch_add(1, Ordering::AcqRel);
    MUTE_HOLD.store(false, Ordering::Release);
    let mut grace = MUTE_GRACE_UNTIL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *grace = None;
}

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
            Ok(mut child) => {
                let pid = child.id();
                // Surface immediate player failures (bad codec, missing sink)
                // that would otherwise look like silent TTS after #41 spawn.
                thread::sleep(Duration::from_millis(120));
                match child.try_wait() {
                    Ok(Some(status)) if !status.success() => {
                        let _ = std::fs::remove_file(&path);
                        clear_pid_if(pid);
                        // Never leave mute armed on a failed player.
                        return Err(format!(
                            "{program} exited immediately ({status}). Softwake could not play the reply."
                        ));
                    }
                    Ok(Some(_)) => {
                        // Tiny clip already finished — brief grace only (reverb).
                        let generation = begin_input_mute();
                        end_input_mute(generation);
                        let _ = std::fs::remove_file(&path);
                        clear_pid_if(pid);
                        return Ok(());
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = std::fs::remove_file(&path);
                        clear_pid_if(pid);
                        return Err(format!("could not check {program}: {error}"));
                    }
                }
                let mute_generation = begin_input_mute();
                remember_pid(pid);
                let path_for_reaper = path.clone();
                let _ = thread::Builder::new()
                    .name("softwake-tts-reaper".to_owned())
                    .spawn(move || {
                        reap_player(child, timeout);
                        let _ = std::fs::remove_file(&path_for_reaper);
                        clear_pid_if(pid);
                        end_input_mute(mute_generation);
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
    use super::{
        PLAYBACK_MUTE_GRACE, PLAYBACK_TIMEOUT, PlaybackMode, begin_input_mute,
        clear_input_mute_for_test, end_input_mute, input_muted, play_audio,
    };
    use std::sync::Mutex;
    use std::thread;
    use std::time::{Duration, Instant};

    /// Mute uses process-wide statics; serialize tests that touch them.
    static MUTE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn record_mode_keeps_bytes_and_rejects_empty() {
        let _guard = MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_input_mute_for_test();
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
        // Record / empty never arms mute-while-speaking.
        assert!(!input_muted());
    }

    #[test]
    fn spawn_mode_returns_before_player_timeout_budget() {
        let _guard = MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_input_mute_for_test();
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
            elapsed < Duration::from_secs(2),
            "spawn path blocked for {elapsed:?}: {result:?}"
        );
        // A missing-player Err must not leave mute stuck forever.
        if result.is_err() {
            assert!(!input_muted(), "failed spawn left input muted");
        }
    }

    #[test]
    fn mute_hold_and_grace_then_clear_stale_generation_ignored() {
        let _guard = MUTE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_input_mute_for_test();
        let mute_gen = begin_input_mute();
        assert!(input_muted());
        // Newer arm invalidates the old generation.
        let newer = begin_input_mute();
        end_input_mute(mute_gen);
        assert!(input_muted(), "stale end must not clear newer hold");
        end_input_mute(newer);
        assert!(input_muted(), "grace keeps mute briefly");
        thread::sleep(PLAYBACK_MUTE_GRACE + Duration::from_millis(50));
        assert!(!input_muted());
        clear_input_mute_for_test();
    }
}
