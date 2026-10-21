#!/usr/bin/env python3
"""Рендер блоков «Люкса» → assets/tex/atlas.png.

Каждая отделка — не «кубик со скосами», а свой предмет: слиток металла,
подушка велюра, огранённый самоцвет, купол леденца, плитка дерева, стеклянный
блок с бликом… Для каждого материала задаётся форма (карта высот), свет
(диффуз + блик Блинна + фреснель по краю), контактная тень и текстура из
assets/tex/raw/<id>.png. Результат — спрайт с прозрачным полем, в игре он
рисуется одним квадом (`push_image`), без скосов на лету.

Сетка 8×8 тайлов по 192 px (1536×1536, PNG с альфой). Блок занимает 150 px
в центре тайла, поле 21 px — под тень и свечение. У каждой отделки два
варианта (разные вырезы текстуры), у красящихся — ещё слой бликов
(белый, поверх окрашенной базы). Порядок слотов печатается и должен совпадать
с `src/ui/finish.rs` (`slot_base`, `slot_gloss`).

Запуск: python3 tools/build_atlas.py [--sheet /tmp/sheet.png]
"""
import os
import sys
import numpy as np
from PIL import Image, ImageFilter

DIR = os.path.join(os.path.dirname(__file__), "..")
RAW = os.path.join(DIR, "assets", "tex", "raw")
OUT = os.path.join(DIR, "assets", "tex", "atlas.png")
TILE, BLOCK, COLS, ROWS = 192, 150, 8, 8
MARGIN = (TILE - BLOCK) // 2
VARIANTS = 2

# id, tint, form, radius(0..1 от половины стороны), shininess, ks, kd, ambient, extra
# form: tile | ingot | pillow | dome | gem | slab
FINISHES = [
    ("gold", False, "ingot", 0.22, 60, 0.9, 0.9, 0.35, {}),
    ("silver", False, "ingot", 0.22, 80, 1.0, 0.85, 0.35, {}),
    ("copper", False, "ingot", 0.22, 50, 0.8, 0.9, 0.35, {}),
    ("bronze", False, "ingot", 0.20, 30, 0.5, 0.9, 0.35, {}),
    ("glass", True, "dome", 0.40, 90, 0.9, 0.6, 0.55, {"fresnel": 0.35, "alpha": 0.9, "grey": 0.86}),
    ("crystal", True, "gem", 0.30, 120, 1.0, 0.7, 0.45, {"fresnel": 0.25, "grey": 0.84}),
    ("velvet", True, "pillow", 0.38, 8, 0.12, 0.8, 0.30, {"ao": 0.35, "grey": 0.60}),
    ("oak", False, "tile", 0.16, 25, 0.25, 0.9, 0.40, {}),
    ("walnut", False, "tile", 0.16, 40, 0.35, 0.9, 0.38, {}),
    ("marble", False, "slab", 0.18, 70, 0.55, 0.85, 0.45, {}),
    ("granite", False, "slab", 0.18, 40, 0.35, 0.9, 0.40, {}),
    ("obsidian", False, "dome", 0.30, 140, 1.0, 0.7, 0.35, {"fresnel": 0.2}),
    ("lava", False, "slab", 0.22, 15, 0.15, 0.9, 0.45, {"emissive": 0.35}),
    ("ice", False, "dome", 0.34, 100, 0.8, 0.7, 0.55, {"fresnel": 0.4, "alpha": 0.93}),
    ("jade", False, "dome", 0.36, 60, 0.6, 0.8, 0.45, {"fresnel": 0.2}),
    ("gem", True, "gem", 0.30, 140, 1.0, 0.7, 0.45, {"fresnel": 0.3, "grey": 0.78}),
    ("pearl", False, "dome", 0.45, 50, 0.7, 0.8, 0.55, {"fresnel": 0.3}),
    ("neon", True, "tile", 0.18, 30, 0.25, 0.5, 0.35, {"glow": 0.7, "grey": 0.28}),
    ("leather", False, "pillow", 0.30, 20, 0.25, 0.85, 0.35, {"ao": 0.25}),
    ("denim", False, "pillow", 0.30, 6, 0.08, 0.85, 0.35, {"ao": 0.25}),
    ("porcelain", True, "dome", 0.32, 90, 0.85, 0.75, 0.55, {"fresnel": 0.15, "grey": 0.90}),
    ("candy", True, "dome", 0.48, 110, 1.0, 0.7, 0.50, {"fresnel": 0.25, "grey": 0.82}),
    ("chocolate", False, "chocolate", 0.14, 45, 0.45, 0.9, 0.35, {}),
    ("hologram", False, "tile", 0.12, 90, 0.8, 0.85, 0.45, {}),
]

