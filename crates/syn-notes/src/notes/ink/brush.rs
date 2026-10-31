//! Кисти: сырые точки штриха → «отпечатки» (центр, радиус, цвет) после
//! сглаживания. Из отпечатков строят и растр страницы ([`super::raster`]),
//! и сетку живого штриха на ГПУ ([`super::mesh`]), и SVG — поэтому вид на
//! экране и в экспорте один.
//!
//! Сглаживание без отставания кончика: средние точки усредняются с
//! соседями (1-2-1), последняя остаётся сырой; между точками — сплайн
//! Катмулла–Рома с шагом ~1,5 единицы.

use super::model::{Brush, InkPoint, Rgba, Stroke, P2};

/// Отпечаток кисти.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dab {
    pub p: P2,
    /// Радиус (половина ширины).
    pub r: f32,
    pub color: Rgba,
}

/// Шаг интерполяции, единицы страницы.
const STEP: f32 = 1.5;
/// Наклон пера у каллиграфии без датчика — 45°.
const NIB_ANGLE: f32 = std::f32::consts::FRAC_PI_4;

/// Цвет маркера по выбранному: полупрозрачный.
pub fn highlighter_color(c: Rgba) -> Rgba {
    if c.0[3] == 255 {
        c.with_alpha(110)
    } else {
        c
    }
}

/// Сгладить положения и давление (концы не трогаем — кончик без задержки).
pub fn smooth(points: &[InkPoint]) -> Vec<InkPoint> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(n);
    out.push(points[0]);
    for i in 1..n - 1 {
        let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
        out.push(InkPoint {
            x: (a.x + 2.0 * b.x + c.x) / 4.0,
            y: (a.y + 2.0 * b.y + c.y) / 4.0,
            p: (a.p + 2.0 * b.p + c.p) / 4.0,
            ..b
        });
    }
    out.push(points[n - 1]);
    out
}

/// Убрать точки ближе `eps` к предыдущей (дрожание стоящего пера), кроме
/// последней.
fn dedup(points: &[InkPoint], eps: f32) -> Vec<InkPoint> {
    let mut out: Vec<InkPoint> = Vec::with_capacity(points.len());
    for (i, p) in points.iter().enumerate() {
        match out.last_mut() {
            Some(last) if last.pos().dist(p.pos()) < eps => {
                if i == points.len() - 1 {
                    // Последнюю точку сохраняем как есть (кончик), но с
                    // наибольшим давлением, чтобы точка пером не пропадала.
                    let pmax = last.p.max(p.p);
                    *last = InkPoint { p: pmax, ..*p };
                } else {
                    last.p = last.p.max(p.p);
                }
            }
            _ => out.push(*p),
        }
    }
    out
}

fn catmull(p0: P2, p1: P2, p2: P2, p3: P2, t: f32) -> P2 {
    let t2 = t * t;
    let t3 = t2 * t;
    let f = |a: f32, b: f32, c: f32, d: f32| {
        0.5 * ((2.0 * b) + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    P2::new(f(p0.x, p1.x, p2.x, p3.x), f(p0.y, p1.y, p2.y, p3.y))
}

/// Множитель ширины кисти от давления.
fn pressure_factor(brush: Brush, p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    match brush {
        Brush::Fountain => 0.3 + 0.95 * p.powf(0.8),
        Brush::Ballpoint => 0.85 + 0.3 * p,
        Brush::Pencil => 0.75 + 0.4 * p,
        Brush::Highlighter => 1.0,
        Brush::Brush => 0.06 + 1.45 * p.powf(1.3),
        Brush::Calligraphy => 0.7 + 0.45 * p,
    }
}

/// Отпечатки штриха.
pub fn dabs(stroke: &Stroke) -> Vec<Dab> {
    dabs_of(stroke.brush, stroke.color, stroke.width, &stroke.points)
}

pub fn dabs_of(brush: Brush, color: Rgba, width: f32, raw: &[InkPoint]) -> Vec<Dab> {
    if raw.is_empty() {
        return Vec::new();
    }
    let base_r = (width / 2.0).max(0.2);
    let pts = smooth(&dedup(raw, 0.35));
    let color = if brush == Brush::Highlighter { highlighter_color(color) } else { color };
    let n = pts.len();
    if n == 1 {
        let p = pts[0];
        return vec![Dab { p: p.pos(), r: base_r * pressure_factor(brush, p.p).max(0.5), color: tone(brush, color, p.p) }];
    }
    // Скорость (ед./мс) по точкам — для перьевой ручки.
    let speed: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = if i == 0 { (pts[0], pts[1]) } else { (pts[i - 1], pts[i]) };
            let dt = b.t.saturating_sub(a.t).max(1) as f32;
            a.pos().dist(b.pos()) / dt
        })
        .collect();
    let mut out = Vec::with_capacity(n * 2);
    // Полная длина — для сужения концов кисти.
    let total: f32 = pts.windows(2).map(|w| w[0].pos().dist(w[1].pos())).sum();
    let mut walked = 0.0;
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)].pos();
        let p1 = pts[i].pos();
        let p2 = pts[i + 1].pos();
        let p3 = pts[(i + 2).min(n - 1)].pos();
        let seg = p1.dist(p2);
        let steps = ((seg / STEP).ceil() as usize).clamp(1, 64);
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let pos = catmull(p0, p1, p2, p3, t);
            let pr = pts[i].p + (pts[i + 1].p - pts[i].p) * t;
            let v = speed[i] + (speed[i + 1] - speed[i]) * t;
            let dir = p2.sub(p1);
            let tilt = (
                pts[i].tx + (pts[i + 1].tx - pts[i].tx) * t,
                pts[i].ty + (pts[i + 1].ty - pts[i].ty) * t,
            );
            let r = radius(brush, base_r, pr, v, dir, tilt, walked + seg * t, total);
            out.push(Dab { p: pos, r, color: tone(brush, color, pr) });
        }
        walked += seg;
    }
    let last = pts[n - 1];
    let dir = last.pos().sub(pts[n - 2].pos());
    out.push(Dab {
        p: last.pos(),
        r: radius(brush, base_r, last.p, speed[n - 1], dir, (last.tx, last.ty), total, total),
        color: tone(brush, color, last.p),
    });
    out
}

