#!/bin/bash
# Выпуск synshell: версия, тег, пакет для AUR.
#
#   packaging/release.sh 0.2.0            подготовить: версия в Cargo.toml и PKGBUILD (pkgrel=1),
#                                         коммит, тег v0.2.0, AUR-вариант в packaging/aur/synshell
#   packaging/release.sh --aur-only       только пересобрать AUR-вариант из текущего PKGBUILD
#
# Ничего не публикует: `git push origin master v0.2.0` и выкладку в AUR
# (`cd packaging/aur/synshell && git push`) делает человек.
#
# Локальный PKGBUILD собирает из рабочей копии ($startdir/..). AUR-вариант — тот же файл, но
# исходники берутся по тегу с GitHub (source=git+…#tag=v$pkgver), сборка — в $srcdir/synshell.
set -euo pipefail
cd "$(dirname "$0")"
root=$(cd .. && pwd)

aur() {
    local out=aur/synshell
    mkdir -p "$out"
    local ver; ver=$(grep -m1 '^pkgver=' PKGBUILD | cut -d= -f2)
    sed -e 's|"\$startdir/\.\."|"$srcdir/synshell"|g' \
        -e 's|\$startdir/\.\./|$srcdir/synshell/|g' \
        -e "s|^url=.*|&\nsource=(\"synshell::git+https://github.com/VitaminDB/synshell.git#tag=v\$pkgver\")\nsha256sums=('SKIP')|" \
        -e 's|^_profile=.*|_profile=release|' \
        PKGBUILD > "$out/PKGBUILD"
    cp synshell-core.install synshell-link.install "$out/"
    (cd "$out" && makepkg --printsrcinfo > .SRCINFO)
    if grep -q 'startdir' "$out/PKGBUILD"; then
        echo "AUR PKGBUILD: остался \$startdir" >&2
        exit 1
    fi
    echo "AUR: $out (версия $ver) — PKGBUILD, .SRCINFO, *.install"
}

if [ "${1:-}" = "--aur-only" ]; then
    aur
    exit 0
fi

ver=${1:?версия, например 0.2.0}
[[ $ver =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "версия вида 0.2.0" >&2; exit 2; }
if [ -n "$(git -C "$root" status --porcelain --untracked-files=no)" ]; then
    echo "в рабочей копии есть незакоммиченные изменения" >&2
    exit 1
fi
sed -i "0,/^version = \".*\"/s//version = \"$ver\"/" "$root/Cargo.toml"
sed -i -e "s/^pkgver=.*/pkgver=$ver/" -e "s/^pkgrel=.*/pkgrel=1/" PKGBUILD
(cd "$root" && cargo metadata --format-version 1 >/dev/null)   # Cargo.lock — новая версия крейтов
git -C "$root" add Cargo.toml Cargo.lock packaging/PKGBUILD
git -C "$root" commit -q -m "Выпуск $ver"
git -C "$root" tag -a "v$ver" -m "synshell $ver"
aur
echo "Готово: коммит и тег v$ver локально. Опубликовать: git push origin master v$ver; затем AUR."
