#!/usr/bin/env python3
"""Фоны оформлений «Цикла» (SVG): сакура, ягодка, лаванда, ромашки, звёздная ночь.

    python3 data/decor/gen.py        # перезаписывает data/decor/*.svg

Холст 440×960 (портрет телефона), растр 938×2048. Украшения — по краям и в углах,
в середине редкие и бледные: поверх лежат цифры календаря.
"""
import math
import os
import random

W, H = 440, 960
OUT = os.path.dirname(os.path.abspath(__file__))


def svg(defs, body, bg1, bg2):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="938" height="2048" viewBox="0 0 {W} {H}">
<defs>
<linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{bg1}"/><stop offset="1" stop-color="{bg2}"/></linearGradient>
{defs}
</defs>
<rect width="{W}" height="{H}" fill="url(#bg)"/>
{body}
</svg>
'''


def edge_points(rng, n, margin=70, center_keep=0.25):
    """Точки, тяготеющие к краям: в средней полосе остаётся доля `center_keep`."""
    pts = []
    while len(pts) < n:
        x, y = rng.uniform(-10, W + 10), rng.uniform(-10, H + 10)
        if margin < x < W - margin and rng.random() > center_keep:
            continue
        pts.append((x, y))
    return pts


# ── Сакура ──────────────────────────────────────────────────────────────────

def blossom(x, y, r, rot, grad, stamen, center, op):
    petals = []
    for i in range(5):
        a = rot + i * 72
        # лепесток с выемкой: от центра к краю, на конце — вырез
        petals.append(
            f'<path transform="rotate({a:.1f} {x:.1f} {y:.1f})" d="M{x:.1f},{y:.1f} '
            f'C{x - r * 0.55:.1f},{y - r * 0.35:.1f} {x - r * 0.55:.1f},{y - r * 0.95:.1f} {x - r * 0.16:.1f},{y - r:.1f} '
            f'L{x:.1f},{y - r * 0.82:.1f} L{x + r * 0.16:.1f},{y - r:.1f} '
            f'C{x + r * 0.55:.1f},{y - r * 0.95:.1f} {x + r * 0.55:.1f},{y - r * 0.35:.1f} {x:.1f},{y:.1f}Z" fill="url(#{grad})"/>')
    st = []
    for i in range(9):
        a = math.radians(rot + i * 40 + 12)
        x2, y2 = x + math.cos(a) * r * 0.45, y + math.sin(a) * r * 0.45
        st.append(f'<line x1="{x:.1f}" y1="{y:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" stroke="{stamen}" stroke-width="{r * 0.04:.2f}" stroke-linecap="round"/>'
                  f'<circle cx="{x2:.1f}" cy="{y2:.1f}" r="{r * 0.06:.2f}" fill="{center}"/>')
    return f'<g opacity="{op:.2f}">{"".join(petals)}{"".join(st)}<circle cx="{x:.1f}" cy="{y:.1f}" r="{r * 0.12:.2f}" fill="{center}"/></g>'


def petal(x, y, s, rot, grad, op):
    return (f'<path transform="translate({x:.1f} {y:.1f}) rotate({rot:.0f}) scale({s:.2f})" '
            f'd="M0,-9 C6,-7 7,3 0,9 C-7,3 -6,-7 0,-9Z M-1.5,-9 L0,-6 L1.5,-9" fill="url(#{grad})" opacity="{op:.2f}"/>')


def branch(points, color, width, op):
    d = "M" + " ".join(f"{x:.1f},{y:.1f}" for x, y in points[:1])
    for i in range(1, len(points) - 1, 2):
        (cx, cy), (x, y) = points[i], points[i + 1]
        d += f" Q{cx:.1f},{cy:.1f} {x:.1f},{y:.1f}"
    return f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{width}" stroke-linecap="round" opacity="{op}"/>'


def sakura(dark):
    rng = random.Random(7)
    if dark:
        bg1, bg2, wood, p1, p2, stamen, center, op = "#22161c", "#1a1216", "#5c3f47", "#d77aa0", "#8e3d5d", "#e7a3bd", "#ffd27a", 0.55
    else:
        bg1, bg2, wood, p1, p2, stamen, center, op = "#fff7f9", "#fdecf2", "#b9959b", "#fff0f5", "#f5a3bf", "#e98aaa", "#f4c96b", 0.9
    defs = (f'<radialGradient id="pg"><stop offset="0" stop-color="{p1}"/><stop offset="1" stop-color="{p2}"/></radialGradient>'
            f'<linearGradient id="lp" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{p1}"/><stop offset="1" stop-color="{p2}"/></linearGradient>')
    body = []
    # ветка сверху справа и снизу слева
    top = []  # ветка сверху справа — в угол, над шапкой календаря
    top.append(branch([(470, 40), (380, 70), (330, 110), (290, 140), (250, 150), (220, 160), (190, 150)], wood, 6, 0.8))
    top.append(branch([(330, 110), (320, 150), (300, 190)], wood, 3.5, 0.8))
    body.append(branch([(-30, 880), (50, 860), (110, 820), (150, 790), (190, 785)], wood, 6, 0.8))
    body.append(branch([(110, 820), (100, 780), (115, 745)], wood, 3.5, 0.8))
    for (x, y, r) in [(205, 150, 17), (250, 148, 20), (300, 120, 22), (345, 92, 18), (300, 190, 15), (395, 62, 16), (275, 165, 12)]:
        top.append(blossom(x, y, r, rng.uniform(0, 72), "pg", stamen, center, op))
    body.append(f'<g transform="translate(70 -95) scale(0.85)">{"".join(top)}</g>')
    for (x, y, r) in [(185, 785, 20), (140, 805, 18), (115, 745, 16), (70, 850, 19), (30, 870, 14), (160, 770, 12)]:
        body.append(blossom(x, y, r, rng.uniform(0, 72), "pg", stamen, center, op))
    # опадающие лепестки — гуще у краёв
    for (x, y) in edge_points(rng, 46, 80, 0.35):
        body.append(petal(x, y, rng.uniform(0.6, 1.3), rng.uniform(0, 360), "lp", rng.uniform(0.35, 0.8) * (0.75 if dark else 1)))
    return svg(defs, "\n".join(body), bg1, bg2)


# ── Ягодка ──────────────────────────────────────────────────────────────────

def heart_path(x, y, s):
    return (f"M{x:.1f},{y + 6 * s:.1f} C{x - 10 * s:.1f},{y - 1 * s:.1f} {x - 9 * s:.1f},{y - 9 * s:.1f} {x - 3.5 * s:.1f},{y - 8 * s:.1f} "
            f"C{x - 1 * s:.1f},{y - 7.5 * s:.1f} {x:.1f},{y - 5 * s:.1f} {x:.1f},{y - 4 * s:.1f} "
            f"C{x:.1f},{y - 5 * s:.1f} {x + 1 * s:.1f},{y - 7.5 * s:.1f} {x + 3.5 * s:.1f},{y - 8 * s:.1f} "
            f"C{x + 9 * s:.1f},{y - 9 * s:.1f} {x + 10 * s:.1f},{y - 1 * s:.1f} {x:.1f},{y + 6 * s:.1f}Z")


def strawberry(x, y, s, rot, rng, op, dark):
    seeds = []
    for _ in range(int(14 * min(s, 1.6))):
        sx, sy = rng.uniform(-8, 8), rng.uniform(-6, 10)
        if (sx / 10) ** 2 + ((sy - 1) / 12) ** 2 < 0.75:
            seeds.append(f'<ellipse cx="{sx:.1f}" cy="{sy:.1f}" rx="0.9" ry="1.4" fill="{"#e9c46a" if dark else "#ffe3a3"}"/>')
    leaf = "#4f8f55" if dark else "#6fbf73"
    leaves = "".join(f'<path transform="rotate({a})" d="M0,-10 C3,-14 6,-16 9,-17 C6,-13 4,-11 0,-10Z" fill="{leaf}"/>' for a in (-60, -20, 20, 60, 100, -100))
    return (f'<g transform="translate({x:.1f} {y:.1f}) rotate({rot:.0f}) scale({s:.2f})" opacity="{op:.2f}">'
            f'<path d="M0,16 C-10,10 -12,-2 -9,-8 C-6,-12 6,-12 9,-8 C12,-2 10,10 0,16Z" fill="url(#sb)"/>'
            f'{"".join(seeds)}{leaves}<rect x="-1" y="-19" width="2" height="7" rx="1" fill="{leaf}"/></g>')


def berry(dark):
    rng = random.Random(11)
    if dark:
        bg1, bg2, s1, s2, hc, dc, op = "#211517", "#1a1112", "#ff6b7d", "#a3202f", "#ff5c70", "#c97a5c", 0.5
    else:
        bg1, bg2, s1, s2, hc, dc, op = "#fffaf5", "#fff0ea", "#ff7b8c", "#e5384f", "#f7a1ae", "#f9c1a4", 0.95
    defs = f'<radialGradient id="sb" cx="0.35" cy="0.3"><stop offset="0" stop-color="{s1}"/><stop offset="1" stop-color="{s2}"/></radialGradient>'
    body = []
    for (x, y, s, r) in [(400, 70, 1.9, 20), (350, 30, 1.2, -15), (420, 150, 1.1, 35), (40, 890, 1.9, -25), (95, 930, 1.2, 15), (25, 805, 1.0, 40),
                         (415, 560, 1.0, 10), (20, 330, 0.9, -10)]:
        body.append(strawberry(x, y, s, r, rng, op, dark))
    for (x, y) in edge_points(rng, 60, 70, 0.3):
        if rng.random() < 0.5:
            body.append(f'<path d="{heart_path(x, y, rng.uniform(0.7, 1.3))}" fill="{hc}" opacity="{rng.uniform(0.3, 0.6) * (0.6 if dark else 1):.2f}"/>')
        else:
            body.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{rng.uniform(1.5, 3.5):.1f}" fill="{dc}" opacity="{rng.uniform(0.35, 0.7) * (0.6 if dark else 1):.2f}"/>')
    return svg(defs, "\n".join(body), bg1, bg2)


# ── Лаванда ─────────────────────────────────────────────────────────────────

def sprig(x0, y0, length, ang, rng, stem, bud1, bud2, op):
    a = math.radians(ang)
    bend = rng.uniform(-0.25, 0.25)
    pts = []
    for i in range(21):
        t = i / 20
        aa = a + bend * t
        pts.append((x0 + math.cos(aa) * length * t, y0 + math.sin(aa) * length * t))
    d = "M" + " L".join(f"{x:.1f},{y:.1f}" for x, y in pts)
    out = [f'<path d="{d}" fill="none" stroke="{stem}" stroke-width="2" stroke-linecap="round"/>']
    # колос: почки на верхних 45 % стебля, к концу мельче
    for i in range(11, 21):
        t = (i - 11) / 9
        x, y = pts[i]
        r = 5.5 * (1 - t * 0.55)
        perp = a + bend * (i / 20) + math.pi / 2
        for side in (-1, 1):
            bx, by = x + math.cos(perp) * side * r * 0.8, y + math.sin(perp) * side * r * 0.8
            out.append(f'<ellipse cx="{bx:.1f}" cy="{by:.1f}" rx="{r:.1f}" ry="{r * 0.62:.1f}" transform="rotate({math.degrees(perp):.0f} {bx:.1f} {by:.1f})" fill="{bud1 if (i + side) % 2 else bud2}"/>')
    return f'<g opacity="{op:.2f}">{"".join(out)}</g>'


def little_flower(x, y, r, rot, petal_c, center_c, op, n=4):
    ps = "".join(f'<ellipse cx="{x + math.cos(math.radians(rot + i * 360 / n)) * r * 0.55:.1f}" cy="{y + math.sin(math.radians(rot + i * 360 / n)) * r * 0.55:.1f}" '
                 f'rx="{r * 0.5:.1f}" ry="{r * 0.33:.1f}" transform="rotate({rot + i * 360 / n:.0f} {x + math.cos(math.radians(rot + i * 360 / n)) * r * 0.55:.1f} {y + math.sin(math.radians(rot + i * 360 / n)) * r * 0.55:.1f})" fill="{petal_c}"/>'
                 for i in range(n))
    return f'<g opacity="{op:.2f}">{ps}<circle cx="{x:.1f}" cy="{y:.1f}" r="{r * 0.22:.1f}" fill="{center_c}"/></g>'


def lavender(dark):
    rng = random.Random(23)
    if dark:
        bg1, bg2, stem, b1, b2, fl, ce, op = "#1b1627", "#15111f", "#4c6b4f", "#7c62c4", "#5e4a9c", "#6f5aa8", "#c9a54a", 0.6
    else:
        bg1, bg2, stem, b1, b2, fl, ce, op = "#f9f6ff", "#f1eafe", "#9dbf9a", "#b49cec", "#9a7fe0", "#d8c9f6", "#f3c65a", 0.85
    body = []
    for i in range(7):  # букет снизу слева
        body.append(sprig(20 + i * 9, 975, rng.uniform(170, 240), -70 - i * 4 + rng.uniform(-6, 6), rng, stem, b1, b2, op))
    for i in range(5):  # и сверху справа, вниз головой
        body.append(sprig(425 - i * 10, -15, rng.uniform(130, 180), 105 + i * 5 + rng.uniform(-6, 6), rng, stem, b1, b2, op))
    for (x, y) in edge_points(rng, 40, 75, 0.3):
        if rng.random() < 0.55:
            body.append(little_flower(x, y, rng.uniform(6, 11), rng.uniform(0, 90), fl, ce, rng.uniform(0.45, 0.85) * (0.7 if dark else 1)))
        else:
            body.append(f'<path transform="translate({x:.1f} {y:.1f}) scale({rng.uniform(0.6, 1.1):.2f})" d="M0,-6 L1.5,-1.5 L6,0 L1.5,1.5 L0,6 L-1.5,1.5 L-6,0 L-1.5,-1.5Z" fill="{b1}" opacity="{rng.uniform(0.3, 0.6):.2f}"/>')
    return svg("", "\n".join(body), bg1, bg2)


# ── Ромашки ─────────────────────────────────────────────────────────────────

def daisy(x, y, r, rot, petal_c, edge_c, op, dark):
    ps = []
    n = 14
    for i in range(n):
        a = rot + i * 360 / n
        ps.append(f'<ellipse cx="{x:.1f}" cy="{y - r * 0.55:.1f}" rx="{r * 0.15:.1f}" ry="{r * 0.45:.1f}" transform="rotate({a:.1f} {x:.1f} {y:.1f})" fill="{petal_c}" stroke="{edge_c}" stroke-width="0.9"/>')
    return f'<g opacity="{op:.2f}">{"".join(ps)}<circle cx="{x:.1f}" cy="{y:.1f}" r="{r * 0.27:.1f}" fill="url(#dc)"/></g>'


def leaf(x, y, s, rot, c, op):
    return f'<path transform="translate({x:.1f} {y:.1f}) rotate({rot:.0f}) scale({s:.2f})" d="M0,0 C6,-8 6,-20 0,-30 C-6,-20 -6,-8 0,0Z M0,-2 L0,-27" fill="{c}" stroke="{c}" stroke-width="0.5" opacity="{op:.2f}"/>'


def daisies(dark):
    rng = random.Random(31)
    if dark:
        bg1, bg2, pc, ec, c1, c2, lc, op = "#16201b", "#111915", "#cfe0d5", "#7f9c8a", "#ffd45a", "#b8860b", "#3d6b4c", 0.4
    else:
        bg1, bg2, pc, ec, c1, c2, lc, op = "#f4fbf6", "#e6f4ea", "#ffffff", "#a9d3b8", "#ffd45a", "#f2a900", "#a8d5b5", 0.95
    defs = f'<radialGradient id="dc" cx="0.4" cy="0.35"><stop offset="0" stop-color="{c1}"/><stop offset="1" stop-color="{c2}"/></radialGradient>'
    body = []
    for (x, y, s, r) in [(30, 960, 2.2, -35), (90, 965, 1.8, 15), (150, 965, 1.4, 40), (5, 900, 1.6, -70), (410, 20, 2.0, 160), (440, 90, 1.7, 220),
                         (360, 0, 1.4, 190), (440, 560, 1.0, 250), (0, 320, 0.9, -80)]:
        body.append(leaf(x, y, s, r, lc, 0.9 if not dark else 0.6))
    for (x, y, r) in [(50, 880, 40), (125, 920, 30), (12, 800, 24), (185, 945, 20), (85, 815, 16),
                      (390, 80, 38), (432, 160, 24), (330, 35, 24), (290, 12, 14), (418, 515, 18), (16, 300, 16), (425, 690, 12)]:
        body.append(daisy(x, y, r, rng.uniform(0, 30), pc, ec, op, dark))
    for (x, y) in edge_points(rng, 34, 70, 0.3):
        if rng.random() < 0.6:
            body.append(daisy(x, y, rng.uniform(7, 12), rng.uniform(0, 30), pc, ec, op * rng.uniform(0.55, 0.9), dark))
        else:
            body.append(leaf(x, y, rng.uniform(0.35, 0.6), rng.uniform(0, 360), lc, rng.uniform(0.4, 0.7) * (0.6 if dark else 1)))
    return svg(defs, "\n".join(body), bg1, bg2)


# ── Звёздная ночь ───────────────────────────────────────────────────────────

def night():
    rng = random.Random(42)
    defs = ('<radialGradient id="glow" cx="0.8" cy="0.08" r="0.6"><stop offset="0" stop-color="#5b3f9e" stop-opacity="0.55"/><stop offset="1" stop-color="#5b3f9e" stop-opacity="0"/></radialGradient>'
            '<radialGradient id="glow2" cx="0.1" cy="0.95" r="0.55"><stop offset="0" stop-color="#9c3f7d" stop-opacity="0.35"/><stop offset="1" stop-color="#9c3f7d" stop-opacity="0"/></radialGradient>'
            '<radialGradient id="moonglow"><stop offset="0.45" stop-color="#ffe9a8" stop-opacity="0.35"/><stop offset="1" stop-color="#ffe9a8" stop-opacity="0"/></radialGradient>'
            '<radialGradient id="moon" cx="0.35" cy="0.35"><stop offset="0" stop-color="#fff8dc"/><stop offset="1" stop-color="#ffd98a"/></radialGradient>'
            '<radialGradient id="star"><stop offset="0" stop-color="#ffffff"/><stop offset="0.3" stop-color="#fff3c4" stop-opacity="0.8"/><stop offset="1" stop-color="#fff3c4" stop-opacity="0"/></radialGradient>'
            '<filter id="blur" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="14"/></filter>'
            '<mask id="crescent"><rect width="440" height="960" fill="#fff"/><circle cx="392" cy="66" r="34" fill="#000"/></mask>')
    body = ['<rect width="440" height="960" fill="url(#glow)"/>', '<rect width="440" height="960" fill="url(#glow2)"/>']
    # облака — размытые овалы
    for (x, y, rx, ry, op) in [(90, 160, 120, 26, 0.10), (330, 430, 140, 24, 0.07), (120, 700, 150, 28, 0.08)]:
        body.append(f'<ellipse cx="{x}" cy="{y}" rx="{rx}" ry="{ry}" fill="#b9a8ff" opacity="{op}" filter="url(#blur)"/>')
    # месяц
    body.append('<circle cx="370" cy="80" r="80" fill="url(#moonglow)"/>')
    body.append('<circle cx="370" cy="80" r="38" fill="url(#moon)" mask="url(#crescent)"/>')
    # звёзды
    for _ in range(170):
        x, y = rng.uniform(0, W), rng.uniform(0, H)
        mid = 60 < x < W - 60
        r = rng.uniform(0.5, 1.6) * (0.8 if mid else 1)
        body.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r:.2f}" fill="#fff8e1" opacity="{rng.uniform(0.25, 0.85) * (0.6 if mid else 1):.2f}"/>')
    for (x, y) in edge_points(rng, 16, 80, 0.2):
        s = rng.uniform(5, 10)
        body.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{s * 1.6:.1f}" fill="url(#star)" opacity="0.6"/>')
        body.append(f'<path transform="translate({x:.1f} {y:.1f})" d="M0,{-s:.1f} Q{s * 0.15:.1f},{-s * 0.15:.1f} {s:.1f},0 Q{s * 0.15:.1f},{s * 0.15:.1f} 0,{s:.1f} Q{-s * 0.15:.1f},{s * 0.15:.1f} {-s:.1f},0 Q{-s * 0.15:.1f},{-s * 0.15:.1f} 0,{-s:.1f}Z" fill="#fff6d6" opacity="0.9"/>')
    # падающая звезда
    body.append('<line x1="60" y1="250" x2="140" y2="215" stroke="#fff3c4" stroke-width="1.4" stroke-linecap="round" opacity="0.35"/>')
    body.append('<circle cx="140" cy="215" r="2" fill="#fffbe8" opacity="0.8"/>')
    return svg(defs, "\n".join(body), "#1c1742", "#100d22")


def main():
    files = {
        "sakura-light.svg": sakura(False), "sakura-dark.svg": sakura(True),
        "berry-light.svg": berry(False), "berry-dark.svg": berry(True),
        "lavender-light.svg": lavender(False), "lavender-dark.svg": lavender(True),
        "daisy-light.svg": daisies(False), "daisy-dark.svg": daisies(True),
        "night.svg": night(),
    }
    for name, text in files.items():
        with open(os.path.join(OUT, name), "w") as f:
            f.write(text)
        print(name, len(text))


if __name__ == "__main__":
    main()
