<div align="center">

# synlink

**Links synshell devices — phone and computer — over USB and Wi-Fi: screen, input, sound, files, notifications, a shared clipboard and `ssh phone` without knowing addresses.**

[Русский](README.ru.md) · [Website](https://alexeyev.dev/synshell/syn-link/) · [AUR: synshell-link-bin](https://aur.archlinux.org/packages/synshell-link-bin) · [AUR: synshell-link-git](https://aur.archlinux.org/packages/synshell-link-git)

</div>

## Features

- **Finds devices by itself** — over the phone's USB cable (NCM network) and in the same Wi-Fi network; with both, one session stays, over USB (plug the cable in and the session moves to it).
- **Secure connection** — QUIC over UDP with TLS 1.3, an ed25519 key per device and pinned fingerprints; independent streams, so copying files does not slow down the screen.
- **Pairing** — over USB the computer agrees at once and the phone asks for one confirmation; over Wi-Fi a 6-digit code is shown on both screens. A computer with `discoverable = false` is not visible in Wi-Fi.
- **Screen** — live view and control in [synlink-view](../syn-link-view/README.md): only the changed areas are sent, large changes go as H.264/HEVC video from the hardware encoder of the other side (V4L2, VAAPI, NVENC), the rest is sent losslessly when the screen stops.
- **Sound** — during the live view the device's sound plays here, and the local microphone appears there as “Microphone <name>”.
- **Input** — mouse, touches, keys and text on the other device.
- **Files** — the device's file system is mounted with FUSE and shows up under “Devices” in the file manager; drop a file on the link icon in the panel to copy it to the device's Downloads.
- **Notifications** — notifications of one device appear on the other.
- **Shared clipboard** — text copied on one device can be pasted on the other; secrets from the password manager are cleared everywhere when cleared at the source.
- **ssh** — `ssh phone` or `ssh <name>` through the link, over the cable or Wi-Fi; without a system sshd the daemon starts a user one on `127.0.0.1`.
- **Wi-Fi networks** — after pairing, the phone offers to take over the computer's Wi-Fi networks (NetworkManager or iwd); passwords travel only after you allow it on the phone.
- **For developers and AI agents** — the `synlink` command line and an MCP server for Claude Code: screenshots, taps, swipes, text, commands, logs, notifications, files and screen recording.

The phone shell shows “USB cable connected · shells linked” and the computer's card in the shade; settings are in “System Settings → Device link” (`[link]` in `config.toml`, applied on the fly). How it works inside: [docs/LINK.md](../../docs/LINK.md).

## Install

synlink comes with the `synshell-link` package (the synlink daemon and the [synlink-view](../syn-link-view/README.md) window) — x86_64 and aarch64:

```sh
yay -S synshell-link-bin      # prebuilt
yay -S synshell-link-git      # built from source
```

Depends on `synshell-core`. Optional: `openssh` (ssh to linked devices), `fuse3` (files of linked devices in the file manager). Part of the `synshell` metapackage: `yay -S synshell-bin` installs the whole desktop. **From source:** `cargo build --release -p syn-link` in the [synshell](../../README.md) repository.

The daemon starts with the session (`/etc/xdg/autostart/synlink.desktop`), one per user.

## Command line

```sh
synlink status                 # this machine, the cable, devices
synlink pair accept phone      # answer a pairing request from the terminal
synlink shot phone -o a.png    # screenshot (PNG)
synlink tap phone 540 1200     # tap (screenshot pixels or fractions 0..1)
synlink swipe phone 0.9 0.5 0.1 0.5 300
synlink type phone 'hello'     # text into the focused field
synlink key phone ctrl+a       # key combination
synlink action phone back      # synwm action: back, shell home, shell shade…
synlink exec phone -- uname -a # command (streamed output, exit code)
synlink shell phone            # terminal
synlink logs phone -f -u synshell
synlink view phone             # live view window
synlink audio phone on         # sound of the live view (off — stop; --no-mic — without the microphone)
synlink mount phone            # files (usually mounted by themselves)
ssh phone                      # ssh inside the link
```

A device is a name, the start of its id, `phone`/`desktop`/`laptop` (if there is only one) or `local` — this machine.

**Claude Code:** `claude mcp add synlink -- synlink mcp` — tools `devices`, `screenshot`, `tap`, `long_press`, `swipe`, `scroll`, `type_text`, `key`, `action`, `windows`, `exec`, `logs`, `notifications`, `read_file`, `write_file`, `record`.

## Settings

```toml
[link]
enabled = true        # daemon in the session
name = ""             # name for other devices (empty — the device model)
discoverable = true   # visible in Wi-Fi for pairing
notifications = true  # forward and show notifications
auto_mount = true     # files of linked devices in the file manager
clipboard = true      # shared clipboard
port = 47471          # UDP QUIC; discovery — port − 1
```

## Part of synshell

synlink is one of the applications of [synshell](../../README.md) — a Wayland desktop environment in Rust for computers and phones, built on the [syngui](https://github.com/VitaminDB/syngui) UI framework.

## License

GPL-3.0-only
