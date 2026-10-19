#!/bin/bash
# Запуск клиента Steam (пакет syn-steam), общий для x86_64 и aarch64.
#   steam [--bigpicture|--desktop] [аргументы клиента…]
# Режим по умолчанию: под мобильной оболочкой (synmobile-shell) — Big Picture (на весь экран, под касания),
# иначе — обычный интерфейс. Клиент уже работает — нужный режим просим ссылкой steam://open/….
#   x86_64  — бутстрап Valve /usr/lib/steam/bin_steam.sh (клиент в ~/.local/share/Steam, обновляет себя сам);
#   aarch64 — нативный aarch64-клиент: /usr/lib/syn-steam/client — начальная копия, живёт в каталоге пользователя.
set -u

mode=
args=()
for a in "$@"; do
  case "$a" in
    --desktop) mode=desktop ;;
    --bigpicture) mode=bigpicture ;;
    *) args+=("$a") ;;
  esac
done
if [ -z "$mode" ]; then
  if pgrep -u "$(id -u)" -x synmobile-shell >/dev/null 2>&1; then mode=bigpicture; else mode=desktop; fi
fi

machine=$(uname -m)
root="${STEAM_ARM64_ROOT:-${XDG_DATA_HOME:-$HOME/.local/share}/Steam}"
if [ "$machine" = aarch64 ]; then
  client="$root/steamrtarm64/steam"
else
  client=/usr/lib/steam/bin_steam.sh
fi

# Клиент уже работает (закрытое окно Steam остаётся в фоне): ключ -gamepadui он передаёт себе и открывает
# обычный интерфейс — режим просим ссылкой, прочие аргументы (steam://rungameid/… из %U) — следом.
if pgrep -u "$(id -u)" -x steam >/dev/null 2>&1 && { [ "$machine" != aarch64 ] || [ -x "$client" ]; }; then
  if [ "$mode" = bigpicture ]; then url=steam://open/bigpicture; else url=; fi
  exec "$client" ${url:+"$url"} "${args[@]}"
fi
[ "$mode" = bigpicture ] && args=(-gamepadui "${args[@]}")

if [ "$machine" != aarch64 ]; then
  exec "$client" "${args[@]}"
fi

# ---- aarch64: нативный клиент Valve ----
dist=/usr/lib/syn-steam/client

if [ ! -x "$client" ]; then
  echo "steam: установка клиента в $root" >&2
  mkdir -p -- "$root"
  cp -a -- "$dist/." "$root/"
  chmod -R u+rwX -- "$root"
fi

mkdir -p -- "$HOME/.steam" "$root/package"
ln -sfn -- "$root" "$HOME/.steam/root"
ln -sfn -- "$root" "$HOME/.steam/steam"
ln -sfn -- "$root/linuxarm64" "$HOME/.steam/sdkarm64"
# оверлей подгружается через .steam/bin64/../steamrtarm64/
ln -sfn -- "$root/steamrtarm64" "$HOME/.steam/bin64"

# Ядро без SysV IPC: эмуляция semget/shmget через /dev/shm. Она нужна и X-серверу —
# Chromium передаёт ему id сегментов разделяемой памяти (MIT-SHM): Xwayland композитора
# получает системную (/usr/lib/libsysv-shim.so), наш запасной — ниже через LD_PRELOAD.
shim=/usr/lib/libsysv-shim.so
[ -e "$shim" ] || shim=/usr/lib/syn-steam/libsysv-shim.so
[ -e /proc/sysvipc ] || export LD_PRELOAD="$shim${LD_PRELOAD:+:$LD_PRELOAD}"

