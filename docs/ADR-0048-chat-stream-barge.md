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

## Amendment — stream-feel (post slice 1)

Soulwright (and any profile with non-deny tools) always advertises tools, so slice-1’s
“stream only when `tools` is empty” meant almost every ask stayed on the non-stream
`complete_chat_turn` path. HUD sat on Thinking until the full reply, then Speaking → TTS
(dead air). Spencer’s “restarted UI only” still ran the #105 binary; the gap was the path.

Follow-up decisions:

1. **Stream every tool-loop round** via `complete_chat_turn_stream` (optional `tools` + SSE
   content and `tool_calls` fragment merge). Plain Message replies with tools advertised
   now grow the HUD mid-ask (`phase=streaming`).
2. **Publish `streaming` before the first token** so Writing… replaces Thinking ASAP.
3. **Early TTS** at sentence boundaries during deltas (non-blocking `spawn_fixed_line`);
   final speak does the remainder only.
4. **HUD polls ~150ms while ask/stream pending** (900ms idle).
5. **Visible build stamp** — UI `app_build_info` + Settings General / HUD chrome; daemon
   `Status.build` (`version · gitsha · built_at`).

## Amendment — early TTS cutoffs (post stream-feel)

#106’s sentence-boundary early TTS cut itself off and left Speaking dead air:

1. **Spawn always interrupted** — each early sentence / final remainder called
   `interrupt_playback`, killing the prior player as soon as the next synth
   finished. Fix: queue under `speak_lock` with `wait_for_playback_idle` + Spawn
   `interrupt: false`; bump a speak generation (+ interrupt) at ask start so
   stale queue items no-op.
2. **Spoken offset across tool rounds** — `partial` resets per round but the
   Atomic offset did not, so finish sliced the final reply with a stale index
   (dropped remainder). Fix: `clamp_spoken_to_text` when `already > len`.
3. **Mid-clause boundaries** — bare `.` at end-of-partial and short abbreviations
   (`Dr.`) committed early. Fix: require whitespace after `.!?`; for `.` require
   next non-ws uppercase and word-before len > 3; finish speaks remainder.
4. **Speaking dead air** — sync `tts_synthesize` on remainder while interrupting
   early audio. Fix: empty remainder skips synth; non-empty after early TTS
   queues via `spawn_fixed_line` (no sync wait).
