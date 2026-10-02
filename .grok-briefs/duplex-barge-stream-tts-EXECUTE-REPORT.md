# EXECUTE REPORT — Soft duplex barge + streaming TTS

**Branch:** `feat/duplex-barge-stream-tts`  
**Base:** main `9e3d04c` (#107)  
**Grok CLI:** plan session started; Softwake-style ship against APPROVED-PLAN.

## Delivered
1. CancelAsk / Escape → `announce::cancel_speech` (interrupt + clear mute).
2. Soft duplex barge-in (`BargeDetector` elevated RMS while TTS mute).
3. Streaming TTS WebSocket + MP3 stdin pipe; unary POST fallback.
4. ADR-0049; ADR-0007 / ADR-0048 amendments; CHANGELOG.

## Honest residual
- Not Voice Agent S2S continuous duplex.
- Barge can false-trigger from loud speaker echo (no AEC).
- Stream path needs live xAI + ffplay/mpv; falls back to unary on failure.
