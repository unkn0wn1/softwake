# APPROVED PLAN — Voice Agent S2S barge-in audio mashup

**Base:** main `bf2e74d` (#112)  
**Branch:** `fix/s2s-audio-mashup`  
**Symptom:** Barge-in usually works; sometimes the new reply plays while the previous sentence continues (two overlapping voices).

## Root cause
1. End-of-utterance uses `PcmPipePlayer::finish()` → stdin drop + **background reaper**; ffplay/mpv keeps playing buffered PCM while `player` Option is already `None`.
2. New response first `output_audio.delta` called `play_pcm_pipe_start(..., interrupt: false)` so it **never** killed the draining prior player via `LAST_PLAYER`.
3. `SpeechStarted` / Cancel only `abort()` when the Option still holds the player — after `finish()`, abort is a no-op → two players = mashup.
4. `interrupt_playback` used SIGTERM and did not wait for death / sink settle.

## Fix (no softwaked/UI restart in this change)
1. Harden `interrupt_playback`: SIGKILL, poll until pid gone (~150ms), short sink settle when a kill happened.
2. `abort_va_player`: always `interrupt_playback()` even when Option is `None` (covers finish-orphaned player).
3. First open in `write_va_audio`: `interrupt: true` so a new utterance never starts beside a draining prior player.
4. CHANGELOG + ADR-0050 note; unit test that interrupt clears remembered pid when process already gone.

## Out of scope
- AEC; Softwake Hands on S2S; HUD poll changes; service restart.

## Verify
`cargo fmt`, `clippy -D warnings`, `cargo test -p softwake-voice`, `cargo test -p softwake-daemon --features live-http`.
