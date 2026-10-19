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
