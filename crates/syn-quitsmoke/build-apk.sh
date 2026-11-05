#!/usr/bin/env bash
# Build Android APK for QuitSmoke
# Usage: ./build-apk.sh [--debug|--release]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# Cargo.toml лежит рядом со скриптом — это и есть корень крейта.
PROJECT_ROOT="$SCRIPT_DIR"
ANDROID_DIR="$SCRIPT_DIR/android"
JNILIBS="$ANDROID_DIR/app/src/main/jniLibs"

export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-21-openjdk}"
# Уважаем ANDROID_HOME из окружения; иначе — стандартный путь SDK.
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"

# cargo-ndk ищет NDK по ANDROID_NDK_HOME/ANDROID_NDK_ROOT. Подхватываем из
# окружения, иначе пробуем системный /opt/android-ndk.
if [[ -z "${ANDROID_NDK_HOME:-}" && -d /opt/android-ndk ]]; then
    export ANDROID_NDK_HOME="/opt/android-ndk"
fi
export ANDROID_NDK_ROOT="${ANDROID_NDK_ROOT:-${ANDROID_NDK_HOME:-}}"

# Ensure local.properties exists
if [[ ! -f "$ANDROID_DIR/local.properties" ]]; then
    echo "sdk.dir=$ANDROID_HOME" > "$ANDROID_DIR/local.properties"
fi

CARGO_FLAGS="--release"
GRADLE_TASK="assembleDebug"
if [[ "${1:-}" == "--debug" ]]; then
    CARGO_FLAGS=""
elif [[ "${1:-}" == "--release" ]]; then
    GRADLE_TASK="assembleRelease"
fi

echo "==> Building native library (arm64${CARGO_FLAGS:+, release})..."
cd "$PROJECT_ROOT"
cargo ndk -t arm64-v8a -o "$JNILIBS" build $CARGO_FLAGS -p syn-quitsmoke --no-default-features --features android

echo "==> Building APK ($GRADLE_TASK)..."
cd "$ANDROID_DIR"
./gradlew "$GRADLE_TASK"

if [[ "$GRADLE_TASK" == "assembleRelease" ]]; then
    APK="$ANDROID_DIR/app/build/outputs/apk/release/app-release.apk"
else
    APK="$ANDROID_DIR/app/build/outputs/apk/debug/app-debug.apk"
fi
echo "==> Done: $APK"
