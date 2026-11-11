<div align="center">

# syn-wine

**Wine рабочего стола synshell — программы и игры Windows на компьютерах x86_64 и телефонах aarch64: касания, мост Steam API из Proton, DXVK, Wine Mono и Gecko в пакете, ARM64EC с FEX на ARM.**

[English](README.md) · [Сайт](https://alexeyev.dev/ru/synshell/syn-wine/)

</div>

syn-wine — пакет pacman (не крейт Cargo), собранный из upstream Wine 11.19 со своими патчами synshell. Один `PKGBUILD` и один набор патчей на x86_64 и aarch64: то, что нужно только ARM, — под `$CARCH` в `PKGBUILD` и под `#ifdef __aarch64__` в патчах. Заменяет `wine`, `wine-mono` и `wine-gecko` (`provides`/`conflicts`) и прежний телефонный пакет `wine-arm64ec`.

## Возможности

- **Касания** — касания Wayland и X11 доходят до программ Windows, в том числе через DirectInput.
- **Мост Steam API из Proton** — `lsteamclient` и `steam.exe` с закреплённого коммита Proton: игры, которые зовут `steam_api`, запускаются под этим Wine с клиентом Steam для Linux.
- **Средство совместимости Steam «syn-wine»** — `/usr/share/steam/compatibilitytools.d/syn-wine`: Windows-игры из библиотеки Steam идут на этом Wine без Proton. У каждой игры свой префикс рядом с её данными Steam, DXVK ставится при первом запуске, а в композиторе synshell игре можно задать своё разрешение экрана (`[[x11_app]]`, «Параметры → Сеанс → X11-программы»).
- **Недостающие компоненты** — если программе не хватает DLL (MFC, рантаймы Visual Basic, OpenAL…) или кодека GStreamer, Wine зовёт [syn-wine-helper](../syn-wine-helper/README.ru.md): окно предлагает поставить их один раз для всех префиксов и запускает программу заново.
- **DXVK** — Direct3D 8–11 через Vulkan; `setup-dxvk install|uninstall` ставит его в префикс (x64 + x32 на x86_64, ARM64EC + x32 на aarch64).
- **Wine Mono 11.3.0 и Wine Gecko 2.47.4 в пакете** — распакованы в `/usr/share/wine/{mono,gecko}`, общие для всех префиксов, так что новый префикс не просит их скачать. Mono — x86 на x86_64 и arm64 на aarch64; Gecko — x86 и x86_64 на обеих (им пользуются x86-программы через WoW64/FEX и x86_64-программы через ARM64EC).
- **Исправления** — EmulateModeset под Xwayland (игра рисует в своём режиме, Wine растягивает картинку на экран), правки swapchain, DPI, Media Foundation и рамок окон; `.exe` запускаются через binfmt, псевдонимы шрифтов Win32 для fontconfig.
- **DLSS 5** — Wine сам выставляет своим программам `VKLayer_DLSS5=1`, когда в [syn-dlss](../syn-dlss/README.ru.md) включён переключатель «Игры syn-wine» (пакет `syn-wine-dlss5`).

### aarch64 (телефоны)

- **ARM64EC и WoW64** — родной код ARM64, x86_64 внутри Wine эмулирует FEX (`libarm64ecfex.dll`), 32-битный x86 — FEX (`libwow64fex.dll`, своя сборка с исправлениями деления на ноль, сбоев `POP` и отладочных регистров) или Box64 — DLL из Hangover; x86-64 RootFS не нужна.
- **Эмулятор x86 по программе** — `HKCU\Software\Wine\AppDefaults\<имя.exe>\Wow64`: значение `x86` выбирает DLL эмулятора, подраздел `Environment` задаёт его окружение (например, `FEX_TSOENABLED=0`) до запуска. Heroes of Might and Magic III сразу идут на FEX без TSO: Box64 зависает, когда звуковой поток Miles приостанавливает поток игры, а FEX по умолчанию микширует так медленно, что игра дёргается.
- **OpenGL ниже 3.2** (Adreno 504 / freedreno): кадровый буфер GL-поверхности — через `glFramebufferTexture2D`, игры DirectDraw/Direct3D больше не чёрные.
- **Своя сборка DXVK ARM64EC** (DXVK 3.1.1, с разделяемыми ресурсами через D3DKMT).
- **Подгонка под телефон** — DPI 180 для новых префиксов, дальше — из `Xft.dpi` X-сервера игры; обходы 39-битного адресного пространства ядер Android и delay-load rpcrt4; Wayland-драйвер, когда у Turnip нет X11 WSI (`SYN_WINE_WAYLAND=1` — принудительно).

## Установка

В AUR syn-wine не публикуется — собирается из этого каталога.

```sh
cd crates/syn-wine
makepkg -si                      # компьютер (x86_64)
```

Телефон: `synport hostpkg syn-wine` (кросс-сборка на хосте, synmobile) или `makepkg` на телефоне. Для aarch64 в каталоге сборки нужны `dxvk-<версия>-arm64ec.tar.gz` (`build-dxvk-arm64ec.sh`) и `libwow64fex-<коммит>.dll` (`build-fex-wow64.sh`) — оба собираются на x86_64-хосте llvm-mingw.

Необязательно: `synshell` (syn-wine-helper — недостающие DLL), `ntsync-autoload` (NTSync), `samba` (NTLM).

## Использование

```sh
wine setup.exe                   # как обычный Wine
setup-dxvk install               # DXVK в $WINEPREFIX (по умолчанию ~/.wine)
setup-dxvk uninstall
```

В Steam: «Свойства игры → Совместимость → Принудительно использовать средство совместимости Steam Play» → «Wine synshell (системный)». Журнал средства — `syn-wine.log` в каталоге compatdata игры. Сам Steam — пакет [syn-steam](../syn-steam/README.ru.md).

## DLSS 5 (`dlss5/`)

Форк [DLSS5VKLayer](https://github.com/bmitch87/DLSS5VKLayer) (от коммита `ab722b0`): DLSS 5 Neural Rendering для Linux-игр и игр под Wine/Proton на NVIDIA. Vulkan-слой `VK_LAYER_NV_dlssnr` (64 и 32 бит) отдаёт кадры помощнику `dlssnr_helper.exe`, который под syn-wine грузит NGX и модель. Отдельный пакет `syn-wine-dlss5` (`cd dlss5/packaging && makepkg -si`) с окном [syn-dlss](../syn-dlss/README.ru.md).

## Часть synshell

syn-wine входит в [synshell](../../README.md): окружение рабочего стола для Wayland на Rust для компьютеров и телефонов, на UI-фреймворке [syngui](https://github.com/VitaminDB/syngui).

## Лицензия

Wine — LGPL-2.1-or-later; DXVK — Zlib.
