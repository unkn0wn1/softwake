# ADR 0007 — Awake STT / TTS path

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25. An optional xAI cloud voice connector (press-to-talk STT, TTS voice `eve`) sits behind `live-http` and the existing provider credentials. Mocks stay the default and the CI path.
- **Amended:** 2026-09-25. Listen-while-awake energy VAD (free speech) shares the same STT→ask→Eve path. TTS playback is spawn-and-return so the HUD never waits on `ffplay`/`mpv`.

## Decision

While Softwake is **awake**, speech-to-text and text-to-speech use a local streaming boundary in `softwake-voice`:

- **STT:** prefer **sherpa-onnx streaming ASR** (same family as the wake keyword spotter in [ADR 0006](ADR-0006-on-device-wake.md)). The default implementation is [`MockStt`](../crates/softwake-voice/src/mock.rs), which injects partial and final transcript text for tests and the typed demo. No microphone and no model download are required on the default path.
- **TTS:** a local stub that records “spoken” strings ([`MockTts`](../crates/softwake-voice/src/mock.rs)). A later feature-gated sherpa-onnx TTS or espeak backend may replace the mock. The default path does not open a speaker.

Cloud-only STT or TTS is **rejected as the only path**. An optional cloud connector is allowed behind the `live-http` feature and the provider credentials already stored for chat ([ADR 0012](ADR-0012-model-providers.md)). Softwake must still build, demo, and CI without cloud API keys. Mocks remain the default.

### Optional xAI cloud voice

When the daemon is built with `live-http` and the selected provider is xAI (sign-in or API key):

- **Press-to-talk.** The HUD mic button arms a PCM buffer on the existing capture stream (16 kHz mono `i16`). Release caps the clip at about 15 seconds and refuses clips shorter than about 0.3 seconds. Hibernate refuses. Sleep enters awake first, the same gate as HUD ask, then buffers.
- **STT.** `POST {api}/v1/stt` as multipart `model` then `file` (WAV). The model is Settings `selected_voice_model`, or the xAI seed `grok-voice-transcribe-2.0` when that field is empty. The transcript is shown and sent through the existing awake `ask` path.
- **TTS.** After a successful ask (typed or spoken), `POST {api}/v1/tts` with JSON `{ "text", "voice_id", "language": "en" }` returns mp3 bytes. `voice_id` is Settings `selected_tts_voice`. Empty means **`eve`**, the documented xAI default. Other documented built-ins may be chosen in Settings. `eve` is not sent to a non-xAI provider; that picker is disabled.
- **Playback.** The daemon writes a temp mp3 and tries `ffplay -nodisp -autoexit -loglevel quiet`, then `mpv --no-video --really-quiet`. A missing player is a HUD error. The player is **spawned and detached**: Softwake returns as soon as the child starts and does **not** `.wait()` on Eve finishing. A reaper joins with a timeout and deletes the temp file. A new ask interrupts the previous player when possible. The HUD bloom and UI stay responsive while she talks.
- **Listen while awake (free speech).** While **awake**, capture running, PTT not armed, and outside a short post-ask cooldown (~2.5s to avoid Eve echo), an energy / silence gate (`EnergyUtterance`) collects an utterance from PipeWire PCM and runs the same STT→ask→Eve path as PTT. Sleep→awake by voice still needs KWS weights. `Status.auto_listening` is additive and omitted when false. PTT always takes priority.
- **IPC.** `talk_start` and `talk_stop` are additive on protocol generation 1. `Status.talking` and `Status.auto_listening` are additive and omitted when false. The HUD fires talk/ask without awaiting the full pipeline; it paints `thinking…` and reads the reply from status polls so rAF/bloom never stalls on network or playback. Status polls themselves must not block the UI thread: `hud_snapshot` is `spawn_blocking`, and serve answers `GetStatus` from a cached snapshot when the runtime mutex is held by STT/ask/TTS ([ADR 0015](ADR-0015-tray-hud.md)).

Settings document version stays 1. `selected_tts_voice` defaults to empty. Phrase keyword spotting (“hey Softwake”) stays out of this path ([ADR 0006](ADR-0006-on-device-wake.md)).

Whisper is **not** the sleep/wake gate ([ADR 0006](ADR-0006-on-device-wake.md)). It is also not required for the awake default path.

## Why this shape

