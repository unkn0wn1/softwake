# ADR 0012 — Model providers

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25. Session chat is [ADR 0013](ADR-0013-session-provider.md). OpenRouter and OpenAI-compatible (API key + base URL) join the Settings provider list. Provider secrets prefer the OS keyring, with an opt-in plaintext fallback. This record stays the Settings, Test, and secret-bag decision.

## Decision

Model credentials and the Settings Test path live in `softwake-providers`.

Five credential kinds ship in Settings:

| Id | Label | Credential |
|----|-------|------------|
| `xai-oauth` | xAI sign-in | Device-code OAuth at `auth.x.ai` |
| `xai-key` | xAI API key | API key |
| `openai` | OpenAI | API key |
| `openrouter` | OpenRouter | API key |
| `openai-compatible` | OpenAI-compatible | API key plus a configured base URL |

Softwake has one acting model role. There is no separate Voice default in this crate. On-device wake and awake STT stay in [ADR 0006](ADR-0006-on-device-wake.md) and [ADR 0007](ADR-0007-awake-stt-tts.md).

### Sign-in and Test

- **xAI OAuth** uses the public device-code client id `b1a00492-073a-47ea-816f-4c329264a828`. There is no client secret. The id is safe to commit. Device code: `https://auth.x.ai/oauth2/device/code`. Token: `https://auth.x.ai/oauth2/token`. Scope: `openid profile email offline_access grok-cli:access api:access`. API base: `https://api.x.ai/v1`.
- **API keys** are pasted in Settings (or later ctl). A saved key wins over `XAI_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` / `OPENAI_COMPATIBLE_API_KEY`. The environment applies when that provider has no saved key.
- **OpenAI-compatible** also requires a non-secret base URL in Settings (for example `http://127.0.0.1:11434/v1`). Softwake does not append `/v1`. Test and chat call `{base}/chat/completions` and `{base}/models`.
- **OpenRouter** uses `https://openrouter.ai/api/v1`.
- **Test** resolves a bearer token (and base URL when needed), probes chat with a one-token completion, then `GET …/models` for that family. The model dropdown stays empty until Test stores a catalog for the selected provider. A failed Test does not invent ids and does not clear a previous catalog.
- Seed chat models (`grok-4.5` for xAI, `gpt-4.1-mini` for OpenAI, `openai/gpt-4.1-mini` for OpenRouter, `gpt-4.1-mini` for OpenAI-compatible) are registry hints only. They appear in the picker only when Test's catalog includes them, or when a passing Test received an empty chat list after a successful probe (seed fallback).

### Secrets

Tokens and API keys never enter git. Non-secret Settings (selected provider id, selected model id, cached model lists) are JSON under `$XDG_CONFIG_HOME/softwake/providers.json` (or `~/.config/softwake/providers.json`). Secrets are a separate bag under `$XDG_STATE_HOME/softwake/secrets.json` (or `~/.local/state/softwake/secrets.json`), file mode `0600`, parent dir mode `0700` when this crate creates them.

Version 1 of the bag was plaintext at rest. That file is still read. The current store prefers the OS keyring: Linux Secret Service, and macOS Keychain or Windows Credential Manager through the same crate (those two are not tested in CI). In keyring mode the file is a pointer with `version: 2`, `plaintext: false`, `backend: "keyring"`, and no secret fields. One secret-service item holds the five secret fields, service `softwake`, user `secret-bag`.

Plaintext remains an opt-in fallback (`backend: "plaintext"`, `plaintext_opt_in: true`, loud warning) when the service is unavailable, or when `SOFTWAKE_SECRET_BACKEND=plaintext`. `SOFTWAKE_SECRET_BACKEND=keyring` forces the service and fails closed. Unset means auto. A version-1 file migrates on the first resolved load when the probe succeeds. A pointer is never rewritten as plaintext. The window shows `storage_backend` and `storage_message` and does not receive secret values. Probe errors stay redacted. The renderer / window script must not log keys or tokens. Probe error messages redact response bodies.

### HTTP and CI

[`Transport`](../crates/softwake-providers/src/transport.rs) is the HTTP seam. [`MockTransport`](../crates/softwake-providers/src/transport.rs) is what unit tests use. Live `ureq` calls sit behind the crate feature `live-http`. The default workspace build and `cargo test --workspace` do not enable that feature and do not open a socket from this crate. Device-code and probe request bodies are built and parsed without a network.

### UI and daemon

The thin Settings panel in `softwake-ui` calls Tauri commands that use this crate in the UI process. IPC protocol generation stays `1`. Provider commands are not socket commands. [`ProviderHandle`](../crates/softwake-providers/src/handle.rs) is the read-only view of the selected provider, selected model, and bearer lookup. Session chat on that handle is [ADR 0013](ADR-0013-session-provider.md).

## Context

Phase 4 needs provider sign-in after the context pack ([ADR 0011](ADR-0011-context-pack.md)). Connectors already keep live cloud clients out of the default build ([ADR 0008](ADR-0008-connector-boundary.md)). Model access follows the same rule: library types and mock transport first, live HTTP opt-in, CI key-free.

Softwake is an acting conductor, not a dual Voice/AI recorder. One provider and one chat model are enough for the session prompt. Wake and STT stay local.

## Alternatives

- Keep OpenRouter and OpenAI-compatible deferred. Superseded by this amendment: Settings now lists both after OpenAI.
- OS keyring as the only store. Rejected for v1. Keyring needs a session secret service in CI and on headless boxes. The XDG file with a plaintext warning is demable offline.
- Encrypt-at-rest with a machine-local key. Rejected in this amendment. It would add a second crypto stack when the keyring plus a plaintext opt-in already covers the headless case.
- `keyring` 4.x. Rejected for this slice. That line raises the workspace MSRV and splits stores into separate crates.
- A process-global keyring mock in CI. Rejected. Tests run in parallel, so the fake client is an owned in-memory store.
- Put provider IPC on the daemon socket and bump protocol generation. Rejected. Settings can use Tauri commands against this crate without a socket change. A later move into the daemon can add additive commands on generation 1.
- Populate the model picker from the registry before Test. Rejected. Empty until Test matches the product rule.
- Wire `TextStubSession` to a live chat client. Rejected. This slice stops at credentials, Test, and the picker.
- Copy an Electron IPC shape wholesale. Rejected. Softwake is Rust/Tauri; the OAuth URLs and Test-then-models behaviour are what transfer.

## How to demo

```bash
cargo test -p softwake-providers
cargo test --workspace
```

Those commands do not need a keyring daemon. A headless plaintext demo sets `SOFTWAKE_SECRET_BACKEND=plaintext`.

With a real key (not in CI):

```bash
cargo test -p softwake-providers --features live-http -- --ignored --skip live_secret_service
```

The skip leaves out the ignored Secret Service round-trip. That test is not part of `cargo test --workspace`.

In the window: open Settings, pick a provider, paste a key or start xAI sign-in, press Test, then choose a model from the filled dropdown.

## Consequences

- A new workspace crate and a Settings section in the window.
- A machine with Secret Service stores provider secrets in the keyring after migration. A machine without it keeps the plaintext file only after opt-in (a pre-existing version-1 file counts). Env keys still apply when no secret is saved.
- Typed session chat is [ADR 0013](ADR-0013-session-provider.md). Live connectors stay open Phase 3 / Phase 4 items.
- OpenRouter and OpenAI-compatible base URL are part of this ADR (amended). Live connector work stays open.
