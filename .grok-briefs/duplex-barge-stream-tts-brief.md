# Softwake — soft duplex barge + streaming TTS

**Model:** grok-4.7 `--effort xhigh` (never `max`)
**Machine:** spencenb `/www/softwake` → worktree
**Goal:** Highest-impact slice toward Grok Bot voice feel / continuous duplex.

## Honest gap
Grok Bot continuous duplex is almost certainly xAI **Voice Agent / speech-to-speech realtime** (single WS, turn detection, barge). Softwake is still **STT → chat LLM → unary TTS → ffplay** with **half-duplex mute-while-speaking**. Full S2S is a new provider path (out of this slice).

## Ship this slice
1. **CancelAsk / Escape stops TTS instantly** (today only cancels the stream; audio keeps playing).
2. **Soft duplex barge-in:** while Eve speaks, mic frames still drain; elevated-energy VAD cancels TTS + ask and starts free-speech capture (echo-tolerant threshold; not true AEC).
3. **Streaming TTS:** xAI `wss://…/v1/tts` (`text.delta`/`audio.delta`), pipe MP3 to ffplay/mpv stdin for lower time-to-first-audio; unary POST fallback.
4. ADR-0049 + amend ADR-0007; CHANGELOG; PR; CI green; merge; `cargo install --force`; **no restart**.

## Out of scope
Voice Agent S2S rewrite, PROTOCOL bump, service restart, AEC.
