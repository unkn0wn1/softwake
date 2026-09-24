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
3. `test` on Ubuntu (PipeWire optional; mocks default)
4. Build daemon + UI (Tauri) on Linux

No deploy pipeline in phase 1. Tag releases later.

## Product gates (phase 1)

Manual or automated as soon as hardware allows:

| Gate | Pass condition |
|------|----------------|
| Sleep silence | Ambient speech produces **zero** tool calls for N minutes |
| Wake | Configured phrase transitions sleep → awake within agreed latency budget |
| Sleep phrase | Awake → sleep; tools stop; mic stays up |
| Hibernate | UI hibernate stops capture (no frames); voice cannot wake |
| UI wake | Hibernate → sleep via UI |
| Soul required | Missing `soul.md` or `user.md` blocks awake with a clear error |
| Safe tool | One allowlisted tool succeeds end-to-end while awake |

## Review bar

- Architecture fit: does this change belong in this crate?
- New dependency justified in the PR text
- State transitions covered by tests
- No widen of tool allowlist without docs update
- Docs updated when behaviour changes (especially voice states)

## Explicit non-gates (phase 1)

- Perfect wake-word accuracy on every mic
- macOS / Windows parity
- Multi-user / multi-seat
- Production packaging (Flatpak etc.) — nice later
