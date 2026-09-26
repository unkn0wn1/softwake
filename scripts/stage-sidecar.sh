#!/usr/bin/env bash
# Copy a built softwaked into crates/softwake-ui/binaries/ with the host
# target-triple suffix expected by Tauri bundle.externalBin.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="$(rustc --print host-tuple 2>/dev/null || rustc -Vv | awk '/^host:/{print $2}')"
EXT=""
SRC="$ROOT/target/release/softwaked"
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT) EXT=".exe"; SRC="$ROOT/target/release/softwaked.exe" ;;
esac
if [[ ! -f "$SRC" ]]; then
  echo "stage-sidecar: missing $SRC (build softwaked --release first)" >&2
  exit 1
fi
DEST_DIR="$ROOT/crates/softwake-ui/binaries"
mkdir -p "$DEST_DIR"
DEST="$DEST_DIR/softwaked-${TRIPLE}${EXT}"
cp -f "$SRC" "$DEST"
chmod +x "$DEST" 2>/dev/null || true
echo "staged $DEST"
