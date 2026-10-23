#!/bin/sh
# Собрать org.lineageos.platform.jar с исправленным мостом уведомлений из оригинала образа
# (lineage-20.0 waydroid, sha256 a45e4836…21af2 — его сверяет container.rs).
# Нужны: baksmali/smali 2.5.2 (bitbucket.org/JesusFreke/smali), Android SDK build-tools (zipalign).
# ./build.sh ОРИГИНАЛ.jar baksmali.jar smali.jar
set -eu
orig=$1 baksmali=$2 smali=$3
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
cd "$t"
unzip -q "$orig"
java -jar "$baksmali" d classes.dex -o smali
python3 "$here/patch.py" 'smali/org/lineageos/platform/internal/WayDroidService$2.smali'
java -jar "$smali" a smali -o classes.dex --api 33
touch -t 200801010000 classes.dex META-INF/MANIFEST.MF
zip -q -0 -X tmp.jar classes.dex META-INF/MANIFEST.MF
zipalign=$(ls -d "$HOME"/Android/Sdk/build-tools/*/ | tail -1)zipalign
"$zipalign" -f -p 4 tmp.jar "$here/org.lineageos.platform.jar"
