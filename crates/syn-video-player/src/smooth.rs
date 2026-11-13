//! «Плавность» без нейросетей — промежуточные кадры по оценке движения, как в SVP / MVTools: ролик
//! 24 кадр/с идёт с частотой ×2, 60 или 120 кадр/с. Ступень цепочки фильтров (`Stage::Interpolate`) на
//! процессоре, во всех сборках (и на телефоне).
//!
//! 1. Яркость кадра — пирамида 1/2, 1/4, 1/8.
//! 2. Движение — блоками по 32 px кадра, симметрично от середины между кадрами (вектор `v`: в прошлом
//!    кадре точка была в `p − v/2`, в новом — в `p + v/2`): на 1/8 — полный перебор, на 1/4 и 1/2 —
//!    уточнение вокруг вектора уровня выше и векторов соседей; штраф за длинный вектор и за отличие от
//!    соседей держит поле гладким. Затем медиана 3×3.
//! 3. Промежуточный кадр в момент `t ∈ (0, 1)` — из прошлого кадра в `p − t·v` и нового в
//!    `p + (1 − t)·v`, вектор — билинейно между центрами блоков. Где совпадение плохое (открылся фон,
//!    блок не нашёлся) — плавно к простому смешиванию кадров: вместо разрывов лёгкий двойной контур.
//! 4. Смена сцены — без промежуточных (иначе одна сцена «перетекает» в другую).
//!
//! Кадры выходят по ровной сетке выбранной частоты (кадр ролика — когда он сам на неё попадает). Не
//! успевает за роликом — кадры как есть до перемотки, причина — в настройках.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use rayon::prelude::*;
use syngui::async_runtime::run_on_main_thread;
use syngui::video::{FrameFilter, VideoFrame, VideoPlayer};

use crate::chain::{self, Stage};

/// Режимы частоты: ключ настроек, подпись; 0 — вдвое.
pub const MODES: [(&str, u32); 3] = [("x2", 0), ("60", 60), ("120", 120)];

pub fn target_of(mode: &str) -> u32 {
    MODES.iter().find(|(k, _)| *k == mode).map(|m| m.1).unwrap_or(0)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    /// Работает: время на исходный кадр (с промежуточными), бюджет кадра ролика, частота на выходе.
    Active { ms: f32, budget_ms: f32, fps: f32 },
    /// Не успевает: кадры как есть до перемотки.
    Slow { ms: f32, budget_ms: f32 },
}

/// Блок оценки движения, px кадра.
const BLOCK: usize = 32;
/// Полный перебор на 1/8: ±RANGE px уровня (×8 — px кадра).
const RANGE: i32 = 10;
/// Разница яркости (0..255, среднее на 1/4) больше — смена сцены.
const SCENE_CUT: u32 = 30;
/// Доля периода кадра ролика, которую можно тратить.
const BUDGET: f32 = 0.85;

/// Яркость одного уровня пирамиды.
struct Plane {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Plane {
    #[inline]
    fn at(&self, x: i32, y: i32) -> u8 {
        let x = x.clamp(0, self.w as i32 - 1) as usize;
        let y = y.clamp(0, self.h as i32 - 1) as usize;
        self.px[y * self.w + x]
    }

    fn half(&self) -> Plane {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut px = vec![0u8; w * h];
        px.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let (sx, sy) = (x as i32 * 2, y as i32 * 2);
                let s = self.at(sx, sy) as u32 + self.at(sx + 1, sy) as u32 + self.at(sx, sy + 1) as u32 + self.at(sx + 1, sy + 1) as u32;
                *o = (s / 4) as u8;
            }
        });
        Plane { w, h, px }
    }
}

/// Пирамида: [1/2, 1/4, 1/8].
struct Pyramid([Plane; 3]);

