#!/usr/bin/env bash
# Install Softwake into a user prefix and enable a systemd --user unit.
# Does not require root. Default prefix: ~/.local
#
# Usage (from an extracted linux tar.gz that contains this script + binaries):
#   ./install-linux.sh
#   PREFIX=$HOME/.local ./install-linux.sh
#   ./install-linux.sh --no-start    # install + enable, do not start yet
#
# The AppImage path does not run this script and does not register systemd.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
START=1
for arg in "$@"; do
  case "$arg" in
    --no-start) START=0 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

need() {
  if [[ ! -f "$1" ]]; then
    echo "install-linux: missing $1" >&2
    exit 1
  fi
}

need "$HERE/softwaked"
need "$HERE/softwake-ui"
UNIT_SRC="$HERE/softwaked.user.service"
if [[ ! -f "$UNIT_SRC" ]]; then
  UNIT_SRC="$HERE/packaging/softwaked.user.service"
fi
need "$UNIT_SRC"
DESKTOP_SRC="$HERE/softwake.desktop"
if [[ ! -f "$DESKTOP_SRC" ]]; then
  DESKTOP_SRC="$HERE/packaging/softwake.desktop"
fi

BINDIR="$PREFIX/bin"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/512x512/apps"

mkdir -p "$BINDIR" "$UNIT_DIR" "$APP_DIR"
install -m755 "$HERE/softwaked" "$BINDIR/softwaked"
install -m755 "$HERE/softwake-ui" "$BINDIR/softwake-ui"

# Substitute install prefix into the unit (no root).
sed "s|@BINDIR@|$BINDIR|g" "$UNIT_SRC" > "$UNIT_DIR/softwaked.service"
chmod 644 "$UNIT_DIR/softwaked.service"

if [[ -f "$DESKTOP_SRC" ]]; then
  # Exec=softwake-ui works when BINDIR is on PATH; also write a full path copy.
  sed "s|^Exec=.*|Exec=$BINDIR/softwake-ui|" "$DESKTOP_SRC" > "$APP_DIR/softwake.desktop"
  chmod 644 "$APP_DIR/softwake.desktop"
fi

if [[ -f "$HERE/softwake.png" ]]; then
  mkdir -p "$ICON_DIR"
  install -m644 "$HERE/softwake.png" "$ICON_DIR/softwake.png"
elif [[ -f "$HERE/icons/icon.png" ]]; then
  mkdir -p "$ICON_DIR"
  install -m644 "$HERE/icons/icon.png" "$ICON_DIR/softwake.png"
fi

if ! command -v systemctl >/dev/null 2>&1; then
  echo "install-linux: systemctl not found; binaries installed to $BINDIR" >&2
  echo "Start manually: $BINDIR/softwaked serve" >&2
  exit 0
fi

systemctl --user daemon-reload
systemctl --user enable softwaked.service
if [[ "$START" -eq 1 ]]; then
  systemctl --user restart softwaked.service
  echo "softwaked systemd --user unit enabled and started."
else
  echo "softwaked systemd --user unit enabled (not started)."
fi

echo "Binaries: $BINDIR/softwaked , $BINDIR/softwake-ui"
if [[ ":$PATH:" != *":$BINDIR:"* ]]; then
  echo "Note: add $BINDIR to PATH if softwake-ui is not found in new shells."
fi
echo "UI: softwake-ui   (or $BINDIR/softwake-ui)"
echo "Unit: systemctl --user status softwaked"
echo "Capture stays mock unless you edit the unit for --capture pipewire (see unit comments)."
