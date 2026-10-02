# EXECUTE REPORT — xAI Voice Agent continuous S2S

**Branch:** `feat/voice-agent-s2s`  
**Date:** 2026-10-02 ICT  
**Base:** main `dd739fe` (#108)

## Process
- Spike (spencenb): Softwake xAI bearer → `wss://api.x.ai/v1/realtime?model=grok-voice-latest` OK; 16 kHz in / 24 kHz out PCM; `response.output_audio.delta`; `server_vad`.
- Plan: `.grok-briefs/voice-agent-s2s-APPROVED-PLAN.md` (authored after spike).
- grok-4.7 `--effort xhigh --always-approve`: stalled (~3 min, 0 file edits). Softwake recovery implemented against the approved plan.

## Shipped
1. **ADR-0050** + amend ADR-0007 / ADR-0049; CHANGELOG.
2. `softwake-providers` `voice_agent` — URL / session.update JSON / event parse + live `VoiceAgentSession` (`live-http`).
3. `PcmPipePlayer` (PCM16 LE stdin; mute optional — S2S keeps mic open).
4. `AppConfig.voice_agent_s2s` + Settings → General + `softwaked ctl voice-agent status|on|off` + `ReloadVoiceAgent`.
5. Daemon bridge: awake + toggle → realtime session; mic PCM up; assistant PCM down; server VAD barge; free-speech STT bypassed while active; text ask unchanged.
6. Optional Voice Agent `web_search` on session.update (server-side only).

## Enable
```bash
softwaked ctl voice-agent on
# or Settings → General → "Voice Agent continuous S2S"
# or softwake.json: "voice_agent_s2s": true
# env wins: SOFTWAKE_VOICE_AGENT_S2S=1
```
Requires: daemon built with `live-http`, xAI provider credentials already in Softwake, **awake**.

## Reinstall (no service restart)
```bash
cargo install --force --path crates/softwake-daemon --features live-http,sherpa-kws,pipewire-capture
cargo install --force --path crates/softwake-ui --features live-http
# Spencer restarts softwaked / UI when ready to pick up new binaries.
```

## Works vs residual

| Works | Residual |
|-------|----------|
| Continuous duplex mic↔Eve via Voice Agent | Softwake Hands / confirm / shell / email / calendar / drive / memory **not** on S2S |
| server_vad barge-in | Full soul tool-loop only on **text ask** |
| Truncated soul as `instructions` | No AEC |
| Status transcripts best-effort | KWS wake/sleep still Softwake-local |
| web_search (Voice Agent built-in) | Softwake tools not bridged as `function` tools |

## Tests
- `cargo fmt`, `clippy --workspace --all-targets -D warnings` green.
- `cargo test --workspace` green (shared target).
- providers `voice_agent` unit tests (6) green with `live-http`.