| | sherpa-onnx streaming ASR | Whisper (local) | Cloud STT only |
|---|---|---|---|
| Fits wake family | Yes — same project as KWS | No | No |
| Offline after weights on disk | Yes | Yes | No |
| CI without download | Stub + MockStt | Possible but heavier default | Needs keys / network |
| Streaming partials | Native online transducer API | Usually utterance batches | Vendor-dependent |
| Sleep gate | Not used for sleep | Rejected as wake/sleep gate | Rejected |

The awake acting channel is separate from always-on wake. Wake stays a small KWS. Awake STT may be heavier once weights load, but the default mock keeps CI and the typed demo mic-free.

TTS records strings in the mock so the demo can prove `say hello` without audio hardware. Status beeps while asleep or hibernating are out of scope; prefer silence outside awake.

## XDG model directories

Weights are not committed. A developer who enables a native backend downloads checkpoints into:

| Role | Path |
|------|------|
| Streaming ASR | `$XDG_DATA_HOME/softwake/asr` when `XDG_DATA_HOME` is set; otherwise `$HOME/.local/share/softwake/asr` |
| TTS assets | `$XDG_DATA_HOME/softwake/tts` when `XDG_DATA_HOME` is set; otherwise `$HOME/.local/share/softwake/tts` |

Wake KWS remains under `.../softwake/kws` ([ADR 0006](ADR-0006-on-device-wake.md)). Softwake does not create these directories in this change and does not fetch files in CI. Read each checkpoint’s model card before redistributing.

Helpers: [`asr_model_dir`](../crates/softwake-voice/src/xdg.rs), [`tts_model_dir`](../crates/softwake-voice/src/xdg.rs).

## Rust wiring

Crate: `softwake-voice`.

- [`SpeechToText::push_samples(&[i16]) -> Result<Option<TranscriptEvent>, _>`](../crates/softwake-voice/src/lib.rs)
- [`TextToSpeech::speak(&str) -> Result<(), _>`](../crates/softwake-voice/src/lib.rs)
- [`TranscriptEvent::Partial` / `Final`](../crates/softwake-voice/src/lib.rs)
- Always-on: `MockStt` (inject / pop / drain), `MockTts` (records `spoken()`)
- Feature `sherpa-asr`: `SherpaAsr` stub, `weights_loaded == false`, `push_samples` → `Ok(None)`
- Feature `sherpa-tts`: `SherpaTts` stub, silent record of `speak` until assets load

IPC ([ADR 0003](ADR-0003-ipc-transport.md)): emit `partial_transcript` when mock (or later real) STT produces a partial. `final_transcript` is an additive event on protocol generation 1. No version bump. Do not emit acting transcripts while asleep or hibernating.

Typed demo commands (no mic):

```text
> wake
> hear hello there
> say hello
> sleep
```

`hear` injects a partial then a final. `say` records through `MockTts`. Both are refused outside awake.

Enable stubs locally. CI does not pass these flags by default:

```bash
cargo test -p softwake-voice --features sherpa-asr
cargo test -p softwake-voice --features sherpa-tts
```

## Alternatives

- Cloud STT/TTS as the only awake path. Rejected. CI and offline demos would need keys; ambient-capable builds would depend on the network for basic speech. An opt-in xAI connector behind `live-http` is the amendment above, not a replacement for mocks.
- Whisper as the default awake STT. Deferred. Heavier than a streaming transducer for partials; not required to close the boundary. May be revisited behind a feature later.
- Bundling weights in the repository. Rejected. License and CI size.
- Acting STT/TTS while asleep or hibernating. Rejected. Sleep scores wake phrases only; hibernate has capture off; prefer TTS silence outside awake.
- Stuffing traits into `softwake-session`. Rejected for this change. The session crate stays a text acting session; voice I/O is a separate seam.

## Consequences

- Phase 2 milestone item 1 (streaming STT/TTS path) can close with mocks + ADR + feature-gated stubs.
- A later PR that links sherpa-onnx ASR/TTS loads from the XDG dirs above without redesigning `SpeechToText` / `TextToSpeech` or the demo inject path.
- Default `cargo test --workspace` does not open a microphone, does not download weights, does not call STT or TTS, and does not enable `sherpa-asr`, `sherpa-tts`, or `live-http`.
- `cargo test -p softwake-providers --features live-http` still uses `MockTransport` for STT and TTS request shape. Ignored live tests stay opt-in.
- Typed wake/sleep remains the primary voice-state demo ([ADR 0002](ADR-0002-wake-engine-spike.md)).
