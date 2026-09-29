# Maintainer: Alexeyev Vitaly
pkgname=synshell
pkgver=0.1.3
pkgrel=1
pkgdesc="Окружение рабочего стола для Wayland на Rust и syngui: композитор, оболочка, параметры"
arch=('x86_64')
license=('MIT OR Apache-2.0')
depends=('mesa' 'libinput' 'seatd' 'systemd-libs' 'libxkbcommon' 'libdrm' 'vulkan-icd-loader'
         'fontconfig' 'freetype2' 'dbus')
optdepends=('xorg-xwayland: X11-программы'
            'wireplumber: громкость (wpctl)'
            'brightnessctl: яркость'
            'networkmanager: апплет сети'
            'playerctl: мультимедийные клавиши'
            'wl-clipboard: снимки и пути к ним в буфер обмена'
            'xdg-desktop-portal-wlr: демонстрация экрана (браузеры, OBS) и снимки через портал'
            'xdg-desktop-portal-kde: портал выбора файлов'
            'grim: снимки экрана из командной строки'
            'shared-mime-info: типы файлов в проводнике'
            'ffmpegthumbnailer: миниатюры видео в проводнике'
            'libheif: HEIC/HEIF/AVIF в просмотрщике и миниатюрах'
            'imagemagick: редкие форматы картинок (JPEG XL, RAW…) в просмотрщике'
            'breeze-icons: тема значков по умолчанию'
            'polkit: установка программ (synpkg) без root'
            'base-devel: сборка пакетов AUR в synpkg'
            'pacman-contrib: проверка обновлений без root (checkupdates)'
            'iwd: Wi-Fi (или networkmanager)'
            'bluez: Bluetooth')
makedepends=('cargo' 'rust')

# Профиль cargo: release (LTO) — для AUR/GitHub; локально быстрее
# `SYNSHELL_PROFILE=fast-release makepkg -f`.
_profile=${SYNSHELL_PROFILE:-release}

build() {
    cd "$startdir"
    cargo build --profile "$_profile" -p synwm -p syndesktop-shell -p synmobile-shell -p synkeyboard -p synsettings -p synfiles -p synshot -p synpkg
}

check() {
    cd "$startdir"
    cargo test --profile "$_profile" -p synshell-common -p synwm -p synshot
}

package() {
    cd "$startdir"
    for b in synwm syndesktop-shell synmobile-shell synkeyboard synsettings synfiles synshot synpkg; do
        install -Dm755 "target/$_profile/$b" "$pkgdir/usr/bin/$b"
    done
    install -Dm755 data/synshell-session "$pkgdir/usr/bin/synshell-session"
    ln -s synwm "$pkgdir/usr/bin/syndesktop"   # совместимость: старое имя композитора
    install -Dm644 data/synshell.desktop "$pkgdir/usr/share/wayland-sessions/synshell.desktop"
    install -Dm644 data/xdg-desktop-portal-wlr/synshell "$pkgdir/etc/xdg/xdg-desktop-portal-wlr/synshell"
    install -Dm644 data/synshell-portals.conf "$pkgdir/usr/share/xdg-desktop-portal/synshell-portals.conf"
    install -Dm644 crates/synsettings/data/synsettings.desktop \
        "$pkgdir/usr/share/applications/synsettings.desktop"
    install -Dm644 crates/synpkg/data/synpkg.desktop \
        "$pkgdir/usr/share/applications/synpkg.desktop"
    install -Dm644 crates/synfiles/data/synfiles.desktop \
        "$pkgdir/usr/share/applications/synfiles.desktop"
    install -Dm644 crates/synfiles/data/synfiles-viewer.desktop \
        "$pkgdir/usr/share/applications/synfiles-viewer.desktop"
    install -Dm644 crates/synshot/data/synshot.desktop \
        "$pkgdir/usr/share/applications/synshot.desktop"
    install -Dm644 crates/synshell-ui/data/syndesktop-lock.pam "$pkgdir/etc/pam.d/syndesktop-lock"
    install -Dm644 crates/synshell-common/default-config.toml \
        "$pkgdir/usr/share/doc/synshell/config.toml.example"
}
