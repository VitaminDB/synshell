#!/usr/bin/env bash
# Собрать, установить и запустить Elle Block на телефоне (arm64-v8a) по USB.
# Usage: ./run-phone.sh [--no-build] [--debug]
# Телефон: переменная PHONE (по умолчанию — первое USB-устройство adb).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ANDROID_DIR="$SCRIPT_DIR/android"
JNILIBS="$ANDROID_DIR/app/src/main/jniLibs"
PACKAGE="com.syngui.elleblock"
ACTIVITY="com.syngui.elleblock.MainActivity"
ABI="arm64-v8a"

export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-21-openjdk}"
export ANDROID_HOME="${ANDROID_HOME_OVERRIDE:-$HOME/Android/Sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"

BUILD=1
CARGO_FLAGS="--release"
GRADLE_TASK="assembleRelease"
APK_KIND="release"
for arg in "$@"; do
    case "$arg" in
        --no-build) BUILD=0 ;;
        --debug) CARGO_FLAGS=""; GRADLE_TASK="assembleDebug"; APK_KIND="debug" ;;
    esac
done

# Wi-Fi: адрес телефона хранится в .phone_ip (включение: adb tcpip 5555 по USB,
# затем adb connect <ip>:5555). Сетевое устройство предпочтительнее USB.
if [[ -z "${PHONE:-}" ]]; then
    if [[ -f "$SCRIPT_DIR/.phone_ip" ]]; then
        IP=$(tr -d '[:space:]' < "$SCRIPT_DIR/.phone_ip")
        adb connect "$IP:5555" >/dev/null 2>&1 || true
    fi
    PHONE=$(adb devices | awk '$2 == "device" && $1 ~ /:/ { print $1 }' | head -1)
    [[ -n "$PHONE" ]] || PHONE=$(adb devices | awk '$2 == "device" { print $1 }' | head -1)
fi
[[ -n "$PHONE" ]] || { echo "Телефон не найден: подключите по USB/Wi-Fi или задайте PHONE=<serial|ip:5555>" >&2; exit 1; }
adb -s "$PHONE" get-state >/dev/null

if [[ "$BUILD" == 1 ]]; then
    [[ -f "$ANDROID_DIR/local.properties" ]] || echo "sdk.dir=$ANDROID_HOME" > "$ANDROID_DIR/local.properties"
    echo "==> Native library ($ABI${CARGO_FLAGS:+, release})..."
    cd "$SCRIPT_DIR"
    cargo ndk -t "$ABI" -P 30 -o "$JNILIBS" build $CARGO_FLAGS --lib --no-default-features --features android
    echo "==> APK ($APK_KIND)..."
    cd "$ANDROID_DIR"
    ./gradlew --offline "$GRADLE_TASK"
fi

APK="$ANDROID_DIR/app/build/outputs/apk/$APK_KIND/app-$APK_KIND.apk"
echo "==> Installing to $PHONE..."
adb -s "$PHONE" install -r "$APK"
echo "==> Launching..."
adb -s "$PHONE" shell am start -S -n "$PACKAGE/$ACTIVITY"
