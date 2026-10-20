#!/usr/bin/env python3
"""Мелодичные звуки тем нейросетью (synaptixd, ACE-Step; YuE2 — запасной).

Для каждой темы и события (вход, выход, заставка игрового режима, запуск
игры, «готово») генерируется несколько вариантов, из них выбирается лучший
по огибающей (быстрая атака, ясный спад, не «ровная простыня»), спектру и
отсутствию перегруза; затем — обрезка тишины, затухание, нормализация.
Результат — data/sounds/ai/<тема>/<событие>.flac (48 кГц, стерео), его
подхватывает synth.py.

    python3 -I data/sounds/gen/ai.py [тема…]
"""

import os
import subprocess
import sys
import tempfile

import numpy as np
from scipy.io import wavfile
from scipy.signal import resample_poly

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import synth  # noqa: E402

SR = 48000
MODEL = os.environ.get("MODEL", "acestep-v15-xl-turbo")
CANDIDATES = int(os.environ.get("CANDIDATES", "3"))
CTL = os.environ.get("SYNAPTIXCTL", "synaptixctl")

# событие → (описание, длительность генерации, предел после обрезки)
EVENTS = {
    "desktop-login": ("short welcome jingle, ascending hopeful melody, logo sting", 6, 3.4),
    "desktop-logout": ("short goodbye jingle, gentle descending melody, calm resolution", 5, 2.4),
    "gameshell-startup": ("video game console boot sound, logo reveal sting, epic but short", 6, 3.4),
    "gameshell-launch": ("game launch sting, rising whoosh into a bright chord", 5, 2.2),
    "complete": ("success chime, short achievement unlocked sound", 4, 1.6),
}


def load(path):
    sr, y = wavfile.read(path)
    y = y.astype(np.float32)
    if np.abs(y).max() > 2:
        y /= 32768.0
    if y.ndim == 1:
        y = np.stack([y, y], 1)
    if sr != SR:
        y = resample_poly(y, SR, sr, axis=0).astype(np.float32)
    return y


def rms_curve(y, win=0.03):
    m = y.mean(1)
    n = int(SR * win)
    k = len(m) // n
    return np.sqrt((m[: k * n].reshape(k, n) ** 2).mean(1)) if k else np.zeros(1)


def trim(y, cap):
    """С первого звука (−35 дБ от пика) до спада (−40 дБ) или предела."""
    a = np.abs(y).max(1)
    peak = a.max()
    if peak <= 1e-6:
        return None
    start_i = np.nonzero(a > peak * 10 ** (-35 / 20))[0]
    start = max(0, start_i[0] - int(0.005 * SR)) if len(start_i) else 0
    y = y[start:]
    r = rms_curve(y)
    pk = int(np.argmax(r))
    thr = r.max() * 10 ** (-40 / 20)
    after = np.nonzero(r[pk:] < thr)[0]
    end = (pk + after[0]) * int(0.03 * SR) if len(after) else len(y)
    end = min(end, int(cap * SR))
    y = y[:end].copy()
    fade = min(len(y) // 3, int(0.35 * SR))
    if fade:
        y[-fade:] *= (np.linspace(1, 0, fade) ** 1.6)[:, None]
    return y


def score(y):
    """Больше — лучше: динамика (пик/среднее), спад к концу, яркость в меру."""
    if y is None or len(y) < int(0.4 * SR):
        return -1e9
    r = rms_curve(y)
    dyn = r.max() / (r.mean() + 1e-9)
    pk_pos = np.argmax(r) / len(r)
    tail = r[int(len(r) * 0.8):].mean() / (r.max() + 1e-9)
    x = y.mean(1)
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x))))
    fr = np.fft.rfftfreq(len(x), 1 / SR)
    cen = (spec * fr).sum() / (spec.sum() + 1e-9)
    clip = (np.abs(y) > 0.99).mean()
    s = dyn * 1.0 - tail * 2.0 - abs(pk_pos - 0.25) * 1.5 - clip * 50
    if not 500 < cen < 8000:
        s -= 2
    return float(s)


def generate(theme_id, event, out_dir):
    th = synth.THEMES[theme_id]
    desc, secs, cap = EVENTS[event]
    prompt = f"instrumental, {desc}, {th['ai']}, clean mix, no vocals, no drums"
    best = (-1e9, None, None)
    with tempfile.TemporaryDirectory() as tmp:
        for k in range(CANDIDATES):
            seed = 1000 + hash((theme_id, event, k)) % 100000
            path = os.path.join(tmp, f"c{k}.wav")
            env = dict(os.environ, SECONDS=str(secs), SEED=str(seed), LYRICS="[instrumental]")
            r = subprocess.run([CTL, "music", MODEL, path, prompt], env=env, capture_output=True, text=True)
            if r.returncode != 0 or not os.path.isfile(path):
                print(f"  {event} #{k}: ошибка {r.stderr.strip()[-200:]}", file=sys.stderr)
                continue
            y = trim(load(path), cap)
            sc = score(y)
            print(f"  {event} #{k} seed={seed}: оценка {sc:.2f}, {len(y) / SR if y is not None else 0:.2f} с", flush=True)
            if sc > best[0]:
                best = (sc, y, seed)
    if best[1] is None:
        return None
    y = best[1] / (np.abs(best[1]).max() + 1e-9) * 10 ** (-3 / 20)
    os.makedirs(out_dir, exist_ok=True)
    p = os.path.join(out_dir, f"{event}.flac")
    synth.soundfile.write(p, np.clip(y, -1, 1), SR, subtype="PCM_16", format="FLAC")
    return p, best[0], best[2]


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    themes = sys.argv[1:] or list(synth.THEMES)
    for tid in themes:
        print(f"# {tid}", flush=True)
        for ev in EVENTS:
            r = generate(tid, ev, os.path.join(here, "..", "ai", tid))
            if r:
                print(f"→ {r[0]} (оценка {r[1]:.2f}, зерно {r[2]})", flush=True)


if __name__ == "__main__":
    main()
