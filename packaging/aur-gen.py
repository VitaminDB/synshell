#!/usr/bin/env python3
"""PKGBUILD'ы для AUR из packaging/PKGBUILD: на каждый пакет — свой `<имя>-git` (сборка
из репозитория только нужных крейтов) и `<имя>-bin` (готовые файлы из релиза GitHub,
x86_64 и aarch64), плюс метапакеты `synshell-git` и `synshell-bin`.

Источник правды — разделённый packaging/PKGBUILD (им собираются пакеты локально):
генератор выполняет каждую `package_<имя>()` «вхолостую» (install, cd, ln — заглушки),
узнаёт её описание, зависимости и какие программы она ставит, а тело переносит в
отдельный PKGBUILD. Описания в AUR — английские (таблица EN ниже).

    packaging/aur-gen.py [--out DIR] [--local] [--sums FILE]

--out    куда писать (по умолчанию packaging/aur/out; каталог packaging/aur в .gitignore)
--local  источник -git — эта рабочая копия (git+file://), для проверки до публикации
--sums   файл «sha256  имя-архива» для -bin (его пишет release.sh); без него — SKIP
"""
import argparse, os, re, shlex, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
URL = "https://github.com/VitaminDB/synshell"
MAINTAINER = "Alexeyev Vitaly"
# Не в AUR synshell: mss-lsp публикуется из репозитория syngui.
SKIP = {"mss-lsp"}

# Английские описания для AUR (в packaging/PKGBUILD — русские, для своей системы).
EN = {
    "synshell": "synshell desktop environment: core and applications (without the neural-network service)",
    "synshell-core": "synshell: synwm compositor, shell, login screen, on-screen keyboard, screenshots and screen sharing, settings, file manager, terminal, software center",
    "synshell-mobile": "synshell for phones: mobile shell, modem (calls, SMS, data), NFC, compass, step counter",
    "synshell-game": "synshell: game mode (full-screen gamepad shell) and on-screen controller",
    "synshell-link": "synshell: computer and phone link — screen, input, files, notifications (synlink)",
    "synshell-ai": "synshell: assistant — chats, voice mode, tools (needs the synaptix service)",
    "synaptix": "synaptix neural-network service: language models, speech, images and video for all apps — on NVIDIA GPU, Intel NPU or CPU",
    "syn-notes": "synshell Notes: canvas pages, kanban, Gantt, mind maps, calendar, reminders",
    "syn-vm": "synshell virtual machines on QEMU/KVM: wizard, built-in display, snapshots, USB and PCI passthrough, guest agent, CLI and MCP",
    "syn-video-player": "synshell video player: hardware decoding, subtitles, neural upscaling and frame interpolation",
    "syn-audio-player": "synshell music player",
    "syn-audio-recorder": "synshell voice recorder",
    "synpass": "synshell password manager",
    "syn-maps": "synshell maps",
    "syn-remote": "synshell remote control: IR blaster and smart TVs",
    "syn-systeminfo": "synshell system monitor: processes, CPU, GPU, memory, disks, network",
    "syn-themeeditor": "synshell theme editor with live preview",
    "syn-woman-cycle": "synshell cycle tracker: periods, ovulation, symptoms — data stays on the device",
    "syndroid": "synshell Android container: run Android apps on the desktop",
    "syn-wine-helper": "synshell Wine helper: run Windows games and programs",
    "syncamera": "synshell camera: photos, video, night mode",
    "syn-blocks": "synshell puzzle game with blocks",
    "syn-epdtest": "synshell E Ink display test",
    "syn-quitsmoke": "QuitSmoke: quit smoking or vaping tracker — milestones, health recovery, savings",
}

