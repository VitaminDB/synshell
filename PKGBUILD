# Maintainer: Alexeyev Vitaly
pkgname=synshell
pkgver=0.1.4
pkgrel=1
pkgdesc="Окружение рабочего стола для Wayland на Rust и syngui: композитор, оболочка, параметры"
arch=('x86_64')
license=('MIT OR Apache-2.0')
depends=('mesa' 'libinput' 'seatd' 'systemd-libs' 'libxkbcommon' 'libdrm' 'vulkan-icd-loader'
         'fontconfig' 'freetype2' 'dbus' 'ffmpeg')
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
            'bluez: Bluetooth'
            'fuse3: файлы связанных устройств (synlink) в проводнике'
            'openssh: ssh на связанные устройства (synlink)'
            'intel-media-driver: аппаратное видео на Intel (трансляция экрана, «Видео»)'
            'libva-mesa-driver: аппаратное видео на AMD'
            'nvidia-utils: кодер NVENC для трансляции экрана')
makedepends=('cargo' 'rust')
# C-части крейтов (ring, zstd) с -flto из makepkg.conf не линкуются с Rust.
options=('!lto')
# Прежнее имя пакета.
provides=('syndesktop')
conflicts=('syndesktop')
replaces=('syndesktop')

# Профиль cargo: release (LTO) — для AUR/GitHub; локально быстрее
# `SYNSHELL_PROFILE=fast-release makepkg -f`.
_profile=${SYNSHELL_PROFILE:-release}

build() {
    cd "$startdir"
    cargo build --profile "$_profile" -p synwm -p syndesktop-shell -p synmobile-shell -p synkeyboard -p synsettings -p synfiles -p synshot -p synpkg -p synlogin -p synlink -p synlink-view -p syn-video-player -p syn-audio-player
}

check() {
    cd "$startdir"
    cargo test --profile "$_profile" -p synshell-common -p synwm -p synshot -p synlogin
}

package() {
    cd "$startdir"
    for b in synwm syndesktop-shell synmobile-shell synkeyboard synsettings synfiles synshot synpkg synlogin synlink synlink-view syn-video-player syn-audio-player; do
        install -Dm755 "target/$_profile/$b" "$pkgdir/usr/bin/$b"
    done
    install -Dm755 data/synshell-session "$pkgdir/usr/bin/synshell-session"
    ln -s synwm "$pkgdir/usr/bin/syndesktop"   # совместимость: старое имя композитора
    install -Dm644 data/synshell.desktop "$pkgdir/usr/share/wayland-sessions/synshell.desktop"
    install -Dm644 data/xdg-desktop-portal-wlr/synshell "$pkgdir/etc/xdg/xdg-desktop-portal-wlr/synshell"
    install -Dm644 data/synshell-portals.conf "$pkgdir/usr/share/xdg-desktop-portal/synshell-portals.conf"
    install -Dm644 data/synshell.portal "$pkgdir/usr/share/xdg-desktop-portal/portals/synshell.portal"
    install -Dm644 crates/synsettings/data/synsettings.desktop \
        "$pkgdir/usr/share/applications/synsettings.desktop"
    install -Dm644 crates/synpkg/data/synpkg.desktop \
        "$pkgdir/usr/share/applications/synpkg.desktop"
    # «Программы»: pacman через pkexec с отменой задания и своё правило polkit.
    install -Dm755 crates/synpkg/data/pacman-helper "$pkgdir/usr/lib/synpkg/pacman-helper"
    install -Dm644 crates/synpkg/data/org.synshell.synpkg.policy \
        "$pkgdir/usr/share/polkit-1/actions/org.synshell.synpkg.policy"
    install -Dm644 crates/synfiles/data/synfiles.desktop \
        "$pkgdir/usr/share/applications/synfiles.desktop"
    install -Dm644 crates/synfiles/data/synfiles-viewer.desktop \
        "$pkgdir/usr/share/applications/synfiles-viewer.desktop"
    install -Dm644 crates/synshot/data/synshot.desktop \
        "$pkgdir/usr/share/applications/synshot.desktop"
    install -Dm644 crates/synlink-view/data/synlink-view.desktop \
        "$pkgdir/usr/share/applications/synlink-view.desktop"
    install -Dm644 crates/syn-video-player/data/syn-video-player.desktop \
        "$pkgdir/usr/share/applications/syn-video-player.desktop"
    install -Dm644 crates/syn-video-player/data/icons/syn-video-player.svg \
        "$pkgdir/usr/share/icons/hicolor/scalable/apps/syn-video-player.svg"
    install -Dm644 crates/syn-audio-player/data/syn-audio-player.desktop \
        "$pkgdir/usr/share/applications/syn-audio-player.desktop"
    install -Dm644 crates/syn-audio-player/data/icons/syn-audio-player.svg \
        "$pkgdir/usr/share/icons/hicolor/scalable/apps/syn-audio-player.svg"
    # Программы по умолчанию для типов файлов (XDG_CURRENT_DESKTOP=synshell).
    { echo "[Default Applications]"; cat crates/*/data/defaults.mimeapps | grep -v '^#' | grep '='; } \
        | install -Dm644 /dev/stdin "$pkgdir/usr/share/applications/synshell-mimeapps.list"
    # Связь устройств: демон стартует с сеансом (synwm читает /etc/xdg/autostart).
    install -Dm644 crates/synlink/autostart/synlink.desktop "$pkgdir/etc/xdg/autostart/synlink.desktop"
    install -Dm644 crates/synshell-ui/data/syndesktop-lock.pam "$pkgdir/etc/pam.d/syndesktop-lock"
    install -Dm644 crates/synlogin/data/synlogin.service "$pkgdir/usr/lib/systemd/system/synlogin.service"
    install -Dm644 crates/synshell-common/default-config.toml \
        "$pkgdir/usr/share/doc/synshell/config.toml.example"
}
