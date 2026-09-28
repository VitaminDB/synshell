# Maintainer: Alexeyev Vitaly
pkgname=syndesktop
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
            'wl-clipboard: снимки в буфер обмена'
            'xdg-desktop-portal-wlr: демонстрация экрана (браузеры, OBS) и снимки через портал'
            'xdg-desktop-portal-kde: портал выбора файлов'
            'grim: снимки экрана из командной строки'
            'shared-mime-info: типы файлов в проводнике'
            'ffmpegthumbnailer: миниатюры видео в проводнике'
            'libheif: HEIC/HEIF/AVIF в просмотрщике и миниатюрах'
            'imagemagick: редкие форматы картинок (JPEG XL, RAW…) в просмотрщике'
            'breeze-icons: тема значков по умолчанию')
makedepends=('cargo' 'rust')

# Профиль cargo: release (LTO) — для AUR/GitHub; локально быстрее
# `SYNDESKTOP_PROFILE=fast-release makepkg -f`.
_profile=${SYNDESKTOP_PROFILE:-release}

build() {
    cd "$startdir"
    cargo build --profile "$_profile" -p syndesktop -p syndesktop-shell -p syndesktop-settings -p syndesktop-files
}

check() {
    cd "$startdir"
    cargo test --profile "$_profile" -p syndesktop-common -p syndesktop
}

package() {
    cd "$startdir"
    for b in syndesktop syndesktop-shell syndesktop-settings syndesktop-files; do
        install -Dm755 "target/$_profile/$b" "$pkgdir/usr/bin/$b"
    done
    install -Dm755 data/syndesktop-session "$pkgdir/usr/bin/syndesktop-session"
    install -Dm644 data/syndesktop.desktop "$pkgdir/usr/share/wayland-sessions/syndesktop.desktop"
    install -Dm644 data/xdg-desktop-portal-wlr/syndesktop "$pkgdir/etc/xdg/xdg-desktop-portal-wlr/syndesktop"
    install -Dm644 data/syndesktop-portals.conf "$pkgdir/usr/share/xdg-desktop-portal/syndesktop-portals.conf"
    install -Dm644 crates/syndesktop-settings/data/syndesktop-settings.desktop \
        "$pkgdir/usr/share/applications/syndesktop-settings.desktop"
    install -Dm644 crates/syndesktop-files/data/syndesktop-files.desktop \
        "$pkgdir/usr/share/applications/syndesktop-files.desktop"
    install -Dm644 crates/syndesktop-files/data/syndesktop-viewer.desktop \
        "$pkgdir/usr/share/applications/syndesktop-viewer.desktop"
    install -Dm644 crates/syndesktop-shell/data/syndesktop-lock.pam "$pkgdir/etc/pam.d/syndesktop-lock"
    install -Dm644 crates/syndesktop-common/default-config.toml \
        "$pkgdir/usr/share/doc/syndesktop/config.toml.example"
}
