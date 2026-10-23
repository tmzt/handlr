#!/usr/bin/env bash
# Hand the app what `handlr pair` printed (the handlr:// URL, the handlr1…
# string or the data: URL) as pair.txt in its data directory, and restart
# it. The app reads pair.txt at start, deletes it, pairs, and shows and
# logs its device code for `handlr pair` (plan 20261022_04, 16:23 EDT
# amendment). The pairing input holds only public keys and addresses.
#
#   scripts/push-pair.sh '<pairing URL, string or data URL>'
#   adb logcat -s handlr | grep 'device code'
set -euo pipefail

TEXT="${1:?usage: push-pair.sh <pairing URL, handlr1… string or data: URL>}"
APP_ID=dev.tmzt.handlr
TMP=/data/local/tmp/handlr-pair.txt
printf '%s\n' "$TEXT" > "${TMPDIR:-/tmp}/handlr-pair.txt"
adb push "${TMPDIR:-/tmp}/handlr-pair.txt" "$TMP" >/dev/null
rm -f "${TMPDIR:-/tmp}/handlr-pair.txt"
adb shell run-as "$APP_ID" cp "$TMP" files/pair.txt
adb shell rm "$TMP"
adb shell am force-stop "$APP_ID"
adb shell am start -n "$APP_ID/dev.highbay.libmobile.MobileNativeActivity" >/dev/null
echo "==> pair.txt pushed; $APP_ID restarted"
