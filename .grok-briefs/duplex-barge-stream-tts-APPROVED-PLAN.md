# APPROVED PLAN — Soft duplex barge + streaming TTS

**Status:** Implement now (Spencer: continuous duplex ask; Softwake recovery if grok stalls).  
**Branch:** `feat/duplex-barge-stream-tts`  
**Base:** main `9e3d04c` (#107)  
**ADR:** **ADR-0049** + amend ADR-0007 / ADR-0048  
**Out of scope:** Voice Agent S2S rewrite, AEC, PROTOCOL bump, service restart

## Honest gap (Grok Bot vs Softwake)

| | Softwake today (#107) | Grok Bot-like continuous duplex |
|--|----------------------|----------------------------------|
| Path | STT → chat LLM SSE → unary `POST /v1/tts` → ffplay file | Likely **Voice Agent / speech-to-speech realtime** WS |
| Mic while TTS | **Muted** (half-duplex, ADR-0007) | Live + barge / turn detection |
| First audio | Sentence-boundary early TTS after full unary synth | Streaming TTS audio deltas / S2S |
| CancelAsk | Stops SSE; **does not kill player** | Instant cut |

True continuous duplex needs the **Voice Agent** provider API (new Softwake path). This slice ships the highest-impact steps **inside** the existing STT→LLM→TTS architecture.

## Design

### A. CancelAsk stops voice instantly
- `cancel_ask_outcome` / a shared `announce::cancel_speech()` bumps speak generation + `interrupt_playback` (+ `force_clear_input_mute`).
- Escape / Cancel / CancelAsk silence Eve immediately even when ask lock is held.

### B. Soft duplex barge-in (mic live enough to interrupt)
- While `input_muted()` from TTS (not user HUD mute), still read RMS.
- `BargeDetector`: elevated `BARGE_RMS` (~0.12) × `BARGE_FRAMES` (~120ms) → fire.
- On fire: `cancel_speech` + `force_clear_input_mute` + `request_cancel_ask` + reset free-speech buffer; subsequent frames use normal awake listen (overlap listen after cut).
- Not AEC: speaker echo can false-trigger; threshold is the guard. Document residual.

### C. Streaming TTS (xAI WebSocket) + progressive playback
- Public API: `wss://api.x.ai/v1/tts?…&optimize_streaming_latency=1` — `text.delta` / `text.done` → `audio.delta` (base64) / `audio.done`.
- `softwake-providers` (`live-http`): sync `tungstenite`+`native-tls` client; unit-test URL + event parse without network.
- `softwake-voice`: `PipePlayer` — `ffplay -f mp3 -i pipe:0` / `mpv -` write chunks as they arrive; same mute/reaper/interrupt contract as file Spawn.
- `talk::speak_reply*`: try stream+pipe; on failure before/without audio → existing unary `tts_synthesize` + file play.
- Early-TTS queue still uses speak_lock + interrupt=false.

### D. Docs / CI / install
- ADR-0049; amend ADR-0007 (streaming + soft barge); ADR-0048 CancelAsk note.
- CHANGELOG Unreleased.
- PR → merge when green.
- `cargo install --force` daemon (`live-http,sherpa-kws,pipewire-capture`) + UI (`live-http`); **no restart**.

## Tests
- providers: `tts_stream_url` https→wss; parse `audio.delta` / reject bad events.
- voice: `BargeDetector` thresholds; `force_clear_input_mute`; Record-mode stream write if feasible.
- daemon: CancelAsk path calls cancel_speech (unit where easy).
