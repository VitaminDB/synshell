#!/bin/bash
# Сборка libwow64fex.dll (FEX для i386 в WoW64) на x86_64-хосте кросс-компилятором llvm-mingw → libwow64fex-<коммит>.dll
# в текущем каталоге ($BUILD). Тот же коммит FEX, что в Hangover (_fex в PKGBUILD), плюс fex-precise-faults.patch:
# #DE при делении на ноль и точный сбой POP в память (страницы под отслеживанием самомодифицирующегося кода),
# и fex-wow64-debugregs.patch: Dr0–Dr3/Dr6/Dr7 из WOW64-контекста читаются назад. Проверка — tests/fex-diff.
#   ./build-fex-wow64.sh [коммит FEX] [llvm-mingw, напр. 20260826]
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
FEX=${1:-$(source "$HERE/PKGBUILD"; echo "$_fex")}; LM=${2:-20260826}
W=${FEX_WORK:-$PWD/fex-work}; mkdir -p "$W"; cd "$W"
LMD=llvm-mingw-$LM-ucrt-ubuntu-22.04-x86_64
[ -d "$LMD" ] || { curl -sL -o "$LMD.tar.xz" "https://github.com/mstorsjo/llvm-mingw/releases/download/$LM/$LMD.tar.xz"; tar xf "$LMD.tar.xz"; rm "$LMD.tar.xz"; }
if [ ! -d fex ]; then
  git clone -q --filter=blob:none https://github.com/FEX-Emu/FEX.git fex
fi
cd fex
git checkout -q -f "$FEX"
git submodule update --init --depth 1 External/vixl Source/Common/cpp-optparse External/fmt External/xxhash External/range-v3 \
  External/zydis External/unordered_dense External/rpmalloc External/tracy External/Vulkan-Headers External/drm-headers External/jemalloc_glibc
git -C External/zydis submodule update --init --depth 1
git apply "$HERE/fex-precise-faults.patch"
git apply "$HERE/fex-wow64-debugregs.patch"
export PATH="$W/$LMD/bin:$PATH"
rm -rf build_wow64
# FMT_MODULE: новый CMake собирает fmt как модуль C++20 — старый fmt FEX так не собирается.
# FIND_ROOT_PATH: иначе find_package находит fmt и др. хоста.
cmake -S . -B build_wow64 -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_TOOLCHAIN_FILE=Data/CMake/toolchain_mingw.cmake \
  -DMINGW_TRIPLE=aarch64-w64-mingw32 -DENABLE_LTO=False -DENABLE_ASSERTIONS=False -DENABLE_JEMALLOC_GLIBC_ALLOC=False \
  -DBUILD_TESTING=False -DTUNE_ARCH=generic -DTUNE_CPU=none -DRANGES_NATIVE=OFF -DFMT_MODULE=OFF \
  -DCMAKE_FIND_ROOT_PATH="$W/$LMD/aarch64-w64-mingw32" -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY=ONLY -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE=ONLY >/dev/null
cmake --build build_wow64 --target wow64fex
git checkout -q -- .
OUT="$W/../libwow64fex-${FEX:0:7}.dll"
cp build_wow64/Bin/libwow64fex.dll "$OUT"
echo "готово: $(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"