#[allow(clippy::too_many_arguments)]
fn radius(brush: Brush, base: f32, p: f32, speed: f32, dir: P2, tilt: (f32, f32), at: f32, total: f32) -> f32 {
    let mut f = pressure_factor(brush, p);
    match brush {
        Brush::Fountain => {
            // Быстрый штрих тоньше.
            f *= (1.0 / (1.0 + 0.35 * speed)).max(0.55);
        }
        Brush::Brush => {
            // Сужение на концах.
            let taper = (base * 6.0).max(4.0);
            let k = (at / taper).min((total - at) / taper).clamp(0.0, 1.0);
            f *= 0.25 + 0.75 * k;
        }
        Brush::Calligraphy => {
            let nib = if tilt.0 != 0.0 || tilt.1 != 0.0 { tilt.1.atan2(tilt.0) + std::f32::consts::FRAC_PI_2 } else { NIB_ANGLE };
            let len = dir.len();
            let s = if len > 1e-4 { (dir.y.atan2(dir.x) - nib).sin().abs() } else { 0.5 };
            f *= 0.18 + 0.95 * s;
        }
        _ => {}
    }
    (base * f).max(0.25)
}

fn tone(brush: Brush, c: Rgba, p: f32) -> Rgba {
    match brush {
        // Карандаш: слабый нажим — светлее (не прозрачнее: на E Ink и в
        // экспорте ровный тон без наложений).
        Brush::Pencil => c.toward_white((1.0 - p.clamp(0.0, 1.0)) * 0.6),
        _ => c,
    }
}

/// Контур отпечатка многоугольником (против часовой в экранных осях —
/// одна ориентация у всех кусков, чтобы заливка «ненулевым» правилом
/// давала объединение).
pub fn circle(c: P2, r: f32, seg: usize) -> Vec<P2> {
    (0..seg)
        .map(|i| {
            let a = -(i as f32) / seg as f32 * std::f32::consts::TAU;
            P2::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

/// Число сторон круга радиуса `r` при масштабе `px` пикселей на единицу.
pub fn circle_segments(r: f32, px: f32) -> usize {
    ((r * px * 1.2).ceil() as usize).clamp(8, 48)
}

/// Четырёхугольник между двумя отпечатками (касательные к кругам), в той
/// же ориентации, что [`circle`]. `None` — круги вложены друг в друга.
pub fn bridge(a: &Dab, b: &Dab) -> Option<[P2; 4]> {
    let d = b.p.sub(a.p);
    let len = d.len();
    if len < 1e-4 || len <= (a.r - b.r).abs() {
        return None;
    }
    let u = d.scale(1.0 / len);
    // Внешние касательные к двум кругам разного радиуса.
    let k = (a.r - b.r) / len;
    let s = (1.0 - k * k).max(0.0).sqrt();
    let n1 = P2::new(u.x * k - u.y * s, u.y * k + u.x * s);
    let n2 = P2::new(u.x * k + u.y * s, u.y * k - u.x * s);
    let mut q = [a.p.add(n1.scale(a.r)), b.p.add(n1.scale(b.r)), b.p.add(n2.scale(b.r)), a.p.add(n2.scale(a.r))];
    // Четырёхугольник касательных выпуклый, но при резкой смене радиуса
    // порядок вершин может дать «бантик» — части с разной обмоткой гасят
    // друг друга при заливке. Упорядочить по углу вокруг центра.
    let c = q.iter().fold(P2::new(0.0, 0.0), |s, p| s.add(*p)).scale(0.25);
    q.sort_by(|u, v| {
        let au = (u.y - c.y).atan2(u.x - c.x);
        let av = (v.y - c.y).atan2(v.x - c.x);
        au.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
    });
    Some(orient(q))
}

/// Привести четырёхугольник к ориентации кругов [`circle`] (отрицательная
/// площадь в осях с Y вниз).
fn orient(mut q: [P2; 4]) -> [P2; 4] {
    if signed_area(&q) > 0.0 {
        q.reverse();
    }
    q
}

pub fn signed_area(pts: &[P2]) -> f32 {
    let n = pts.len();
    let mut s = 0.0;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        s += a.x * b.y - b.x * a.y;
    }
    s / 2.0
}

/// Наибольший радиус (запас для области перерисовки).
pub fn max_radius(d: &[Dab]) -> f32 {
    d.iter().map(|x| x.r).fold(0.0, f32::max)
}
