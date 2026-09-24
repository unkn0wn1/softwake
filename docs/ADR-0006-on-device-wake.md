# ADR 0006 — On-device wake engine

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

The production wake engine is **sherpa-onnx keyword spotting** (a streaming Zipformer transducer), not openWakeWord, not Porcupine, not Whisper-as-wake, and not a cloud wake service.

The engine scores 16 kHz mono `i16` PCM through the existing [`WakeDetector::push_samples`](../crates/softwake-wake/src/lib.rs) seam and returns a [`PhraseHit`](../crates/softwake-wake/src/lib.rs). Wake and sleep stay local. No audio leaves the machine for the wake decision.

This decision does not vendor model weights and does not link `sherpa-onnx` yet. The default PCM detector remains `NullDetector`, which returns `PhraseHit::None` for every window. The `sherpa-kws` feature compiles `SherpaKwsDetector`, the same trait with the same `None` result, and records where weights will load. The typed demo still uses `TextWakeDetector` ([ADR 0002](ADR-0002-wake-engine-spike.md)) for `wake` and `sleep`.

## Why this engine

| | sherpa-onnx KWS | openWakeWord | Porcupine |
|---|---|---|---|
| License of the code | Apache-2.0 | Apache-2.0 | Vendor terms; an access key is required to initialize |
| Weights in a public repo | Not vendored. Download an English Zipformer KWS checkpoint into the XDG data directory. Confirm that checkpoint's model card before redistributing the files. | Published pretrained models are CC BY-NC-SA 4.0 (upstream README). Custom phrases need a training run. | Keyword files are produced in the vendor console and are not an open-source artifact to commit. |
| Rust path | Official `sherpa-onnx` crate: `KeywordSpotter`, `KeywordSpotterConfig`, `OnlineStream`, `KeywordResult`. | No official crate. Community ONNX paths (`ort`, or tract via a third-party wrapper). Several models per phrase (mel spectrogram, embedding, classifier). | `pv_porcupine` binding. Initialization takes the access key. |
| Phrase setup | Keywords are text at runtime (`keywords_file`, or `KeywordSpotter::create_stream_with_keywords`). The existing phrase table can become that list. | One trained classifier per phrase. `hey softwake` is not a bundled model. | One console-trained keyword file per phrase. |
| Always-on cost | Streaming model on the order of a few million parameters (the English checkpoint published as `sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01`). CPU, offline after the files are on disk. | Small ONNX classifiers once trained. CPU, offline. | Smaller footprint and published accuracy numbers. CPU, offline after init. The account check is the problem, not the math. |

Porcupine is the stronger always-on detector on paper. It is the wrong default because the wake gate would depend on a vendor account. openWakeWord's code license is fine and its published weights are not: CC BY-NC-SA is a poor fit for a public repository, and Softwake's phrases are already text rather than a trained classifier each.

Whisper-as-wake is rejected. A full speech-to-text decode is the wrong amount of CPU and latency for an always-on sleep state, and a transcript that is then searched for a substring has the same false-trigger shape as the text spike. Cloud wake is rejected, as in [ADR 0002](ADR-0002-wake-engine-spike.md): ambient audio would leave the machine, and sleep would depend on the network.

Accuracy expectations, without a new benchmark in this change:

- Offline after the model directory is populated. The wake path does not open a socket.
- Good enough to try as the always-on gate for a short phrase list. Similar-sounding speech will false-accept sometimes. sherpa-onnx exposes a keyword score and a threshold; those knobs are chosen when weights are loaded, not here.
- Heavier than Porcupine, much lighter than running Whisper while asleep.

## License and what ships

- The `sherpa-onnx` crate is Apache-2.0. Adding it later behind `sherpa-kws` is allowed. This PR does not add it: `sherpa-onnx-sys` bundles an ONNX runtime, and CI must not download that blob or any checkpoint.
- Weights are not committed. A developer downloads an English Zipformer keyword-spotting model (the GigaSpeech 3.3M checkpoint above, or a successor with a license that allows the intended use) into the model directory:
  - `$XDG_DATA_HOME/softwake/kws` when `XDG_DATA_HOME` is set
  - `$HOME/.local/share/softwake/kws` otherwise
- Read the checkpoint's model card before copying those files into a release artifact. Training-data terms are not the same thing as the Apache-2.0 code license.
- `SherpaKwsDetector` stores the directory path and the phrase lists. It does not create the directory and does not read it. `weights_loaded` is `false`.

## Rust wiring

Target crate and types, when the dependency is added:

