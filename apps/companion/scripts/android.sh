#!/usr/bin/env bash
# Build the companion app (apps/companion) for Android, and optionally
# install and launch it. Modelled on libvoicexn's scripts/android.sh (v0.2.0).
#
#   scripts/android.sh [--install] [-- <extra cargo args>]
#
# Two stages: cargo builds the .so (build.rs writes the whole Gradle project
# through libmorph-build), then gradle packages it. arm64-v8a only; the
# rustflags are in apps/.cargo/config.toml, which cargo reads because this
# script builds from apps/.
#
# The models are pushed separately (push-model.sh, push-stt.sh), and a
# pairing input with push-pair.sh.
#
# Needs: rustup target aarch64-linux-android, an Android SDK with an NDK,
# gradle 9.x and a JDK 17/21. Override discovery with ANDROID_HOME,
# ANDROID_NDK_ROOT and JAVA_HOME.
set -euo pipefail

INSTALL=0
CARGO_EXTRA=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --install) INSTALL=1; shift ;;
        --) shift; CARGO_EXTRA=("$@"); break ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

APP="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$(cd "$APP/.." && pwd)"
APP_ID=dev.flankr.companion
TARGET=aarch64-linux-android
API=29

if [[ -z "${ANDROID_HOME:-}" ]]; then
    for candidate in "$HOME/Library/Android/sdk" /opt/homebrew/share/android-commandlinetools "$HOME/Android/Sdk"; do
        [[ -d "$candidate" ]] && ANDROID_HOME="$candidate" && break
    done
fi
[[ -d "${ANDROID_HOME:-}" ]] || { echo "error: set ANDROID_HOME" >&2; exit 1; }
export ANDROID_HOME
if [[ -z "${ANDROID_NDK_ROOT:-}" ]]; then
    ANDROID_NDK_ROOT="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)"
fi
[[ -d "${ANDROID_NDK_ROOT:-}" ]] || { echo "error: no NDK under $ANDROID_HOME/ndk" >&2; exit 1; }
if [[ -z "${JAVA_HOME:-}" ]]; then
    for candidate in /opt/homebrew/opt/openjdk@21 /opt/homebrew/opt/openjdk@17; do
        [[ -d "$candidate" ]] && JAVA_HOME="$candidate" && break
    done
fi
[[ -n "${JAVA_HOME:-}" ]] && export JAVA_HOME

HOST_TAG="$(uname -s | tr '[:upper:]' '[:lower:]')-x86_64" # the NDK ships x86_64 host tools only
LLVM="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/$HOST_TAG/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$LLVM/${TARGET}${API}-clang"
export CC_aarch64_linux_android="$LLVM/${TARGET}${API}-clang"
export CXX_aarch64_linux_android="$LLVM/${TARGET}${API}-clang++"
export AR_aarch64_linux_android="$LLVM/llvm-ar"
# Either would take precedence over .cargo/config.toml's target rustflags.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS

cd "$WORKSPACE"
echo "==> cargo build ($TARGET, release)"
cargo build --manifest-path "$WORKSPACE/Cargo.toml" -p companion --lib --release --target "$TARGET" ${CARGO_EXTRA[@]+"${CARGO_EXTRA[@]}"}

PROJECT="$WORKSPACE/target/$TARGET/release/gradle/companion"
echo "==> gradle assembleDebug ($PROJECT)"
gradle -p "$PROJECT" --quiet assembleDebug
APK="$PROJECT/app/build/outputs/apk/debug/app-debug.apk"
echo "==> $APK"

if [[ "$INSTALL" == 1 ]]; then
    adb install -r "$APK"
    adb shell am start -n "$APP_ID/dev.highbay.libmobile.MobileNativeActivity"
    echo "==> logs: adb logcat -s flankr libmobile libmorph"
fi
