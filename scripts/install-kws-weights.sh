#!/usr/bin/env bash
# Download English Zipformer KWS weights into Softwake's XDG data dir.
# Operator-consent only. Weights are not vendored in git.
set -euo pipefail

MODEL_URL="${SOFTWAKE_KWS_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01.tar.bz2}"
MODEL_NAME="sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01"

if [[ -n "${XDG_DATA_HOME:-}" ]]; then
  DEST="${XDG_DATA_HOME}/softwake/kws"
else
  DEST="${HOME}/.local/share/softwake/kws"
fi

mkdir -p "${DEST}"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

echo "Downloading ${MODEL_URL}"
curl -fL --progress-bar "${MODEL_URL}" -o "${TMP}/kws.tar.bz2"
echo "Extracting into ${DEST}"
tar -xjf "${TMP}/kws.tar.bz2" -C "${TMP}"

SRC="${TMP}/${MODEL_NAME}"
if [[ ! -d "${SRC}" ]]; then
  # Some mirrors wrap an extra directory; find encoder*.onnx.
  SRC="$(dirname "$(find "${TMP}" -name 'encoder*.onnx' | head -n1)")"
fi
if [[ ! -d "${SRC}" ]]; then
  echo "error: could not find model files in archive" >&2
  exit 1
fi

copy_one() {
  local prefix="$1"
  local int8 fp32
  int8="$(find "${SRC}" -maxdepth 1 -name "${prefix}*.int8.onnx" | head -n1 || true)"
  fp32="$(find "${SRC}" -maxdepth 1 -name "${prefix}*.onnx" ! -name '*.int8.onnx' | head -n1 || true)"
  if [[ -n "${int8}" ]]; then
    cp -f "${int8}" "${DEST}/$(basename "${int8}")"
    # Also install stable alias used by Softwake discovery.
    cp -f "${int8}" "${DEST}/${prefix}.int8.onnx"
    echo "  $(basename "${int8}") (+ ${prefix}.int8.onnx)"
  elif [[ -n "${fp32}" ]]; then
    cp -f "${fp32}" "${DEST}/$(basename "${fp32}")"
    cp -f "${fp32}" "${DEST}/${prefix}.onnx"
    echo "  $(basename "${fp32}") (+ ${prefix}.onnx)"
  else
    echo "error: no ${prefix}*.onnx in ${SRC}" >&2
    exit 1
  fi
}

copy_one encoder
copy_one decoder
copy_one joiner

cp -f "${SRC}/tokens.txt" "${DEST}/tokens.txt"
if [[ -f "${SRC}/bpe.model" ]]; then
  cp -f "${SRC}/bpe.model" "${DEST}/bpe.model"
fi
echo "Installed KWS weights in ${DEST}"
echo "Rebuild daemon with: cargo build -p softwake-daemon --features sherpa-kws,pipewire-capture"
echo "Set the active profile name in Settings → Profiles, then speak that name (or hey Softwake) to wake."
