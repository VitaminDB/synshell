#!/usr/bin/env python3
"""Сборка атласа отделок «Эльвиры»: assets/tex/raw/<id>.png → assets/tex/atlas.jpg.

Сетка 8×4 тайлов по 256 px (2048×1024). Содержимое тайла — 224 px, вокруг
16 px «гаттера» — продолжение краёв, чтобы на mip-уровнях не протекали
соседи. Порядок тайлов = порядок в `src/ui/finish.rs::FINISHES`.
Красящиеся отделки (tint) переводятся в нейтральный серый с нормализацией
контраста: цвет фигуры умножается в игре. Запуск: python3 tools/build_atlas.py
"""
import os
import numpy as np
from PIL import Image, ImageFilter

DIR = os.path.join(os.path.dirname(__file__), "..")
RAW = os.path.join(DIR, "assets", "tex", "raw")
OUT = os.path.join(DIR, "assets", "tex", "atlas.jpg")
TILE, GUTTER, COLS, ROWS = 256, 16, 8, 4
CONTENT = TILE - GUTTER * 2

# (id, tint, яркость целевая для tint 0..1 — насколько светлой должна быть серая база)
FINISHES = [
    ("gold", False, 0), ("silver", False, 0), ("copper", False, 0), ("bronze", False, 0),
    ("glass", True, 0.82), ("crystal", True, 0.80), ("velvet", True, 0.62), ("oak", False, 0),
    ("walnut", False, 0), ("marble", False, 0), ("granite", False, 0), ("obsidian", False, 0),
    ("lava", False, 0), ("ice", False, 0), ("jade", False, 0), ("gem", True, 0.80),
    ("pearl", False, 0), ("neon", True, 0.30), ("leather", False, 0), ("denim", False, 0),
    ("porcelain", True, 0.86), ("candy", True, 0.80), ("chocolate", False, 0), ("hologram", False, 0),
]


def load_tile(name, tint, target):
    path = os.path.join(RAW, name + ".png")
    if os.path.exists(path):
        im = Image.open(path).convert("RGB")
        w, h = im.size
        s = min(w, h)
        im = im.crop(((w - s) // 2, (h - s) // 2, (w - s) // 2 + s, (h - s) // 2 + s))
        im = im.resize((CONTENT, CONTENT), Image.LANCZOS)
    else:
        print(f"  ! нет {path}, процедурная заглушка")
        im = procedural(name)
    if tint:
        g = np.asarray(im.convert("L"), dtype=np.float32) / 255.0
        # нормализация контраста и сдвиг к целевой яркости
        lo, hi = np.percentile(g, 2), np.percentile(g, 98)
        g = np.clip((g - lo) / max(hi - lo, 1e-3), 0, 1)
        g = target + (g - g.mean()) * 0.55
        g = np.clip(g, 0, 1)
        im = Image.fromarray((np.stack([g, g, g], axis=-1) * 255).astype(np.uint8))
    return im


def procedural(name):
    """Заглушка на случай отсутствия файла: шумовая фактура."""
    rng = np.random.default_rng(abs(hash(name)) % (2**32))
    n = rng.normal(0.5, 0.12, (CONTENT, CONTENT)).clip(0, 1)
    im = Image.fromarray((np.stack([n, n, n], axis=-1) * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.2))
    return im


def main():
    atlas = Image.new("RGB", (TILE * COLS, TILE * ROWS), (40, 40, 48))
    for i, (name, tint, target) in enumerate(FINISHES):
        tile = load_tile(name, tint, target)
        # гаттер: тайл с растянутыми краями (pad по краю)
        padded = Image.new("RGB", (TILE, TILE))
        padded.paste(tile, (GUTTER, GUTTER))
        arr = np.asarray(padded).copy()
        arr[:GUTTER, GUTTER:GUTTER + CONTENT] = arr[GUTTER:GUTTER + 1, GUTTER:GUTTER + CONTENT]
        arr[GUTTER + CONTENT:, GUTTER:GUTTER + CONTENT] = arr[GUTTER + CONTENT - 1:GUTTER + CONTENT, GUTTER:GUTTER + CONTENT]
        arr[:, :GUTTER] = arr[:, GUTTER:GUTTER + 1]
        arr[:, GUTTER + CONTENT:] = arr[:, GUTTER + CONTENT - 1:GUTTER + CONTENT]
        x, y = (i % COLS) * TILE, (i // COLS) * TILE
        atlas.paste(Image.fromarray(arr), (x, y))
        print(f"{i:2d} {name:10s} slot ({i % COLS},{i // COLS}) tint={tint}")
    atlas.save(OUT, "JPEG", quality=88, subsampling=0, optimize=True)
    print(f"→ {OUT} {os.path.getsize(OUT) // 1024} КБ")


if __name__ == "__main__":
    main()