LIGHT = np.array([-0.42, -0.55, 0.72])
LIGHT /= np.linalg.norm(LIGHT)
VIEW = np.array([0.0, 0.0, 1.0])
HALF = LIGHT + VIEW
HALF /= np.linalg.norm(HALF)


def grid(n):
    c = (np.arange(n) + 0.5) / n * 2 - 1
    return np.meshgrid(c, c)


def rounded_sdf(u, v, r):
    """SDF скруглённого квадрата [-1,1]², радиус r."""
    qx = np.abs(u) - (1 - r)
    qy = np.abs(v) - (1 - r)
    outside = np.sqrt(np.maximum(qx, 0) ** 2 + np.maximum(qy, 0) ** 2)
    inside = np.minimum(np.maximum(qx, qy), 0)
    return outside + inside - r


def smoothstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0, 1)
    return t * t * (3 - 2 * t)


def height(form, u, v, r, px):
    """Карта высот 0..1 внутри маски."""
    d = rounded_sdf(u, v, r)
    aa = 1.5 / px
    mask = 1 - smoothstep(-aa, aa, d)
    t = np.clip(-d, 0, 1)  # расстояние внутрь от края
    if form == "tile":
        # плитка: мягко скруглённый край (без грани), почти плоская середина
        e = np.clip(t / 0.16, 0, 1)
        h = np.sqrt(1 - (1 - e) ** 2) * 0.9 + 0.1 * (1 - (u ** 2 + v ** 2) * 0.15)
    elif form == "slab":
        e = np.clip(t / 0.22, 0, 1)
        h = np.sqrt(1 - (1 - e) ** 2) * 0.9 + 0.1 * (1 - (u ** 2 + v ** 2) * 0.1)
    elif form == "ingot":
        e = np.clip(t / 0.30, 0, 1)
        h = np.sqrt(1 - (1 - e) ** 2) * 0.85 + 0.15 * (1 - (u ** 2 + v ** 2) * 0.25)
    elif form == "pillow":
        h = np.sqrt(np.clip((1 - u ** 2) * (1 - v ** 2), 0, 1)) ** 0.6 * (0.3 + 0.7 * np.clip(t / 0.5, 0, 1) ** 0.5)
    elif form == "dome":
        h = np.sqrt(np.clip(1 - 0.7 * (u ** 2 + v ** 2), 0, 1)) * np.clip(t / 0.22, 0, 1) ** 0.5
    elif form == "gem":
        # огранка: восьмиугольная площадка и восемь боковых граней
        o = np.maximum(np.maximum(np.abs(u), np.abs(v)), (np.abs(u) + np.abs(v)) / np.sqrt(2))
        h = np.clip((1.0 - o) * 2.4, 0, 1) * 0.55 + 0.45 * (o < 0.52)
        h = np.minimum(h, 1.0)
    elif form == "chocolate":
        # плитка: бортик и вдавленная площадка
        inner = rounded_sdf(u / 0.72, v / 0.72, 0.25)
        h = np.sqrt(np.clip(t / 0.10, 0, 1)) * 0.75 + 0.25 * (1 - smoothstep(-0.05, 0.05, inner))
    else:
        h = np.ones_like(u)
    return h * mask, mask


def normals(h, strength):
    # градиент в единицах u/v (сторона блока = 2), высота 1 ≈ strength·половины стороны
    dy, dx = np.gradient(h)
    k = BLOCK / 2 * strength
    n = np.stack([-dx * k, -dy * k, np.ones_like(h)], -1)
    n /= np.linalg.norm(n, axis=-1, keepdims=True)
    return n


def load_texture(name, variant):
    path = os.path.join(RAW, name + ".png")
    if not os.path.exists(path):
        print(f"  ! нет {path}")
        rng = np.random.default_rng(1)
        return np.clip(rng.normal(0.5, 0.1, (BLOCK, BLOCK, 3)), 0, 1)
    im = Image.open(path).convert("RGB")
    w, hgt = im.size
    s = int(min(w, hgt) * 0.62)
    rng = np.random.default_rng(hash(name) % (2 ** 31) + variant * 977)
    x0 = int(rng.integers(0, w - s + 1))
    y0 = int(rng.integers(0, hgt - s + 1))
    im = im.crop((x0, y0, x0 + s, y0 + s)).resize((BLOCK, BLOCK), Image.LANCZOS)
    if variant % 2 == 1:
        im = im.transpose(Image.FLIP_LEFT_RIGHT)
    return np.asarray(im, dtype=np.float32) / 255.0


def to_grey(tex, target):
    g = tex @ np.array([0.299, 0.587, 0.114])
    lo, hi = np.percentile(g, 2), np.percentile(g, 98)
    g = np.clip((g - lo) / max(hi - lo, 1e-3), 0, 1)
    g = np.clip(target + (g - g.mean()) * 0.5, 0, 1)
    return np.stack([g, g, g], -1)