# Клиенту нужен X11. Обычно это Xwayland композитора (DISPLAY задан): окна Steam — обычные
# окна оболочки, экран в физических пикселях, масштаб — из Xft.dpi. Без DISPLAY под Wayland
# поднимаем свой полноэкранный Xwayland (запасной путь: без масштаба, без GPU).
xpid= wmpid= own_x=
if [ -z "${DISPLAY:-}" ]; then
  own_x=1
  if [ -z "${WAYLAND_DISPLAY:-}" ] || ! command -v Xwayland >/dev/null; then
    echo "steam: нет X11 (DISPLAY) и нет Xwayland" >&2
    exit 1
  fi
  n=1
  while [ -e "/tmp/.X$n-lock" ] || [ -e "/tmp/.X11-unix/X$n" ]; do n=$((n + 1)); done
  # Композитор может не сообщить размер экрана (тогда было бы 640x480),
  # поэтому берём реальный режим экрана; -hidpi — без размытия при scale > 1.
  geom="${STEAM_ARM64_GEOMETRY:-}"
  if [ -z "$geom" ] && command -v wayland-info >/dev/null; then
    geom=$(wayland-info 2>/dev/null | sed -n 's/.*width: \([0-9]*\) px, height: \([0-9]*\) px.*/\1x\2/p' | head -1)
  fi
  # shellcheck disable=SC2086
  Xwayland ":$n" ${geom:+-geometry "$geom"} -fullscreen -hidpi -noreset -nolisten tcp \
    ${STEAM_ARM64_XWAYLAND_ARGS:-} &
  xpid=$!
  for _ in $(seq 100); do
    [ -S "/tmp/.X11-unix/X$n" ] && break
    sleep 0.1
  done
  export DISPLAY=":$n"

  # без оконного менеджера окна обычного интерфейса висят маленькими в углу/центре
  if [ "$mode" = desktop ] && command -v openbox >/dev/null; then
    env -u LD_PRELOAD openbox --config-file /usr/share/syn-steam/openbox-rc.xml &
    wmpid=$!
  fi
fi

# Клиент ищет XRRGetOutputInfo среди уже загруженных библиотек (в Steam Runtime
# libXrandr загружена заранее), иначе падает. Только для клиента, не для Xwayland.
export LD_PRELOAD="${LD_PRELOAD:+$LD_PRELOAD:}/usr/lib/libXrandr.so.2"

# OpenGL интерфейса клиента. В Xwayland композитора есть DRI3 (glamor на GPU): GL аппаратный
# (zink поверх turnip); в своём Xwayland — программный: zink поверх Turnip без X11 WSI
# (только VK_KHR_wayland_surface) падает в driBindContext. STEAM_ARM64_HWGL=0|1 — принудительно.
if [ -z "${STEAM_ARM64_HWGL:-}" ]; then
  STEAM_ARM64_HWGL=0
  [ -z "$own_x" ] && xdpyinfo -queryExtensions 2>/dev/null | grep -q DRI3 && STEAM_ARM64_HWGL=1
fi
if [ "$STEAM_ARM64_HWGL" = 1 ]; then
  # turnip без X11 WSI: zink только без kopper (показ через DRI3)
  grep -aq VK_KHR_xlib_surface /opt/turnip/lib/libvulkan_freedreno.so 2>/dev/null || export LIBGL_KOPPER_DISABLE=true
else
  unset MESA_LOADER_DRIVER_OVERRIDE
  export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe
fi

# Ядро без user namespaces: контейнеру Steam (pressure-vessel) нужен setuid bwrap.
if [ -z "${PRESSURE_VESSEL_BWRAP:-}" ] && [ -u /usr/lib/syn-steam/bwrap ]; then
  export PRESSURE_VESSEL_BWRAP=/usr/lib/syn-steam/bwrap
fi

# масштаб интерфейса клиента (на телефоне без него всё мелко): Xft.dpi композитора / 96
if [ -z "${STEAM_FORCE_DESKTOPUI_SCALING:-}" ]; then
  dpi=$(xrdb -query 2>/dev/null | sed -n 's/^Xft\.dpi:[[:space:]]*\([0-9]*\).*/\1/p')
  if [ -z "$own_x" ] && [ -n "$dpi" ]; then
    STEAM_FORCE_DESKTOPUI_SCALING=$(awk -v d="$dpi" 'BEGIN { printf "%.2f", d / 96 }')
  else
    STEAM_FORCE_DESKTOPUI_SCALING=1.5
  fi
fi
export STEAM_FORCE_DESKTOPUI_SCALING

cleanup() {
  [ -n "$wmpid" ] && kill "$wmpid" 2>/dev/null
  [ -n "$xpid" ] && kill "$xpid" 2>/dev/null
}
trap cleanup EXIT

# Valve завершает клиент с кодом 42, когда просит перезапуска (после самообновления).
while :; do
  "$client" "${args[@]}"
  status=$?
  [ "$status" -ne 42 ] && exit "$status"
  echo "steam: клиент просит перезапуск" >&2
done