fn pyramid(rgba: &[u8], w: usize, h: usize) -> Pyramid {
    let (w2, h2) = ((w / 2).max(1), (h / 2).max(1));
    let mut px = vec![0u8; w2 * h2];
    px.par_chunks_mut(w2).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut s = 0u32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (x * 2 + dx).min(w - 1);
                let sy = (y * 2 + dy).min(h - 1);
                let i = (sy * w + sx) * 4;
                s += (rgba[i] as u32 * 77 + rgba[i + 1] as u32 * 150 + rgba[i + 2] as u32 * 29) >> 8;
            }
            *o = (s / 4) as u8;
        }
    });
    let l0 = Plane { w: w2, h: h2, px };
    let l1 = l0.half();
    let l2 = l1.half();
    Pyramid([l0, l1, l2])
}

/// Поле векторов на сетке блоков (px кадра) и надёжность 0..255.
#[derive(Clone)]
struct Field {
    gw: usize,
    gh: usize,
    v: Vec<(i32, i32)>,
    r: Vec<u8>,
}

/// SAD блока `(ox, oy)` размера `bs` уровня при симметричном сдвиге `v` (px уровня).
#[inline]
fn sad(a: &Plane, b: &Plane, ox: i32, oy: i32, bs: i32, v: (i32, i32)) -> u32 {
    let (hx, hy) = (v.0.div_euclid(2), v.1.div_euclid(2));
    let (ax, ay) = (ox - hx, oy - hy);
    let (bx, by) = (ox + v.0 - hx, oy + v.1 - hy);
    let inside = ax >= 0 && ay >= 0 && bx >= 0 && by >= 0 && ax + bs <= a.w as i32 && ay + bs <= a.h as i32 && bx + bs <= b.w as i32 && by + bs <= b.h as i32;
    let mut s = 0u32;
    if inside {
        for y in 0..bs {
            let ra = &a.px[(ay + y) as usize * a.w + ax as usize..][..bs as usize];
            let rb = &b.px[(by + y) as usize * b.w + bx as usize..][..bs as usize];
            for (p, q) in ra.iter().zip(rb) {
                s += p.abs_diff(*q) as u32;
            }
        }
    } else {
        for y in 0..bs {
            for x in 0..bs {
                s += a.at(ax + x, ay + y).abs_diff(b.at(bx + x, by + y)) as u32;
            }
        }
    }
    s
}

/// Цена вектора: SAD плюс штраф за длину и за отличие от подсказки.
#[inline]
fn cost(s: u32, v: (i32, i32), hint: (i32, i32), bs: i32) -> u32 {
    let len = (v.0.abs() + v.1.abs()) as u32;
    let off = ((v.0 - hint.0).abs() + (v.1 - hint.1).abs()) as u32;
    s + (len + 2 * off) * (bs as u32) / 4
}

