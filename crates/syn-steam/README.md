# syn-steam

Steam synshell — пакет pacman (не крейт Cargo) для x86_64 и aarch64 с одним `PKGBUILD`. Заменяет `steam`,
`steam-devices` и прежний телефонный `steam-arm64`.

- Общее: запускатель `/usr/bin/steam` (`--bigpicture`/`--desktop`; по умолчанию Big Picture под synmobile-shell,
  иначе обычный интерфейс; работающему клиенту режим передаётся ссылкой), ярлыки, иконки и udev-правила из тарбола Valve.
- x86_64: бутстрап Valve (`/usr/lib/steam/bin_steam.sh`), клиент ставит и обновляет себя сам в `~/.local/share/Steam`.
- aarch64: нативный aarch64-клиент из манифеста Valve (`/usr/lib/syn-steam/client` — начальная копия), эмуляция SysV IPC
  (`sysv-shim.c`, та же, что synmobile `compat/x11/sysv-shim.c` — менять вместе), setuid bwrap 0.10.0 для
  pressure-vessel, свой Xwayland/Openbox при запуске без DISPLAY. Новая версия клиента — `./update-sources.sh`.

Windows-игры — compat tool «syn-wine» из пакета `syn-wine`.
