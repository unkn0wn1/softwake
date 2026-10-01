# ADR-0048: Chat token streaming and CancelAsk barge-in

## Status

Accepted (slice 1).

## Context

Ask/chat used a single non-stream POST. The HUD only saw the final reply after completion (+ TTS). Softwake could not feel like Grok Bot (token deltas, Escape to abort).

## Decision

1. **SSE stream for text-only rounds** — `complete_chat_stream` / `complete_chat_turn_text_stream` set `stream: true` and parse `choices[0].delta.content`. Tool-bearing rounds stay non-stream. The tool-loop finalize round (empty tools) streams.
2. **Mid-ask HUD** — deltas update the shared status cache (`phase=streaming`, growing `message`) so `GetStatus` try_lock misses show Writing… and grow the assistant bubble.
3. **CancelAsk** — additive `ClientMessage::CancelAsk` sets `Arc<AtomicBool>` without taking the runtime lock. The stream loop stops between deltas and keeps partial text as the assistant reply. HUD Escape / Cancel button calls `hud_cancel_ask`.
4. **TTS** — this slice keeps TTS after the stream completes (`finish_ask_reply`). Early/sentence-boundary TTS is a follow-up so it never blocks the SSE reader.

## Consequences

- PROTOCOL_VERSION stays 1 (additive message).
- Long streams still share the chat HTTP timeout budget.
- Mic mute alone does not abort; CancelAsk / Escape / Cancel button does.
