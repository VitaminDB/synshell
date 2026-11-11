<div align="center">

# syn-wine

**Wine for the synshell desktop — Windows programs and games on x86_64 computers and aarch64 phones: touch input, the Steam API bridge from Proton, DXVK, Wine Mono and Gecko in the package, ARM64EC with FEX on ARM.**

[Русский](README.ru.md) · [Website](https://alexeyev.dev/synshell/syn-wine/)

</div>

syn-wine is a pacman package (not a Cargo crate) built from upstream Wine 11.19 with synshell patches. One `PKGBUILD` and one set of patches serve both x86_64 and aarch64; ARM-only parts sit under `$CARCH` in the `PKGBUILD` and `#ifdef __aarch64__` in the patches. It replaces `wine`, `wine-mono` and `wine-gecko` (`provides`/`conflicts`) and the former phone package `wine-arm64ec`.

## Features

- **Touch** — Wayland and X11 touches reach Windows programs, including DirectInput.
- **Steam API bridge from Proton** — `lsteamclient` and `steam.exe` from a pinned Proton commit: games that call `steam_api` start under this Wine with the Linux Steam client.
- **Steam compatibility tool “syn-wine”** — `/usr/share/steam/compatibilitytools.d/syn-wine`: Windows games from the Steam library run on this Wine without Proton. Each game gets its own prefix next to its Steam data, DXVK is installed on the first start, and with the synshell compositor a game can get its own screen resolution (`[[x11_app]]`, “System Settings → Session → X11 programs”).
- **Missing components** — when a program lacks a DLL (MFC, Visual Basic runtimes, OpenAL…) or a GStreamer codec, Wine calls [syn-wine-helper](../syn-wine-helper/README.md): a window offers to install it once for all prefixes and starts the program again.
- **DXVK** — Direct3D 8–11 over Vulkan; `setup-dxvk install|uninstall` puts it into a prefix (x64 + x32 on x86_64, ARM64EC + x32 on aarch64).
- **Wine Mono 11.3.0 and Wine Gecko 2.47.4 in the package** — unpacked into `/usr/share/wine/{mono,gecko}`, shared by all prefixes, so a new prefix does not ask to download them. Mono is x86 on x86_64 and arm64 on aarch64; Gecko is x86 and x86_64 on both (used by x86 programs through WoW64/FEX and by x86_64 programs through ARM64EC).
- **Fixes** — EmulateModeset under Xwayland (a game renders in its own mode, Wine scales it to the screen), swapchain, DPI, Media Foundation and window frame fixes; `.exe` files start through binfmt, Win32 font aliases for fontconfig.
- **DLSS 5** — Wine sets `VKLayer_DLSS5=1` for its programs when “syn-wine games” is switched on in [syn-dlss](../syn-dlss/README.md) (package `syn-wine-dlss5`).

### aarch64 (phones)

- **ARM64EC and WoW64** — native ARM64 code with x86_64 emulated inside Wine by FEX (`libarm64ecfex.dll`) and 32-bit x86 by FEX (`libwow64fex.dll`, own build with fixes for division by zero, `POP` faults and debug registers) or Box64 — DLLs from Hangover; no x86-64 RootFS.
- **x86 emulator per program** — `HKCU\Software\Wine\AppDefaults\<name.exe>\Wow64`: value `x86` picks the emulator DLL, subkey `Environment` sets its environment (e.g. `FEX_TSOENABLED=0`) before it starts. Heroes of Might and Magic III runs on FEX without TSO out of the box: Box64 deadlocks when the Miles sound thread suspends the game thread, and default FEX mixes sound so slowly the game stutters.
- **OpenGL below 3.2** (Adreno 504 / freedreno): the framebuffer GL surface uses `glFramebufferTexture2D` — DirectDraw/Direct3D games are no longer black.
- **Own DXVK ARM64EC build** (DXVK 3.1.1, with shared resources through D3DKMT).
- **Phone adjustments** — DPI 180 for new prefixes, then taken from `Xft.dpi` of the game's X server; workarounds for the 39-bit address space of Android kernels and the rpcrt4 delay-load; the Wayland driver when Turnip lacks X11 WSI (`SYN_WINE_WAYLAND=1` forces it).

## Install

syn-wine is not published to AUR; it is built from this directory.

```sh
cd crates/syn-wine
makepkg -si                      # computer (x86_64)
```

Phone: `synport hostpkg syn-wine` (cross-build on the host, synmobile) or `makepkg` on the phone. For aarch64 the build directory needs `dxvk-<version>-arm64ec.tar.gz` (`build-dxvk-arm64ec.sh`) and `libwow64fex-<commit>.dll` (`build-fex-wow64.sh`) — both are built on an x86_64 host with llvm-mingw.

Optional: `synshell` (syn-wine-helper — missing DLLs), `ntsync-autoload` (NTSync), `samba` (NTLM).

## Usage

```sh
wine setup.exe                   # as usual Wine
setup-dxvk install               # DXVK into $WINEPREFIX (default ~/.wine)
setup-dxvk uninstall
```

In Steam: a game's “Properties → Compatibility → Force a specific Steam Play compatibility tool” → “Wine synshell (системный)”. The tool's log is `syn-wine.log` in the game's compatdata folder. Steam itself is the [syn-steam](../syn-steam/README.md) package.

## DLSS 5 (`dlss5/`)

A fork of [DLSS5VKLayer](https://github.com/bmitch87/DLSS5VKLayer) (from commit `ab722b0`): DLSS 5 Neural Rendering for Linux games and Wine/Proton games on NVIDIA. The Vulkan layer `VK_LAYER_NV_dlssnr` (64 and 32 bit) hands frames to `dlssnr_helper.exe`, which loads NGX and the model under syn-wine. It is a separate package, `syn-wine-dlss5` (`cd dlss5/packaging && makepkg -si`), with the [syn-dlss](../syn-dlss/README.md) window.

## Part of synshell

syn-wine is part of [synshell](../../README.md) — a Wayland desktop environment in Rust for computers and phones, built on the [syngui](https://github.com/VitaminDB/syngui) UI framework.

## License

Wine — LGPL-2.1-or-later; DXVK — Zlib.
