# APPROVED PLAN — Voice Agent first-audio latency

**Base:** main `1940139` (#110/#111)  
**Branch:** `fix/voice-agent-first-audio`  
**Symptom:** S2S conversation OK; voice does not start on first transcript — usually after a couple subsequent HUD stream/status entries.

## Root cause
Assistant PCM is opened/written only in `VoiceAgentBridge::pump()`, which runs from `drain_pcm` → client `GetStatus`. HUD adaptive poll is **900ms** when not in text-stream pending. Audio/transcript deltas pile in the mpsc until the next poll; ffplay also lacks low-latency flags and stdin is not flushed — so audible start lags several status ticks.

## Fix (no daemon restart in this change)
1. **Play in the Voice Agent worker** on first `response.output_audio.delta` (open `PcmPipePlayer`, write, flush) — do not wait for HUD `pump()`.
2. **`write_chunk` flush** stdin after every PCM write; same for MP3 pipe if cheap.
3. **Low-latency player flags:** ffplay `-fflags nobuffer -flags low_delay -probesize 32 -analyzeduration 0 -infbuf`; mpv `--cache=no --audio-buffer=0` (or equivalent).
4. **Worker owns player lifecycle:** Cancel / SpeechStarted / AudioDone / TranscriptDone / Stop finish or abort in-worker; `pump()` only updates transcript + HUD phase (Speaking on audio notify).
5. CHANGELOG + short ADR-0050 note; unit test that pcm_pipe commands include nobuffer.

## Out of scope
- Softwake Hands on S2S; AEC; changing HUD poll defaults globally.
- Service restart (Spencer restarts when ready). Reinstall via `cargo install --force`.

## Verify
`cargo fmt`, `clippy -D warnings`, `cargo test -p softwake-voice -p softwake-daemon --features live-http` (and workspace if time).
