#!/usr/bin/env python3
"""Синтез звуковых тем synshell (короткие звуки интерфейса).

Каждая тема — свой «инструмент» (стекло, консоль, челеста, 8 бит, дерево) и
свой лад; события freedesktop (уведомление, громкость, снимок, блокировка,
устройства…) и игрового режима (навигация, выбор, назад, вкладки, меню…)
собираются из одних и тех же приёмов: нот с огибающей, шумовых щелчков и
«вжухов», лёгкой реверберации. Длинные мелодичные звуки (вход, выход,
заставка игрового режима, запуск игры, «готово») берутся из ai/<тема>/<событие>.flac,
если там есть сгенерированный нейросетью вариант (см. ai.py), иначе тоже
синтезируются.

    python3 -I data/sounds/gen/synth.py data/sounds/themes
"""

import json
import os
import sys

import numpy as np
import soundfile
from scipy.io import wavfile
from scipy.signal import butter, fftconvolve, sosfilt

SR = 48000
RNG = np.random.default_rng(20261020)


def t_axis(dur):
    return np.arange(int(SR * dur)) / SR


def env(n, attack=0.004, decay=0.25, curve=1.0):
    """Огибающая: быстрая атака, экспоненциальный спад (decay — до −60 дБ)."""
    t = np.arange(n) / SR
    a = np.clip(t / max(attack, 1e-4), 0, 1)
    d = np.exp(-6.9 * (t / max(decay, 1e-3)) ** curve)
    return a * d


def note(freq, dur, inst, amp=1.0):
    t = t_axis(dur)
    n = len(t)
    kind = inst["kind"]
    if kind == "square":
        duty = inst.get("duty", 0.5)
        ph = (t * freq) % 1.0
        y = np.where(ph < duty, 1.0, -1.0) * 0.5
        # Лёгкое сглаживание ступенек, чтобы не резало.
        y = sosfilt(butter(2, min(9000, SR / 2 - 100), fs=SR, output="sos"), y)
        e = env(n, 0.002, inst.get("decay", 0.18), 0.6)
        if inst.get("arp_slide"):
            y *= 1.0
        return y * e * amp
    partials = inst["partials"]  # [(отношение, громкость, спад-множитель)]
    y = np.zeros(n)
    for ratio, gain, dmul in partials:
        f = freq * ratio
        if f >= SR / 2 - 200:
            continue
        y += gain * np.sin(2 * np.pi * f * t + RNG.uniform(0, 2 * np.pi)) * env(n, inst.get("attack", 0.003), inst["decay"] * dmul)
    if inst.get("vibrato"):
        pass
    return y * amp


def noise_click(dur=0.02, lo=1500, hi=9000, amp=0.4, decay=0.012):
    n = int(SR * dur)
    w = RNG.standard_normal(n)
    w = sosfilt(butter(2, [lo, min(hi, SR / 2 - 100)], btype="band", fs=SR, output="sos"), w)
    return w / (np.abs(w).max() + 1e-9) * env(n, 0.0005, decay) * amp


def thump(freq=110, dur=0.09, amp=0.7):
    t = t_axis(dur)
    f = freq * (1 + 0.8 * np.exp(-t / 0.012))  # лёгкий «пик» высоты в начале
    ph = 2 * np.pi * np.cumsum(f) / SR
    return np.sin(ph) * env(len(t), 0.001, dur * 0.8) * amp


def whoosh(dur=0.3, f0=600, f1=4000, amp=0.5, q=1.5):
    """Шум с плывущей полосой (по блокам), огибающая — подъём и спад."""
    n = int(SR * dur)
    w = RNG.standard_normal(n)
    out = np.zeros(n)
    blk = 512
    for i in range(0, n, blk):
        k = i / max(n - 1, 1)
        fc = f0 * (f1 / f0) ** k
        bw = fc / q
        lo, hi = max(40, fc - bw / 2), min(SR / 2 - 200, fc + bw / 2)
        seg = w[max(0, i - 256):i + blk]
        y = sosfilt(butter(2, [lo, hi], btype="band", fs=SR, output="sos"), seg)
        out[i:i + blk] = y[-len(out[i:i + blk]):]
    t = np.arange(n) / n
    shape = np.sin(np.pi * t) ** 1.5
    out = out / (np.abs(out).max() + 1e-9)
    return out * shape * amp