- `sherpa_onnx::KeywordSpotter::create(&KeywordSpotterConfig) -> Option<KeywordSpotter>`
- `KeywordSpotter::create_stream` or `create_stream_with_keywords`
- `OnlineStream::accept_waveform(16_000, &[f32])`
- `KeywordSpotter::is_ready`, `decode`, `get_result` → `KeywordResult`

`WakeDetector::push_samples` stays `&[i16] -> PhraseHit`. The adapter will:

1. Treat the slice as mono 16 kHz, which is [`AudioFormat::WAKE`](../crates/softwake-audio/src/frame.rs).
2. Convert each sample to `f32` in about `-1.0..=1.0` (`f32::from(sample) / f32::from(i16::MAX)`).
3. Append it with `accept_waveform` and decode while `is_ready`.
4. Map the decoded keyword with the same longest-match rule as `PhraseTable::score`. An equal-length wake and sleep tie is sleep. No match is `PhraseHit::None`.

Until that adapter exists, both `NullDetector` and `SherpaKwsDetector` return `PhraseHit::None` and ignore the samples. Silence (`[0; 160]`, 10 ms) is a useful fixture: it must not wake.

Enable the stub locally. CI does not pass this flag:

```bash
cargo test -p softwake-wake --features sherpa-kws
```

## Capture in front of the detector

`AudioCapture::poll_frame` returns `Result<Option<AudioFrame>, Error>`. `AudioFrame::samples` is the `&[i16]` passed to `push_samples`. `score_frame` in the daemon is that one call.

- `MockAudioCapture` is the default for tests, `softwaked demo`, and `softwaked serve`. Its error type is infallible. `stop` still drops queued frames.
- The default `pipewire` feature compiles `PipeWireCapture` and does not link `libpipewire`. `start` returns `PipeWireError::NotLinked`. `poll_frame` returns `PipeWireError::NotRunning` rather than `Ok(None)`, so the stub is not an idle microphone. `format` is still `AudioFormat::WAKE`, which is the layout a later stream must deliver.
- `pipewire-native` compiles the same module and returns `PipeWireError::StreamUnwired` from `start`. It does not link `libpipewire` and it does not open a device. CI does not enable the feature and does not need a `PipeWire` daemon or a microphone.

```bash
cargo test -p softwake-audio --features pipewire-native
```

A later change that links `libpipewire` needs the `libpipewire-0.3-dev` package (or the distro's equivalent) and must stay out of the default CI job. The process callback should queue `AudioFrame` values at `AudioFormat::WAKE`. `poll_frame` is already the pull API the daemon uses for the mock.

The typed demo pushes 10 ms of silence through mock capture and then through `NullDetector` before the text detector's hit is applied. The text hit is what moves the voice state. Serve calls the same PCM score on any frame already queued before it handles a command. With no microphone, that queue is empty.

## Context

[ADR 0002](ADR-0002-wake-engine-spike.md) kept wake deterministic without a microphone or weights. Phase 2 can pick the production engine without dropping that property. The next change that loads weights should not have to redesign `PhraseHit`, the voice-state events, or the capture trait.

## Alternatives

- openWakeWord as the default. Rejected for the weight license and because each Softwake phrase would need its own trained model.
- Porcupine as the default. Rejected because initialization needs a vendor access key.
- Whisper, or any full speech-to-text model, as the wake gate. Rejected. Wrong cost for always-on sleep, and the transcript search repeats the spike's false triggers.
- Cloud wake. Rejected. Audio would leave the machine.
- Linking `sherpa-onnx` and downloading weights in this change. Rejected. CI would depend on a native runtime and a checkpoint fetch. The trait and the feature gate are the merge bar.
- Linking `libpipewire` in the default build. Rejected. CI has no microphone requirement, and the default job must not need a `PipeWire` daemon.

## Consequences

- [`ADR 0002`](ADR-0002-wake-engine-spike.md) remains the text spike. This ADR is the production-engine choice.
- `PhraseHit` and the voice-state mapping do not change when `NullDetector` is replaced by a loaded `SherpaKwsDetector`.
- The phrase table remains configuration. It will be rendered into sherpa-onnx's keyword list. It is not thrown away.
- Default `cargo test --workspace` does not open a microphone, does not start `PipeWire`, does not download weights, and does not enable `sherpa-kws` or `pipewire-native`.
- Packaging is unchanged. Streaming speech-to-text and text-to-speech are [ADR 0007](ADR-0007-awake-stt-tts.md).
