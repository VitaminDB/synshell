#!/bin/bash
# Сборка DXVK под ARM64EC на x86_64-хосте кросс-компилятором llvm-mingw → dxvk-<версия>-arm64ec.tar.gz
# (d3d8/d3d9/d3d10core/d3d11/dxgi). Нужно для wine-arm64ec: в архиве Hangover DXVK 2.7.1 без разделяемых
# ресурсов через D3DKMT (Wine ≥ 10.17; видео Unity через Media Foundation — IDXGIResource::GetSharedHandle).
#   ./build-dxvk-arm64ec.sh [версия DXVK, напр. 3.1.1] [llvm-mingw, напр. 20260922]
set -euo pipefail
VER=${1:-3.1.1}; LM=${2:-20260922}
W=${DXVK_WORK:-$PWD/dxvk-work}; mkdir -p "$W"; cd "$W"
LMD=llvm-mingw-$LM-ucrt-ubuntu-22.04-x86_64
[ -d "$LMD" ] || { gh release download -R mstorsjo/llvm-mingw "$LM" -p "$LMD.tar.xz"; tar xf "$LMD.tar.xz"; }
[ -d "dxvk-$VER" ] || git clone -q --depth 1 -b "v$VER" --recurse-submodules --shallow-submodules https://github.com/doitsujin/dxvk.git "dxvk-$VER"
T=$W/$LMD/bin
cat > build-arm64ec.txt <<CROSS
[binaries]
c = '$T/arm64ec-w64-mingw32-clang'
cpp = '$T/arm64ec-w64-mingw32-clang++'
ar = '$T/arm64ec-w64-mingw32-ar'
strip = '$T/arm64ec-w64-mingw32-strip'
windres = '$T/arm64ec-w64-mingw32-windres'

[properties]
needs_exe_wrapper = true

[host_machine]
system = 'windows'
cpu_family = 'aarch64'
cpu = 'arm64ec'
endian = 'little'
CROSS
cd "dxvk-$VER"
[ -d build-arm64ec ] || meson setup build-arm64ec --cross-file ../build-arm64ec.txt --buildtype release -Dbuild_id=false --prefix /tmp/dxvk-prefix
ninja -C build-arm64ec
rm -rf "$W/out"; mkdir -p "$W/out"
for d in d3d8 d3d9 d3d10core d3d11 dxgi; do cp "build-arm64ec/src/${d%core}/$d.dll" "$W/out/"; done
tar -C "$W/out" --owner=0 --group=0 --numeric-owner -czf "$W/../dxvk-$VER-arm64ec.tar.gz" .
echo "готово: $(cd "$W/.." && pwd)/dxvk-$VER-arm64ec.tar.gz"
