# Coding style

Softwake optimises for **changeability and onboarding**, not cleverness. A mid-level Rust engineer should follow a crate in one sitting.

## Global

1. **KISS** — simplest design that meets the phase goal.
2. **DRY** — dedupe when duplication hides a single rule; do not abstract on first repetition.
3. **Readability over cleverness** — explicit `match` beats dense combinators; clear names beat short ones.
4. **One responsibility per file / type / crate** — if you need “and” to describe it, split it.
5. **Boundaries first** — traits at edges (audio, wake, model, tools); keep the state machine boring.
6. **Errors are values** — `thiserror` for libraries, `anyhow` only at binary edges; never `unwrap()` in library code except documented invariants in tests.
7. **No speculative frameworks** — add a dependency when a milestone needs it, not before.
8. **British or American English** — pick American for code/identifiers (`deserialize`, `behavior` in serde); stay consistent within a file for prose.
9. **Comments explain why** — not what the next line obviously does. Public APIs get short rustdoc.
10. **Security by structure** — allowlists, least privilege, no drive-by `Command::new("sh")`.

## Rust-specific

1. **Edition 2024** (or current stable edition at bootstrap) — one edition for the workspace.
2. **Workspace** — shared `clippy` / `rustfmt` / crate versions via workspace deps.
3. **`clippy::pedantic`** allowed with small, documented `allow`s — no blanket silence.
4. **Prefer `&str` / owned `String` clearly** — don’t hide allocations in surprising helpers.
5. **Async** — one runtime (`tokio`) chosen at the daemon binary; library crates stay runtime-agnostic where practical (`async-trait` sparingly).
6. **`Send + Sync`** bounds only where required; don’t sprinkle them to “make it compile later.”
7. **Avoid `unsafe`** — if needed (audio FFI), isolate in a tiny module with safety comments and tests.
8. **Feature flags** — for optional backends (e.g. `pipewire`, `mock-audio`), not for carving the app into spaghetti.
9. **Logging** — `tracing` with spans around state transitions and tool calls; no `println!` in libraries.
10. **Tests** — unit tests next to modules; state machine transitions get exhaustive tests; audio/wake use traits + mocks so CI doesn’t need a mic.
11. **File size** — prefer small modules. If a file grows past ~400–500 lines, look for a split before adding more.
12. **Naming** — `SnakeCase` files matching main type when there is one; avoid `utils.rs` / `helpers.rs` grab-bags. Name by domain (`state.rs`, `wake_gate.rs`).
13. **Clippy / fmt are law** — CI fails on either; no “I’ll format later.”

## Anti-patterns

- God daemon `main.rs` that owns audio + HTTP + tools + UI protocol
- Sharing a global `Mutex<App>` for everything
- Electron/webview mic path “just for the prototype”
- Copy-pasting a prior Python conductor into a subprocess and calling it done
- Hidden network calls inside soul markdown loading
- Wake word implemented only as “ask the cloud model if they said the name”

## Structure example (illustrative)

```text
crates/
  softwake-state/src/
    lib.rs
    transition.rs      # pure transitions + invariants
    ids.rs
  softwake-audio/src/
    lib.rs
    traits.rs
    pipewire.rs        # feature-gated
    mock.rs
```

## Commit hygiene

- One logical change per commit when practical.
- Messages say why.
- Never commit secrets, API keys, or personal `user.md` contents with private data.
