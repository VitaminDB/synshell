<div align="center">

# synshot

**Screenshots for the synshell desktop — the screen freezes, you select an area or click a window, then copy, save or open it.**

[Русский](README.ru.md) · [Website](https://alexeyev.dev/synshell/syn-shot/) · [AUR: synshell-core-bin](https://aur.archlinux.org/packages/synshell-core-bin) · [AUR: synshell-core-git](https://aur.archlinux.org/packages/synshell-core-git)

![Area selection](docs/screenshots/select-en.png)

</div>

## Features

- **Frozen screen** — on Print the compositor (synwm) captures all monitors at once, so menus and tooltips stay in the picture.
- **Area, window or monitor** — drag a frame; or click: the window under the pointer is taken, or the whole monitor if there is no window. The candidate is highlighted with its name and size before the click.
- **Precise selection** — handles, moving the frame, size in pixels, a magnifier at the pointer; arrow keys move the frame by 1 px (Ctrl — by 10 px), Shift+arrows change its size.
- **Actions** — Copy (image to the clipboard), Save (PNG to the screenshots folder), Copy path, Open (in the default program); the shell shows a notification.
- **Keyboard** — Enter or Ctrl+S save (without a selection — the whole screen), Ctrl+C copy, Ctrl+Shift+C copy the path, Ctrl+O open, Ctrl+A the monitor under the pointer, Ctrl+Shift+A all monitors, Esc cancels.
- **Pointer** — the “Cursor” checkbox adds the mouse pointer to the picture.
- **Several monitors** — one overlay per monitor, the selection can span them.
- **Screen recording** — video of the whole screen, an area or the active window (the recording follows the window) to MP4: a panel button (red dot and timer), Super+Alt+R or `synshot --record`. Steady 30/60 fps, hardware encoding with NVENC or VAAPI (no frame copies through the CPU), otherwise x264; H.264 or HEVC; system sound and microphone from PipeWire.
- **14 languages** — the UI follows the system or the language chosen in settings.

| Window under the pointer |
|---|
| ![Window under the pointer](docs/screenshots/window-en.png) |

## Install

**Arch Linux (AUR)** — x86_64 and aarch64. synshot is part of `synshell-core`, the base set of desktop programs (compositor, shell, settings, file manager, terminal, software center, screenshots):

```sh
yay -S synshell-core-bin      # prebuilt
yay -S synshell-core-git      # built from source
```

Optional: `wl-clipboard` (images and paths to the clipboard).

synshot works with the synshell compositor synwm: it asks synwm for the frames of the monitors.

The whole desktop: `yay -S synshell-bin`. **From source:** `cargo build --release -p syn-shot` in the [synshell](../../README.md) repository.

## Usage

| Keys | Action |
|---|---|
| Print | synshot: select an area or a window |
| Shift+Print | the whole screen at once (synwm) |
| Alt+Print | the active window at once (synwm) |

The keys are set in `[keybindings]` of `config.toml` (`screenshot-interactive`, `screenshot`, `screenshot-window`); the screenshots folder is `[general] screenshot_dir` (`~/Pictures/Screenshots` by default).

```sh
synshot          # freeze the screen now and select
```

### Screen recording

| How | Action |
|---|---|
| Panel button (before the clock) | click — start or stop; right click — full screen, area, window, sound, microphone, 30/60 fps |
| Super+Alt+R | start or stop (what to record — Settings → Session → “Screen recording”) |

Files go to `~/Videos/Записи экрана/Запись_YYYY-MM-DD_HH-MM-SS.mp4`; when done, a notification offers “Open” and “Show in folder”. Settings live in `[recording]` of `config.toml`.

```sh
synshot --record start                       # like the button: mode from the settings
synshot --record start --area                # select an area in the overlay
synshot --record start --area 100,100,1280,720 --no-audio
synshot --record start --window              # the active window (or --window ID)
synshot --record start --duration 10 --wait  # 10 seconds, answer after the recording
synshot --record stop | toggle | status | wait
```

The answer is a JSON line: path, size, frame rate, encoder, frames, duration, error. Options: `--fps 30|60`, `--audio on|off`, `--mic on|off`, `--cursor on|off`, `--codec h264|hevc`, `--encoder auto|nvenc|vaapi|x264`, `--quality high|medium|low`, `--out FILE.mp4`. Recording is done by `syn-screencast --record` (wlr-screencopy into DMA-BUF, libav encoding).

## Part of synshell

synshot is one of the applications of [synshell](../../README.md) — a Wayland desktop environment in Rust for computers and phones, built on the [syngui](https://github.com/VitaminDB/syngui) UI framework.

## License

GPL-3.0-only