def render(spec, variant):
    name, tint, form, r, shin, ks, kd, amb, extra = spec
    u, v = grid(BLOCK)
    h, mask = height(form, u, v, r, BLOCK)
    strength = {"tile": 0.35, "slab": 0.4, "ingot": 0.6, "pillow": 0.6, "dome": 0.9, "gem": 0.8, "chocolate": 0.5}[form]
    n = normals(h, strength)
    ndl = np.clip(n @ LIGHT, 0, 1)
    ndh = np.clip(n @ HALF, 0, 1)
    tex = load_texture(name, variant)
    if tint:
        tex = to_grey(tex, extra.get("grey", 0.8))
    diffuse = amb + kd * ndl
    spec = ks * ndh ** shin
    fres = extra.get("fresnel", 0.0) * (1 - n[..., 2]) ** 2.5
    ao = extra.get("ao", 0.0)
    if ao:
        d = rounded_sdf(u, v, r)
        diffuse *= 1 - ao * (1 - np.clip(-d / 0.35, 0, 1))
    base = tex * diffuse[..., None]
    if extra.get("emissive"):
        # лава: светлые (горячие) места светятся сами
        hot = np.clip((tex[..., 0] - 0.45) * 2.5, 0, 1)
        base = base + tex * (hot * extra["emissive"])[..., None]
    gloss = np.clip(spec + fres, 0, 1)
    if tint:
        rgb = np.clip(base, 0, 1)
        gloss_rgb = np.ones((BLOCK, BLOCK, 3))
    else:
        rgb = np.clip(base + gloss[..., None] * (1 - base) * 0.9, 0, 1)
        gloss_rgb = None
    alpha = mask * extra.get("alpha", 1.0)
    return rgb, alpha, (gloss_rgb, gloss * mask) if tint else None, mask


def place(atlas, slot, rgb, alpha, glow=None, shadow=True):
    tile = np.zeros((TILE, TILE, 4), dtype=np.float32)
    m = MARGIN
    if shadow:
        # контактная тень: размытая маска, смещённая вниз
        sh = np.zeros((TILE, TILE), dtype=np.float32)
        sh[m:m + BLOCK, m:m + BLOCK] = alpha
        sh = np.asarray(Image.fromarray((sh * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(7)), dtype=np.float32) / 255.0
        sh = np.roll(sh, 7, axis=0)
        tile[..., 3] = sh * 0.5
    if glow is not None:
        g = np.zeros((TILE, TILE), dtype=np.float32)
        g[m:m + BLOCK, m:m + BLOCK] = alpha
        g = np.asarray(Image.fromarray((g * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(9)), dtype=np.float32) / 255.0
        ga = g * glow
        tile[..., :3] = tile[..., :3] * (1 - ga[..., None]) + 1.0 * ga[..., None]
        tile[..., 3] = np.maximum(tile[..., 3], ga)
    # блок поверх (straight alpha): out = src*a + dst*(1-a)
    a = alpha[..., None]
    reg = tile[m:m + BLOCK, m:m + BLOCK]
    reg[..., :3] = rgb * a + reg[..., :3] * (1 - a)
    reg[..., 3] = alpha + reg[..., 3] * (1 - alpha)
    x, y = (slot % COLS) * TILE, (slot // COLS) * TILE
    atlas[y:y + TILE, x:x + TILE] = tile


def main():
    atlas = np.zeros((TILE * ROWS, TILE * COLS, 4), dtype=np.float32)
    slot = 0
    table = []
    for spec in FINISHES:
        name, tint = spec[0], spec[1]
        base_slots, gloss_slots = [], []
        for variant in range(VARIANTS):
            rgb, alpha, gloss, mask = render(spec, variant)
            place(atlas, slot, rgb, alpha, glow=spec[8].get("glow"))
            base_slots.append(slot)
            slot += 1
            if gloss is not None:
                grgb, ga = gloss
                place(atlas, slot, grgb, ga, shadow=False)
                gloss_slots.append(slot)
                slot += 1
        table.append((name, base_slots, gloss_slots))
    assert slot <= COLS * ROWS, slot
    img = Image.fromarray((np.clip(atlas, 0, 1) * 255).astype(np.uint8), "RGBA")
    img.save(OUT, optimize=True)
    print(f"→ {OUT} {os.path.getsize(OUT) // 1024} КБ, слотов {slot}")
    for name, b, g in table:
        print(f"{name:10s} base={b} gloss={g}")
    if "--sheet" in sys.argv:
        path = sys.argv[sys.argv.index("--sheet") + 1]
        bg = Image.new("RGBA", img.size, (38, 24, 44, 255))
        bg.alpha_composite(img)
        bg.convert("RGB").save(path)
        print("лист:", path)


if __name__ == "__main__":
    main()