/// Оценка движения между пирамидами `a` (прошлый) и `b` (новый); `prev` — поле прошлой пары
/// (подсказка: движение обычно продолжается).
fn estimate(a: &Pyramid, b: &Pyramid, w: usize, h: usize, prev: Option<&Field>) -> Field {
    let gw = w.div_ceil(BLOCK);
    let gh = h.div_ceil(BLOCK);
    let n = gw * gh;
    // Уровень 1/8: полный перебор, блок 4 px.
    let l = 2;
    let bs = (BLOCK >> (l + 1)) as i32;
    let mut v: Vec<(i32, i32)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (bx, by) = ((i % gw) as i32, (i / gw) as i32);
            let (ox, oy) = (bx * bs, by * bs);
            let hint = prev.filter(|p| p.gw == gw && p.gh == gh).map(|p| (p.v[i].0 >> (l + 1), p.v[i].1 >> (l + 1))).unwrap_or((0, 0));
            let mut best = ((0, 0), u32::MAX);
            for dy in -RANGE..=RANGE {
                for dx in -RANGE..=RANGE {
                    let c = cost(sad(&a.0[l], &b.0[l], ox, oy, bs, (dx, dy)), (dx, dy), hint, bs);
                    if c < best.1 {
                        best = ((dx, dy), c);
                    }
                }
            }
            best.0
        })
        .collect();
    // Уровни 1/4 и 1/2: вектор выше ×2 и соседи, уточнение ±1.
    for l in [1usize, 0] {
        let bs = (BLOCK >> (l + 1)) as i32;
        let up = v.clone();
        v = (0..n)
            .into_par_iter()
            .map(|i| {
                let (bx, by) = ((i % gw) as i32, (i / gw) as i32);
                let (ox, oy) = (bx * bs, by * bs);
                let own = (up[i].0 * 2, up[i].1 * 2);
                let mut cands = vec![own, (0, 0)];
                for (nx, ny) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    let (x, y) = (bx + nx, by + ny);
                    if x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh {
                        let u = up[y as usize * gw + x as usize];
                        cands.push((u.0 * 2, u.1 * 2));
                    }
                }
                let mut best = ((0, 0), u32::MAX);
                for c in cands {
                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let cv = (c.0 + dx, c.1 + dy);
                            let k = cost(sad(&a.0[l], &b.0[l], ox, oy, bs, cv), cv, own, bs);
                            if k < best.1 {
                                best = (cv, k);
                            }
                        }
                    }
                }
                best.0
            })
            .collect();
    }
    // Медиана 3×3 (по компонентам) — убрать одиночные выбросы.
    let med: Vec<(i32, i32)> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (bx, by) = ((i % gw) as i32, (i / gw) as i32);
            let mut xs = Vec::with_capacity(9);
            let mut ys = Vec::with_capacity(9);
            for ny in -1..=1 {
                for nx in -1..=1 {
                    let (x, y) = (bx + nx, by + ny);
                    if x >= 0 && y >= 0 && (x as usize) < gw && (y as usize) < gh {
                        let u = v[y as usize * gw + x as usize];
                        xs.push(u.0);
                        ys.push(u.1);
                    }
                }
            }
            xs.sort_unstable();
            ys.sort_unstable();
            (xs[xs.len() / 2], ys[ys.len() / 2])
        })
        .collect();
    // Надёжность — по средней разнице на пиксель (уровень 1/2).
    let bs = (BLOCK / 2) as i32;
    let r: Vec<u8> = (0..n)
        .into_par_iter()
        .map(|i| {
            let (bx, by) = ((i % gw) as i32, (i / gw) as i32);
            let s = sad(&a.0[0], &b.0[0], bx * bs, by * bs, bs, med[i]) as f32 / (bs * bs) as f32;
            (255.0 * (1.0 - ((s - 6.0) / 18.0).clamp(0.0, 1.0))) as u8
        })
        .collect();
    // В px кадра: уровень 1/2 — ×2.
    Field { gw, gh, v: med.into_iter().map(|(x, y)| (x * 2, y * 2)).collect(), r }
}

/// Средняя разница яркости на 1/4 — смена сцены.
fn scene_cut(a: &Pyramid, b: &Pyramid) -> bool {
    let (pa, pb) = (&a.0[1], &b.0[1]);
    if pa.px.len() != pb.px.len() || pa.px.is_empty() {
        return true;
    }
    let s: u64 = pa.px.iter().zip(&pb.px).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    (s / pa.px.len() as u64) as u32 > SCENE_CUT
}

