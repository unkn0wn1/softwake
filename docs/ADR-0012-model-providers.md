# ADR 0012 — Model providers

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25. Session chat is [ADR 0013](ADR-0013-session-provider.md). OpenRouter and OpenAI-compatible (API key + base URL) join the Settings provider list. This record stays the Settings, Test, and secret-bag decision.

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

v1 of the bag is **plaintext at rest** with a `plaintext: true` marker and a one-line warning on load and save. Softwake does not yet depend on an OS keyring crate. A later change may encrypt the bag or move entries into the session keyring; the file format and path stay documented here. The renderer / window script must not log keys or tokens. Probe error messages redact response bodies.

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
- Put provider IPC on the daemon socket and bump protocol generation. Rejected. Settings can use Tauri commands against this crate without a socket change. A later move into the daemon can add additive commands on generation 1.
- Populate the model picker from the registry before Test. Rejected. Empty until Test matches the product rule.
- Wire `TextStubSession` to a live chat client. Rejected. This slice stops at credentials, Test, and the picker.
- Copy an Electron IPC shape wholesale. Rejected. Softwake is Rust/Tauri; the OAuth URLs and Test-then-models behaviour are what transfer.

## How to demo

```bash
cargo test -p softwake-providers
cargo test --workspace
```

With a real key (not in CI):

```bash
cargo test -p softwake-providers --features live-http -- --ignored
```

In the window: open Settings, pick a provider, paste a key or start xAI sign-in, press Test, then choose a model from the filled dropdown.

## Consequences

- A new workspace crate and a Settings section in the window.
- Secrets can sit on disk in plaintext until a later encryption or keyring change. Operators who need stronger storage wait for that change or keep keys only in the environment and never save.
- Typed session chat is [ADR 0013](ADR-0013-session-provider.md). Live connectors stay open Phase 3 / Phase 4 items.
- OpenRouter and OpenAI-compatible base URL are part of this ADR (amended). Live connector work stays open.
