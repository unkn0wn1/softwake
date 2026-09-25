# ADR 0013 — Session chat on the selected provider

- **Status:** Accepted
- **Date:** 2026-09-25
- **Amended:** 2026-09-25 (budgeted FileMemory recall on ask/chat)

## Decision

While awake, `TextStubSession::ask` sends the instructions stored at `open` (the rendered context pack from [ADR 0011](ADR-0011-context-pack.md)) as the system message and the typed user line as the user message.

The completer is injected. The session crate has no HTTP client and no provider dependency.

Readiness uses `ProviderHandle` from [ADR 0012](ADR-0012-model-providers.md): Settings file present, last Test ok for the selected id, selected model in that provider's cache, bearer from the secret bag or the env fallback. Failures are fixed sentences. Ask does not hang and does not open a socket on those failures.

| Failure | What the operator sees |
|---------|------------------------|
| No Settings file | `No provider is configured. Choose a provider in Settings and run Test.` |
| Last Test missing or not ok | `Test has not succeeded for {id}. Run Test in Settings.` |
| No selected model, or the id is not in that provider's cache | `No chat model is selected. Choose one in Settings after Test.` |
| No bearer | `No xAI sign-in is configured.` / `No xAI API key is configured.` / `No OpenAI API key is configured.` |
| Transport failure | `Could not reach the provider.` |
| HTTP 401 or 403 | `Provider rejected the credentials.` |
| Other non-2xx | `Chat completion failed ({status}).` |
| Empty content | `The provider returned an empty reply.` |
| Unreadable content | `The provider reply could not be read.` |

`complete_chat` is the chat/completions call. `MockTransport` is the CI client. `LiveTransport::bounded` (30s connect, read, and overall) runs only when `softwake-daemon` is built with `live-http`.

The operator path is `softwaked demo`: `ask` and `chat`. Mic, STT, `serve`, and `ctl` are not on this path. Protocol generation stays 1.

One completion per ask. The request is the system message plus this user line. Prior turns are not replayed. User lines accumulate on the session for the awake period and are dropped on close. Assistant text is the return value and the demo line. It is not stored on the session.

Ask does not refresh OAuth. An expired access token surfaces as HTTP 401 with the rejection sentence above.

Assemble order for one ask: rendered pack → budgeted memory snippets → user turn. `TextStubSession::ask` takes a `memory_appendix` string (empty leaves the pack unchanged). The daemon builds that appendix with [`recall_for_prompt`](../crates/softwake-memory/src/recall.rs): at most 4 snippets, at most 2048 UTF-8 bytes of snippet text, query is the user line, oldest-first from `recall`, skip a hit that would blow the remaining byte budget. Disabled memory, missing `memory.json`, resolve/open/recall errors → empty appendix (fail-open). Appended after `render_instructions` so the runtime policy stub stays last among pack sections. Snippets do not override rules. `softwake-session` still does not depend on `softwake-memory`. `softwake-daemon` does.

OpenRouter and a custom OpenAI-compatible base URL stay deferred on ADR 0012's milestone.

Without `live-http`, a prepared disk ask returns `Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.` and does not open a socket. That check happens before the user line is recorded. An HTTP or parse error records the user line, because `ask` records and then calls the completer.

## Context

ADR 0012 stopped at credentials, Test, the picker, and the handle stub. The context pack already lands in the session on awake. The milestone is to wire awake session chat to the selected provider. The typed demo is the path that enters awake without a microphone.

## Alternatives

- Amend ADR 0012 in place. Rejected. Settings and the acting turn are different readers of the same handle.
- Put `ureq` inside `TextStubSession`. Rejected. The session could not be tested without a feature flag, and the daemon would no longer be the crate that chooses live HTTP.
- Require a protocol bump and `ctl ask` in the same PR. Rejected for this slice. The typed demo is the awake entry. A later additive message can sit on generation 1.
- Call the model with the registry seed when no model is selected. Rejected. Empty-until-Test is the Settings rule.
- Refresh OAuth inside ask. Rejected here. It writes the secret bag and adds a second call. A 401 is the operator-facing result.
- Attach unbounded `FileMemory` recall. Rejected. Budgeted recall (this amendment) keeps the prompt small and fail-open.

## How to demo

```bash
cargo test -p softwake-providers
cargo test -p softwake-session
cargo test -p softwake-daemon
cargo test --workspace
```

Those commands do not need `XAI_API_KEY`, `OPENAI_API_KEY`, or a network.

With Settings already tested and a model selected, the default demo still refuses the live call:

```bash
cargo run -p softwake-daemon -- demo
```

```text
> wake
> ask hello
rejected: Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.
```

To call the provider:

```bash
cargo run -p softwake-daemon --features live-http -- demo
```

```text
> wake
> ask hello
assistant: <model text>
```

`chat` is the same command. Sleep and hibernate refuse both.

## Consequences

- `softwake-daemon` depends on `softwake-providers`.
- Default `softwaked` still has no chat socket. A configured machine that wants a live answer rebuilds the daemon with `live-http`.
- Settings Test can still wait on a server that accepts and never sends a body (`LiveTransport::new`). Ask cannot; it uses `bounded`.
- `o`-series models that reject `max_tokens` fail with `Chat completion failed ({status}).` No special case in this slice.
- Assistant text is printed on the demo stdout. The bearer is not.
