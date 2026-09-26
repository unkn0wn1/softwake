# ADR 0019 — Multi-platform builds and GitHub Releases

- **Status:** Accepted
- **Date:** 2026-09-26
- **Updated:** 2026-09-26 (AppImage, Windows setup/portable, Linux systemd --user)

## Decision

Softwake ships **linux-x86_64** and **windows-x86_64** through **GitHub Releases** on tags matching `v*`.

| Asset | Purpose |
| --- | --- |
| Linux **AppImage** | Preferred portable download; bundles `softwake-ui` + `softwaked`; UI autostarts the daemon when needed; **no** systemd |
| Linux **tar.gz** + `install-linux.sh` | User-prefix install (`~/.local` by default) and **systemd --user** unit for `softwaked` |
| Windows **setup.exe** (NSIS) | Per-user installer with both binaries |
| Windows **portable.zip** | Both executables + `start-softwake.bat`; no service |

`.deb` / `.msi` / macOS remain out of scope for this slice. A system-wide systemd unit is optional documentation only; the default install path does not require root.

### Platform inventory (Linux-first today)

| Area | Linux | Windows v1 |
| --- | --- | --- |
| IPC | Unix domain socket ([ADR 0003](ADR-0003-ipc-transport.md)) | TCP `127.0.0.1` + port file at the resolved path (or direct `host:port` via `SOFTWAKE_SOCKET` / `--socket`) |
| Capture | Mock default; `pipewire-capture` for real mic | Mock default; `wasapi-capture` compiles a **stub** (no device yet) |
| Wake / KWS | Optional `sherpa-kws` when weights are installed | Same feature flag if the crate builds; otherwise omit from the Windows release matrix and document |
| Tray / HUD | Tauri + Ayatana AppIndicator (CI packages) | Tauri tray/HUD compile; polished placement parity deferred |
| Config / secrets file modes | `0700` / `0600` where Softwake creates paths | Create/write without POSIX mode bits |
| Packaging | Tauri AppImage (`externalBin` softwaked) + tar.gz install script | Tauri NSIS + portable zip |

### Daemon lifecycle

- Packaged UI calls ensure-daemon on startup: if the socket/port is down, spawn sibling `softwaked serve` (unless `SOFTWAKE_NO_AUTOSTART` is set).
- Linux install script enables `systemd --user` `softwaked.service` so the daemon survives without the UI.
- AppImage stays portable and does not write systemd units.

### IPC

The public `Listener` / `Client` API stays path-oriented. On Windows the path is a **port file** (`%LOCALAPPDATA%\softwake\softwaked.port` by default) containing `127.0.0.1:PORT`. Protocol framing and hello remain generation `1`.

### Capture

[`AudioCapture`](../crates/softwake-audio/src/traits.rs) remains the seam. PipeWire stays Linux/`pipewire-native`. WASAPI is a stub behind `wasapi` / daemon `wasapi-capture` so the Windows target links; operators use `--capture mock` until a native backend lands.

### Releases

Workflow [`.github/workflows/release.yml`](../.github/workflows/release.yml):

- Triggers: push tag `v*`, and `workflow_dispatch` (dry-run upload optional).
- Linux (`ubuntu-latest`): build daemon + UI, stage `softwaked` as Tauri sidecar, emit AppImage + tar.gz (with install script and user unit).
- Windows (`windows-latest`): same with `wasapi-capture`; emit NSIS setup.exe + portable zip.

See [releases.md](releases.md) for download and feature notes.

## Context

Phase 1 locked Unix sockets and PipeWire on Linux. Contributors and CI were Linux-only. Spencer queued a shipable multi-platform + Releases slice after the keyring time-box (#47) so Windows can at least build and run mock / live-http paths. A follow-up asked for AppImage and Windows setup/portable (installer plus portable zip) with both binaries in each package, plus a no-root Linux install that registers systemd --user.

## Alternatives

- Named pipes on Windows instead of TCP localhost. Rejected for v1 complexity; TCP + port file is enough and keeps framing identical.
- Cross-compile Windows from Linux runners. Rejected for Tauri/WebView2 friction; `windows-latest` is the smaller path.
- Full WASAPI + tray parity in the same PR. Deferred so Releases can land first.
- Root-only `/usr` install or system systemd unit as the only Linux path. Rejected; user-prefix + systemd --user is the default.
- Two AppImages (UI vs daemon). Rejected; one AppImage bundles both.

## Consequences

- Tagging `vX.Y.Z` publishes Release assets; `workflow_dispatch` can exercise the workflow without inventing a version.
- Already-published tags are not moved; the next tag picks up installer changes.
- Windows mic wake and polished HUD stay follow-ups.
- Public docs must not mention private shop paths.
