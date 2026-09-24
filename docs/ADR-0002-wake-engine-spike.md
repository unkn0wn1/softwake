# ADR 0002 — Wake engine spike

- **Status:** Accepted
- **Date:** 2026-09-24

## Decision

Phase 1 scores wake and sleep with a local phrase table (`PhraseTable`, `TextWakeDetector`). A UTF-8 window matches when its trimmed, Unicode-lowercase form contains a configured phrase. The longest matching phrase wins. When a wake phrase and a sleep phrase tie on length, the result is sleep, so a sleep phrase is not scored as a wake.

This is a spike. It is not an on-device wake-word model. [ADR 0006](ADR-0006-on-device-wake.md) chooses sherpa-onnx keyword spotting as the production engine. The PCM `WakeDetector` trait stays in place for it. `NullDetector` still never matches. Weights are not linked in the default build.

The daemon demo submits configured phrase text through the text detector and then through the voice-state machine. `hibernate` and `resume` are UI events and do not go through the phrase detector. No cloud speech-to-text is consulted for wake or sleep.

`PipeWireCapture` implements the capture trait and returns an error. The default `pipewire` feature compiles that stub and does not link a native audio library. `pipewire-native` is optional, still does not link `libpipewire`, and is not enabled in CI. The demo uses `MockAudioCapture`.

## Context

Sleep must score audio only for wake phrases, and hibernate must not listen. A production wake engine still needs an accuracy, CPU, and licensing choice. The spike has to be deterministic in CI, with no microphone and no model weights, so the daemon can demonstrate sleep, awake, cooldown, and hibernate.

## Alternatives

- Cloud speech-to-text as the wake gate. Sleep would depend on a network round trip, and ambient audio would leave the machine. Rejected.
- An on-device model in this change. It needs weights, a license review, and a microphone in CI. The PCM trait is where that engine will land. Rejected for the spike.
- Treating the demo words `wake` and `sleep` as the phrases themselves, with no table. The demo would skip the detector and drift from configured phrases. Rejected.

## Consequences

- Tests cover a wake hit, a sleep hit, no match, and case folding without audio hardware.
- Substring matching false-triggers when ordinary speech contains a phrase. That is acceptable for the spike and unacceptable for the production engine.
- Replacing the spike means implementing `WakeDetector` for the engine in [ADR 0006](ADR-0006-on-device-wake.md) and keeping the same `PhraseHit` to voice-state event mapping. The phrase table can remain as configuration after the engine changes.
- `softwaked demo` simulates voice by submitting phrase text. Capture still has to be running before that text is scored. The frame is also passed to the PCM detector (`NullDetector` in the default build).
