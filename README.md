# Softwake

Voice-first local conductor: asleep until hailed, awake with tools, hibernate when you want silence.

Rust end-to-end (daemon + Tauri UI). Phase 1 nails reliable **wake / sleep / hibernate** and a single safe tool loop. Phase 2 adds a confirmation gate for one risky tool. Personality and rules live in a **soul pack** (`soul.md`, `user.md`, `rules.md`, `glossary.md`) with multi-profile support under Settings → Profiles ([ADR 0017](docs/ADR-0017-profiles.md)). Long-term memory is a separate local trait ([ADR 0009](docs/ADR-0009-long-term-memory.md)), off by default, and not required to run the daemon.

## Status

Cargo workspace on stable Rust (edition 2024). `softwake-state` implements sleep / awake / hibernate, including rejected transitions and phrase cooldowns. `softwake-audio` has a mock capture backend (`stop` ends frame delivery) and a trait-shaped PipeWire stub (default `pipewire` feature, no native library). `softwake-wake` scores configured phrases with a local text matcher ([ADR 0002](docs/ADR-0002-wake-engine-spike.md)). The production on-device wake engine is sherpa-onnx keyword spotting ([ADR 0006](docs/ADR-0006-on-device-wake.md)). While awake, STT/TTS use a local streaming boundary ([ADR 0007](docs/ADR-0007-awake-stt-tts.md)): mock inject/record by default, sherpa stubs behind features, and an optional xAI press-to-talk / listen-while-awake path (voice Eve) behind `live-http`. Weights are not in the repo, and the demo stays typed. `softwake-soul` loads `soul.md`, `user.md`, `rules.md`, and `glossary.md`, checks them, and renders system instructions ([ADR 0011](docs/ADR-0011-context-pack.md)). `softwaked serve` speaks newline-delimited JSON on a Unix socket ([ADR 0003](docs/ADR-0003-ipc-transport.md)). `softwaked ctl` and the Tauri window `softwake-ui` are clients of that socket. Entering awake opens a text session with the rendered soul instructions. `echo` runs immediately while awake ([ADR 0004](docs/ADR-0004-first-safe-tool.md)). `notify` waits for confirmation and then appends a line to an in-memory sink. `shell` is confirm-gated and off until Settings → Tools enables it ([ADR 0018](docs/ADR-0018-tools-settings-shell.md), [ADR 0005](docs/ADR-0005-tool-confirmation.md)). Phase 3 wires the connector boundary to the tool bus ([ADR 0008](docs/ADR-0008-connector-boundary.md)): `email_send` waits for confirmation, and confirming it appends one message to an in-memory outbox (or a local draft when live email is opted in and draft-only). `MockDrive` and `MockCalendar` list files and events stored on that value. In the connector registry, `drive` / `list` and `calendar` / `list` are confirm, and delete actions are denied. Those list mocks are not tools on the bus. The default build has no live cloud client. `softwake-memory` is a `Memory` trait, an in-memory `MockMemory`, and an opt-in `FileMemory` that writes `memory.json` only after `open_enabled`. Both stay off until that value is enabled. When `memory.json` exists under the Softwake state directory, awake `ask` / `chat` attach a budgeted recall appendix after the rendered pack ([ADR 0009](docs/ADR-0009-long-term-memory.md), [ADR 0013](docs/ADR-0013-session-provider.md)). Missing or disabled memory is fail-open (no snippets). `softwake-policy` classifies the existing tool and connector allowlists. Unknown names are denied. The daemon uses that classification, and its override map is empty ([ADR 0010](docs/ADR-0010-policy-engine.md)). `softwake-providers` holds xAI device-code OAuth, xAI API key, OpenAI API key, OpenRouter API key, and OpenAI-compatible (key + base URL) Settings ([ADR 0012](docs/ADR-0012-model-providers.md)). Secrets stay in an XDG state bag (plaintext v1 with a warning). The window Settings panel runs Test, then fills the model picker. Mock transport keeps default tests offline; `live-http` enables real HTTPS. While awake, typed `ask` and `chat` in `softwaked demo` send the rendered context pack and the user line to the selected provider ([ADR 0013](docs/ADR-0013-session-provider.md)). Default tests use `MockTransport`. The daemon `live-http` feature performs the real call and is off by default. `softwaked ctl ask` and `ctl chat` send that same turn to a running `softwaked serve`. `softwaked ctl wake` enters awake on a running `softwaked serve` when the four-file pack is valid. `ctl resume` still lands in sleep. Serve can open a real mic with `pipewire-capture`; wake-from-voice needs `sherpa-kws` + installed weights. Protocol generation stays 1. OpenRouter and OpenAI-compatible base URL are available in Settings. Budgeted memory snippets on ask/chat are shipped. A missing or invalid four-file pack refuses awake; `reload-soul` re-reads that pack and applies on the next awake.