# Английские пояснения optdepends (ключ — имя пакета).
OPT_EN = {
    "xorg-xwayland": "X11 applications", "wireplumber": "volume control (wpctl)",
    "sound-theme-freedesktop": "freedesktop sound theme (event fallback)", "brightnessctl": "brightness",
    "networkmanager": "network applet", "iwd": "Wi-Fi (or networkmanager)", "bluez": "Bluetooth",
    "playerctl": "media keys", "wl-clipboard": "screenshots and paths to the clipboard",
    "xdg-desktop-portal-kde": "fallback file chooser portal", "shared-mime-info": "file types in the file manager",
    "ffmpegthumbnailer": "video thumbnails in the file manager", "libheif": "HEIC/HEIF/AVIF in the image viewer and thumbnails",
    "imagemagick": "rare image formats (JPEG XL, RAW…) in the image viewer", "breeze-icons": "default icon theme",
    "archlinux-appstream-data": "categories, icons and screenshots in the software center",
    "polkit": "installing software without root", "base-devel": "building AUR packages in the software center",
    "pacman-contrib": "update checks without root (checkupdates)", "udisks2": "USB drives and disks in the file manager",
    "gvfs-mtp": "Android phones in the file manager (MTP)", "gvfs-gphoto2": "cameras in the file manager (PTP)",
    "gvfs-afc": "iPhone in the file manager", "gvfs-smb": "Windows and Samba shares in the file manager",
    "git": "git status in the file manager", "fuse3": "archives as folders; files of linked devices",
    "intel-media-driver": "hardware video on Intel", "libva-mesa-driver": "hardware video on AMD",
    "nvidia-utils": "NVIDIA driver (NVENC, neural networks on the GPU)", "openssh": "ssh to linked devices",
    "synshell-ai": "assistant (needs the synaptix service)", "synshell-mobile": "phone shell and services",
    "qemu-desktop": "the virtual machines themselves", "edk2-ovmf": "UEFI", "swtpm": "TPM 2.0 for Windows 11",
    "virtiofsd": "virtiofs shared folder (Windows)", "samba": "SMB shared folder",
    "virtio-win": "Windows drivers and guest agent (AUR)", "poppler-glib": "PDF in the reader",
    "cuda": "NVRTC kernel compiler for NVIDIA GPUs",
    "openvino-intel-npu-plugin": "neural networks on Intel NPU (Core Ultra)",
    "intel-npu-driver": "Intel NPU driver", "intel-npu-compiler": "Intel NPU graph compiler",
}

STUBS = r'''
install() { :; }
ln() { :; }
cd() { :; }
cat() { :; }
_BINS=()
_bin() { _BINS+=("$1"); }
_desktop() { :; }
_app() { _BINS+=("$1"); }
'''


def bash(script):
    return subprocess.run(["bash", "-c", script], cwd=HERE, capture_output=True, text=True, check=True).stdout


def arr(text, name):
    """Значение массива/строки из `declare -p`."""
    m = re.search(r"declare -[a-zA-Z-]* " + re.escape(name) + r"=(\(.*?\)|\".*?\")\s*$", text, re.M | re.S)
    if not m:
        return None
    v = m.group(1)
    if v.startswith('"'):
        return v[1:-1]
    return [x for _, x in re.findall(r'\[(\d+)\]="((?:[^"\\]|\\.)*)"', v)]


def read_pkgbuild():
    base = bash("source PKGBUILD; declare -p pkgname pkgver makedepends options license; declare -f _crate _syngui_dir; echo ARRAYS; declare -p _core_bins _mobile_bins _game_bins _link_bins _ai_bins")
    pk = {"pkgname": arr(base, "pkgname"), "pkgver": arr(base, "pkgver"), "makedepends": arr(base, "makedepends"),
          "options": arr(base, "options"), "license": arr(base, "license"),
          "crate_fn": base[base.index("_crate ()"):base.index("ARRAYS")],
          "arrays": "\n".join(l.replace("declare -a ", "") for l in base[base.index("ARRAYS") + 6:].strip().split("\n"))}
    pk["packages"] = {}
    for name in pk["pkgname"]:
        out = bash(
            f"source PKGBUILD\n{STUBS}\npkgdesc=; depends=(); optdepends=(); provides=(); conflicts=(); replaces=(); backup=(); install_=; "
            f"package_{name}; declare -p pkgdesc depends optdepends provides conflicts replaces backup _BINS; echo \"INSTALL=$install\"; declare -f package_{name}"
        )
        info = {k: arr(out, k) or [] for k in ("depends", "optdepends", "provides", "conflicts", "replaces", "backup", "_BINS")}
        info["pkgdesc"] = arr(out, "pkgdesc") or ""
        m = re.search(r"^INSTALL=(.*)$", out, re.M)
        info["install"] = m.group(1).strip() if m else ""
        body = out[out.index(f"package_{name} ()"):]
        info["body"] = body
        pk["packages"][name] = info
    return pk