#[cfg(test)]
/// Вектор и надёжность в точке кадра — билинейно между центрами блоков.
#[inline]
fn field_at(f: &Field, x: f32, y: f32) -> (f32, f32, f32) {
    let fx = (x / BLOCK as f32 - 0.5).clamp(0.0, (f.gw - 1) as f32);
    let fy = (y / BLOCK as f32 - 0.5).clamp(0.0, (f.gh - 1) as f32);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(f.gw - 1), (y0 + 1).min(f.gh - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let g = |x: usize, y: usize| {
        let i = y * f.gw + x;
        (f.v[i].0 as f32, f.v[i].1 as f32, f.r[i] as f32 / 255.0)
    };
    let (a, b, c, d) = (g(x0, y0), g(x1, y0), g(x0, y1), g(x1, y1));
    let lerp = |p: f32, q: f32, t: f32| p + (q - p) * t;
    let top = (lerp(a.0, b.0, tx), lerp(a.1, b.1, tx), lerp(a.2, b.2, tx));
    let bot = (lerp(c.0, d.0, tx), lerp(c.1, d.1, tx), lerp(c.2, d.2, tx));
    (lerp(top.0, bot.0, ty), lerp(top.1, bot.1, ty), lerp(top.2, bot.2, ty))
}

/// Промежуточный кадр в момент `t` между `a` и `b` (RGBA `w×h`).
///
/// Вектор плитки 8×8 — лучший из векторов её блока и соседних по совпадению двух кадров вдоль него:
/// на границе движущегося предмета плитка берёт вектор предмета или фона, а не их среднее. Пиксель
/// смешивает результаты векторов четырёх ближайших плиток с весами по расстоянию (перекрывающиеся
/// блоки, OBMC) — без ступенек на стыках плиток.
fn synth(a: &[u8], b: &[u8], w: usize, h: usize, f: &Field, t: f32) -> Vec<u8> {
    const TILE: usize = 8;
    let (wi, hi) = (w as i32, h as i32);
    let idx = |x: i32, y: i32| (y.clamp(0, hi - 1) as usize * w + x.clamp(0, wi - 1) as usize) * 4;
    let (tw, th) = (w.div_ceil(TILE), h.div_ceil(TILE));
    // 1. Вектор каждой плитки: (сдвиг в прошлом кадре, сдвиг в новом, надёжность 0..256).
    let tiles: Vec<((i32, i32), (i32, i32), i32)> = (0..tw * th)
        .into_par_iter()
        .map(|i| {
            let (x0, y0) = ((i % tw) * TILE, (i / tw) * TILE);
            let (bx, by) = (((x0 + TILE / 2) / BLOCK) as i32, ((y0 + TILE / 2) / BLOCK) as i32);
            let mut best = ((0i32, 0i32), (0i32, 0i32), 0i32, u32::MAX);
            let mut seen: [(i32, i32); 9] = [(i32::MIN, 0); 9];
            let mut n = 0;
            for ny in -1..=1 {
                for nx in -1..=1 {
                    let (gx, gy) = (bx + nx, by + ny);
                    if gx < 0 || gy < 0 || gx as usize >= f.gw || gy as usize >= f.gh {
                        continue;
                    }
                    let gi = gy as usize * f.gw + gx as usize;
                    let v = f.v[gi];
                    if seen[..n].contains(&v) {
                        continue;
                    }
                    seen[n] = v;
                    n += 1;
                    let (ax, ay) = ((-t * v.0 as f32).round() as i32, (-t * v.1 as f32).round() as i32);
                    let (cx, cy) = (v.0 + ax, v.1 + ay);
                    let mut s = 0u32;
                    for dy in (0..TILE as i32).step_by(2) {
                        for dx in (0..TILE as i32).step_by(2) {
                            let (x, y) = (x0 as i32 + dx, y0 as i32 + dy);
                            s += (a[idx(x + ax, y + ay) + 1] as i32 - b[idx(x + cx, y + cy) + 1] as i32).unsigned_abs();
                        }
                    }
                    // при равенстве — короче вектор
                    let s = s * 16 + (v.0.abs() + v.1.abs()) as u32;
                    if s < best.3 {
                        best = ((ax, ay), (cx, cy), f.r[gi] as i32 + 1, s);
                    }
                }
            }
            (best.0, best.1, best.2)
        })
        .collect();
    // 2. Пиксели: четыре ближайшие плитки с весами по расстоянию до их центров.
    let mut out = vec![0u8; w * h * 4];
    let wt = (t * 256.0) as u32;
    // Плитки и веса по столбцам — одни на все строки.
    let cols: Vec<(i32, i32, i32)> = (0..w)
        .map(|x| {
            let fx = (x as f32 + 0.5) / TILE as f32 - 0.5;
            let tx0 = (fx.floor() as i32).clamp(0, tw as i32 - 1);
            let tx1 = (tx0 + 1).min(tw as i32 - 1);
            (tx0, tx1, ((fx - tx0 as f32).clamp(0.0, 1.0) * 256.0) as i32)
        })
        .collect();
    out.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        let yi = y as i32;
        let fy = (y as f32 + 0.5) / TILE as f32 - 0.5;
        let ty0 = (fy.floor() as i32).clamp(0, th as i32 - 1);
        let ty1 = (ty0 + 1).min(th as i32 - 1);
        let wy = ((fy - ty0 as f32).clamp(0.0, 1.0) * 256.0) as i32;
        for x in 0..w {
            let xi = x as i32;
            let (tx0, tx1, wx) = cols[x];
            let pl = idx(xi, yi);
            let mut acc = [0i32; 3];
            for (tx, ty, wgt) in [(tx0, ty0, (256 - wx) * (256 - wy)), (tx1, ty0, wx * (256 - wy)), (tx0, ty1, (256 - wx) * wy), (tx1, ty1, wx * wy)] {
                if wgt == 0 {
                    continue;
                }
                let ((ax, ay), (cx, cy), r) = tiles[ty as usize * tw + tx as usize];
                let (pa, pb) = (idx(xi + ax, yi + ay), idx(xi + cx, yi + cy));
                // совпали ли точки двух кадров: нет — меньше доверия вектору, ближе к смешиванию
                let diff = (a[pa] as i32 - b[pb] as i32).abs() + (a[pa + 1] as i32 - b[pb + 1] as i32).abs() + (a[pa + 2] as i32 - b[pb + 2] as i32).abs();
                let k = r * (256 - ((diff - 40) * 3).clamp(0, 256)) >> 8;
                for c in 0..3 {
                    let mc = (a[pa + c] as i32 * (256 - wt as i32) + b[pb + c] as i32 * wt as i32) >> 8;
                    let lin = (a[pl + c] as i32 * (256 - wt as i32) + b[pl + c] as i32 * wt as i32) >> 8;
                    acc[c] += ((mc * k + lin * (256 - k)) >> 8) * (wgt >> 8);
                }
            }
            let o = &mut row[x * 4..][..4];
            for c in 0..3 {
                o[c] = (acc[c] >> 8).clamp(0, 255) as u8;
            }
            o[3] = 255;
        }
    });
    out
}

