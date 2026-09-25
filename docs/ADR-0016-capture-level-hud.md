# ADR 0016 — Capture level on the wire for HUD particles

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

The daemon exposes an additive `capture_level: Option<f32>` on protocol-generation-1 `Status` (`0.0..=1.0`, peak-normalized RMS). The HUD capsule prefers that value for particle bloom and falls back to the local sine mock from [ADR 0015](ADR-0015-tray-hud.md) when the field is absent.

### Capture path

- `softwake_audio::rms_level` scores each drained PCM window.
- Default **mock** capture remains the CI and demo path. While capture is running, `GetStatus` queues a short synthetic listening tone so HUD polls see frame-derived levels without a microphone.
- Hibernating (capture stopped) clears the level.
- The `pipewire-native` feature stays an opt-in stub that does not link `libpipewire` and is not enabled in CI. When a future native stream queues frames, the same RMS → `Status` path applies.

### HUD

- `hud_snapshot` sets `level_mocked: false` when `capture_level` is present; otherwise it keeps the ADR 0015 sine and `level_mocked: true`.

### Non-goals

- Shipping sherpa weights, cloud STT, or skills/webhooks.
- Linking `libpipewire` in default CI.
- Protocol generation bump (field is additive and omitted when absent).

## Context

ADR 0015 shipped tray + HUD with a UI-only mock level because `Status` had no energy field. Operators still need particles that track something on the capture path before a real mic stream exists.

## Alternatives

1. **UI-only mock forever** — rejected; particles would never reflect capture PCM.
2. **New event stream for levels** — deferred; status polling already drives the HUD.
3. **Require PipeWire for any level** — rejected; CI and default demos stay mic-free.

## Consequences

- `softwaked ctl status` prints `capture level: …` when present.
- README documents how to try audio (serve + UI, optional `pipewire-native`).
- ADR 0015 level section points here for the real path.
