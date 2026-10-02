# APPROVED PLAN — xAI Voice Agent continuous S2S

**Status:** Implement now (Spencer yes; spike green on spencenb).  
**Branch:** `feat/voice-agent-s2s`  
**Base:** main `dd739fe` (#108)  
**ADR:** **ADR-0050** + amend ADR-0007 / ADR-0049  
**Model:** grok-4.7 `--effort xhigh`  
**Out of scope:** Softwake Hands→Voice Agent function bridge, AEC, PROTOCOL bump, service restart

## Spike facts (do not re-litigate)

| Fact | Value |
|------|-------|
| URL | `wss://api.x.ai/v1/realtime?model=grok-voice-latest` |
| Auth | Softwake existing xAI bearer (key or oauth access token) |
| In | PCM16 LE mono **16000** Hz (matches Softwake capture) |
| Out | PCM16 LE mono **24000** Hz via `response.output_audio.delta` base64 |
| VAD | `turn_detection: { type: "server_vad" }` |
| Mic event | `input_audio_buffer.append` `{ "type", "audio": "<b64>" }` |
| Voice | Settings `selected_tts_voice` (fallback `eve` / current resolver) |

## Design

### A. Config toggle
- `AppConfig.voice_agent_s2s: bool` (serde default **false**) in `softwake-soul` / `softwake.json`.
- `set_voice_agent_s2s(config_dir, enabled)` helper (mirror webhook).
- Env `SOFTWAKE_VOICE_AGENT_S2S` (`1`/`true`/`on` wins over file when resolving).
- UI Settings → General checkbox; Tauri command + live ctl `reload-voice-agent` (or piggyback a small `ReloadVoiceAgent` / reuse pattern of `reload-free-speech`) that reconnects or tears down the session if awake.
- Status additive: `voice_agent_s2s: bool` (omit when false) so HUD can show mode.

### B. Provider client (`softwake-providers`, feature `live-http`)
New module `voice_agent.rs` (export from lib):

1. `voice_agent_realtime_url(api_base, model)` → `wss://…/v1/realtime?model=…` (default model `grok-voice-latest`).
2. `voice_agent_session_update_json({voice, instructions, input_rate, output_rate, web_search})` → session.update payload with nested audio + server_vad + optional `tools: [{type:web_search}]` + `reasoning: {effort: "none"}` for snappy talk.
3. `parse_voice_agent_event(&str) -> VoiceAgentEvent` enum covering at least:
   - `OutputAudioDelta(Vec<u8>)` from `response.output_audio.delta` (also accept legacy `response.audio.delta` if seen)
   - `OutputAudioDone`
   - `TranscriptDelta(String)` / `TranscriptDone`
   - `SpeechStarted` / `SpeechStopped` if present (`input_audio_buffer.speech_started` / `.speech_stopped`)
   - `ResponseDone` / `Error(String)` / `Ignored`
4. Live session struct (sync tungstenite+native-tls, same style as `tts_stream.rs`):
   - `connect(url, bearer) -> Session`
   - `send_session_update(...)`
   - `append_pcm16(&[i16])` (LE bytes → b64 append)
   - `read_event() -> Result<VoiceAgentEvent>`
   - `cancel_response()` best-effort `response.cancel`
   - `close()`
5. Unit tests: URL, JSON contains expected keys, parse fixtures (no network).

### C. PCM playback (`softwake-voice`)
- `PcmPipePlayer` parallel to `Mp3PipePlayer`: spawn `ffplay -nodisp -autoexit -loglevel quiet -f s16le -ar <rate> -ac 1 -i pipe:0` then `mpv --no-video --really-quiet --demuxer=rawaudio --demuxer-rawaudio-format=s16le --demuxer-rawaudio-rate=<rate> --demuxer-rawaudio-channels=1 -`.
- Same mute-hold / interrupt / force_clear contracts as MP3 path (CancelAsk + sleep must cut audio).
- Record mode for tests: accumulate PCM bytes.

### D. Daemon integration (`softwake-daemon`)
New `voice_agent_session.rs` (or module under runtime):

**When to run S2S path:**
`voice_agent_s2s` resolved on + `live-http` + DiskChat readiness (xAI family) + state Awake.

**Lifecycle:**
1. On transition → Awake (or toggle on while awake): spawn/connect session; build instructions from soul pack (truncate hard, e.g. 6–8 KB; second-person Softwake greeting + rules excerpt). Include agent name. Do **not** dump secrets.
2. Each capture frame while awake + mode on + !user_muted: `append_pcm16` (even during assistant speech — server_vad handles barge). **Skip** Softwake free-speech EnergyUtterance→STT→ask and Softwake `BargeDetector` path for mic acting while S2S owns the mic. KWS sleep/hibernate phrases **still** run on the same frames (local).
3. Reader thread/task: on OutputAudioDelta → write PCM pipe (start player lazily); TranscriptDelta → update status message / append HUD chat assistant bubble best-effort; SpeechStarted → interrupt local player (barge); Error → status sentence, fallback disable session for this awake (do not crash).
4. On sleep / hibernate / mode off / CancelAsk: `cancel_response`, `interrupt_playback` / force_clear mute, close WS.
5. Typed/text ask (`ctl ask`, HUD text): **unchanged** tool-loop path (document residual: parallel worlds — voice S2S ≠ Hands).

**PTT:** while S2S on, PTT can either (prefer) just ensure mic unmuted / no-op into the same stream, or briefly note “Voice Agent mode listens continuously”. Do not double-fire STT on PTT release.

### E. UI
- General pane: checkbox “Voice Agent (continuous S2S)” with one-line help: uses xAI realtime; tools/souls Hands loop stay on text ask; requires xAI + live build.
- Wire snapshot/set like utterance_prefs.

### F. Docs
- `docs/ADR-0050-voice-agent-s2s.md`
- Amend ADR-0007 + ADR-0049 “S2S now optional behind toggle”
- CHANGELOG Unreleased
- `.grok-briefs/voice-agent-s2s-EXECUTE-REPORT.md` at end

### G. Install / merge
- PR to main; wait CI green; merge.
- `cargo install --force --path crates/softwake-daemon --features live-http,sherpa-kws,pipewire-capture`
- `cargo install --force --path crates/softwake-ui --features live-http`
- **No** systemctl/UI restart.

## Residuals (must document verbatim in ADR + report)

| Capability | S2S mode | Text tool loop |
|------------|----------|----------------|
| Continuous duplex + server VAD barge | Yes | Soft barge #108 only |
| Softwake Hands / confirm / shell / email / calendar / drive / memory write | **No** (v1) | Yes |
| Soul pack as full system + multi-turn compact | Truncated `instructions` only | Full |
| KWS wake/sleep/hibernate | Softwake-local | Softwake-local |
| Streaming TTS / unary TTS | Bypassed while S2S owns speak | Yes |
| web_search | Voice Agent server tool only | Softwake tools N/A |

## Implementation order
1. providers voice_agent (URL/JSON/parse + tests)
2. voice PcmPipePlayer + tests
3. soul AppConfig + setters + tests
4. daemon session + runtime wiring (awake frames, skip free-speech STT)
5. ipc status field + reload ctl
6. UI General toggle
7. ADR/CHANGELOG/README blurb
8. fmt/clippy/test → commit → push → PR → CI → merge → cargo install

## Honesty rule
If live event names differ from spike, adjust parse table to observed names and note in ADR. Do not claim Hands tools work over S2S.
