# Changelog

## Unreleased

- Live chat, speech-to-text, and speech-synthesis HTTP calls now wait up to 120 seconds (`CHAT_TIMEOUT`) instead of 30. A slow provider read was aborting the completion, so long replies never finished. The HUD and Settings status show the full reply text. Spoken audio can still be cut by the 60 second playback reaper if the clip itself runs longer than that.
- The HUD collapses to a square bloom (no composer, chat, or hint). Click expands a panel of chat bubbles labeled You or the active profile name, each with a timestamp, plus Send and a mic icon (aria-label “Hold to talk”). It collapses again after the pointer has been outside for N seconds. A non-empty ask draft keeps it open. Settings → General sets N from 1 to 30 seconds (default 3). The key is `hud_idle_collapse_ms` in ui-prefs.json (1000–30000). The HUD re-reads that file; softwaked does not need a restart. Press-to-talk still ignores keys typed in the ask field. Free-speech silence, keyword spotting, and natural-language confirms are unchanged.

- Settings → General can tune free-speech end-of-utterance silence (default 2.0 s, range 0.5–4.0 s). The key is `free_speech_end_silence_ms` in softwake.json (500–4000). `SOFTWAKE_FREE_SPEECH_END_SILENCE_MS` wins over the file. Changes apply live (`reload_utterance` / `softwaked ctl reload-utterance`) without restarting softwaked and without rebuilding the keyword spotter. Press-to-talk is unchanged.
- Natural-language sleep phrases with soft closers (`go to sleep for a little while`, `I'm going to sleep`, `sleep for a few minutes`) now arm `Sleep now?` before the chat model, so Softwake no longer invents an “okay I’ll sleep” reply while staying awake.
- Free-speech end-of-utterance silence hangover raised to ~2.0 s (200 × 10 ms capture frames) so a mid-thought pause is less likely to cut the STT utterance. Press-to-talk is unchanged.

- Natural-language sleep and hibernate while awake ask a short confirm (`Sleep now?` / `Hibernate now?`) before changing state. A clear yes applies the transition and the existing state voice. No, or 15 seconds of silence, stays awake.
- While asleep, a below-threshold wake-word near-miss (not bare `hi`) or a typed wake attempt asks `Were you trying to wake me?` at most once every 45 seconds. Yes wakes. No or silence stays asleep.
- Hard keyword hits (`hi`, the profile name, `sleep`, `deep sleep`, and the other configured phrases) stay immediate and do not ask. Fire thresholds are unchanged. The near-miss probe runs during sleep so fuzzy wake can hear those hits; it does not lower the fire threshold.
