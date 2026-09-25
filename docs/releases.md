# Softwake releases

GitHub Releases publish **linux-x86_64** and **windows-x86_64** archives for tagged versions (`v*`). This page is the public install and feature matrix. Design notes live in [ADR 0019](ADR-0019-multiplatform-releases.md).

## How a release is published

1. Merge the work you want on `main` with CI green.
2. Tag from that tip, for example:
   ```bash
   git tag -a v0.1.0 -m "softwake v0.1.0"
   git push origin v0.1.0
   ```
3. The [Release](../.github/workflows/release.yml) workflow builds and uploads assets to the GitHub Release for that tag.

To exercise the workflow without cutting a version, use **Actions → Release → Run workflow** (`workflow_dispatch`). That run builds the same matrix; attach artifacts from the run if the job is configured not to create a Release.

## Assets

Typical names:

| File | Contents |
| --- | --- |
| `softwake-vX.Y.Z-linux-x86_64.tar.gz` | `softwaked`, `softwake-ui` |
| `softwake-vX.Y.Z-windows-x86_64.zip` | `softwaked.exe`, `softwake-ui.exe` |

Unpack, put the binaries on your `PATH` (or run by path), then:

```bash
# Linux example
softwaked serve
softwake-ui
```

```powershell
# Windows example
.\softwaked.exe serve
.\softwake-ui.exe
```

## Feature matrix

| Feature | Linux release | Windows release |
| --- | --- | --- |
| Daemon + ctl (`softwaked`) | yes | yes |
| Settings / tray / HUD (`softwake-ui`) | yes | yes (placement polish deferred) |
| Live chat / STT / TTS (`live-http`) | yes (when built with the feature) | yes (when built with the feature) |
| Real microphone | `pipewire-capture` | stub only (`wasapi-capture`); use mock |
| On-device wake (`sherpa-kws`) | when linked + weights installed | when linked + weights; else omit |
| IPC | Unix socket | TCP localhost + port file |

Default capture remains **mock** (no microphone). Linux operators who want a real mic rebuild or download a build with `pipewire-capture` and pass `--capture pipewire`. Windows operators stay on mock until WASAPI is implemented.

## Socket / port path

- **Linux:** `--socket`, `SOFTWAKE_SOCKET`, `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, else `/tmp/softwake-$UID/softwaked.sock` ([ADR 0003](ADR-0003-ipc-transport.md)).
- **Windows:** `--socket`, `SOFTWAKE_SOCKET`, else `%LOCALAPPDATA%\softwake\softwaked.port` (file holds `127.0.0.1:PORT`). A value that looks like `host:port` is used directly.

## Known Windows gaps (v1)

- No real WASAPI capture yet (stub fails on `start` with a clear error).
- Tray / HUD visual parity with Linux is unfinished.
- No `.msi` installer.
- macOS is not in the matrix.

## See also

- [README](../README.md) — develop from a checkout
- [ADR 0019](ADR-0019-multiplatform-releases.md) — decisions