## Build and test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --all-targets
```

See [Cargo features](#cargo-features) for `pipewire`, `pipewire-native`, and `sherpa-kws`.

## Releases

Tagged builds (`v*`) publish a Linux **AppImage** (preferred portable), a Linux **tar.gz** with `install-linux.sh` (user-prefix + systemd --user), a Windows **setup.exe**, and a Windows **portable** zip. Each package includes both `softwaked` and `softwake-ui`. See [docs/releases.md](docs/releases.md) for download, install, the OS feature matrix, and how to cut a tag. Design: [ADR 0019](docs/ADR-0019-multiplatform-releases.md).

## Cargo features

| Crate | Feature | Default | What it compiles |
|-------|---------|---------|------------------|
| `softwake-audio` | `pipewire` | yes | Capture stub. Does not link `libpipewire`. |
| `softwake-audio` | `pipewire-native` | no | Links `libpipewire` and opens the default input at 16 kHz mono. Not enabled in CI. Linux only. |
| `softwake-audio` | `wasapi` | yes | WASAPI capture stub for Windows. Does not open a device. |
| `softwake-wake` / `softwake-daemon` | `sherpa-kws` | no | Real sherpa-onnx KWS when weights exist under `$XDG_DATA_HOME/softwake/kws`. No weights in git. Not enabled in CI. |
| `softwake-voice` | `sherpa-asr` | no | Streaming ASR stub. No weights and no ONNX download. Not enabled in CI. |
| `softwake-voice` | `sherpa-tts` | no | TTS stub. No weights and no synthesizer download. Not enabled in CI. |
| `softwake-providers` | `live-http` | no | Real HTTPS via `ureq` for OAuth and Test. Unit tests use `MockTransport`. Not required for `cargo test -p softwake-providers`. |
| `softwake-daemon` | `pipewire-capture` | no | Real mic via `softwake-audio/pipewire-native`. Use `serve --capture pipewire`. Not enabled in CI. Linux only. |
| `softwake-daemon` | `wasapi-capture` | no | WASAPI stub via `softwake-audio/wasapi`. Use `serve --capture wasapi` (fails until native). |
| `softwake-daemon` | `live-http` | no | Enables `softwake-providers/live-http` so typed `ask` / `chat` can call the selected provider. The default build rejects that call and does not open a socket. |
| `softwake-ui` | `live-http` | yes | Enables `softwake-providers/live-http` so Settings Test and xAI sign-in can reach the network. |

`--no-default-features` on `softwake-audio` omits the `pipewire` stub. The sherpa-onnx keyword-spotting choice is [ADR 0006](docs/ADR-0006-on-device-wake.md).

```bash
cargo test -p softwake-audio --features pipewire-native
cargo test -p softwake-wake --features sherpa-kws
cargo test -p softwake-voice --features sherpa-asr,sherpa-tts
```

Real capture (`pipewire-native` / daemon `pipewire-capture`) needs `libpipewire-0.3-dev` at build time and a PipeWire session at run time. The default build does not link that library.

## Install

From the repository root, using the workspace `Cargo.lock`:

```bash
cargo install --path crates/softwake-daemon --locked
```

That installs the `softwaked` binary. The window binary is `softwake-ui` (Settings + system tray + HUD). From a checkout, start `softwaked serve`, then:

```bash
cargo run -p softwake-ui
```

The tray stays while Settings is closed. The HUD capsule is always-on-top at the **bottom-right of the primary screen**, tiny until you click it; then the type strip expands. Any non-empty submit wakes Softwake when it is asleep, runs ask, and shows the assistant reply (or a clear error) in the strip. Spaces work in the field.

The same lockfile install for the window:

```bash
cargo install --path crates/softwake-ui --locked
```

Linux packages for the window are listed under [Window](#window). Socket and soul paths are under [Serve and ctl](#serve-and-ctl).

[`packaging/softwake.desktop`](packaging/softwake.desktop) is a sample launcher. `Exec` is `softwake-ui` and `Icon` is the theme name `softwake`. After `softwake-ui` is on `PATH`, copy the file to `~/.local/share/applications/` and set `Icon=` to an icon you provide.

## Demo

`softwaked` with no arguments prints the initial voice state and exits:

```bash
cargo run -p softwake-daemon
```

```text
softwaked state: sleep
```

The interactive demo is typed commands only. The microphone is not opened. Mock capture is the default, and native PipeWire is an optional feature that is not linked in the default build. It starts in sleep with mock capture running. `wake` and `sleep` submit the configured phrases to the text detector, then apply the voice-state machine, including the 800 ms phrase cooldown. `hibernate` stops capture. A voice command is rejected until `resume`, which returns to sleep and starts capture again. `wake` also requires a valid soul pack. Copy the repo templates into the config directory first (or pass `--soul-dir`):

```bash
mkdir -p ~/.config/softwake/soul
cp soul/*.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- demo
```

```text
softwaked demo
typed commands only — mock capture; native PipeWire is feature-gated
state: sleep
capture: running
soul: ok
commands: wake, sleep, hibernate, resume, status, reload-soul, tool, confirm, cancel, hear, say, ask, chat, quit
>
```

A directory that only has `soul.md` and `user.md` refuses awake until `rules.md` and `glossary.md` are copied too. Without those files the status line is `soul: missing` and `wake` stays in sleep. `hibernate`, `resume`, and `sleep` still run. `reload-soul` reads the directory again. `tool` is refused until a wake succeeds.

Wake, run the tool, sleep, then hibernate:

```text
> wake
heard: "hey softwake" -> wake
transition sleep -> awake (wake phrase)
effect: open session
state: awake
capture: running
soul: ok
> tool echo hello
tool echo: echo: hello
state: awake
capture: running
soul: ok
last tool: echo safe ran
> tool echo
tool echo: pong
state: awake
capture: running
soul: ok
last tool: echo safe ran
> sleep
heard: "softwake sleep" -> sleep
transition awake -> sleep (sleep phrase)
effect: release acting resources
state: sleep
capture: running
soul: ok
last tool: echo safe ran
> hibernate
transition sleep -> hibernate (UI hibernate)
effect: stop capture
state: hibernate
capture: stopped
soul: ok
last tool: echo safe ran
```

`sleep` in the 800 ms after `wake` stays awake, so a fast paste of `wake` then `sleep` will not leave awake. `hibernate` from awake does not use that cooldown. It closes the session and stops capture. `tool volume` is rejected. `tool shell` is denied until Tools → shell is enabled; then it confirm-gates. `tool` while asleep or hibernating is rejected.

While awake, inject a mock STT transcript or record mock TTS (no microphone, no model download — [ADR 0007](docs/ADR-0007-awake-stt-tts.md)):

```text
> hear hello there
partial transcript: "hello there"
final transcript: "hello there"
state: awake
capture: running
soul: ok
> say hello
said: "hello"
state: awake
capture: running
soul: ok
last said: hello
```

`hear` and `say` are refused while asleep or hibernating. Sleep and hibernate drop any queued mock STT and do not speak.

While awake, `ask` and `chat` send the rendered context pack and the typed line to the provider chosen in Settings ([ADR 0013](docs/ADR-0013-session-provider.md)):

```text
> ask hello
assistant: <model text>
state: awake
capture: running
soul: ok
```

`chat` is the same command. The same turn against a running daemon is `softwaked ctl ask` or `softwaked ctl chat` once that `softwaked serve` process is awake. `softwaked ctl wake` enters awake on that serve when the four-file pack is valid, and `ctl resume` still lands in sleep. Serve can open a real mic with `pipewire-capture`; wake-from-voice needs `sherpa-kws` + installed weights. See [Serve and ctl](#serve-and-ctl). Without the daemon `live-http` feature, a fully configured Settings file still gets `Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.` and does not open a provider socket. With the feature:

```bash
cargo run -p softwake-daemon --features live-http -- demo
```

Then `wake`, then `ask hello`.

Confirm a notification, or cancel one:

```text
> wake
> tool notify hello
pending 1: notify — Append a notification to the in-memory sink.
waiting for confirm
state: awake
capture: running
soul: ok
pending: 1 notify hello
last tool: notify confirm pending
> confirm
confirmed 1: tool notify: hello
state: awake
capture: running
soul: ok
last tool: notify confirm confirmed
notification: hello
> tool notify later
> cancel
cancelled 2: notify
> sleep
```

`confirm-tool 1` is the same confirm with an explicit id. `cancel-tool 1` is the same cancel. The notification line is appended only after confirm. Sleep and hibernate drop a pending confirmation. `sleep` in the 800 ms after `wake` stays awake, including in this confirm example.

Send one in-memory message the same way:

```text
> wake
> tool email_send ada@example.com hello a short note
pending 1: email_send — Append one message to the in-memory outbox.
waiting for confirm
state: awake
capture: running
soul: ok
pending: 1 email_send ada@example.com hello a short note
last tool: email_send confirm pending
> confirm
confirmed 1: tool email_send: sent 1
state: awake
capture: running
soul: ok
last tool: email_send confirm confirmed
email: ada@example.com | hello | a short note
```

The outbox line appears only after `confirm`. `cancel` prints `cancelled 1: email_send` and adds no `email:` line. A call with no body is rejected and does not wait for confirm.

A `> ` prompt is printed before each line is read. The same path accepts a pipe (`printf 'wake\nstatus\nquit\n' | cargo run -p softwake-daemon -- demo`). `softwaked --demo` is the same mode.

`softwaked demo --verbose` and `softwaked demo -v` (also `--demo -v`) print extra `verbose:` lines for each command: the raw input, the parsed command, for `wake` / `sleep` the phrase, the detector hit, and whether the transition succeeded or why it was rejected, and for `ask` / `chat` the provider id and model id. The bearer is not printed. `SOFTWAKE_LOG=debug` enables that same detail. `softwaked --help` prints usage.

Type one command per line. `sleep` in the 800 ms after `wake` stays awake. `wake` in the 800 ms after `sleep` or `resume` stays asleep. `hibernate` is a UI command and applies on the next line.

## Model providers

Settings in `softwake-ui` configure one acting provider ([ADR 0012](docs/ADR-0012-model-providers.md)). That panel is the Providers pane in the window:

1. Choose **xAI sign-in**, **xAI API key**, **OpenAI**, **OpenRouter**, or **OpenAI-compatible**.
2. For a key provider, paste the key and press **Save key**. For **OpenAI-compatible**, also set the **Base URL** (for example `http://127.0.0.1:11434/v1`) and press **Save base URL**. For xAI sign-in, press **Start sign-in**. Softwake opens the verification page in the default browser and shows that address as a link next to the user code. Enter the code on that page, then **Poll** (or wait for the automatic poll). If the browser does not open, use the link in Settings.
3. Press **Test**. On success, the **Chat model** and **Voice model** dropdowns fill from `GET /v1/models` (chat vs speech-to-text split, with a registry seed fallback). Both stay empty until Test succeeds.
4. Pick a chat model (the acting session) and a voice / STT model. On xAI, pick a **TTS voice** (empty uses Eve). Other providers leave TTS disabled. Live press-to-talk is [ADR 0007](docs/ADR-0007-awake-stt-tts.md) and needs the daemon `live-http` feature.

Secrets are stored under `$XDG_STATE_HOME/softwake/secrets.json` (or `~/.local/state/softwake/secrets.json`), mode `0600`. When the OS keyring answers (Linux Secret Service; macOS Keychain and Windows Credential Manager through the same crate), that file is a version-2 pointer and the bag is one keyring item (`softwake` / `secret-bag`). Plaintext is an opt-in fallback (`SOFTWAKE_SECRET_BACKEND=plaintext`, or the Settings button when the keyring is unavailable) and still shows a warning. An existing version-1 file migrates on the first resolved load when the keyring probe succeeds. A pointer is never rewritten as plaintext. `SOFTWAKE_SECRET_BACKEND=keyring` fails closed when the service is down. Non-secret selection and the model cache are `$XDG_CONFIG_HOME/softwake/providers.json`. The public xAI device-code client id is safe to commit; refresh tokens and API keys are not. Environment fallbacks: `XAI_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `OPENAI_COMPATIBLE_API_KEY` when no key is saved.

```bash
cargo test -p softwake-providers
```

Default workspace tests do not call the network. Live HTTPS is the `live-http` feature.

## Serve and ctl

`softwaked serve` keeps the voice-state machine and mock capture running and listens for clients. `softwaked --serve` is the same mode. A stale socket file is removed on startup. If another serve is already listening, startup fails and leaves that socket in place.

```bash
cargo run -p softwake-daemon -- serve
```

In another terminal:

```bash
cargo run -p softwake-daemon -- ctl status
cargo run -p softwake-daemon -- ctl hibernate
cargo run -p softwake-daemon -- ctl status    # hibernate, capture stopped
cargo run -p softwake-daemon -- ctl resume    # back to sleep; does not enter awake
cargo run -p softwake-daemon -- ctl wake      # sleep -> awake; needs a valid soul pack
cargo run -p softwake-daemon -- ctl sleep     # rejected while already asleep
cargo run -p softwake-daemon -- ctl reload-soul
cargo run -p softwake-daemon -- ctl tool echo hello
cargo run -p softwake-daemon -- ctl tool notify hello
cargo run -p softwake-daemon -- ctl confirm-tool 1
cargo run -p softwake-daemon -- ctl cancel-tool 1
# after that same daemon is awake:
cargo run -p softwake-daemon -- ctl ask hello
cargo run -p softwake-daemon -- ctl chat hello there
```

A live answer is opt-in and is not what CI runs:

```bash
cargo run -p softwake-daemon --features live-http,pipewire-capture -- serve --capture pipewire
```

Press-to-talk (mic in, Eve out) uses that same process ([ADR 0007](docs/ADR-0007-awake-stt-tts.md)):

1. In Settings, select **xAI sign-in** or **xAI API key**, save the credential, press **Test**, pick a chat model, and leave **TTS voice** empty (Eve) or pick another built-in xAI voice.
2. Start serve with `live-http` and `pipewire-capture` as above. `ffplay` or `mpv` must be on `PATH` to hear the reply.
3. Open the HUD, expand it, and **hold** the mic button while you speak. Release returns the mic button to idle immediately; Softwake shows **thinking…** while STT → ask → Eve run in the background. The bloom must keep animating while she thinks and while Eve speaks (playback is fire-and-forget; status polls use a non-blocking snapshot so the particle loop never waits on the daemon lock).
4. **Free speech while awake:** with the daemon on `live-http` + `pipewire-capture` and Softwake **awake** (typed ask, `softwaked ctl wake`, or PTT-from-sleep), just speak — silence-gated utterances run the same STT → ask → Eve path without holding PTT. Hint shows “speak or hold”. PTT still wins if you hold. Sleep→awake by voice needs `sherpa-kws` + installed weights (see Voice wake / sleep).
5. Drag the capsule (bloom area, not the mic/type controls) to reposition it. Softwake remembers the spot in `hud-position.json` under the Softwake config dir and will not yank it back to the primary bottom-right on expand/collapse. Delete that file (or call reset) to park at bottom-right again. On first open (no save), the HUD must spawn at **primary bottom-right**, not centred over Settings.
5. A typed HUD ask speaks the same way when the provider is xAI and `live-http` is on.

Default `cargo test --workspace` does not open a microphone and does not call STT or TTS. Phrase spotting (“hey Softwake” / profile name) needs `sherpa-kws` + installed weights.

`ctl` prints `state`, `capture`, `soul` (`ok` or `missing`), and `soul reload`, and exits non-zero when the daemon rejects the command or cannot be reached. `ctl ask` and `ctl chat` send one line to that daemon. The reply is the line after the status lines. `ctl wake` enters awake from sleep when `soul.md`, `user.md`, `rules.md`, and `glossary.md` are valid. A missing or invalid file refuses `ctl wake` and leaves the voice state unchanged. Hibernate, sleep, and resume still run. After `ctl sleep` or `ctl resume`, a wake phrase waits out the 800 ms cooldown. `ctl resume` still lands in sleep. Serve can open a real mic with `pipewire-capture`; wake-from-voice needs `sherpa-kws` + installed weights. `ctl ask` still requires that same process to be awake. Without `live-http`, a ready Settings file still gets `Live HTTP is not enabled in this build. Re-run with the live-http feature to call the provider.` `resume` is wake-from-hibernate and lands in sleep. `reload-soul` re-reads the four-file pack from disk. The new text applies on the next awake, not in the middle of a session that is already awake. `ctl tool` runs one safe tool, or stages a confirm-gated tool. The daemon starts in sleep, so `ctl tool echo hello` is refused until the daemon is awake. Serve has no microphone path into awake. A successful `echo` prints its result on the line after the status lines (`echo: hello`, or `pong` when `echo` has no arguments). `ctl tool notify hello` prints the pending id and does not append. `ctl tool email_send ada@example.com hello body` does the same, and `ctl confirm-tool <id>` appends one in-memory message. `ctl cancel-tool <id>` drops the pending call.

The socket path is the first match of `--socket PATH`, `SOFTWAKE_SOCKET`, `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, and `/tmp/softwake-$UID/softwaked.sock` when `XDG_RUNTIME_DIR` is unset.

The soul directory is the first match of `--soul-dir PATH` (on `serve` and `demo`), `SOFTWAKE_SOUL_DIR`, `$XDG_CONFIG_HOME/softwake/soul`, and `~/.config/softwake/soul` when `XDG_CONFIG_HOME` is unset. See [docs/03-soul-pack.md](docs/03-soul-pack.md).

## Window

`softwake-ui` opens a **system tray** icon, a small **always-on-top HUD capsule**, and a resizable Settings window (860 by 680). Closing Settings hides it; Quit from the tray exits. The HUD parks at the **bottom-right of the primary monitor** by default (always-on-top above Settings), stays collapsed (bloom only) until clicked, then expands to a fixed size with the type strip, a press-and-hold mic button, and the reply. Drag the bloom to move it; Softwake persists that spot and stops re-anchoring until `hud-position.json` is cleared. Settings left-nav shows **exactly one** content pane at a time. Particles follow capture level while listening ([ADR 0015](docs/ADR-0015-tray-hud.md), [ADR 0016](docs/ADR-0016-capture-level-hud.md)): the daemon sends peak-normalized RMS on `Status` when PCM is scored, and the UI falls back to a local sine only when that field is absent. A left nav has four panes. Status is selected when the Settings window opens.

**Status** shows the daemon state, whether capture is running, whether the soul pack is `ok` or `missing` (and the reason when the daemon sent one), whether a soul reload is pending, the latest tool line, and a confirm-gated tool when one is waiting. Buttons are Hibernate, Wake (leave hibernate into sleep), Sleep, Reload soul, Confirm, and Cancel. The Status Wake button is `wake_from_ui` / `ctl resume` (hibernate → sleep). `softwaked ctl wake` is the separate command that enters awake from sleep and requires a valid soul pack. Reload reads `soul.md`, `user.md`, `rules.md`, and `glossary.md`. The new text applies on the next awake. The status snapshot does not include the socket path. The window uses the same default socket as `softwaked ctl`. Start `softwaked serve` first. Provider commands are not socket commands.

**Providers** is the model Settings panel ([ADR 0012](docs/ADR-0012-model-providers.md)): choose a provider, save a key or sign in, press Test, then pick a chat model and a voice (STT) model. Both lists stay empty until Test succeeds. The **TTS voice** list is the xAI built-in roster (Eve is the default). It is disabled for other providers.

**General** edits `soul.md`, `user.md`, `rules.md`, and `glossary.md` in the resolved soul directory (`SOFTWAKE_SOUL_DIR`, or the XDG default). Save writes the four files. Reload soul applies a valid pack on the next awake.

**Email** is an opt-in live scaffold (off by default). Save SMTP fields and a password into the secret bag, press Test (no socket), and keep mode on draft-only unless you accept the not-wired send scaffold. The pane does not send mail; confirm-gated `email_send` on Status still owns send/draft after awake confirm.

```bash
cargo run -p softwake-ui
```

On Linux the window links WebKitGTK. The packages used in CI are `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `patchelf`, `libxdo-dev`, and `libssl-dev`.


## Trying audio

Default builds stay mic-free: mock capture, no `libpipewire`, no sherpa weights.

1. Copy the soul templates if needed, then start the daemon:

```bash
mkdir -p ~/.config/softwake/soul
cp soul/*.md ~/.config/softwake/soul/
cargo run -p softwake-daemon -- serve
```

2. In another terminal, open the UI (tray + HUD):

```bash
cargo run -p softwake-ui
```

3. Leave Softwake in **sleep** or **awake** (capture running). The HUD particles should breathe from **daemon capture levels** (mock listening tones scored as RMS). `softwaked ctl status` prints `capture level: …` while listening. Hibernate stops capture; the level disappears and particles quiet down.

4. Real microphone (HUD particles from your voice). Needs `libpipewire-0.3-dev`, a PipeWire session, and stays out of the default CI job:

```bash
# terminal A — daemon with live capture
cargo run -p softwake-daemon --features pipewire-capture -- serve --capture pipewire

# terminal B — UI (particles follow Status.capture_level)
cargo run -p softwake-ui

# optional: watch level from ctl
softwaked ctl status
# or: SOFTWAKE_CAPTURE=pipewire cargo run -p softwake-daemon --features pipewire-capture -- serve
```

Speak into the default input; the HUD capsule particles should bloom with your voice.

### Voice wake / sleep (KWS)

1. Install weights (once): `./scripts/install-kws-weights.sh` → files under `$XDG_DATA_HOME/softwake/kws` (or `~/.local/share/softwake/kws`).
2. Build with KWS + mic: `cargo build -p softwake-daemon --features sherpa-kws,pipewire-capture` (and the UI as usual).
3. Set the active profile **name** in Settings → Profiles (e.g. `Ada`). That name is the primary wake word; `hey Softwake` / `Softwake` remain fallbacks.
4. Start serve with PipeWire capture; leave Softwake in **sleep**.
5. Say the profile name (or “hey Softwake”) → awake. Say “go to sleep” or “goodnight &lt;name&gt;” → sleep.
6. Free speech / PTT / Eve TTS while awake are unchanged; sleep-phrase KWS still runs so you can dismiss by voice.

Without weights or without `sherpa-kws`, PCM stays on `NullDetector` and CI stays mic-free. See [ADR 0006](docs/ADR-0006-on-device-wake.md). Download is operator-consent only; weights are not vendored.

```bash
cargo test -p softwake-audio --features pipewire-native
cargo test -p softwake-daemon --features pipewire-capture
```

## Docs

| Doc | Purpose |
|-----|---------|
| [docs/00-overview.md](docs/00-overview.md) | What it is, non-goals, phases |
| [docs/01-architecture.md](docs/01-architecture.md) | Processes, IPC, audio, tools |
| [docs/02-voice-states.md](docs/02-voice-states.md) | Sleep / wake / hibernate |
| [docs/03-soul-pack.md](docs/03-soul-pack.md) | Four-file context pack; memory is [ADR 0009](docs/ADR-0009-long-term-memory.md) |
| [docs/04-coding-style.md](docs/04-coding-style.md) | KISS, DRY, SRP, Rust rules |
| [docs/05-quality-gates.md](docs/05-quality-gates.md) | CI and definition of done |
| [docs/06-milestones.md](docs/06-milestones.md) | Phase checklist |
| [docs/ADR-0001-name-and-scope.md](docs/ADR-0001-name-and-scope.md) | Name and phase-1 scope |
| [docs/ADR-0002-wake-engine-spike.md](docs/ADR-0002-wake-engine-spike.md) | Text phrase table for the wake spike |
| [docs/ADR-0003-ipc-transport.md](docs/ADR-0003-ipc-transport.md) | Unix socket and newline-delimited JSON |
| [docs/ADR-0004-first-safe-tool.md](docs/ADR-0004-first-safe-tool.md) | Why the first tool is `echo` |
| [docs/ADR-0005-tool-confirmation.md](docs/ADR-0005-tool-confirmation.md) | Safe, confirm, and deny tools |
| [docs/ADR-0006-on-device-wake.md](docs/ADR-0006-on-device-wake.md) | On-device wake engine (sherpa-onnx keyword spotting) |
| [docs/ADR-0007-awake-stt-tts.md](docs/ADR-0007-awake-stt-tts.md) | Awake speech-to-text and text-to-speech |
| [docs/ADR-0008-connector-boundary.md](docs/ADR-0008-connector-boundary.md) | Connector boundary: in-memory email (opt-in live scaffold), Drive, and calendar; confirm or deny |
| [docs/ADR-0009-long-term-memory.md](docs/ADR-0009-long-term-memory.md) | Long-term memory: local trait, in-memory mock, opt-in JSON file |
| [docs/ADR-0010-policy-engine.md](docs/ADR-0010-policy-engine.md) | Policy engine: one evaluation path, default deny, tighten-only overrides |
| [docs/ADR-0011-context-pack.md](docs/ADR-0011-context-pack.md) | Context pack and confirm-echo foundation |
| [docs/ADR-0012-model-providers.md](docs/ADR-0012-model-providers.md) | Provider Settings: xAI sign-in, API keys, Test, chat and voice/STT pickers |
| [docs/ADR-0013-session-provider.md](docs/ADR-0013-session-provider.md) | Awake session chat to the selected provider |
| [docs/ADR-0014-skills-hub.md](docs/ADR-0014-skills-hub.md) | Skills hub, refine loop, and webhook wake (direction) |
| [docs/ADR-0015-tray-hud.md](docs/ADR-0015-tray-hud.md) | System tray and always-on-top HUD |
| [docs/ADR-0016-capture-level-hud.md](docs/ADR-0016-capture-level-hud.md) | Capture level on Status → HUD particles |
