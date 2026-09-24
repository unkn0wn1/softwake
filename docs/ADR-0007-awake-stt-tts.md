# ADR 0007 — Awake STT / TTS path

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

While Softwake is **awake**, speech-to-text and text-to-speech use a local streaming boundary in `softwake-voice`:

- **STT:** prefer **sherpa-onnx streaming ASR** (same family as the wake keyword spotter in [ADR 0006](ADR-0006-on-device-wake.md)). The default implementation is [`MockStt`](../crates/softwake-voice/src/mock.rs), which injects partial and final transcript text for tests and the typed demo. No microphone and no model download are required on the default path.
- **TTS:** a local stub that records “spoken” strings ([`MockTts`](../crates/softwake-voice/src/mock.rs)). A later feature-gated sherpa-onnx TTS or espeak backend may replace the mock. The default path does not open a speaker.

Cloud-only STT or TTS is **rejected as the only path**. A future optional cloud connector may exist behind an explicit feature and operator config; Softwake must still build, demo, and CI without cloud API keys.

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

- Cloud STT/TTS as the only awake path. Rejected. CI and offline demos would need keys; ambient-capable builds would depend on the network for basic speech.
- Whisper as the default awake STT. Deferred. Heavier than a streaming transducer for partials; not required to close the boundary. May be revisited behind a feature later.
- Bundling weights in the repository. Rejected. License and CI size.
- Acting STT/TTS while asleep or hibernating. Rejected. Sleep scores wake phrases only; hibernate has capture off; prefer TTS silence outside awake.
- Stuffing traits into `softwake-session`. Rejected for this change. The session crate stays a text acting session; voice I/O is a separate seam.

## Consequences

- Phase 2 milestone item 1 (streaming STT/TTS path) can close with mocks + ADR + feature-gated stubs.
- A later PR that links sherpa-onnx ASR/TTS loads from the XDG dirs above without redesigning `SpeechToText` / `TextToSpeech` or the demo inject path.
- Default `cargo test --workspace` does not open a microphone, does not download weights, and does not enable `sherpa-asr` or `sherpa-tts`.
- Typed wake/sleep remains the primary voice-state demo ([ADR 0002](ADR-0002-wake-engine-spike.md)).
