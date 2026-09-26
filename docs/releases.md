# Softwake releases

GitHub Releases publish **linux-x86_64** and **windows-x86_64** installers and archives for tagged versions (`v*`). This page is the public install and feature matrix. Design notes live in [ADR 0019](ADR-0019-multiplatform-releases.md).

## How a release is published

1. Merge the work you want on `main` with CI green.
2. Tag from that tip, for example:
   ```bash
   git tag -a v0.1.1 -m "softwake v0.1.1"
   git push origin v0.1.1
   ```
3. The [Release](../.github/workflows/release.yml) workflow builds and uploads assets to the GitHub Release for that tag.

Do **not** move or retag an already-published tag (for example `v0.1.0`). Cut the next version so the new AppImage / setup / portable assets appear.

To exercise the workflow without cutting a version, use **Actions → Release → Run workflow** (`workflow_dispatch`). That run builds the same matrix; set `create_release=false` to keep artifacts on the workflow run only.

## Assets

| File | Role |
| --- | --- |
| `softwake-vX.Y.Z-linux-x86_64.AppImage` | **Preferred Linux download.** Portable: bundles `softwake-ui` and `softwaked`. No systemd. The UI starts `softwaked serve` when nothing is listening. |
| `softwake-vX.Y.Z-linux-x86_64.tar.gz` | Power-user / install path: both binaries, `install-linux.sh`, `softwaked.user.service`, desktop file. |
| `softwake-vX.Y.Z-windows-x86_64-setup.exe` | **Windows installer** (NSIS, per-user). Installs `softwake-ui` and `softwaked`. |
| `softwake-vX.Y.Z-windows-x86_64-portable.zip` | **Windows portable**: both executables + `start-softwake.bat`. No installer, no Windows service. |

`softwaked ctl` is the same binary as `softwaked` (subcommand), not a third file.

## Linux: AppImage (portable)

```bash
chmod +x softwake-vX.Y.Z-linux-x86_64.AppImage
./softwake-vX.Y.Z-linux-x86_64.AppImage
```

If FUSE is missing on the host:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ./softwake-vX.Y.Z-linux-x86_64.AppImage
```

The AppImage does **not** install a systemd unit. For a lingering daemon, use the tar.gz install path below. Set `SOFTWAKE_NO_AUTOSTART=1` if you manage `softwaked` yourself and do not want the UI to spawn it.

## Linux: user install + systemd --user

From the extracted tar.gz (no root required):

```bash
tar -xzf softwake-vX.Y.Z-linux-x86_64.tar.gz
cd softwake-vX.Y.Z-linux-x86_64
./install-linux.sh
```

That installs into `PREFIX` (default `~/.local`):

- `~/.local/bin/softwaked` and `softwake-ui`
- `~/.config/systemd/user/softwaked.service` (enable + start)
- a desktop entry under `~/.local/share/applications/`

```bash
systemctl --user status softwaked
softwake-ui
```

Default capture remains **mock**. Builds that include `pipewire-capture` can use a real mic after a user drop-in:

```bash
systemctl --user edit softwaked
# [Service]
# ExecStart=
# ExecStart=%h/.local/bin/softwaked serve --capture pipewire
systemctl --user restart softwaked
```

A system-wide unit under `/etc/systemd/system` is optional and out of scope for the default installer.

## Windows: setup.exe

Run `softwake-vX.Y.Z-windows-x86_64-setup.exe` and follow the NSIS prompts (per-user install). Shortcuts launch `softwake-ui`, which starts `softwaked serve` when the daemon is not already listening. There is no Windows service in v1; start-on-login can be added later if needed.

## Windows: portable

```powershell
Expand-Archive softwake-vX.Y.Z-windows-x86_64-portable.zip
cd softwake-vX.Y.Z-windows-x86_64-portable
.\start-softwake.bat
# or:
.\softwake-ui.exe
```

## Feature matrix

| Feature | Linux release | Windows release |
| --- | --- | --- |
| Daemon + ctl (`softwaked`) | yes (AppImage, tar.gz, systemd --user install) | yes (setup + portable) |
| Settings / tray / HUD (`softwake-ui`) | yes | yes (placement polish deferred) |
| Live chat / STT / TTS (`live-http`) | yes (when built with the feature) | yes (when built with the feature) |
| Real microphone | `pipewire-capture` | stub only (`wasapi-capture`); use mock |
| On-device wake (`sherpa-kws`) | when linked + weights installed | when linked + weights; else omit |
| IPC | Unix socket | TCP localhost + port file |
| systemd --user | tar.gz `install-linux.sh` only (not AppImage) | n/a |

Default capture remains **mock** (no microphone). Linux operators who want a real mic use a build with `pipewire-capture` and pass `--capture pipewire` (or edit the user unit). Windows operators stay on mock until WASAPI is implemented.

## Socket / port path

- **Linux:** `--socket`, `SOFTWAKE_SOCKET`, `$XDG_RUNTIME_DIR/softwake/softwaked.sock`, else `/tmp/softwake-$UID/softwaked.sock` ([ADR 0003](ADR-0003-ipc-transport.md)).
- **Windows:** `--socket`, `SOFTWAKE_SOCKET`, else `%LOCALAPPDATA%\softwake\softwaked.port` (file holds `127.0.0.1:PORT`). A value that looks like `host:port` is used directly.

## Known gaps (v1 installers)

- No `.deb` / `.rpm` yet (user-prefix script covers the no-root install path).
- AppImage does not register systemd.
- Windows has no service / start-on-login toggle yet.
- No code signing for Windows or Linux in this workflow.
- macOS is not in the matrix.
- `sherpa-kws` omitted from the Release job (no ONNX download in CI).

## See also

- [README](../README.md) — develop from a checkout
- [ADR 0019](ADR-0019-multiplatform-releases.md) — decisions
