# Brief — Softwake Voice Agent continuous S2S

**Date:** 2026-10-02 (ICT)  
**Model:** grok-4.7, `--effort xhigh` (never `max`)  
**Branch:** `feat/voice-agent-s2s`  
**Base:** main `dd739fe` (#108 duplex barge + streaming TTS)  
**Spencer:** yes to full S2S via xAI Voice Agent. Burn remaining Grok Build credits on cool Softwake features before renew tomorrow. Prefer ambitious but shippable: ADR + working path (settings/profile toggle or mode), mic audio up / voice audio down, barge-in/turn detection from Voice Agent, document residual vs text tool loop/souls.

## Spike (already done on spencenb — trust these facts)

Live probe with Softwake’s existing xAI bearer against:

`wss://api.x.ai/v1/realtime?model=grok-voice-latest`

Confirmed:

- Auth: `Authorization: Bearer <Softwake xAI key/oauth>` works (server-side daemon; no ephemeral token needed).
- `session.update` accepted with nested `audio.input/output.format` (`audio/pcm` 16000 in / 24000 out), `transport: json`, `voice: carina`, `turn_detection: {type: server_vad}`, `reasoning.effort: none`.
- Text turn → `response.output_audio.delta` (base64 PCM16) + `response.output_audio_transcript.delta` + `response.done`. ~130 KB PCM returned.
- Softwake capture is already **16 kHz mono i16** (`AudioFormat::WAKE`) — match input rate **16000**; play output at **24000**.

Honest API notes (do not invent OpenAI-only names):

- Preferred audio events: `response.output_audio.delta` / `.done` (not only legacy `response.audio.delta`).
- Transcript: `response.output_audio_transcript.delta` / `.done`.
- Client mic path with `server_vad`: stream `input_audio_buffer.append` `{audio: base64 PCM16}`; do **not** require manual commit.
- Cancel: prefer `response.cancel` when interrupting; Softwake Escape/CancelAsk must also kill local PCM player.

## Goal

Ship an **opt-in continuous speech-to-speech path** using xAI Voice Agent realtime, behind a Settings/General (softwake.json) toggle. Default remains today’s STT→chat tool-loop→TTS (+ soft duplex #108).

## Must ship

1. **ADR-0050** — Voice Agent S2S mode; amend ADR-0007 / ADR-0049.
2. **Working path** when toggle ON + `live-http` + xAI bearer ready + awake:
   - Open realtime WS; `session.update` with soul-ish `instructions` (truncated pack), Settings TTS voice, `server_vad`, 16k/24k PCM json transport.
   - Mic PCM frames → `input_audio_buffer.append` continuously while awake (respect HUD user mute).
   - Play `response.output_audio.delta` via new **PCM s16le pipe** player (`ffplay -f s16le -ar 24000 -ac 1 -i pipe:0` / mpv equivalent).
   - Surface assistant transcript deltas in HUD status/chat best-effort (additive; no PROTOCOL bump).
3. **Barge-in / turn detection from Voice Agent** (`server_vad`) — do **not** rely on Softwake `BargeDetector` for this mode; still kill local player on CancelAsk / sleep / hibernate / mode-off.
4. **Settings toggle** `voice_agent_s2s` in `softwake.json` (default **false**), Settings → General checkbox, live apply via ctl/IPC reload pattern (like free-speech / webhook). Env override optional `SOFTWAKE_VOICE_AGENT_S2S=1`.
5. **Docs residual** in ADR + CHANGELOG + execute report: Hands/tools/souls tool-loop, confirm policy, memory writes, remote agent, Telegram — **not** bridged into Voice Agent custom functions in this slice. Text `ctl ask` / typed HUD ask **keeps** the existing tool loop. KWS wake/sleep stays Softwake-local. When S2S mode is on, **bypass** free-speech STT→ask→Eve for mic audio (avoid double path).
6. Cool but small: allow `tools: [{ "type": "web_search" }]` on session.update (server-side; no Softwake Hands bridge). Document as Voice Agent built-in only.
7. Tests: URL builder, session.update JSON shape, event parse (audio delta / transcript / error) **without network**. Daemon unit for “mode off → old path”; mode on skip free-speech STT where easy.
8. PR → merge when CI green. `cargo install --force` daemon (`live-http,sherpa-kws,pipewire-capture`) + UI (`live-http`). **No softwaked/UI restart.**

## Non-goals (this slice)

- Bridging Softwake Hands / policy / confirm tools into Voice Agent `function` tools.
- AEC / full echo cancellation.
- Ephemeral browser tokens (daemon is server-side).
- PROTOCOL generation bump.
- Service restart.
- Replacing KWS wake with Voice Agent.

## Quality gates

`cargo fmt`, `clippy -D warnings`, `cargo test --workspace` (and feature-gated live-http tests that do not need network).

## Ops

Auth via existing Softwake xAI key/oauth already used for chat/STT/TTS. Honest if mid-impl discovers different event names — adjust to spike/docs, do not fake.
