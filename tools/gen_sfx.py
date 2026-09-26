#!/usr/bin/env python3
"""Синтез звуковых эффектов Elle Block → assets/sfx/*.wav (48 кГц, моно, 16 бит).

Всё процедурно: синусы с гармониками, FM-«маримба», шум с фильтрами, простой
хвост реверберации. Запуск: python3 tools/gen_sfx.py
"""
import math
import os
import numpy as np
from scipy.io import wavfile
from scipy.signal import butter, sosfilt

SR = 48000
OUT = os.path.join(os.path.dirname(__file__), "..", "assets", "sfx")
rng = np.random.default_rng(2027)


def t(dur):
    return np.arange(int(SR * dur)) / SR


def env(n, a=0.002, d=0.1, s=0.0, r=0.05, hold=0.0):
    """ADSR по сэмплам (a/d/r в секундах, s — уровень сустейна, hold — длина сустейна)."""
    a_n, d_n, h_n, r_n = int(a * SR), int(d * SR), int(hold * SR), int(r * SR)
    e = np.zeros(n)
    i = 0
    seg = np.linspace(0, 1, max(a_n, 1))
    e[i:i + len(seg)] = seg[: max(0, n - i)]
    i += a_n
    seg = np.linspace(1, s, max(d_n, 1))
    e[i:i + len(seg)] = seg[: max(0, n - i)]
    i += d_n
    e[i:i + h_n] = s
    i += h_n
    seg = np.linspace(s, 0, max(r_n, 1))
    e[i:i + len(seg)] = seg[: max(0, n - i)]
    return e[:n]


def note(freq, dur, kind="marimba", vel=1.0):
    tt = t(dur)
    if kind == "marimba":
        # FM-подобный удар: основной тон + 4-я гармоника (характерный призвук маримбы)
        body = np.sin(2 * np.pi * freq * tt) * np.exp(-tt * 6.0)
        ping = 0.35 * np.sin(2 * np.pi * freq * 4.0 * tt) * np.exp(-tt * 28.0)
        shim = 0.12 * np.sin(2 * np.pi * freq * 10.0 * tt) * np.exp(-tt * 60.0)
        x = (body + ping + shim) * env(len(tt), a=0.001, d=dur * 0.6, s=0.25, r=dur * 0.35)
    elif kind == "bell":
        x = (np.sin(2 * np.pi * freq * tt) * np.exp(-tt * 3.0)
             + 0.5 * np.sin(2 * np.pi * freq * 2.76 * tt) * np.exp(-tt * 7.0)
             + 0.25 * np.sin(2 * np.pi * freq * 5.4 * tt) * np.exp(-tt * 12.0))
        x *= env(len(tt), a=0.001, d=dur * 0.7, s=0.2, r=dur * 0.3)
    elif kind == "pluck":
        # Karplus-Strong
        n = len(tt)
        period = int(SR / freq)
        buf = rng.uniform(-1, 1, period)
        x = np.zeros(n)
        for i in range(n):
            x[i] = buf[i % period]
            buf[i % period] = 0.5 * (buf[i % period] + buf[(i + 1) % period]) * 0.996
        x *= env(n, a=0.001, d=dur * 0.5, s=0.3, r=dur * 0.5)
    else:  # soft sine pad
        x = np.sin(2 * np.pi * freq * tt) + 0.3 * np.sin(2 * np.pi * freq * 2 * tt)
        x *= env(len(tt), a=0.02, d=dur * 0.5, s=0.5, r=dur * 0.5)
    return x * vel


def noise_burst(dur, lo, hi, decay=40.0):
    n = int(SR * dur)
    x = rng.normal(0, 1, n)
    sos = butter(4, [lo, hi], btype="band", fs=SR, output="sos")
    x = sosfilt(sos, x)
    return x * np.exp(-np.arange(n) / SR * decay)


def reverb(x, tail=0.35, wet=0.18):
    n = int(SR * tail)
    ir = rng.normal(0, 1, n) * np.exp(-np.arange(n) / SR * (6.0 / tail))
    ir[0] = 0
    ir /= np.sqrt(np.sum(ir ** 2)) + 1e-9
    wet_sig = np.convolve(x, ir)
    dry = np.pad(x, (0, len(wet_sig) - len(x)))
    return dry + wet * wet_sig


def mix(*parts):
    n = max(len(p) for p in parts)
    out = np.zeros(n)
    for p in parts:
        out[: len(p)] += p
    return out


def at(x, offset):
    return np.pad(x, (int(offset * SR), 0))


def finish(x, gain=0.9, fade=0.01):
    x = np.tanh(x * 1.2)  # мягкое ограничение
    peak = np.max(np.abs(x)) + 1e-9
    x = x / peak * gain
    f = int(SR * fade)
    x[-f:] *= np.linspace(1, 0, f)
    return x


def save(name, x):
    x = finish(x)
    wavfile.write(os.path.join(OUT, name + ".wav"), SR, (x * 32767).astype(np.int16))
    print(f"{name:10s} {len(x) / SR:5.2f}s")


def midi(n):
    return 440.0 * 2 ** ((n - 69) / 12)


os.makedirs(OUT, exist_ok=True)

# Взять фигуру: короткий мягкий тик
tt = t(0.06)
pick = np.sin(2 * np.pi * (700 + 500 * tt / 0.06) * tt) * env(len(tt), a=0.001, d=0.03, s=0.0, r=0.03)
save("pick", mix(pick * 0.6, noise_burst(0.03, 2000, 6000, 90) * 0.15))

