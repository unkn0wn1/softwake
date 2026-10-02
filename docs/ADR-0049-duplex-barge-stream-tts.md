# ADR 0049 — Soft duplex barge-in and streaming TTS

- **Status:** Accepted (slice 1)
- **Date:** 2026-10-02
- **Related:** [ADR 0007](ADR-0007-awake-stt-tts.md), [ADR 0048](ADR-0048-chat-stream-barge.md)

## Context

After #105–#107 Softwake streams chat tokens and speaks early at sentence boundaries, but Spencer reports Softwake speech is still nothing like Grok Bot: long thinking, Speaking dead air historically, and **no continuous duplex**. Softwake remains half-duplex (mute mic while Eve plays). CancelAsk aborted the LLM stream but left the player running.

xAI documents (1) unary `POST /v1/tts`, (2) **streaming TTS WebSocket** (`text.delta` → `audio.delta`), and (3) a separate **Voice Agent / speech-to-speech** realtime API. Grok Bot’s continuous duplex almost certainly uses (3). Softwake’s acting path is still STT → chat → TTS.

## Decision

1. **Honest boundary** — Softwake does **not** adopt Voice Agent S2S in this slice. Continuous full duplex remains a future provider path.
2. **CancelAsk kills TTS** — cancel bumps the speak generation, `interrupt_playback`, and clears the half-duplex mute hold immediately so Escape/Cancel silence audio now.
3. **Soft duplex barge-in** — while TTS mute is armed (and the HUD mic is not user-muted), Softwake scores elevated RMS; sustained energy cancels speech + ask and returns to awake listen. This is echo-tolerant thresholding, **not** AEC.
4. **Streaming TTS** — when `live-http` is on, prefer xAI streaming TTS WebSocket with `optimize_streaming_latency`, piping MP3 chunks into `ffplay`/`mpv` stdin for lower time-to-first-audio. Unary `POST /v1/tts` remains the fallback.

## Consequences

- First audio can start before the full utterance MP3 is buffered.
- Barge-in can false-trigger from loud speaker echo; thresholds may need Settings later.
- Voice Agent S2S is explicitly deferred (new ADR when tackled).
