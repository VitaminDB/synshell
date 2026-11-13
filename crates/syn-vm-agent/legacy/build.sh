#!/bin/sh
# Агенты syn-vm для старых систем: build.sh win9x|dos [профиль] — собрать SYNAGENT.EXE (Windows 95/98/ME,
# XP) или SYNAGENT.COM (DOS) и выдать в stdout (для files.run пакета syn-vm), ход сборки — в stderr.
set -eu
D="$(cd "$(dirname "$0")" && pwd)"
case "${1:-}" in
  win9x)
    P="${2:-release}"
    (cd "$D/win9x" && cargo build --profile "$P" 1>&2)
    cat "$D/win9x/target/i586-win9x/$P/synagent.exe"
    ;;
  dos)
    P="${2:-release}"
    (cd "$D/dos" && cargo build --profile "$P" 1>&2)
    T="$(mktemp)"
    objcopy -O binary "$D/dos/target/i386-code16/$P/synagent" "$T"
    cat "$T"
    rm -f "$T"
    ;;
  *) echo "build.sh win9x|dos [профиль]" >&2; exit 2 ;;
esac