def crates_for(pk, bins):
    out = []
    for b in bins:
        c = bash(f"{pk['crate_fn']}\n_crate {shlex.quote(b)}").strip()
        if c not in out:
            out.append(c)
    return out


def q(xs):
    return " ".join(shlex.quote(x) for x in xs)


def opt_en(lines):
    res = []
    for l in lines:
        name = l.split(":", 1)[0].strip()
        res.append(f"{name}: {OPT_EN.get(name, l.split(':', 1)[1].strip() if ':' in l else '')}".rstrip(": "))
    return res


def body_of(info):
    """Тело package_<имя>() после метаданных: всё от `cd "$startdir/.."` (метаданные — до него)."""
    lines = info["body"].rstrip().split("\n")[2:-1]  # без «package_x ()», «{» и «}»
    for i, l in enumerate(lines):
        if re.match(r'^\s*cd "\$startdir/\.\."', l):
            return "\n".join(lines[i:])
    return ""


HELPERS = r'''
# Программа: файл в /usr/bin.
_bin() {
    install -Dm755 "$srcdir/synshell/target/release/$1" "$pkgdir/usr/bin/$1"
}

# Ярлык и значок приложения из crates/<имя>/data (что есть).
_desktop() {
    local d="$srcdir/synshell/crates/$(_crate "$1")/data"
    [ -f "$d/$1.desktop" ] && install -Dm644 "$d/$1.desktop" "$pkgdir/usr/share/applications/$1.desktop"
    [ -f "$d/icons/$1.svg" ] && install -Dm644 "$d/icons/$1.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/$1.svg"
    [ -f "$d/icons/$1.png" ] && install -Dm644 "$d/icons/$1.png" "$pkgdir/usr/share/icons/hicolor/256x256/apps/$1.png"
    true
}

_app() {
    _bin "$1"
    _desktop "$1"
}
'''


def header(name, desc, pkgver, depends, optdepends, extra):
    s = [f"# Maintainer: {MAINTAINER}", "# Generated by packaging/aur-gen.py from packaging/PKGBUILD — edit that, not this.",
         f"pkgname={name}", f"pkgver={pkgver}", "pkgrel=1", f"pkgdesc={shlex.quote(desc)}", "arch=('x86_64' 'aarch64')",
         f"url={shlex.quote(URL)}", "license=('GPL-3.0-only')"]
    if depends:
        s.append(f"depends=({q(depends)})")
    if optdepends:
        s.append("optdepends=(" + "\n            ".join(shlex.quote(o) for o in optdepends) + ")")
    s += extra
    return "\n".join(s)


def gen_git(pk, name, info, local):
    base = name
    crates = crates_for(pk, info["_BINS"]) if info["_BINS"] else []
    if name == "synaptix":
        build = 'cargo build --offline --release -p synaptixd --bins --features synaptixd/cuda,synaptixd/npu'
    elif crates:
        build = "cargo build --offline --release " + " ".join(f"-p {c}" for c in crates)
    else:
        build = ""
    src = f"synshell::git+file://{ROOT}" if local else f"synshell::git+{URL}.git"
    provides = [f"{base}"] + info["provides"]
    conflicts = [base] + info["conflicts"]
    extra = [f"makedepends=({q(pk['makedepends'])})", f"provides=({q(provides)})", f"conflicts=({q(conflicts)})"]
    if info["replaces"]:
        extra.append(f"replaces=({q(info['replaces'])})")
    if info["backup"]:
        extra.append(f"backup=({q(info['backup'])})")
    if info["install"]:
        extra.append(f"install={info['install']}")
    extra += [f"options=({q(pk['options'])})", f"source=({shlex.quote(src)})", "sha256sums=('SKIP')"]
    s = header(f"{base}-git", EN.get(base, info["pkgdesc"]), pk["pkgver"], info["depends"], opt_en(info["optdepends"]), extra)
    s += r'''

pkgver() {
    cd synshell
    if git describe --tags >/dev/null 2>&1; then
        git describe --long --tags --abbrev=7 | sed 's/^v//;s/\([^-]*-g\)/r\1/;s/-/./g'
    else
        printf '%s.r%s.g%s' "$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)" "$(git rev-list --count HEAD)" "$(git rev-parse --short=7 HEAD)"
    fi
}

prepare() {
    cd synshell
    export RUSTUP_TOOLCHAIN=stable
    # Без --locked: у -git зависимости из git (syngui, synaptix) — свежие, Cargo.lock обновится.
    cargo fetch --target "$(rustc -vV | sed -n 's/host: //p')"
}
'''
    s += "\n" + pk["crate_fn"].rstrip() + "\n"
    if "_bins[@]" in info["body"]:
        s += "\n" + pk["arrays"] + "\n"
    if build:
        s += f'''
build() {{
    cd synshell
    export RUSTUP_TOOLCHAIN=stable CARGO_TARGET_DIR=target
    {build}
}}
'''
    s += HELPERS
    body = re.sub(r'cd "\$startdir/\.\."', 'cd "$srcdir/synshell"', body_of(info))
    s += "\npackage() {\n" + body + "\n}\n" if body.strip() else "\npackage() {\n    true\n}\n"
    return s