# Поставить: глухой «ток» — низкий удар + короткий щелчок
tt = t(0.16)
thock = np.sin(2 * np.pi * (170 * np.exp(-tt * 12) + 90) * tt) * np.exp(-tt * 26)
click = noise_burst(0.02, 1500, 7000, 160) * 0.5
save("place", mix(thock * 1.0, click))

# Вернуть в трей: нисходящий блип
tt = t(0.12)
back = np.sin(2 * np.pi * (420 * np.exp(-tt * 9)) * tt) * env(len(tt), a=0.002, d=0.08, s=0.0, r=0.04)
save("back", back * 0.5)

# Нельзя: тихий глухой стук
tt = t(0.1)
save("deny", mix(np.sin(2 * np.pi * 110 * tt) * np.exp(-tt * 40) * 0.7, noise_burst(0.03, 200, 900, 120) * 0.3))

# Очистка линий: маримбовые аккорды, по нарастающей
def chord(notes, spacing, dur, kind="marimba", vel=0.9):
    return mix(*[at(note(midi(n), dur, kind, vel), i * spacing) for i, n in enumerate(notes)])

save("clear1", reverb(chord([84, 88], 0.045, 0.5)))                       # C6 E6
save("clear2", reverb(chord([84, 88, 91], 0.05, 0.6)))                    # + G6
save("clear3", reverb(mix(chord([79, 84, 88, 91, 96], 0.045, 0.75), noise_burst(0.25, 4000, 12000, 18) * 0.25), 0.5, 0.25))
save("clear4", reverb(mix(chord([79, 84, 88, 91, 96, 100], 0.04, 0.9, "bell", 0.8), noise_burst(0.35, 3000, 12000, 14) * 0.3), 0.7, 0.3))

# Идеальная очистка: арфовое глиссандо + колокол
gliss = [72, 76, 79, 84, 88, 91, 96, 100, 103, 108]
save("perfect", reverb(mix(chord(gliss, 0.055, 0.9, "pluck", 0.7), at(note(midi(96), 1.4, "bell", 0.8), 0.5),
                           noise_burst(0.6, 5000, 14000, 9) * 0.25), 0.9, 0.35))

# Новый рекорд: три ноты фанфары
save("best", reverb(mix(at(note(midi(84), 0.35, "bell", 0.8), 0.0), at(note(midi(88), 0.35, "bell", 0.8), 0.12),
                        at(note(midi(91), 0.9, "bell", 1.0), 0.24), at(note(midi(96), 1.0, "bell", 0.6), 0.24)), 0.8, 0.3))

# Конец игры: нисходящий минорный мотив
over = mix(at(note(midi(76), 0.5, "pad", 0.8), 0.0), at(note(midi(72), 0.5, "pad", 0.8), 0.28),
           at(note(midi(69), 0.6, "pad", 0.8), 0.56), at(note(midi(64), 1.4, "pad", 0.9), 0.84),
           at(note(midi(52), 1.6, "pad", 0.5), 0.84))
save("over", reverb(over, 0.9, 0.3))

# Добор фигур: три быстрых мягких «попа» по восходящей
pops = []
for i, f in enumerate([520, 660, 800]):
    tt = t(0.07)
    pops.append(at(np.sin(2 * np.pi * f * tt) * env(len(tt), a=0.002, d=0.04, s=0.0, r=0.03) * 0.6, i * 0.07))
save("refill", mix(*pops))

# Клик по кнопке
save("click", mix(noise_burst(0.025, 1200, 5000, 120) * 0.6, np.sin(2 * np.pi * 1000 * t(0.03)) * np.exp(-t(0.03) * 120) * 0.5))

# Комбо-акцент (накладывается на clear): восходящий «свуш» + динь
tt = t(0.35)
swoosh = noise_burst(0.35, 800, 9000, 8) * np.linspace(0.2, 1.0, len(tt)) * 0.35
save("combo", reverb(mix(swoosh, at(note(midi(100), 0.4, "bell", 0.7), 0.25)), 0.5, 0.25))

# Материалы при очистке
# дерево: сухой треск — серия коротких щелчков + низкий удар
cracks = [at(noise_burst(0.03, 900, 4000, 140) * (0.5 + 0.5 * rng.uniform()), 0.03 * i + 0.01 * rng.uniform()) for i in range(7)]
tt = t(0.25)
save("wood", mix(*cracks, np.sin(2 * np.pi * (140 * np.exp(-tt * 10) + 70) * tt) * np.exp(-tt * 18) * 0.7))
# хрусталь: звон осколков — высокие колокольчики вразнобой
shards = [at(note(midi(96 + rng.integers(0, 12)), 0.5, "bell", 0.35), 0.02 * i) for i in range(9)]
save("crystal", reverb(mix(*shards, noise_burst(0.15, 6000, 15000, 30) * 0.3), 0.7, 0.35))
# золото: россыпь монет — металлические «дзинь» с быстрым затуханием
coins = []
for i in range(8):
    f = 2200 + rng.uniform(-300, 900)
    tt = t(0.18)
    coins.append(at((np.sin(2 * np.pi * f * tt) + 0.6 * np.sin(2 * np.pi * f * 2.41 * tt)) * np.exp(-tt * 22) * 0.35, 0.035 * i + 0.01 * rng.uniform()))
save("gold", reverb(mix(*coins), 0.5, 0.25))
# лава: глухой «вуф» пламени + шипение
tt = t(0.5)
whoomp = np.sin(2 * np.pi * (60 + 40 * np.exp(-tt * 6)) * tt) * env(len(tt), a=0.01, d=0.2, s=0.3, r=0.25)
hiss = noise_burst(0.5, 1500, 9000, 6) * np.linspace(0.2, 1.0, len(tt)) * 0.35
save("lava", reverb(mix(whoomp * 0.9, hiss), 0.4, 0.2))
