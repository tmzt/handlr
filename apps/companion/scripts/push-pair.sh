#!/usr/bin/env bash
# Hand the app what `flankr pair` printed (the flankr:// URL, the flankr1…
# string or the data: URL) as pair.txt in its data directory, and restart
# it. The app reads pair.txt at start, deletes it, pairs, and shows and
# logs its device code for `flankr pair` (plan 20261022_04, 16:23 EDT
# amendment). The pairing input holds only public keys and addresses.
#
#   scripts/push-pair.sh '<pairing URL, string or data URL>'
#   adb logcat -s flankr | grep 'device code'
set -euo pipefail

TEXT="${1:?usage: push-pair.sh <pairing URL, flankr1… string or data: URL>}"
APP_ID=dev.flankr.companion
TMP=/data/local/tmp/flankr-pair.txt
printf '%s\n' "$TEXT" > "${TMPDIR:-/tmp}/flankr-pair.txt"
adb push "${TMPDIR:-/tmp}/flankr-pair.txt" "$TMP" >/dev/null
rm -f "${TMPDIR:-/tmp}/flankr-pair.txt"
adb shell run-as "$APP_ID" cp "$TMP" files/pair.txt
adb shell rm "$TMP"
adb shell am force-stop "$APP_ID"
adb shell am start -n "$APP_ID/dev.highbay.libmobile.MobileNativeActivity" >/dev/null
echo "==> pair.txt pushed; $APP_ID restarted"