def gen_bin(pk, name, info, sums):
    base = name
    v = pk["pkgver"]
    tar = lambda a: f"{base}-{v}-{a}.tar.zst"
    extra = [f"provides=({q([base] + info['provides'])})", f"conflicts=({q([base] + info['conflicts'])})"]
    if info["replaces"]:
        extra.append(f"replaces=({q(info['replaces'])})")
    if info["backup"]:
        extra.append(f"backup=({q(info['backup'])})")
    if info["install"]:
        extra.append(f"install={info['install']}")
    extra.append("options=('!strip' '!debug')")
    for a in ("x86_64", "aarch64"):
        extra.append(f"source_{a}=(\"{tar(a)}::{URL}/releases/download/v$pkgver/{tar(a)}\")")
        extra.append(f"noextract=({q([tar('x86_64'), tar('aarch64')])})") if a == "x86_64" else None
        extra.append(f"sha256sums_{a}=('{sums.get(tar(a), 'SKIP')}')")
    s = header(f"{base}-bin", EN.get(base, info["pkgdesc"]), v, info["depends"], opt_en(info["optdepends"]), extra)
    s += f'''

package() {{
    bsdtar -xpf "$srcdir/{base}-$pkgver-$CARCH.tar.zst" -C "$pkgdir"
}}
'''
    return s


def gen_meta(pk, suffix):
    info = pk["packages"]["synshell"]
    deps = [f"{d}-{suffix}" if d in pk["packages"] else d for d in info["depends"]]
    opts = [f"{o.split(':', 1)[0].strip()}-{suffix}: {OPT_EN.get(o.split(':', 1)[0].strip(), '')}" for o in info["optdepends"]]
    extra = [f"provides=('synshell')", f"conflicts=('synshell')"]
    return header(f"synshell-{suffix}", EN["synshell"], pk["pkgver"], deps, opts, extra) + "\n\npackage() {\n    true\n}\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(HERE, "aur", "out"))
    ap.add_argument("--local", action="store_true")
    ap.add_argument("--sums")
    a = ap.parse_args()
    sums = {}
    if a.sums:
        for l in open(a.sums):
            p = l.split()
            if len(p) == 2:
                sums[p[1]] = p[0]
    pk = read_pkgbuild()
    made = []
    for name, info in pk["packages"].items():
        if name in SKIP:
            continue
        if name == "synshell":
            files = {"synshell-git": gen_meta(pk, "git"), "synshell-bin": gen_meta(pk, "bin")}
        else:
            files = {f"{name}-git": gen_git(pk, name, info, a.local), f"{name}-bin": gen_bin(pk, name, info, sums)}
        for d, text in files.items():
            os.makedirs(os.path.join(a.out, d), exist_ok=True)
            open(os.path.join(a.out, d, "PKGBUILD"), "w").write(text)
            if info["install"] and name != "synshell":
                src = os.path.join(HERE, info["install"])
                open(os.path.join(a.out, d, info["install"]), "w").write(open(src).read())
            r = subprocess.run(["makepkg", "--printsrcinfo"], cwd=os.path.join(a.out, d), capture_output=True, text=True)
            if r.returncode != 0:
                print(f"{d}: .SRCINFO не получился:\n{r.stderr}", file=sys.stderr)
                sys.exit(1)
            open(os.path.join(a.out, d, ".SRCINFO"), "w").write(r.stdout)
            made.append(d)
    print(f"{len(made)} пакетов в {a.out}")


if __name__ == "__main__":
    main()
