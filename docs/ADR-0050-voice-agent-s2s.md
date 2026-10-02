# ADR 0050 — xAI Voice Agent continuous speech-to-speech

- **Status:** Accepted (slice 1)
- **Date:** 2026-10-02
- **Related:** [ADR 0007](ADR-0007-awake-stt-tts.md), [ADR 0049](ADR-0049-duplex-barge-stream-tts.md)

## Context

After ADR-0049 Softwake still acts as STT → chat tool-loop → TTS with soft duplex barge. Spencer asked for Grok Bot–like continuous duplex. xAI’s **Voice Agent / Speech-to-Speech** realtime WebSocket (`wss://api.x.ai/v1/realtime?model=grok-voice-latest`) is the honest provider for that. A live spike on spencenb (2026-10-02) confirmed Softwake’s existing xAI bearer authenticates, nested `audio` PCM **16 kHz in / 24 kHz out** is accepted, `turn_detection.server_vad` works, and the server emits `response.output_audio.delta` (base64 PCM16) plus transcript deltas.

## Decision

1. **Opt-in mode** — `voice_agent_s2s` in `softwake.json` (default **false**). Settings → General checkbox and `softwaked ctl voice-agent on|off|status`. Env `SOFTWAKE_VOICE_AGENT_S2S` wins when set. Live apply via additive `ReloadVoiceAgent` (no PROTOCOL bump).
2. **Working path** (requires `live-http` + xAI provider + awake): open realtime WS; `session.update` with truncated soul `instructions`, Settings TTS voice, `server_vad`, optional server `web_search` tool; stream capture PCM via `input_audio_buffer.append`; play assistant PCM through `PcmPipePlayer` (**without** Softwake half-duplex mute so barge works); surface transcripts on Status.
3. **Barge-in** — Voice Agent `server_vad` / `input_audio_buffer.speech_started` cancels local playback. Softwake `BargeDetector` is skipped while S2S owns the mic. CancelAsk / sleep / mode-off still kill the player and cancel the response.
4. **Bypass free-speech STT→ask→Eve** while the S2S session is active (no double path). KWS wake/sleep/hibernate stays Softwake-local. Typed / HUD text ask keeps the existing Hands tool-loop.

## Residuals (honest)

| Capability | Voice Agent S2S mode | Text tool loop |
|------------|----------------------|----------------|
| Continuous duplex + server VAD | Yes | Soft barge only (#108) |
| Softwake Hands / confirm / shell / email / calendar / drive / memory | **No** | Yes |
| Full soul multi-turn + compact | Truncated `instructions` only | Full |
| web_search | Voice Agent server tool only | Softwake tools N/A |
| Streaming/unary TTS | Bypassed while S2S speaks | Yes |

Bridging Softwake Hands into Voice Agent `function` tools is a future ADR.

## Consequences

- Operators get continuous talk when they opt in; default behaviour unchanged.
- Soul “personality” on voice is a truncated prompt, not the acting session.
- Speaker echo can still confuse server VAD; AEC remains out of scope.
