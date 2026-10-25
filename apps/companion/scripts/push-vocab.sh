#!/usr/bin/env bash
# Copy vocab.txt, the STT boost list (plan 20261022_04, decision 6), into the
# app's data directory, where the app reads it when the STT model loads and
# rewrites it on each `Vocabulary` from flankr. Copied from libvoicexn's
# scripts/push-vocab.sh (v0.2.0).
#
#   scripts/push-vocab.sh <vocab file>
set -euo pipefail

SRC="${1:?usage: push-vocab.sh <vocab file>}"
APP_ID=dev.flankr.companion
adb push "$SRC" /data/local/tmp/flankr-vocab.txt >/dev/null
adb shell run-as "$APP_ID" cp /data/local/tmp/flankr-vocab.txt files/vocab.txt
adb shell rm /data/local/tmp/flankr-vocab.txt
echo "==> $APP_ID files/vocab.txt: $(adb shell run-as "$APP_ID" grep -vc '^#' files/vocab.txt) phrases"
