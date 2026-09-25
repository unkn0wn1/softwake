# Quality gates

Definition of done for Softwake work. Phase 1 does not ship without these.

## Local (every change)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Optional later: `cargo deny` / `cargo audit` once deps exist.

## CI (when GitHub exists)

On every PR to `main`:

1. `fmt` check
2. `clippy -D warnings`
3. `test` on Ubuntu (PipeWire optional; mocks default). The job installs the WebKitGTK packages the Tauri window needs.
4. Build daemon + UI (Tauri) on Linux

No deploy pipeline in phase 1. Tag releases later.

## Product gates (phase 1)

Manual checklist. Run it after clone + soul templates. The typed demo (`softwaked demo`) is the supported path that enters awake. Default builds use mock capture and do not open a microphone. Native PipeWire is the `pipewire-native` feature and CI does not enable it. Wake and sleep phrases in the demo are typed commands that feed the same state machine. The PCM detector still sees a silent frame on that path ([ADR 0006](ADR-0006-on-device-wake.md)).

Copy templates first:

```bash
mkdir -p ~/.config/softwake/soul
cp soul/soul.md soul/user.md ~/.config/softwake/soul/
```

Then:

```bash
cargo run -p softwake-daemon -- demo
```

| Gate | Pass condition | How to check today |
|------|----------------|--------------------|
| Sleep silence | Ambient speech produces **zero** tool calls for N minutes | Automated in unit tests for the asleep refuse path. Live ambient speech waits on a real mic (out of phase 1). Typed demo: `tool echo` while asleep is rejected. |
| Wake | Configured phrase transitions sleep → awake within agreed latency budget | Typed demo: `wake` (or the configured wake phrase path). Needs a valid soul pack. |
| Sleep phrase | Awake → sleep; tools stop; mic stays up | Typed demo: `sleep` after the post-wake cooldown. Capture stays running. Session closes. Further `tool` calls are rejected. |
| Hibernate | UI hibernate stops capture (no frames); voice cannot wake | Typed demo / UI / `ctl hibernate`: capture stopped; `wake` is rejected until `resume`. |
| UI wake | Hibernate → sleep via UI | UI button or `ctl resume` / demo `resume`. Lands in sleep, not awake. |
| Soul required | Missing `soul.md` or `user.md` blocks awake with a clear error | Remove a soul file and `wake`; status shows soul missing and state stays sleep. Hibernate / resume / sleep still work. |
| Safe tool | One safe tool succeeds end-to-end while awake | Typed demo while awake: `tool echo hello` → `echo: hello`; `tool echo` → `pong`. Unknown names (`tool volume`) are rejected. `ctl tool echo hello` works against `serve` only after the daemon is awake. |
| Confirm tool | A confirm-gated tool does not run until confirm | Typed demo while awake: `tool notify hello` prints `waiting for confirm` and does not append; `confirm` appends `hello`; `cancel` appends nothing. `tool shell` is denied. Sleep or hibernate clears a pending confirmation. |

Phase 1 shipped `echo` only ([ADR 0004](ADR-0004-first-safe-tool.md)). Phase 2 registers `echo` (safe), `notify` (confirm), and `shell` (deny) ([ADR 0005](ADR-0005-tool-confirmation.md)). Do not add a tool without a docs update.

## Review bar

- Architecture fit: does this change belong in this crate?
- New dependency justified in the PR text
- State transitions covered by tests
- No widen of tool allowlist without docs update
- A connector action is a separate registry from tools. A network client or a new tool name needs an ADR
- Docs updated when behaviour changes (especially voice states)

## Explicit non-gates (phase 1)

- Perfect wake-word accuracy on every mic
- macOS / Windows parity
- Multi-user / multi-seat
- Production packaging (Flatpak etc.) — nice later
