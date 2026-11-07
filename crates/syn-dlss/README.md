<div align="center">

# syn-dlss

**DLSS 5 for the synshell desktop — NVIDIA's neural rendering for Linux games, Wine games and the video player: one window to switch it on, tune the picture and run the NGX helper.**

[Русский](README.ru.md) · [Website](https://alexeyev.dev/synshell/syn-dlss/)

</div>

syn-dlss is the window of the `syn-wine-dlss5` package — a fork of [DLSS5VKLayer](https://github.com/bmitch87/DLSS5VKLayer) that lives in [`crates/syn-wine/dlss5`](../syn-wine/README.md#dlss-5-dlss5). The Vulkan layer `VK_LAYER_NV_dlssnr` (64 and 32 bit) hands the frames of a game to the `dlssnr_helper.exe` helper, which runs under [syn-wine](../syn-wine/README.md) and loads NVIDIA NGX and the DLSS 5 model; the edited frame goes back to the game. syn-dlss replaces the Qt window of the fork.

## Features

- **Main page** — neural rendering on or off, style (Default, Natural — gentler with faces, Cinematic — with a film tone), motion quality (Fast, Balanced, Maximum), effect strength and sharpness (applied at once), a before/after wipe in the frame.
- **Where it works** — “syn-wine games”: the layer in every program of syn-wine without any variable, from its next start; Steam games: a launch-options line `VKLayer_DLSS5=1 %command%` to copy; the [video player](../syn-video-player/README.md): the “Filters” button in its header, with its own video profile.
- **Per-game settings** — the layer reports the running game; “Save for this game” keeps its own settings, “Forget” returns it to the shared ones. Named profiles in `~/.config/dlssnr/profiles`.
- **All settings** — tabs for fine tuning: Rendering (preset, intensity, local structure and tone, skin, passes, model resolution up to supersampling), Quality (motion vectors estimated by the GPU optical flow, HDR input, 16-bit intermediates), Composition (detail and colour strength, colour bound, highlight guard, how the answer is applied, white point), Inspect (side-by-side and wipe comparison, hold frame, debug views, toggle key, before/after frame captures), Helper (runner, rebuild spacing, log, reset).
- **Helper status** — start, stop and restart the helper; the model time per frame, upload and readback, video memory and NGX features. When the synaptixd neural networks keep the video memory busy, “Free” asks synaptixd to unload them.
- **Model import** — the model `nvngx_dlssnr.dll` is NVIDIA's proprietary file and is not shipped: “Import…” takes a folder or file (with `nvngx.dll`, `nvapi64.dll`, `sl.*.dll` and licenses if present), “Find on disk” looks for it in Steam games and downloads.
- **Computer window** — its own title bar, the desktop theme follows live; 14 languages.

## Install

`syn-wine-dlss5` is not published to AUR; it is built from the synshell repository (x86_64, NVIDIA):

```sh
cd crates/syn-wine/dlss5/packaging
makepkg -si
```

Depends on `syn-wine`, `nvidia-utils`, `vulkan-icd-loader`, `pciutils`; optional `lib32-vulkan-icd-loader` for 32-bit Vulkan games. DXVK comes from syn-wine (`/usr/share/dxvk`), DXVK-NVAPI is in the package, the NGX core comes from the NVIDIA driver. **The window alone, from source:** `cargo build --release -p syn-dlss`.

## Usage

A game needs Vulkan frames: for Direct3D 8–11 — DXVK (Proton has it; in syn-wine — `setup-dxvk install`).

```sh
syn-dlss                          # the window
dlssnr-helper start               # start the helper (the layer also starts it itself; DLSSNR_AUTOSTART=0 — off)
dlssnr-helper status | stop | doctor
dlssnr-helper import-binaries DIR # import nvngx_dlssnr.dll and companions
VKLayer_DLSS5=1 game              # the layer in a Linux game; in Steam: VKLayer_DLSS5=1 %command%
syn-dlss --screenshot shot.png --size 760x860 --tab quality
                                  # window snapshot (tabs: rendering, quality, composition, inspect, helper)
```

## Part of synshell

syn-dlss is one of the applications of [synshell](../../README.md) — a Wayland desktop environment in Rust for computers and phones, built on the [syngui](https://github.com/VitaminDB/syngui) UI framework.

## License

syn-dlss — GPL-3.0-only; the `syn-wine-dlss5` package (layer and helper of the DLSS5VKLayer fork) — AGPL-3.0-only, with DXVK (Zlib) and DXVK-NVAPI (MIT).
