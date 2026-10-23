#!/usr/bin/env bash
# Copy Phonon (phonon-31m-en) into the app's data directory on the phone,
# where it loads it (<data dir>/phonon-31m-en, Android's internal files
# directory). Needs the debuggable APK installed, for run-as. Copied from
# libvoicexn's scripts/push-model.sh (v0.2.0).
#
#   scripts/push-model.sh [<model dir>]     # default: <models>/phonon-31m-en
#
# <models> is $HANDLR_MODELS, or apps/companion/models.
set -euo pipefail

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODELS="${HANDLR_MODELS:-$APP/models}"
SRC="${1:-$MODELS/phonon-31m-en}"
APP_ID=dev.tmzt.handlr
TMP=/data/local/tmp/handlr-tts

[[ -f "$SRC/model.q8.gguf" ]] || { echo "error: no model in $SRC (libvoicexn's scripts/fetch-model.sh fetches it)" >&2; exit 1; }
adb shell rm -rf "$TMP"
adb shell mkdir -p "$TMP/phonon-31m-en"
# `/.`: the contents, also when the folder is a link to a shared one.
adb push "$SRC/." "$TMP/phonon-31m-en/" >/dev/null
adb shell run-as "$APP_ID" sh -c "'rm -rf files/phonon-31m-en && mkdir -p files && cp -r $TMP/phonon-31m-en files/'"
adb shell rm -rf "$TMP"
echo "==> $(adb shell run-as "$APP_ID" ls files/phonon-31m-en/voices | wc -l | tr -d ' ') voices in $APP_ID files/phonon-31m-en"
