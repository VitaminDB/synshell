#!/usr/bin/env bash
# Сборка .ipa приложения synshell на syngui под iPhone — на Linux, без Xcode.
#
#   packaging/ios/build-ipa.sh syn-woman-cycle [--release]
#
# Нужно: rustup target add aarch64-apple-ios; clang + lld (ld64.lld); rsvg-convert; zip;
# iOS SDK (заголовки и .tbd) в $IOS_SDK (по умолчанию ~/.local/share/ios-sdk/src/iPhoneOS16.5.sdk,
# из github.com/theos/sdks). Линковка под iOS настроена в ~/.cargo/config.toml.
#
# .ipa не подписан: подпись и установку делает установщик на Apple ID (Plume Impactor,
# AltServer/SideStore, Sideloadly) — он же подставляет свой bundle id и профиль.
set -euo pipefail

APP=${1:?"какое приложение: build-ipa.sh <крейт> [--release]"}
PROFILE=fast-release
[[ ${2:-} == --release ]] && PROFILE=release

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SDK=${IOS_SDK:-$HOME/.local/share/ios-sdk/src/iPhoneOS16.5.sdk}
MIN_IOS=14.0
[[ -d $SDK ]] || { echo "нет iOS SDK: $SDK" >&2; exit 1; }

# Свойства приложений: bundle id, имя под иконкой, SVG иконки и её viewBox без полей
# (iOS сам скругляет углы — иконка должна быть квадратной во весь край, без прозрачности).
case $APP in
  syn-woman-cycle)
    BUNDLE_ID=io.github.vitamindb.syncycle
    NAME="Цикл"
    ICON=$ROOT/crates/syn-woman-cycle/data/icons/syn-woman-cycle.svg
    ICON_VIEWBOX="8 8 112 112"
    ICON_UNROUND='rx="30"'
    ;;
  *) echo "неизвестное приложение: $APP (добавьте его в build-ipa.sh)" >&2; exit 1 ;;
esac

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
BUILD=$(git -C "$ROOT" rev-list --count HEAD)

export SDKROOT=$SDK IPHONEOS_DEPLOYMENT_TARGET=$MIN_IOS
export CC_aarch64_apple_ios=clang CXX_aarch64_apple_ios=clang++ AR_aarch64_apple_ios=llvm-ar
export CFLAGS_aarch64_apple_ios="--target=arm64-apple-ios$MIN_IOS -isysroot $SDK"
export CXXFLAGS_aarch64_apple_ios="$CFLAGS_aarch64_apple_ios"

cargo build --manifest-path "$ROOT/Cargo.toml" -p "$APP" --target aarch64-apple-ios --profile "$PROFILE"
BIN=$ROOT/target/aarch64-apple-ios/$PROFILE/$APP

OUT=$ROOT/target/ios/$APP
rm -rf "$OUT"
DOT_APP=$OUT/Payload/$APP.app
mkdir -p "$DOT_APP"
cp "$BIN" "$DOT_APP/$APP"
llvm-strip -x "$DOT_APP/$APP" 2>/dev/null || true

# Иконки: iPhone 60pt @2x/@3x, iPad 76pt @2x, 83.5pt @2x (имена — как у Xcode без Assets.car).
SVG=$OUT/icon.svg
sed -e "s/viewBox=\"[^\"]*\"/viewBox=\"$ICON_VIEWBOX\"/" -e "s/$ICON_UNROUND/rx=\"0\"/" "$ICON" > "$SVG"
icon() { rsvg-convert -w "$2" -h "$2" -b white "$SVG" -o "$DOT_APP/$1"; }
icon AppIcon60x60@2x.png 120
icon AppIcon60x60@3x.png 180
icon AppIcon76x76@2x~ipad.png 152
icon AppIcon83.5x83.5@2x~ipad.png 167

SDK_VER=$(basename "$SDK" .sdk | sed 's/iPhoneOS//')
cat > "$DOT_APP/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>ru</string>
  <key>CFBundleLocalizations</key><array><string>ru</string><string>en</string></array>
  <key>CFBundleExecutable</key><string>$APP</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD</string>
  <key>CFBundleSupportedPlatforms</key><array><string>iPhoneOS</string></array>
  <key>DTPlatformName</key><string>iphoneos</string>
  <key>DTPlatformVersion</key><string>$SDK_VER</string>
  <key>DTSDKName</key><string>iphoneos$SDK_VER</string>
  <key>MinimumOSVersion</key><string>$MIN_IOS</string>
  <key>LSRequiresIPhoneOS</key><true/>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>UIRequiredDeviceCapabilities</key><array><string>arm64</string><string>metal</string></array>
  <key>UILaunchScreen</key><dict/>
  <key>UIRequiresFullScreen</key><true/>
  <key>UIApplicationSupportsIndirectInputEvents</key><true/>
  <key>UISupportedInterfaceOrientations</key>
  <array><string>UIInterfaceOrientationPortrait</string><string>UIInterfaceOrientationLandscapeLeft</string><string>UIInterfaceOrientationLandscapeRight</string></array>
  <key>UISupportedInterfaceOrientations~ipad</key>
  <array><string>UIInterfaceOrientationPortrait</string><string>UIInterfaceOrientationPortraitUpsideDown</string><string>UIInterfaceOrientationLandscapeLeft</string><string>UIInterfaceOrientationLandscapeRight</string></array>
  <key>ITSAppUsesNonExemptEncryption</key><false/>
  <key>CFBundleIcons</key>
  <dict><key>CFBundlePrimaryIcon</key><dict>
    <key>CFBundleIconFiles</key><array><string>AppIcon60x60</string></array>
  </dict></dict>
  <key>CFBundleIcons~ipad</key>
  <dict><key>CFBundlePrimaryIcon</key><dict>
    <key>CFBundleIconFiles</key><array><string>AppIcon60x60</string><string>AppIcon76x76</string><string>AppIcon83.5x83.5</string></array>
  </dict></dict>
</dict>
</plist>
PLIST
printf 'APPL????' > "$DOT_APP/PkgInfo"

IPA=$OUT/$APP-$VERSION.ipa
(cd "$OUT" && zip -qr -X "$IPA" Payload)
rm -f "$SVG"
echo "$IPA"
