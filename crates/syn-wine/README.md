# syn-wine

Wine synshell — пакет pacman (не крейт Cargo) для x86_64 и aarch64 с одним `PKGBUILD` и одним набором патчей.
Заменяет пакет `wine` (`provides=wine`, `conflicts=wine`) и прежний телефонный `wine-arm64ec`.

- Общие правки: касания Wayland/X11 → DirectInput, мост Steam API из Proton (`lsteamclient`, `steam.exe`),
  `syn-wine-helper` (недостающие DLL — общий каталог, окно «поставить?»), EmulateModeset под Xwayland,
  исправления swapchain/DPI/Media Foundation/полей эмулированного режима, DXVK (`setup-dxvk`).
- Только aarch64 (`$CARCH` в `PKGBUILD`, `#ifdef __aarch64__` и проверки условий в патчах): ARM64EC/WoW64,
  FEX и Box64 из Hangover, свой DXVK ARM64EC и `libwow64fex.dll`, обход rpcrt4 delay-load, DPI 180 для телефона,
  правки 39-битного адресного пространства ядер Android.
- Compat tool Steam «syn-wine» — `/usr/share/steam/compatibilitytools.d/syn-wine`: Windows-игры этим Wine без Proton.

Сборка: десктоп — `makepkg -si`; телефон — `synport hostpkg syn-wine` (кросс-сборка на хосте, synmobile) или
`makepkg` на телефоне. Для aarch64 в `$BUILD` нужны `dxvk-<версия>-arm64ec.tar.gz` (`build-dxvk-arm64ec.sh`) и
`libwow64fex-<коммит>.dll` (`build-fex-wow64.sh`) — оба собираются на x86_64-хосте llvm-mingw.

## DLSS 5 (`dlss5/`)

Форк [DLSS5VKLayer](https://github.com/bmitch87/DLSS5VKLayer) (от коммита `ab722b0`) — DLSS 5 Neural Rendering
для Linux-игр и игр под Wine/Proton на NVIDIA: Vulkan-слой `VK_LAYER_NV_dlssnr` (64/32 бит) отдаёт кадры помощнику
`dlssnr_helper.exe`, который под syn-wine грузит NGX и модель. Отдельный пакет `syn-wine-dlss5`
(`cd dlss5/packaging && makepkg -si`).

- Связка с syn-wine: DXVK `dxgi.dll` — из `/usr/share/dxvk/x64`, DXVK-NVAPI — в пакете (`/usr/lib/dlssnr/nvapi`),
  `nvngx.dll`/`_nvngx.dll` — из драйвера (`/usr/lib/nvidia/wine`); ничего не скачивается при запуске.
- Модели `nvngx_dlssnr.dll` в пакете нет: `dlssnr-helper import-binaries <каталог>` или окно `syn-dlss` (crates/syn-dlss, syngui; Qt-окно форка в пакет не входит).
- Включение в игре: `VKLayer_DLSS5=1 %command%`; помощник — `dlssnr-helper start`.
