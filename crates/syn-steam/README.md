# syn-steam

Steam synshell — пакет pacman для x86_64 и aarch64 с одним `PKGBUILD` (установщик клиента aarch64 — крейт `setup/`). Заменяет `steam`,
`steam-devices` и прежний телефонный `steam-arm64`.

- Общее: запускатель `/usr/bin/steam` (`--bigpicture`/`--desktop`; по умолчанию Big Picture под synmobile-shell,
  иначе обычный интерфейс; работающему клиенту режим передаётся ссылкой), ярлыки, иконки и udev-правила из тарбола Valve.
- x86_64: бутстрап Valve (`/usr/lib/steam/bin_steam.sh`), клиент ставит и обновляет себя сам в `~/.local/share/Steam`.
- aarch64: клиента Valve в пакете нет. При первом запуске `steam` зовёт `/usr/lib/syn-steam/syn-steam-setup`
  (крейт `setup/` workspace synshell): окно с Соглашением подписчика Steam и ходом загрузки, компоненты по манифесту
  `steam_client_linuxarm64` с CDN Valve (общие `*_all` и `*_linuxarm64_linuxarm64`, размер и sha256 из манифеста)
  распаковываются в `~/.local/share/Steam`, дальше клиент обновляет себя сам. Без сети — сообщение и «Повторить».
  `syn-steam-setup --no-window | --check | --manifest [--channel publicbeta] [--root КАТАЛОГ]` — то же в терминале.
  Ещё: эмуляция SysV IPC (`sysv-shim.c`, та же, что synmobile `compat/x11/sysv-shim.c` — менять вместе), setuid
  bwrap 0.10.0 для pressure-vessel, свой Xwayland/Openbox при запуске без DISPLAY. Кросс-сборка на хосте —
  synmobile `synport hostpkg syn-steam`.

Windows-игры — compat tool «syn-wine» из пакета `syn-wine`.