struct Prev {
    frame: VideoFrame,
    pyr: Pyramid,
    field: Option<Field>,
}

struct Inner {
    seek_gen: u64,
    prev: Option<Prev>,
    /// Последний выданный момент сетки (pts) — сетка не идёт назад.
    last_out: f64,
    slow_gen: Option<u64>,
    recent: std::collections::VecDeque<f32>,
    made: u64,
}

pub struct SmoothFilter {
    /// 0 — вдвое, иначе — частота на выходе.
    pub target: u32,
    inner: Mutex<Inner>,
    status: Mutex<Status>,
}

impl SmoothFilter {
    pub fn new(target: u32) -> Arc<Self> {
        Arc::new(Self {
            target,
            inner: Mutex::new(Inner { seek_gen: u64::MAX, prev: None, last_out: f64::NEG_INFINITY, slow_gen: None, recent: Default::default(), made: 0 }),
            status: Mutex::new(Status::Active { ms: 0.0, budget_ms: 0.0, fps: 0.0 }),
        })
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    /// Моменты промежуточных кадров между `p0` и `p1` (без концов) и попадает ли `p1` на сетку.
    fn times(&self, p0: f64, p1: f64, last_out: f64) -> (Vec<f64>, bool) {
        let dt = p1 - p0;
        if self.target == 0 {
            return (vec![p0 + dt / 2.0], true);
        }
        let step = 1.0 / self.target as f64;
        if dt <= step * 1.05 {
            // ролик и так не реже нужной частоты
            return (Vec::new(), true);
        }
        // Сетка n/частота: точки внутри — промежуточные, точка у самого p1 — сам кадр ролика.
        // Кадр ролика мимо сетки не выходит: шаг на экране остаётся ровным.
        let mut out = Vec::new();
        let mut original = false;
        let mut n = (p0 / step).floor() as i64 + 1;
        loop {
            let t = n as f64 * step;
            if (t - p1).abs() <= step * 0.25 {
                original = true;
                break;
            }
            if t > p1 {
                break;
            }
            if t > p0 + step * 0.25 && t > last_out + step * 0.5 {
                out.push(t);
            }
            n += 1;
        }
        (out, original)
    }
}

impl FrameFilter for SmoothFilter {
    fn apply(&self, _frame: &mut VideoFrame) {}

