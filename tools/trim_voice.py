#!/usr/bin/env python3
"""Обрезка реплики: оставить только начало — первые речевые сегменты, которые
укладываются в ожидаемую длину фразы. VoxCPM2 после короткой фразы часто
дописывает лишние слова/бормотание; они отбрасываются.

Usage: trim_voice.py in.wav out.wav "<текст фразы>"
Выход — 48 кГц float32 WAV без нормализации (её делает ffmpeg дальше).
"""
import sys
import numpy as np
from scipy.io import wavfile

WIN = 0.02
THRESH_DB = -40.0
GAP = 0.30


def segments(x, sr):
    win = int(sr * WIN)
    n = len(x) // win
    e = 20 * np.log10(np.sqrt((x[: n * win].reshape(n, win) ** 2).mean(1)) + 1e-9)
    on = e > THRESH_DB
    segs, start = [], None
    for i, v in enumerate(on):
        if v and start is None:
            start = i
        if not v and start is not None:
            if i - start >= 3:
                segs.append((start * WIN, i * WIN))
            start = None
    if start is not None:
        segs.append((start * WIN, n * WIN))
    merged = []
    for s in segs:
        if merged and s[0] - merged[-1][1] < GAP:
            merged[-1] = (merged[-1][0], s[1])
        else:
            merged.append(s)
    return merged


def main():
    src, dst, text = sys.argv[1], sys.argv[2], sys.argv[3]
    sr, x = wavfile.read(src)
    x = x.astype(np.float32)
    if x.dtype != np.float32 or np.abs(x).max() > 1.5:
        x = x / 32768.0
    if x.ndim > 1:
        x = x[:, 0]
    words = max(1, len(text.split()))
    limit = 0.5 * words + 0.9
    segs = segments(x, sr)
    end = None
    for a, b in segs:
        if b <= limit or end is None:
            end = b
        else:
            break
    if end is None:
        end = len(x) / sr
    a0 = max(0.0, segs[0][0] - 0.05) if segs else 0.0
    b0 = min(len(x) / sr, end + 0.08)
    y = x[int(a0 * sr): int(b0 * sr)]
    # мягкий фейд по краям
    f = int(sr * 0.01)
    y[:f] *= np.linspace(0, 1, f)
    y[-f:] *= np.linspace(1, 0, f)
    wavfile.write(dst, sr, y.astype(np.float32))
    print(f"{src}: {len(x) / sr:.2f}s → {len(y) / sr:.2f}s (сегментов {len(segs)}, лимит {limit:.1f}s)")


if __name__ == "__main__":
    main()
