# ADR 0004 — First safe tool

- **Status:** Accepted
- **Date:** 2026-09-25

## Decision

Phase 1 ships one allowlisted tool, named `echo`.

`echo` with no arguments returns `pong`. With arguments it returns the prefix `echo:` and those arguments joined by a single space (`echo: hello world`). The arguments are data. They are not passed to a shell, a filesystem API, a clipboard API, or an audio API.

The phase-1 allowlist is exactly that name. `Allowlist::contains` is true only for `echo`. Any other name is rejected, including while awake.

A tool call is legal only while the voice state is awake. The daemon asks `permit_tool_dispatch` before it consults the allowlist. Sleep and hibernate refuse the call and do not run the tool. A refusal does not emit `tool_started` or `tool_finished`. A successful run emits `tool_started`, then `tool_finished`, then the response.

Entering awake opens a text session and stores the rendered soul instructions (identity, user profile, and the runtime policy that names `echo`). Sleep closes that session. Hibernate from awake closes it and then stops capture. The session does not call a model.

The wire message is an additive `tool_request`. Protocol generation stays `1`. [ADR 0003](ADR-0003-ipc-transport.md) records the frame.

## Context

The milestone exit is a demo someone else can run: wake, one safe tool, sleep, hibernate. The tool has to be deterministic in CI. A volume change needs PipeWire or Pulse, which this workspace does not link and which CI does not provide. A clipboard tool needs an OS selection API and a desktop session. Either one would make the test depend on a machine's audio graph or display server, and either one can change something the operator did not mean to change.

`echo` proves the gate (asleep refuses, awake runs, unknown name refuses) without those dependencies. A later tool can replace it once confirmation and a real side effect are in scope. The allowlist stays a single name until that ADR.

## Alternatives

- Set the system volume. Rejected. It needs a real audio server and it changes machine state.
- Read or write the OS clipboard. Rejected. It needs a desktop session and it can leak or overwrite the operator's clipboard.
- `notify_local` writing into an in-memory log. Acceptable, and not what shipped. `echo` is the same idea with a return value instead of a side log, which is easier to assert in a test.
- A shell runner behind the allowlist. Rejected. Phase 1 does not confirm dangerous tools, and a shell is not a safe default.

## How to demo

Copy the soul templates, then run the typed demo:

```bash
mkdir -p ~/.config/softwake/soul
cp soul/soul.md soul/user.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
> wake
> tool echo hello
> sleep
> hibernate
```

`sleep` inside the 800 ms phrase cooldown stays awake. `hibernate` from awake does not wait for that cooldown. It closes the session and stops capture.

`softwaked ctl tool echo hello` talks to a running `softwaked serve`. The daemon starts in sleep, so the call is refused until something has entered awake. The typed demo is the path that enters awake today. Serve does not yet feed capture into the wake engine.

## Consequences

- The soul pack runtime policy names `echo` and still marks confirm-rules as a placeholder.
- Adding a second tool means updating this allowlist, the policy line, and this ADR. A drive-by registration is a bug.
- `echo` will look trivial next to a real tool. That is the point of the first one: the gate is tested before the side effect exists.
