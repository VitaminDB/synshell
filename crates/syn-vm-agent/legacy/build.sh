#!/bin/sh
# Агенты syn-vm для старых систем: build.sh win9x [профиль] — собрать SYNAGENT.EXE (Windows 95/98/ME, XP)
# и выдать его в stdout (для files.run пакета syn-vm), ход сборки — в stderr.
set -eu
D="$(cd "$(dirname "$0")" && pwd)"
case "${1:-}" in
  win9x)
    P="${2:-release}"
    (cd "$D/win9x" && cargo build --profile "$P" 1>&2)
    cat "$D/win9x/target/i586-win9x/$P/synagent.exe"
    ;;
  *) echo "build.sh win9x [профиль]" >&2; exit 2 ;;
esac