def place(buf, sig, at):
    i = int(at * SR)
    end = min(len(buf), i + len(sig))
    buf[i:end] += sig[: end - i]


def reverb(x, size=0.6, wet=0.18, width=1.0):
    """Синтетический зал: экспоненциальный шум, свой для каждого канала."""
    if wet <= 0:
        return np.stack([x, x], 1)
    n = int(SR * (0.25 + 1.4 * size))
    t = np.arange(n) / SR
    decay = 0.3 + 1.2 * size
    out = []
    for ch in range(2):
        ir = RNG.standard_normal(n) * np.exp(-6.9 * t / decay)
        ir = sosfilt(butter(1, 7000, fs=SR, output="sos"), ir)
        ir[: int(0.012 * SR)] = 0
        ir /= np.sqrt((ir ** 2).sum()) + 1e-9
        r = fftconvolve(x, ir)
        dry = np.pad(x, (0, len(r) - len(x)))
        out.append(dry * (1 - wet * 0.5) + r * wet * (1.0 if ch == 0 else width))
    y = np.stack(out, 1)
    return y


def finish(y, peak_db=-3.0, tail=True):
    """Обрезать тишину в конце, плавно погасить, нормализовать."""
    if y.ndim == 1:
        y = np.stack([y, y], 1)
    a = np.abs(y).max(1)
    thr = a.max() * 10 ** (-50 / 20)
    idx = np.nonzero(a > thr)[0]
    if len(idx):
        y = y[: idx[-1] + 1]
    fade = min(len(y) // 4, int(0.03 * SR))
    if fade > 0 and tail:
        y[-fade:] *= np.linspace(1, 0, fade)[:, None]
    m = np.abs(y).max()
    if m > 0:
        y = y / m * 10 ** (peak_db / 20)
    return y


def write(path, y):
    """FLAC 16 бит: без потерь и вдвое меньше WAV."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    soundfile.write(path, np.clip(y, -1, 1), SR, subtype="PCM_16", format="FLAC")


# ─── инструменты и темы ─────────────────────────────────────────────────────

GLASS = {"kind": "add", "partials": [(1, 1.0, 1.0), (2.0, 0.25, 0.6), (2.76, 0.35, 0.45), (5.4, 0.12, 0.25)], "decay": 0.55, "attack": 0.002}
SOFT = {"kind": "add", "partials": [(1, 1.0, 1.0), (2, 0.18, 0.5), (3, 0.06, 0.3)], "decay": 0.35, "attack": 0.004}
CELESTA = {"kind": "add", "partials": [(1, 1.0, 1.0), (3.01, 0.42, 0.5), (6.13, 0.22, 0.3), (9.4, 0.08, 0.2)], "decay": 0.7, "attack": 0.001}
SQUARE = {"kind": "square", "duty": 0.25, "decay": 0.16}
MARIMBA = {"kind": "add", "partials": [(1, 1.0, 1.0), (3.93, 0.45, 0.22), (9.9, 0.12, 0.08)], "decay": 0.42, "attack": 0.0015}
KALIMBA = {"kind": "add", "partials": [(1, 1.0, 1.0), (5.47, 0.25, 0.25), (2.0, 0.1, 0.5)], "decay": 0.6, "attack": 0.001}


def midi(n):
    return 440.0 * 2 ** ((n - 69) / 12)


THEMES = {
    "synshell": {
        "name": "synshell", "name_ru": "synshell",
        "comment": "Soft glass chimes — the default synshell theme", "comment_ru": "Мягкое стекло — тема synshell по умолчанию",
        "inst": GLASS, "tick": SOFT, "root": 72, "scale": [0, 2, 4, 7, 9], "verb": (0.5, 0.2), "click": "soft",
        "ai": "soft glass chimes, warm airy synth pad, gentle",
    },
    "synshell-console": {
        "name": "synshell Console", "name_ru": "synshell Консоль",
        "comment": "Deep soft clicks and airy whooshes, game console style", "comment_ru": "Глубокие мягкие щелчки и воздушные переходы, как у игровой консоли",
        "inst": SOFT, "tick": SOFT, "root": 64, "scale": [0, 3, 5, 7, 10], "verb": (0.45, 0.16), "click": "thock",
        "ai": "modern game console UI, deep warm synth, soft bass, airy pad, sleek, cinematic",
    },
    "synshell-crystal": {
        "name": "synshell Crystal", "name_ru": "synshell Хрусталь",
        "comment": "Bright celesta and shimmering glass", "comment_ru": "Яркая челеста и мерцающее стекло",
        "inst": CELESTA, "tick": CELESTA, "root": 79, "scale": [0, 2, 4, 7, 11], "verb": (0.75, 0.28), "click": "glass",
        "ai": "bright celesta, crystal glass bells, shimmering sparkles, magical",
    },
    "synshell-retro": {
        "name": "synshell Retro", "name_ru": "synshell Ретро",
        "comment": "8-bit chiptune blips", "comment_ru": "8-битные чиптюн-сигналы",
        "inst": SQUARE, "tick": SQUARE, "root": 72, "scale": [0, 4, 7, 12, 16], "verb": (0.0, 0.0), "click": "chip",
        "ai": "8-bit chiptune, retro video game, square wave arpeggio, NES style",
    },
    "synshell-wood": {
        "name": "synshell Wood", "name_ru": "synshell Дерево",
        "comment": "Marimba and kalimba, warm and organic", "comment_ru": "Маримба и калимба — тёплые деревянные звуки",
        "inst": MARIMBA, "tick": KALIMBA, "root": 67, "scale": [0, 2, 4, 7, 9], "verb": (0.35, 0.12), "click": "wood",
        "ai": "marimba, kalimba, wooden percussion, warm, organic, acoustic",
    },
}


def deg(th, d, octave=0):
    """Ступень лада темы → частота."""
    sc = th["scale"]
    o, i = divmod(d, len(sc))
    return midi(th["root"] + 12 * (o + octave) + sc[i])


def seq(th, steps, inst=None, gap=0.07, dur=0.5, amp=1.0, octave=0):
    inst = inst or th["inst"]
    total = gap * len(steps) + dur + 0.05
    buf = np.zeros(int(SR * total))
    for k, d in enumerate(steps):
        if d is None:
            continue
        place(buf, note(deg(th, d, octave), dur, inst, amp * (0.85 ** k if k else 1.0)), k * gap)
    return buf


def click(th, amp=1.0, pitch=0):
    kind = th["click"]
    buf = np.zeros(int(SR * 0.08))
    if kind == "thock":
        place(buf, thump(150 * 2 ** (pitch / 12), 0.05, 0.8 * amp), 0)
        place(buf, noise_click(0.015, 800, 4000, 0.25 * amp, 0.006), 0)
    elif kind == "glass":
        place(buf, note(deg(th, 4 + pitch, 1), 0.07, CELESTA, 0.6 * amp), 0)
    elif kind == "chip":
        place(buf, note(deg(th, 2 + pitch, 1), 0.035, {"kind": "square", "duty": 0.5, "decay": 0.03}, 0.7 * amp), 0)
    elif kind == "wood":
        place(buf, note(deg(th, 3 + pitch, 1), 0.06, MARIMBA | {"decay": 0.05}, 0.8 * amp), 0)
        place(buf, noise_click(0.008, 2000, 6000, 0.12 * amp, 0.003), 0)
    else:
        place(buf, note(deg(th, 4 + pitch, 1), 0.05, SOFT | {"decay": 0.04}, 0.7 * amp), 0)
        place(buf, noise_click(0.01, 3000, 9000, 0.1 * amp, 0.004), 0)
    return buf


def fx(th, y, verb_scale=1.0):
    size, wet = th["verb"]
    return reverb(y, size, wet * verb_scale)


def build(th):
    """Все события темы: имя → стерео-сигнал."""
    inst, tick = th["inst"], th["tick"]
    retro = th["click"] == "chip"
    s = {}
    # ── игровой режим ──
    s["gameshell-navigate"] = fx(th, click(th, 0.8), 0.4)
    bump = np.zeros(int(SR * 0.16))
    place(bump, thump(70, 0.12, 0.7) if not retro else note(midi(48), 0.08, SQUARE, 0.5), 0)
    place(bump, click(th, 0.3, -5), 0)
    s["gameshell-bump"] = fx(th, bump, 0.3)
    sel = seq(th, [2, 4], tick if retro else inst, gap=0.055, dur=0.28, amp=0.9, octave=1)
    place(sel, click(th, 0.6), 0)
    s["gameshell-select"] = fx(th, sel)
    s["gameshell-back"] = fx(th, seq(th, [4, 1], tick if retro else inst, gap=0.06, dur=0.25, amp=0.8, octave=1))
    tab = np.zeros(int(SR * 0.22))
    if not retro:
        place(tab, whoosh(0.16, 900, 3500, 0.35), 0)
    place(tab, note(deg(th, 3, 1), 0.18, tick, 0.5), 0.02)
    s["gameshell-tab"] = fx(th, tab, 0.6)
    mo = np.zeros(int(SR * 0.5))
    if not retro:
        place(mo, whoosh(0.3, 500, 5000, 0.45), 0)
    place(mo, seq(th, [0, 2, 4], inst, gap=0.045, dur=0.32, amp=0.6), 0.04)
    s["gameshell-menu-open"] = fx(th, mo)
    mc = np.zeros(int(SR * 0.4))
    if not retro:
        place(mc, whoosh(0.24, 5000, 600, 0.4), 0)
    place(mc, seq(th, [4, 2], inst, gap=0.05, dur=0.25, amp=0.5), 0.03)
    s["gameshell-menu-close"] = fx(th, mc)
    s["gameshell-favorite"] = fx(th, seq(th, [2, 4, 5, 7], CELESTA if not retro else SQUARE, gap=0.05, dur=0.45, amp=0.7, octave=1), 1.3)
    launch = np.zeros(int(SR * 1.6))
    if not retro:
        place(launch, whoosh(0.7, 300, 6000, 0.55), 0)
    for k, d in enumerate([0, 2, 4, 7]):
        place(launch, note(deg(th, d), 1.1, inst, 0.6), 0.25 + 0.06 * k)
    s["gameshell-launch"] = fx(th, launch, 1.3)
    st = np.zeros(int(SR * 2.6))
    for k, d in enumerate([0, 2, 4, 5, 7, 9]):
        place(st, note(deg(th, d), 1.6, inst, 0.55), 0.12 * k)
    if not retro:
        place(st, whoosh(1.2, 200, 3000, 0.25), 0)
    s["gameshell-startup"] = fx(th, st, 1.5)
    # ── события freedesktop ──
    s["button-pressed"] = fx(th, click(th, 0.7), 0.3)
    s["audio-volume-change"] = fx(th, note(deg(th, 4, 1), 0.09, tick | ({"decay": 0.07} if not retro else {}), 0.8), 0.4)
    s["message-new-instant"] = fx(th, seq(th, [2, 4, 7], inst, gap=0.09, dur=0.6, amp=0.9, octave=1))
    s["message"] = s["message-new-instant"]
    s["dialog-information"] = fx(th, seq(th, [4], inst, dur=0.7, octave=1))
    s["dialog-warning"] = fx(th, seq(th, [4, 4], inst, gap=0.14, dur=0.4))
    err = seq(th, [3, 0], inst, gap=0.13, dur=0.45, octave=0)
    place(err, thump(90, 0.12, 0.4), 0)
    s["dialog-error"] = fx(th, err)
    s["bell"] = fx(th, seq(th, [5], inst, dur=0.9, octave=1))
    s["complete"] = fx(th, seq(th, [0, 2, 4, 7], inst, gap=0.08, dur=0.7, amp=0.8, octave=1), 1.2)
    shutter = np.zeros(int(SR * 0.2))
    place(shutter, noise_click(0.05, 1200, 9000, 0.7, 0.02), 0)
    place(shutter, noise_click(0.04, 800, 6000, 0.5, 0.015), 0.07)
    place(shutter, click(th, 0.5), 0.0)
    s["screen-capture"] = fx(th, shutter, 0.5)
    s["desktop-screen-lock"] = fx(th, seq(th, [4, 2, 0], inst, gap=0.07, dur=0.4, amp=0.8))
    s["desktop-screen-unlock"] = fx(th, seq(th, [0, 2, 4], inst, gap=0.07, dur=0.4, amp=0.8))
    login = np.zeros(int(SR * 2.4))
    for k, d in enumerate([0, 4, 7, 9]):
        place(login, note(deg(th, d), 1.4, inst, 0.6), 0.15 * k)
    s["desktop-login"] = fx(th, login, 1.5)
    s["desktop-logout"] = fx(th, seq(th, [7, 4, 2, 0], inst, gap=0.14, dur=0.9, amp=0.7), 1.3)
    s["device-added"] = fx(th, seq(th, [0, 4], inst, gap=0.08, dur=0.4, octave=1))
    s["device-removed"] = fx(th, seq(th, [4, 0], inst, gap=0.08, dur=0.4, octave=1))
    s["power-plug"] = fx(th, seq(th, [0, 2, 4], tick, gap=0.05, dur=0.3, octave=1))
    s["power-unplug"] = fx(th, seq(th, [4, 2, 0], tick, gap=0.05, dur=0.3, octave=1))
    s["battery-low"] = fx(th, seq(th, [1, None, 1], inst, gap=0.18, dur=0.3, amp=0.8))
    return s


# Длинные мелодичные — можно заменить нейросетевыми (ai/<тема>/<событие>.wav).
AI_EVENTS = ["desktop-login", "desktop-logout", "gameshell-startup", "gameshell-launch", "complete"]


def index_theme(tid, th):
    return (
        "[Sound Theme]\n"
        f"Name={th['name']}\n"
        f"Name[ru]={th['name_ru']}\n"
        f"Comment={th['comment']}\n"
        f"Comment[ru]={th['comment_ru']}\n"
        + ("Inherits=synshell\n" if tid != "synshell" else "Inherits=freedesktop\n")
        + "Directories=stereo\n\n[stereo]\nOutputProfile=stereo\n"
    )


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "data/sounds/themes"
    here = os.path.dirname(os.path.abspath(__file__))
    ai_dir = os.path.join(here, "..", "ai")
    report = {}
    for tid, th in THEMES.items():
        sounds = build(th)
        # Прошлая сборка (другие форматы) — убрать, чтобы не было двух файлов события.
        sd = os.path.join(out, tid, "stereo")
        if os.path.isdir(sd):
            for f in os.listdir(sd):
                os.remove(os.path.join(sd, f))
        used_ai = []
        for ev in AI_EVENTS:
            p = os.path.join(ai_dir, tid, f"{ev}.flac")
            if os.path.isfile(p):
                y, sr = soundfile.read(p, dtype="float32")
                if sr != SR:
                    raise SystemExit(f"{p}: частота {sr}, нужна {SR}")
                sounds[ev] = y if y.ndim == 2 else np.stack([y, y], 1)
                used_ai.append(ev)
        d = os.path.join(out, tid)
        os.makedirs(os.path.join(d, "stereo"), exist_ok=True)
        with open(os.path.join(d, "index.theme"), "w") as f:
            f.write(index_theme(tid, th))
        for ev, y in sounds.items():
            peak = -9.0 if ev in ("gameshell-navigate", "button-pressed") else -4.0
            write(os.path.join(d, "stereo", f"{ev}.flac"), finish(y, peak, tail=ev not in used_ai))
        report[tid] = {"sounds": len(sounds), "ai": used_ai}
    print(json.dumps(report, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
