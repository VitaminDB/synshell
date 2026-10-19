#!/bin/sh
# Обновляет версию нативного клиента aarch64 (_client_arm64) и список его архивов в PKGBUILD из манифеста Valve.
#   ./update-sources.sh [stable|publicbeta]
set -e
cd "$(dirname "$0")"
case "${1:-stable}" in
  stable) m=steam_client_linuxarm64 ;;
  publicbeta) m=steam_client_publicbeta_linuxarm64 ;;
  *) echo "канал: stable | publicbeta" >&2; exit 1 ;;
esac
curl -fsSL "https://client-update.fastly.steamstatic.com/$m" -o manifest.vdf
trap 'rm -f manifest.vdf' EXIT
python3 - <<'PY'
import re
s = open('manifest.vdf').read()
ver = re.search(r'"version"\s+"(\d+)"', s).group(1)
comps = [c for c in re.findall(
    r'"([a-z0-9_]+)"\s*\{\s*"file"\s+"([^"]+)"\s*"size"\s+"\d+"\s*"sha2"\s+"([0-9a-f]+)"', s)
    if re.search(r'(_all|_linuxarm64_linuxarm64)$', c[0])]
src = ''.join(f'  "{n}.zip::$_cdn/{f}"\n' for n, f, _ in comps)
sums = ''.join(f"  '{h}'\n" for _, _, h in comps)
p = open('PKGBUILD').read()
p = re.sub(r'^_client_arm64=.*$', f'_client_arm64={ver}', p, flags=re.M)
p = re.sub(r'(# BEGIN components\n).*?(# END components\n)', lambda m: m.group(1) + src + m.group(2), p, flags=re.S)
p = re.sub(r'(# BEGIN sums\n).*?(# END sums\n)', lambda m: m.group(1) + sums + m.group(2), p, flags=re.S)
open('PKGBUILD', 'w').write(p)
print('version', ver, 'components', len(comps))
PY
