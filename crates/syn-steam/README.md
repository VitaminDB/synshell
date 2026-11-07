<div align="center">

# syn-steam

**Steam for the synshell desktop — one package for x86_64 computers and aarch64 phones: Valve's bootstrap on the computer, the native ARM64 Steam client installed on first start on the phone, Big Picture under the phone shell.**

[Русский](README.ru.md) · [Website](https://alexeyev.dev/synshell/syn-steam/)

</div>

syn-steam is a pacman package with one `PKGBUILD` for both architectures (the aarch64 client installer is the `setup/` crate of the synshell workspace, binary `syn-steam-setup`). It replaces `steam`, `steam-devices` and the former phone package `steam-arm64`. Valve's client itself is not in the package — it may not be redistributed; it installs itself into `~/.local/share/Steam` and keeps itself up to date.

## Features

- **One launcher** — `/usr/bin/steam [--bigpicture|--desktop]`: under the phone shell (`synmobile-shell`) Big Picture opens by default, full screen and made for touch; otherwise the usual interface. If the client is already running, the mode is passed to it with a `steam://open/…` link.
- **Computer (x86_64)** — Valve's bootstrap (`/usr/lib/steam/bin_steam.sh`), as in the Arch `steam` package.
- **Phone (aarch64)** — on first start `steam` calls `/usr/lib/syn-steam/syn-steam-setup`: a window with the Steam Subscriber Agreement and download progress. The components of the `steam_client_linuxarm64` manifest are taken from Valve's CDN, checked by size and SHA-256 and unpacked into `~/.local/share/Steam`; after that the client updates itself. Without a network there is a message and “Retry”. 14 languages.
- **Android kernels** — SysV IPC emulation over `/dev/shm` (`LD_PRELOAD`), a setuid `bwrap` 0.10.0 for pressure-vessel (kernels without user namespaces), its own Xwayland and Openbox when started without `DISPLAY`.
- **Devices** — shortcuts, icons and the udev rules for controllers and VR headsets (as `steam-devices`).
- **Windows games** — the “syn-wine” compatibility tool of the [syn-wine](../syn-wine/README.md) package runs them on the system Wine without Proton; on a phone x86 code is emulated by FEX inside Wine.

## Install

syn-steam is not published to AUR; it is built from this directory.

```sh
cd crates/syn-steam
makepkg -si                      # computer (x86_64)
```

Phone: cross-build on an x86_64 host with synmobile `synport hostpkg syn-steam`. Optional: `syn-wine` (Windows games), `ntsync-autoload` (NTSync), `xdg-desktop-portal-impl` (file and folder pickers).

## Usage

```sh
steam                            # Big Picture under the phone shell, the usual interface otherwise
steam --bigpicture               # always Big Picture
steam --desktop                  # always the usual interface
```

The aarch64 installer also works in a terminal:

```sh
/usr/lib/syn-steam/syn-steam-setup --no-window   # install without a window
/usr/lib/syn-steam/syn-steam-setup --check       # exit code 0 if the client is installed
/usr/lib/syn-steam/syn-steam-setup --manifest --channel publicbeta
                                                  # print the manifest (beta channel)
```

`--root DIR` installs into another directory; `STEAM_ARM64_CHANNEL=publicbeta` chooses the beta channel for the first install started by `steam`.

## Part of synshell

syn-steam is part of [synshell](../../README.md) — a Wayland desktop environment in Rust for computers and phones, built on the [syngui](https://github.com/VitaminDB/syngui) UI framework.

## License

Valve's files — the Steam license (`LicenseRef-steam`); udev rules of steam-devices — MIT; `syn-steam-setup` — GPL-3.0-only.
