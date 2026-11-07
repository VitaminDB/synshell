<div align="center">

# syn-steam

**Steam рабочего стола synshell — один пакет для компьютеров x86_64 и телефонов aarch64: бутстрап Valve на компьютере, родной ARM64-клиент Steam, который ставится при первом запуске на телефоне, Big Picture под оболочкой телефона.**

[English](README.md) · [Сайт](https://alexeyev.dev/ru/synshell/syn-steam/)

</div>

syn-steam — пакет pacman с одним `PKGBUILD` на обе архитектуры (установщик клиента для aarch64 — крейт `setup/` workspace synshell, бинарник `syn-steam-setup`). Заменяет `steam`, `steam-devices` и прежний телефонный пакет `steam-arm64`. Самого клиента Valve в пакете нет — его нельзя распространять: он ставит себя в `~/.local/share/Steam` и обновляет себя сам.

## Возможности

- **Один запускатель** — `/usr/bin/steam [--bigpicture|--desktop]`: под оболочкой телефона (`synmobile-shell`) по умолчанию открывается Big Picture — на весь экран и под касания, иначе обычный интерфейс. Если клиент уже работает, режим передаётся ему ссылкой `steam://open/…`.
- **Компьютер (x86_64)** — бутстрап Valve (`/usr/lib/steam/bin_steam.sh`), как в пакете `steam` Arch.
- **Телефон (aarch64)** — при первом запуске `steam` зовёт `/usr/lib/syn-steam/syn-steam-setup`: окно с Соглашением подписчика Steam и ходом загрузки. Компоненты манифеста `steam_client_linuxarm64` берутся с CDN Valve, сверяются по размеру и SHA-256 и распаковываются в `~/.local/share/Steam`; дальше клиент обновляет себя сам. Без сети — сообщение и «Повторить». 14 языков.
- **Ядра Android** — эмуляция SysV IPC через `/dev/shm` (`LD_PRELOAD`), setuid `bwrap` 0.10.0 для pressure-vessel (ядра без user namespaces), свой Xwayland и Openbox при запуске без `DISPLAY`.
- **Устройства** — ярлыки, значки и правила udev для геймпадов и VR-шлемов (как `steam-devices`).
- **Windows-игры** — средство совместимости «syn-wine» из пакета [syn-wine](../syn-wine/README.ru.md) запускает их системным Wine без Proton; на телефоне x86-код внутри Wine эмулирует FEX.

## Установка

В AUR syn-steam не публикуется — собирается из этого каталога.

```sh
cd crates/syn-steam
makepkg -si                      # компьютер (x86_64)
```

Телефон: кросс-сборка на x86_64-хосте — synmobile `synport hostpkg syn-steam`. Необязательно: `syn-wine` (Windows-игры), `ntsync-autoload` (NTSync), `xdg-desktop-portal-impl` (выбор файлов и папок).

## Использование

```sh
steam                            # под оболочкой телефона — Big Picture, иначе обычный интерфейс
steam --bigpicture               # всегда Big Picture
steam --desktop                  # всегда обычный интерфейс
```

Установщик aarch64 работает и в терминале:

```sh
/usr/lib/syn-steam/syn-steam-setup --no-window   # установка без окна
/usr/lib/syn-steam/syn-steam-setup --check       # код выхода 0, если клиент установлен
/usr/lib/syn-steam/syn-steam-setup --manifest --channel publicbeta
                                                  # показать манифест (бета-канал)
```

`--root КАТАЛОГ` — установка в другой каталог; `STEAM_ARM64_CHANNEL=publicbeta` — бета-канал для первой установки, которую запускает `steam`.

## Часть synshell

syn-steam входит в [synshell](../../README.md): окружение рабочего стола для Wayland на Rust для компьютеров и телефонов, на UI-фреймворке [syngui](https://github.com/VitaminDB/syngui).

## Лицензия

Файлы Valve — лицензия Steam (`LicenseRef-steam`); правила udev из steam-devices — MIT; `syn-steam-setup` — GPL-3.0-only.
