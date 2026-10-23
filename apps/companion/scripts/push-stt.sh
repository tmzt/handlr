#!/usr/bin/env bash
# Copy the STT model (what the app loads: model.q4k.gguf, which holds the LM
# and Mimi's encoder, config.json and tokenizer.model; about 1.02 GB) into
# the app's data directory, <data dir>/stt-1b-en_fr. Needs the debuggable
# APK installed, for run-as. Copied from libvoicexn's scripts/push-stt.sh
# (v0.2.0).
#
#   scripts/push-stt.sh [<model dir>]           # default: <models>/stt-1b-en_fr
#   STT_QUANTS="q8_0 q4_0" scripts/push-stt.sh  # other quantizations
#
# <models> is $HANDLR_MODELS, or apps/companion/models. Files are staged in
# $STAGE (default /data/local/tmp/handlr-stt) and kept there; a file already
# staged at its full size is not pushed again, so STAGE=/data/local/tmp/voicexn-stt
# reuses what libvoicexn's script staged.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELS="${HANDLR_MODELS:-$APP/models}"
SRC="${1:-$MODELS/stt-1b-en_fr}"
APP_ID=dev.tmzt.handlr
TMP="${STAGE:-/data/local/tmp/handlr-stt}"

[[ -f "$SRC/config.json" ]] || { echo "error: no model in $SRC (libvoicexn's scripts/fetch-stt.sh fetches it)" >&2; exit 1; }
# Retried: wireless adb drops large transfers now and then.
adb shell mkdir -p "$TMP"
GGUFS=()
for quant in ${STT_QUANTS:-q4k}; do
    GGUFS+=("model.${quant/q8_0/q8}.gguf")
done
FILES=("${GGUFS[@]}" config.json tokenizer.model)
for f in "${FILES[@]}"; do
    size=$(stat -L -f %z "$SRC/$f" 2>/dev/null || stat -L -c %s "$SRC/$f")
    for attempt in 1 2 3; do
        [[ "$(adb shell stat -c %s "$TMP/$f" 2>/dev/null | tr -d '\r')" == "$size" ]] && break
        echo "==> push $f (attempt $attempt)"
        adb push "$SRC/$f" "$TMP/$f" >/dev/null || { sleep 2; adb reconnect >/dev/null 2>&1 || true; sleep 3; }
    done
    [[ "$(adb shell stat -c %s "$TMP/$f" | tr -d '\r')" == "$size" ]] || { echo "error: $f did not arrive" >&2; exit 1; }
done
COPY=""
for f in "${FILES[@]}"; do COPY+=" $TMP/$f"; done
adb shell run-as "$APP_ID" sh -c "'mkdir -p files/stt-1b-en_fr && cp$COPY files/stt-1b-en_fr/'"
echo "==> $APP_ID files/stt-1b-en_fr: $(adb shell run-as "$APP_ID" du -sh files/stt-1b-en_fr | cut -f1)"
