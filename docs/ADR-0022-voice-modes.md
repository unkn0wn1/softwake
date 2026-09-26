# ADR 0022 — Voice phrases, state voice, and voice test mode

- **Status:** Accepted
- **Date:** 2026-09-26
- **Relates to:** [ADR 0006](ADR-0006-on-device-wake.md), [ADR 0007](ADR-0007-awake-stt-tts.md), [ADR 0021](ADR-0021-multi-turn-compact.md)

## Decision

Keyword lists gain three phrases and do not drop the existing ones:

- bare `hi` → wake
- bare `sleep` → sleep
- `deep sleep` → hibernate

`deep sleep` enters hibernate from sleep or from awake and stops capture. It does not leave hibernate. `ctl resume` and UI Resume still land in sleep. `ctl wake` and UI Wake still enter awake from sleep when the soul pack is valid.

sherpa-onnx keyword spotting has no grammar. Softwake does not pretend otherwise. Short words are a best effort:

1. Single words of at most eight characters keep the per-keyword threshold `#0.10` (global multi-word default `0.15`). Operators may lower further via config/env.
2. The online stream soft-resets after about five seconds of **quiet** accepted audio, and hard-resets after about ten seconds even during speech, so a long awake session keeps emitting keywords without chopping a slow phrase mid-utterance. A ~400 ms silence reset was tried and withdrawn: pauses and quiet edges chopped a keyword before the window was accepted.
3. Short hits are not dropped because free speech or press-to-talk has been buffering. A ~1.5 second suppress was tried and withdrawn: bare `sleep` disappeared during awake chat. Multi-word phrases were never the problem.

A state change speaks one short line. The line is a one-shot completion: the loaded soul as system, and one fixed instruction as the only user turn, then the existing TTS path. That turn is not appended to the awake session. If the completion cannot run, TTS speaks a fixed fallback (`I'm awake.`, `Listening for wake.`, `Deep sleep.`) when a voice is configured.

Voice test mode defaults off. `softwaked serve --voice-test`, `softwaked ctl voice-test on|off`, and Settings → General set it on the running daemon. It does not persist. Phrases and the state line still run. Microphone speech is not sent to the chat model. Typed `ctl ask` still works.

Verbose (`-v` / `-vv`) KWS lines include the loaded profile name and, when it differs, the profile directory id. Ask lines log tokens about to be sent and the estimate from before compaction (`char/4`, ADR 0021). A compact line logs before, after, and the real compact percent.

## Non-goals

- A general sentence grammar. A small fixed heuristic covers confirm and fuzzy wake only: awake ask text that clearly means sleep or hibernate, yes/no while that question is open, and a below-threshold wake near-miss while asleep.
- Leaving hibernate by voice.
- A protocol generation bump. `set_voice_test` and `Status.voice_test` are additive on generation 1.
- Persisting voice test mode in Settings JSON.

## Consequences

Bare `hi` can false-wake from ordinary speech while asleep. `#0.10` makes that short word easier to spot, and there is no speech-buffer gate in sleep. If `hi` is too noisy or too weak on a given microphone, prefer `hey <name>` or the product phrases (`hey softwake`, `softwake`).

Bare `sleep` is scored again during awake chat. `go to sleep` and `<name> sleep` remain the more reliable multi-word options. If the spotter emits the short tag `sleep` for a longer phrase, longest-match cannot recover the longer phrase.

A profile whose name is exactly `sleep` or `hi` ties with that bare word and loses the tie-break (hibernate, then sleep, then wake). `hey <name>` still wakes.

Announcement speech can lag the HUD by one completion plus TTS. Rapid transitions are serialized on a playback lock and can speak a line after the state has already moved on.

Confirm prompts are fixed lines in the profile voice (`Sleep now?`, `Hibernate now?`, `Were you trying to wake me?`), not a one-shot completion. The wait is 15 seconds. Fuzzy wake asks at most once every 45 seconds, and a probe hit on bare `hi` does not ask. A real keyword fire still changes state immediately and clears a waiting confirm. No protocol field was added: the question is `Status.message` only. Voice-test mode still does not send microphone speech to speech-to-text; a typed yes or no still answers, and a real wake keyword still wakes.