    fn process(&self, frame: VideoFrame, emit: &mut dyn FnMut(VideoFrame)) {
        let (w, h) = (frame.width as usize, frame.height as usize);
        if frame.rgba.len() < w * h * 4 || w < BLOCK || h < BLOCK {
            emit(frame);
            return;
        }
        let started = Instant::now();
        let mut g = self.inner.lock().unwrap();
        if g.seek_gen != frame.seek_generation {
            g.seek_gen = frame.seek_generation;
            g.prev = None;
            g.last_out = f64::NEG_INFINITY;
            g.recent.clear();
        }
        if g.slow_gen == Some(frame.seek_generation) {
            drop(g);
            emit(frame);
            return;
        }
        let pyr = pyramid(&frame.rgba, w, h);
        let mut field_out = None;
        let mut original = true;
        let prev = g.prev.take();
        let mut last_out = g.last_out;
        let mut made = 0u64;
        if let Some(p) = prev.as_ref().filter(|p| p.frame.width == frame.width && p.frame.height == frame.height) {
            let dt = frame.pts_sec - p.frame.pts_sec;
            if dt > 0.0 && dt < 0.25 && !scene_cut(&p.pyr, &pyr) {
                let field = estimate(&p.pyr, &pyr, w, h, p.field.as_ref());
                let (times, on_grid) = self.times(p.frame.pts_sec, frame.pts_sec, last_out);
                original = on_grid;
                for t_pts in times {
                    let t = ((t_pts - p.frame.pts_sec) / dt) as f32;
                    let px = synth(&p.frame.rgba, &frame.rgba, w, h, &field, t);
                    let mut o = frame.clone();
                    o.rgba = px.into();
                    o.pts_sec = t_pts;
                    o.surface = None;
                    o.yuv = None;
                    emit(o);
                    made += 1;
                    last_out = t_pts;
                }
                field_out = Some(field);
            }
        }
        g.made += made;
        g.last_out = last_out;
        let budget_ms = prev.as_ref().map(|p| ((frame.pts_sec - p.frame.pts_sec) * 1000.0) as f32).filter(|d| *d > 0.0 && *d < 250.0).unwrap_or(0.0);
        g.last_out = g.last_out.max(frame.pts_sec);
        g.prev = Some(Prev { frame: frame.clone(), pyr, field: field_out });
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        g.recent.push_back(ms);
        if g.recent.len() > 24 {
            g.recent.pop_front();
        }
        let avg = g.recent.iter().sum::<f32>() / g.recent.len() as f32;
        let fps = if self.target == 0 && budget_ms > 0.0 { 2000.0 / budget_ms } else { self.target as f32 };
        let status = if budget_ms > 0.0 && g.recent.len() >= 12 && avg > budget_ms * BUDGET {
            g.slow_gen = Some(frame.seek_generation);
            tracing::warn!("плавность: {avg:.1} мс на кадр при бюджете {budget_ms:.1} — кадры как есть до перемотки");
            Status::Slow { ms: avg, budget_ms }
        } else {
            Status::Active { ms: avg, budget_ms, fps }
        };
        *self.status.lock().unwrap() = status;
        drop(g);
        if original {
            emit(frame);
        }
    }
}

// ─── плеер ──────────────────────────────────────────────────────────────────

static FILTER: Mutex<Option<Arc<SmoothFilter>>> = Mutex::new(None);

trait ReplaceOpt<T> {
    /// Заменить значение; `true` — прежнее было.
    fn replace_with_opt(&mut self, v: Option<T>) -> bool;
}

impl<T> ReplaceOpt<T> for Option<T> {
    fn replace_with_opt(&mut self, v: Option<T>) -> bool {
        std::mem::replace(self, v).is_some()
    }
}

/// Нужны ли кадрам пиксели в памяти (плавность включена): аппаратный вывод прямо на экран
/// (Android: MediaCodec → Surface) тогда не годится.
pub fn needs_cpu_frames(st: crate::ui::St) -> bool {
    st.settings.get_untracked().smooth
}

/// Поставить или снять ступень на только что открытом плеере.
pub fn attach(st: crate::ui::St, p: &VideoPlayer) {
    let s = st.settings.get_untracked();
    let f = s.smooth.then(|| SmoothFilter::new(target_of(&s.smooth_mode)));
    let was = FILTER.lock().unwrap().replace_with_opt(f.clone());
    match f {
        Some(f) => {
            tracing::info!("плавность: включена ({})", s.smooth_mode);
            chain::set(p, Stage::Interpolate, Some(f));
        }
        None => {
            // ступень была наша — снять; RIFE (на компьютере) поставит свою, если включён
            if was {
                chain::set(p, Stage::Interpolate, None);
                crate::interp::attach(st, p);
            }
            set_status(st, Status::Off);
        }
    }
}

/// Применить настройки к открытому ролику.
pub fn apply(st: crate::ui::St) {
    if let Some(p) = st.playing.get_untracked() {
        if let Ok(pl) = p.player.lock() {
            attach(st, &pl);
        }
    }
}

fn set_status(st: crate::ui::St, s: Status) {
    run_on_main_thread(move || {
        if st.smooth.get_untracked() != s {
            st.smooth.set(s);
        }
    });
}

/// Состояние для настроек (из потока наблюдателя).
pub fn poll(st: crate::ui::St) {
    let s = FILTER.lock().unwrap().as_ref().map(|f| f.status()).unwrap_or_default();
    set_status(st, s);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Кадр с квадратом 64×64 в точке `(x, y)` на сером фоне с текстурой.
    fn frame(w: usize, h: usize, x: usize, y: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for j in 0..h {
            for i in 0..w {
                let o = (j * w + i) * 4;
                // неповторяющаяся текстура: фон стоит, узор квадрата едет вместе с ним
                let hash = |u: usize, v: usize| ((u.wrapping_mul(73_856_093) ^ v.wrapping_mul(19_349_663)) >> 4) as u8;
                let inside = i >= x && i < x + 64 && j >= y && j < y + 64;
                let c = if inside { 160 + hash((i - x) / 4, (j - y) / 4) % 90 } else { 20 + hash(i / 4 + 1000, j / 4) % 90 };
                v[o..o + 4].copy_from_slice(&[c, c, c, 255]);
            }
        }
        v
    }

    #[test]
    fn finds_motion_and_builds_middle_frame() {
        let (w, h) = (320, 192);
        let a = frame(w, h, 64, 64);
        let b = frame(w, h, 96, 72);
        let (pa, pb) = (pyramid(&a, w, h), pyramid(&b, w, h));
        assert!(!scene_cut(&pa, &pb));
        let f = estimate(&pa, &pb, w, h, None);
        // блок в центре квадрата: движение (32, 8)
        let (vx, vy, r) = field_at(&f, 112.0, 100.0);
        assert!((vx - 32.0).abs() <= 4.0 && (vy - 8.0).abs() <= 4.0, "вектор {vx},{vy}");
        assert!(r > 0.5, "надёжность {r}");
        // середина: квадрат около (80, 68) — как настоящий кадр в момент 0.5
        let mid = synth(&a, &b, w, h, &f, 0.5);
        let want = frame(w, h, 80, 68);
        let err: u64 = (60..120).flat_map(|y| (60..160).map(move |x| (x, y))).map(|(x, y)| (mid[(y * w + x) * 4] as i32 - want[(y * w + x) * 4] as i32).unsigned_abs() as u64).sum();
        let lin: Vec<u8> = a.iter().zip(&b).map(|(p, q)| ((*p as u16 + *q as u16) / 2) as u8).collect();
        let err_lin: u64 = (60..120).flat_map(|y| (60..160).map(move |x| (x, y))).map(|(x, y)| (lin[(y * w + x) * 4] as i32 - want[(y * w + x) * 4] as i32).unsigned_abs() as u64).sum();
        assert!(err * 3 < err_lin, "по движению {err}, смешиванием {err_lin}");
    }

    /// Реальные кадры: `SMOOTH_FRAMES=папка` с f1.png, f2.png, f3.png подряд — середина между 1 и 3
    /// против настоящего 2-го (PSNR), время на кадр, mid.png рядом.
    #[test]
    #[ignore]
    fn real_frames() {
        let Ok(dir) = std::env::var("SMOOTH_FRAMES") else { return };
        let load = |n: &str| image::open(format!("{dir}/{n}")).unwrap().to_rgba8();
        let (a, want, b) = (load("f1.png"), load("f2.png"), load("f3.png"));
        let (w, h) = (a.width() as usize, a.height() as usize);
        let t0 = Instant::now();
        let (pa, pb) = (pyramid(&a, w, h), pyramid(&b, w, h));
        let t1 = Instant::now();
        let f = estimate(&pa, &pb, w, h, None);
        let t2 = Instant::now();
        let mid = synth(&a, &b, w, h, &f, 0.5);
        let t3 = Instant::now();
        let psnr = |x: &[u8]| {
            let mse: f64 = x.iter().zip(want.as_raw()).enumerate().filter(|(i, _)| i % 4 != 3).map(|(_, (p, q))| (*p as f64 - *q as f64).powi(2)).sum::<f64>() / (w * h * 3) as f64;
            10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
        };
        let lin: Vec<u8> = a.as_raw().iter().zip(b.as_raw()).map(|(p, q)| ((*p as u16 + *q as u16) / 2) as u8).collect();
        let moving = f.v.iter().filter(|v| v.0 != 0 || v.1 != 0).count();
        println!(
            "{w}x{h}: пирамиды {:.1} мс, движение {:.1} мс, кадр {:.1} мс; PSNR по движению {:.2} дБ, смешиванием {:.2} дБ, прошлый кадр {:.2} дБ; блоков с движением {moving}/{}",
            (t1 - t0).as_secs_f64() * 1e3, (t2 - t1).as_secs_f64() * 1e3, (t3 - t2).as_secs_f64() * 1e3,
            psnr(&mid), psnr(&lin), psnr(a.as_raw()), f.v.len()
        );
        image::RgbaImage::from_raw(w as u32, h as u32, mid).unwrap().save(format!("{dir}/mid.png")).unwrap();
    }

    #[test]
    fn grid_times() {
        let f = SmoothFilter::new(60);
        let (t, _) = f.times(0.0, 1.0 / 24.0, f64::NEG_INFINITY);
        // между 0 и 41.7 мс — 16.7 и 33.3
        assert_eq!(t.len(), 2);
        assert!((t[0] - 1.0 / 60.0).abs() < 1e-9);
        // кадр ролика на 2/24 = 5/60 — на сетке, на 1/24 — нет
        let (t2, orig) = f.times(1.0 / 24.0, 2.0 / 24.0, 1.0 / 24.0);
        assert!(orig && t2.len() == 2, "{t2:?}");
        assert!(!f.times(0.0, 1.0 / 24.0, f64::NEG_INFINITY).1);
        let f2 = SmoothFilter::new(0);
        assert_eq!(f2.times(0.0, 0.04, f64::NEG_INFINITY).0, vec![0.02]);
    }
}
