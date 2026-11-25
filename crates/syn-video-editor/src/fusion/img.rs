//! Растровый буфер и графические операции графа Fusion (премультиплицированный f32 RGBA,
//! многопоточность `rayon`). Координаты Fusion: `x ∈ [0, 1]` слева направо,
//! `y ∈ [0, 1]` снизу вверх.

use rayon::prelude::*;

/// Премультиплицированный RGBA-цвет `[0.0, 1.0]`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Color4 {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color4 {
    pub const TRANSPARENT: Self = Self { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    pub fn from_straight(r: f32, g: f32, b: f32, a: f32) -> Self {
        let a = a.clamp(0.0, 1.0);
        Self { r: r.clamp(0.0, 1.0) * a, g: g.clamp(0.0, 1.0) * a, b: b.clamp(0.0, 1.0) * a, a }
    }

    pub fn scaled(self, k: f32) -> Self {
        Self { r: self.r * k, g: self.g * k, b: self.b * k, a: self.a * k }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let m = 1.0 - t;
        Self {
            r: self.r * m + other.r * t,
            g: self.g * m + other.g * t,
            b: self.b * m + other.b * t,
            a: self.a * m + other.a * t,
        }
    }
}

/// Кадр графа: премультиплицированный RGBA `f32`, строки сверху вниз (`py = 0` — верх).
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` чисел `[r, g, b, a]`.
    pub data: Vec<f32>,
}

impl Image {
    pub fn empty(width: u32, height: u32) -> Self {
        Self { width, height, data: vec![0.0; (width as usize) * (height as usize) * 4] }
    }

    pub fn solid(width: u32, height: u32, c: Color4) -> Self {
        let n = (width as usize) * (height as usize);
        let mut data = Vec::with_capacity(n * 4);
        for _ in 0..n {
            data.extend_from_slice(&[c.r, c.g, c.b, c.a]);
        }
        Self { width, height, data }
    }

    pub fn from_rgba8(width: u32, height: u32, rgba: &[u8]) -> Self {
        let n = (width as usize) * (height as usize);
        let mut data = vec![0.0f32; n * 4];
        for (dst, src) in data.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            let a = src[3] as f32 / 255.0;
            dst[0] = (src[0] as f32 / 255.0) * a;
            dst[1] = (src[1] as f32 / 255.0) * a;
            dst[2] = (src[2] as f32 / 255.0) * a;
            dst[3] = a;
        }
        Self { width, height, data }
    }

    /// Преобразовать обратно в straight RGBA8 (для `syngui::video::RgbaFrame` и PNG).
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = vec![0u8; (self.width as usize) * (self.height as usize) * 4];
        out.par_chunks_exact_mut(4)
            .zip(self.data.par_chunks_exact(4))
            .for_each(|(dst, src)| {
                let a = src[3].clamp(0.0, 1.0);
                if a > 1e-5 {
                    let inv = 1.0 / a;
                    dst[0] = ((src[0] * inv).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    dst[1] = ((src[1] * inv).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    dst[2] = ((src[2] * inv).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    dst[3] = (a * 255.0 + 0.5) as u8;
                }
            });
        out
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Color4 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return Color4::TRANSPARENT;
        }
        let k = ((y as u32 * self.width + x as u32) * 4) as usize;
        Color4 { r: self.data[k], g: self.data[k + 1], b: self.data[k + 2], a: self.data[k + 3] }
    }

    /// Билинейная выборка в координатах Fusion (`u ∈ [0, 1]` слева направо, `v ∈ [0, 1]` снизу вверх).
    /// `edges`: 0 = Canvas (прозрачно), 1 = Wrap, 2 = Duplicate (clamp), 3 = Mirror.
    #[inline]
    pub fn sample_uv(&self, mut u: f32, mut v: f32, edges: i32) -> Color4 {
        match edges {
            1 => {
                u = u.rem_euclid(1.0);
                v = v.rem_euclid(1.0);
            }
            2 => {
                u = u.clamp(0.0, 1.0);
                v = v.clamp(0.0, 1.0);
            }
            3 => {
                let m = |x: f32| {
                    let r = x.rem_euclid(2.0);
                    if r > 1.0 { 2.0 - r } else { r }
                };
                u = m(u);
                v = m(v);
            }
            _ => {
                if u < 0.0 || u > 1.0 || v < 0.0 || v > 1.0 {
                    return Color4::TRANSPARENT;
                }
            }
        }
        let fx = u * self.width as f32 - 0.5;
        let fy = (1.0 - v) * self.height as f32 - 0.5;
        let x0 = fx.floor() as i32;
        let y0 = fy.floor() as i32;
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let clamp_edge = edges == 2 || edges == 3;
        let at = |x: i32, y: i32| {
            if clamp_edge {
                self.get(x.clamp(0, self.width as i32 - 1), y.clamp(0, self.height as i32 - 1))
            } else {
                self.get(x, y)
            }
        };
        let c00 = at(x0, y0);
        let c10 = at(x0 + 1, y0);
        let c01 = at(x0, y0 + 1);
        let c11 = at(x0 + 1, y0 + 1);
        c00.lerp(c10, tx).lerp(c01.lerp(c11, tx), ty)
    }

    /// Извлечь альфу маски в пикселе (или яркость, если альфа = 1 у монохромного входа).
    #[inline]
    pub fn mask_at(&self, idx: usize) -> f32 {
        self.data[idx * 4 + 3].clamp(0.0, 1.0)
    }
}

/// Умножить изображение на маску (для генераторов `Background`, `TextPlus`).
pub fn apply_mask_mul(img: &mut Image, mask: &Image) {
    img.data
        .par_chunks_exact_mut(4)
        .zip(mask.data.par_chunks_exact(4))
        .for_each(|(p, m)| {
            let a = m[3].clamp(0.0, 1.0);
            p[0] *= a;
            p[1] *= a;
            p[2] *= a;
            p[3] *= a;
        });
}

/// Смешать результат фильтра с исходником по маске и `blend` (`out = lerp(orig, res, mask * blend)`).
pub fn blend_with_mask(orig: &Image, res: &mut Image, mask: Option<&Image>, blend: f32) {
    let blend = blend.clamp(0.0, 1.0);
    if mask.is_none() && (blend - 1.0).abs() < 1e-5 {
        return;
    }
    let mdata = mask.map(|m| m.data.as_slice());
    res.data
        .par_chunks_exact_mut(4)
        .enumerate()
        .for_each(|(i, dst)| {
            let m = mdata.map(|md| md[i * 4 + 3].clamp(0.0, 1.0)).unwrap_or(1.0) * blend;
            let inv = 1.0 - m;
            let k = i * 4;
            dst[0] = orig.data[k] * inv + dst[0] * m;
            dst[1] = orig.data[k + 1] * inv + dst[1] * m;
            dst[2] = orig.data[k + 2] * inv + dst[2] * m;
            dst[3] = orig.data[k + 3] * inv + dst[3] * m;
        });
}

/// Восстановить каналы, для которых `ProcessRed/Green/Blue/Alpha == 0`.
pub fn apply_channel_mask(orig: &Image, res: &mut Image, channels: [bool; 4]) {
    if channels == [true, true, true, true] {
        return;
    }
    res.data
        .par_chunks_exact_mut(4)
        .zip(orig.data.par_chunks_exact(4))
        .for_each(|(dst, src)| {
            for c in 0..4 {
                if !channels[c] {
                    dst[c] = src[c];
                }
            }
        });
}

// ── Background / Gradient ────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct GradStop {
    pub pos: f32,
    pub color: Color4,
}

#[allow(clippy::too_many_arguments)]
pub fn render_background(
    w: u32,
    h: u32,
    bg_type: &str,
    tl: Color4,
    tr: Color4,
    bl: Color4,
    br: Color4,
    grad_type: &str,
    start: (f32, f32),
    end: (f32, f32),
    stops: &[GradStop],
) -> Image {
    if bg_type == "Solid" || bg_type.is_empty() {
        return Image::solid(w, h, tl);
    }
    let mut img = Image::empty(w, h);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let len2 = (dx * dx + dy * dy).max(1e-6);
    let len = len2.sqrt();
    let sample_grad = |t: f32| -> Color4 {
        if stops.is_empty() {
            return tl.lerp(tr, t.clamp(0.0, 1.0));
        }
        let t = t.clamp(0.0, 1.0);
        if t <= stops[0].pos {
            return stops[0].color;
        }
        for s in stops.windows(2) {
            if t >= s[0].pos && t <= s[1].pos {
                let d = (s[1].pos - s[0].pos).max(1e-6);
                return s[0].color.lerp(s[1].color, (t - s[0].pos) / d);
            }
        }
        stops.last().unwrap().color
    };

    img.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let c = match bg_type {
                "Horizontal" => tl.lerp(tr, u),
                "Vertical" => bl.lerp(tl, v),
                "Four Corner" => bl.lerp(br, u).lerp(tl.lerp(tr, u), v),
                "Gradient" => {
                    let t = match grad_type {
                        "Radial" => ((u - start.0).hypot(v - start.1) / len).clamp(0.0, 1.0),
                        "Reflect" => ((((u - start.0) * dx + (v - start.1) * dy) / len2).abs()).clamp(0.0, 1.0),
                        "Square" => ((u - start.0).abs().max((v - start.1).abs()) / len).clamp(0.0, 1.0),
                        "Angle" => {
                            let a = (v - start.1).atan2(u - start.0);
                            (a / std::f32::consts::TAU).rem_euclid(1.0)
                        }
                        _ => (((u - start.0) * dx + (v - start.1) * dy) / len2).clamp(0.0, 1.0),
                    };
                    sample_grad(t)
                }
                _ => tl,
            };
            let k = px * 4;
            row[k] = c.r;
            row[k + 1] = c.g;
            row[k + 2] = c.b;
            row[k + 3] = c.a;
        }
    });
    img
}

// ── Маски ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct MaskCommon {
    pub level: f32,
    pub soft_edge: f32,
    pub border_width: f32,
    pub solid: bool,
    pub invert: bool,
    pub paint_mode: &'static str,
}

fn finalize_mask(mut img: Image, common: MaskCommon, parent_mask: Option<&Image>) -> Image {
    if common.soft_edge > 1e-4 {
        let r = common.soft_edge * img.width.max(img.height) as f32 * 0.5;
        img = box_blur(&img, r, r);
    }
    img.data.par_chunks_exact_mut(4).enumerate().for_each(|(i, px)| {
        let mut a = px[3].clamp(0.0, 1.0);
        if common.invert {
            a = 1.0 - a;
        }
        a *= common.level.clamp(0.0, 1.0);
        if let Some(pm) = parent_mask {
            let b = pm.mask_at(i);
            a = match common.paint_mode {
                "Subtract" => (b - a).max(0.0),
                "Multiply" | "Minimum" => b.min(a),
                "Add" => (b + a).min(1.0),
                "Invert" => (b - a).abs(),
                "Replace" => a,
                _ => (b + a * (1.0 - b)).clamp(0.0, 1.0), // Merge (Over)
            };
        }
        px[0] = a;
        px[1] = a;
        px[2] = a;
        px[3] = a;
    });
    img
}

/// Прямоугольная или эллиптическая маска Fusion (с поворотом, скруглением, `BorderWidth` и `SoftEdge`).
#[allow(clippy::too_many_arguments)]
pub fn render_shape_mask(
    w: u32,
    h: u32,
    ellipse: bool,
    center: (f32, f32),
    mw: f32,
    mh: f32,
    corner: f32,
    angle_deg: f32,
    common: MaskCommon,
    parent_mask: Option<&Image>,
) -> Image {
    let mut img = Image::empty(w, h);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let aspect = wf / hf;
    let (sin_a, cos_a) = (-angle_deg.to_radians()).sin_cos();
    let hx = (mw * 0.5).max(1e-4);
    let hy = (mh * 0.5).max(1e-4);
    let aa = 1.0 / wf.min(hf);
    let rad = corner.clamp(0.0, 1.0) * hx.min(hy / aspect);

    img.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            // Перевод в изотропные координаты вокруг center
            let dx = u - center.0;
            let dy = (v - center.1) / aspect;
            let rx = dx * cos_a - dy * sin_a;
            let ry = (dx * sin_a + dy * cos_a) * aspect;
            let d = if ellipse {
                let qx = rx / hx;
                let qy = ry / hy;
                let r = qx.hypot(qy);
                (r - 1.0) * hx.min(hy)
            } else {
                // Rounded rect SDF в координатах X (с aspect для Y)
                let qx = rx.abs() - (hx - rad);
                let qy = (ry.abs() / aspect) - (hy / aspect - rad);
                qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad
            };
            let cov = if common.solid {
                let d_eff = d - common.border_width * 0.5;
                (0.5 - d_eff / aa).clamp(0.0, 1.0)
            } else {
                let hw = (common.border_width * 0.5).max(aa);
                (0.5 - (d.abs() - hw) / aa).clamp(0.0, 1.0)
            };
            let k = px * 4;
            row[k] = cov;
            row[k + 1] = cov;
            row[k + 2] = cov;
            row[k + 3] = cov;
        }
    });
    finalize_mask(img, common, parent_mask)
}

/// Треугольная маска `TriangleMask`.
#[allow(clippy::too_many_arguments)]
pub fn render_triangle_mask(
    w: u32,
    h: u32,
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    common: MaskCommon,
    parent_mask: Option<&Image>,
) -> Image {
    let pts = vec![(p1.0 as f64 - 0.5, p1.1 as f64 - 0.5), (p2.0 as f64 - 0.5, p2.1 as f64 - 0.5), (p3.0 as f64 - 0.5, p3.1 as f64 - 0.5)];
    render_poly_mask(w, h, &pts, true, (0.5, 0.5), 1.0, 0.0, 1.0, 0.0, common, parent_mask)
}

/// Полигональная / кривая маска (`PolylineMask`, `BSplineMask`, `TriangleMask`), включая `WriteLength` и `BorderWidth`.
#[allow(clippy::too_many_arguments)]
pub fn render_poly_mask(
    w: u32,
    h: u32,
    rel_pts: &[(f64, f64)],
    closed: bool,
    center: (f32, f32),
    size: f32,
    angle_deg: f32,
    write_len: f32,
    write_pos: f32,
    common: MaskCommon,
    parent_mask: Option<&Image>,
) -> Image {
    let mut img = Image::empty(w, h);
    if rel_pts.len() < 2 {
        return finalize_mask(img, common, parent_mask);
    }
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let aspect = wf / hf;
    let (sin_a, cos_a) = angle_deg.to_radians().sin_cos();

    // Преобразование точек в нормализованные (u, v) Fusion
    let mut pts: Vec<(f32, f32)> = rel_pts
        .iter()
        .map(|&(rx, ry)| {
            let sx = rx as f32 * size;
            let sy = (ry as f32 * size) / aspect;
            let x = sx * cos_a - sy * sin_a;
            let y = (sx * sin_a + sy * cos_a) * aspect;
            (center.0 + x, center.1 + y)
        })
        .collect();

    // Обрезка по WritePosition / WriteLength (для анимированных линий!)
    let wlen = write_len.clamp(0.0, 1.0);
    let wpos = write_pos.clamp(0.0, 1.0);
    if wlen < 0.999 || wpos > 0.001 {
        let mut cum = vec![0.0f32];
        let mut total = 0.0f32;
        for seg in pts.windows(2) {
            total += (seg[1].0 - seg[0].0).hypot((seg[1].1 - seg[0].1) / aspect);
            cum.push(total);
        }
        if total > 1e-6 {
            let s0 = wpos * total;
            let s1 = (wpos + wlen).min(1.0) * total;
            let mut clipped = Vec::new();
            let interp = |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for i in 0..pts.len() - 1 {
                let (d0, d1) = (cum[i], cum[i + 1]);
                if d1 < s0 || d0 > s1 {
                    continue;
                }
                let seg = (d1 - d0).max(1e-6);
                let a = if d0 < s0 { interp(pts[i], pts[i + 1], (s0 - d0) / seg) } else { pts[i] };
                let b = if d1 > s1 { interp(pts[i], pts[i + 1], (s1 - d0) / seg) } else { pts[i + 1] };
                if clipped.is_empty() {
                    clipped.push(a);
                }
                clipped.push(b);
            }
            pts = clipped;
        }
    }
    if pts.len() < 2 {
        return finalize_mask(img, common, parent_mask);
    }

    let stroke_only = !common.solid || !closed;
    let hw = (common.border_width.abs() * 0.5).max(1.0 / wf.min(hf));
    let aa = 1.0 / wf.min(hf);
    let n = pts.len();
    let seg_count = if closed && !stroke_only { n } else { n - 1 };

    // Ограничивающий прямоугольник для пропуска пустых строк
    let mut min_v = 1.0f32;
    let mut max_v = 0.0f32;
    for &(_, y) in &pts {
        min_v = min_v.min(y);
        max_v = max_v.max(y);
    }
    let pad_v = (hw + aa * 2.0) * aspect;
    min_v -= pad_v;
    max_v += pad_v;

    img.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        if v < min_v || v > max_v {
            return;
        }
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let mut min_d = f32::MAX;
            let mut inside = false;
            for i in 0..seg_count {
                let (ax, ay) = pts[i];
                let (bx, by) = pts[(i + 1) % n];
                if !stroke_only {
                    // Ray casting (even-odd)
                    if (ay > v) != (by > v) {
                        let x_int = ax + (v - ay) * (bx - ax) / (by - ay);
                        if u < x_int {
                            inside = !inside;
                        }
                    }
                }
                // Расстояние до отрезка в изотропном пространстве
                let abx = bx - ax;
                let aby = (by - ay) / aspect;
                let apx = u - ax;
                let apy = (v - ay) / aspect;
                let ab2 = (abx * abx + aby * aby).max(1e-10);
                let t = ((apx * abx + apy * aby) / ab2).clamp(0.0, 1.0);
                let d = (apx - abx * t).hypot(apy - aby * t);
                if d < min_d {
                    min_d = d;
                }
            }
            let cov = if stroke_only {
                (0.5 - (min_d - hw) / aa).clamp(0.0, 1.0)
            } else {
                let signed = if inside { -min_d } else { min_d } - common.border_width * 0.5;
                (0.5 - signed / aa).clamp(0.0, 1.0)
            };
            if cov > 0.0 {
                let k = px * 4;
                row[k] = cov;
                row[k + 1] = cov;
                row[k + 2] = cov;
                row[k + 3] = cov;
            }
        }
    });
    finalize_mask(img, common, parent_mask)
}

// ── Transform ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn transform_image(
    src: &Image,
    center: (f32, f32),
    pivot: (f32, f32),
    sx: f32,
    sy: f32,
    angle_deg: f32,
    flip_h: bool,
    flip_v: bool,
    edges: i32,
) -> Image {
    let (w, h) = (src.width, src.height);
    let mut out = Image::empty(w, h);
    if sx.abs() < 1e-5 || sy.abs() < 1e-5 {
        return out;
    }
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let aspect = wf / hf;
    // Обратное преобразование: из выходной точки (u, v) в исходную (su, sv)
    // В Fusion: p' = center + R(angle) * S(sx, sy) * (p - pivot) + (pivot - 0.5)
    let (sin_a, cos_a) = (-angle_deg.to_radians()).sin_cos();
    let inv_sx = if flip_h { -1.0 / sx } else { 1.0 / sx };
    let inv_sy = if flip_v { -1.0 / sy } else { 1.0 / sy };
    let off_x = center.0 - 0.5;
    let off_y = center.1 - 0.5;

    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        let dy0 = (v - pivot.1 - off_y) / aspect;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let dx0 = u - pivot.0 - off_x;
            let rx = dx0 * cos_a - dy0 * sin_a;
            let ry = (dx0 * sin_a + dy0 * cos_a) * aspect;
            let su = rx * inv_sx + pivot.0;
            let sv = ry * inv_sy + pivot.1;
            let c = src.sample_uv(su, sv, edges);
            let k = px * 4;
            row[k] = c.r;
            row[k + 1] = c.g;
            row[k + 2] = c.b;
            row[k + 3] = c.a;
        }
    });
    out
}

// ── Merge / Dissolve ─────────────────────────────────────────────────────

fn blend_ch(mode: &str, d: f32, s: f32) -> f32 {
    match mode {
        "Multiply" => d * s,
        "Screen" => d + s - d * s,
        "Darken" => d.min(s),
        "Lighten" => d.max(s),
        "Overlay" => if d < 0.5 { 2.0 * d * s } else { 1.0 - 2.0 * (1.0 - d) * (1.0 - s) },
        "Hard Light" => if s < 0.5 { 2.0 * d * s } else { 1.0 - 2.0 * (1.0 - d) * (1.0 - s) },
        "Soft Light" => (1.0 - 2.0 * s) * d * d + 2.0 * s * d,
        "Difference" => (d - s).abs(),
        "Exclusion" => d + s - 2.0 * d * s,
        "Color Dodge" => if s >= 1.0 { 1.0 } else { (d / (1.0 - s)).min(1.0) },
        "Color Burn" => if s <= 0.0 { 0.0 } else { (1.0 - (1.0 - d) / s).max(0.0) },
        "Linear Dodge (Add)" | "Add" => (d + s).min(1.0),
        _ => s,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn merge_images(
    bg: &Image,
    fg: &Image,
    center: (f32, f32),
    size: f32,
    angle_deg: f32,
    blend: f32,
    alpha_gain: f32,
    apply_mode: &str,
    operator: &str,
    mask: Option<&Image>,
) -> Image {
    let fg_xform = if (center.0 - 0.5).abs() > 1e-4 || (center.1 - 0.5).abs() > 1e-4 || (size - 1.0).abs() > 1e-4 || angle_deg.abs() > 1e-4 {
        Some(transform_image(fg, center, (0.5, 0.5), size, size, angle_deg, false, false, 0))
    } else {
        None
    };
    let fg_ref = fg_xform.as_ref().unwrap_or(fg);
    let mut out = Image::empty(bg.width, bg.height);
    let blend = blend.clamp(0.0, 1.0);
    let mdata = mask.map(|m| m.data.as_slice());

    out.data.par_chunks_exact_mut(4).enumerate().for_each(|(i, dst)| {
        let k = i * 4;
        let br = bg.data[k];
        let bg_ = bg.data[k + 1];
        let bb = bg.data[k + 2];
        let ba = bg.data[k + 3].clamp(0.0, 1.0);

        let m = mdata.map(|md| md[k + 3].clamp(0.0, 1.0)).unwrap_or(1.0) * blend;
        let sr = fg_ref.data[k] * m;
        let sg = fg_ref.data[k + 1] * m;
        let sb = fg_ref.data[k + 2] * m;
        let sa = (fg_ref.data[k + 3] * m * alpha_gain).clamp(0.0, 1.0);

        // Применяем ApplyMode к пересечению
        let (cr, cg, cb) = if apply_mode == "Normal" || apply_mode.is_empty() || ba < 1e-5 || sa < 1e-5 {
            (sr, sg, sb)
        } else if apply_mode == "Linear Dodge (Add)" || apply_mode == "Add" {
            let r = (br + sr).min(1.0);
            let g = (bg_ + sg).min(1.0);
            let b = (bb + sb).min(1.0);
            let a = (ba + sa * (1.0 - ba)).clamp(0.0, 1.0);
            dst[0] = r;
            dst[1] = g;
            dst[2] = b;
            dst[3] = a;
            return;
        } else {
            let (dr, dg, db) = (br / ba, bg_ / ba, bb / ba);
            let (fr, fg0, fb) = (sr / sa, sg / sa, sb / sa);
            let mr = blend_ch(apply_mode, dr, fr);
            let mg = blend_ch(apply_mode, dg, fg0);
            let mb = blend_ch(apply_mode, db, fb);
            (
                sa * (1.0 - ba) * fr + sa * ba * mr,
                sa * (1.0 - ba) * fg0 + sa * ba * mg,
                sa * (1.0 - ba) * fb + sa * ba * mb,
            )
        };

        // Porter-Duff Operator
        let (r, g, b, a) = match operator {
            "In" | "Mask" => (cr * ba, cg * ba, cb * ba, sa * ba),
            "Held Out" | "Stencil" => (br * (1.0 - sa), bg_ * (1.0 - sa), bb * (1.0 - sa), ba * (1.0 - sa)),
            "Atop" => (cr * ba + br * (1.0 - sa), cg * ba + bg_ * (1.0 - sa), cb * ba + bb * (1.0 - sa), ba),
            "XOr" => (
                cr * (1.0 - ba) + br * (1.0 - sa),
                cg * (1.0 - ba) + bg_ * (1.0 - sa),
                cb * (1.0 - ba) + bb * (1.0 - sa),
                sa * (1.0 - ba) + ba * (1.0 - sa),
            ),
            "Under" => (br + cr * (1.0 - ba), bg_ + cg * (1.0 - ba), bb + cb * (1.0 - ba), ba + sa * (1.0 - ba)),
            _ => (cr + br * (1.0 - sa), cg + bg_ * (1.0 - sa), cb + bb * (1.0 - sa), sa + ba * (1.0 - sa)),
        };
        dst[0] = r.clamp(0.0, 1.0);
        dst[1] = g.clamp(0.0, 1.0);
        dst[2] = b.clamp(0.0, 1.0);
        dst[3] = a.clamp(0.0, 1.0);
    });
    out
}

pub fn dissolve_images(bg: &Image, fg: &Image, mix: f32) -> Image {
    let t = mix.clamp(0.0, 1.0);
    let m = 1.0 - t;
    let mut out = Image::empty(bg.width, bg.height);
    out.data
        .par_chunks_exact_mut(4)
        .zip(bg.data.par_chunks_exact(4).zip(fg.data.par_chunks_exact(4)))
        .for_each(|(dst, (b, f))| {
            dst[0] = b[0] * m + f[0] * t;
            dst[1] = b[1] * m + f[1] * t;
            dst[2] = b[2] * m + f[2] * t;
            dst[3] = b[3] * m + f[3] * t;
        });
    out
}

// ── TextPlus ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn render_text_plus(
    w: u32,
    h: u32,
    text: &str,
    family: &str,
    style: &str,
    size: f32,
    color: Color4,
    center: (f32, f32),
    tracking: f32,
    line_spacing: f32,
    h_align: f32,
    v_align: f32,
    angle_deg: f32,
) -> Image {
    if text.is_empty() || color.a <= 1e-4 {
        return Image::empty(w, h);
    }
    if !family.is_empty() {
        crate::fonts::ensure(family);
    }
    let st_low = style.to_lowercase();
    let weight = if st_low.contains("black") || st_low.contains("heavy") || st_low.contains("ultra") {
        900
    } else if st_low.contains("extrabold") || st_low.contains("extra bold") {
        800
    } else if st_low.contains("bold") {
        700
    } else if st_low.contains("semi") || st_low.contains("demi") {
        600
    } else if st_low.contains("medium") {
        500
    } else if st_low.contains("light") {
        300
    } else if st_low.contains("thin") {
        200
    } else {
        400
    };
    let italic = st_low.contains("italic") || st_low.contains("oblique");
    let display_text = if st_low.contains("caps") { text.to_uppercase() } else { text.to_string() };
    // В Fusion Size — доля высоты кадра (примерно cap-height; ~1.18 * h * size для em-box)
    let size_px = (size * h as f32 * 1.18).clamp(4.0, h as f32 * 2.0);

    let rast = syngui::text::rasterize_text(
        &display_text,
        &syngui::text::TextRasterOpts {
            family,
            weight,
            italic,
            size_px,
            tracking,
            line_spacing,
            h_align,
            v_align,
        },
    );
    if rast.width == 0 || rast.height == 0 {
        return Image::empty(w, h);
    }
    let mut img = Image::empty(w, h);
    let anchor_x = center.0 * w as f32;
    let anchor_y = (1.0 - center.1) * h as f32;
    let x0 = (anchor_x + rast.offset_x).round() as i32;
    let y0 = (anchor_y + rast.offset_y).round() as i32;

    for row in 0..rast.height as i32 {
        let dy = y0 + row;
        if dy < 0 || dy >= h as i32 {
            continue;
        }
        let srow = (row as u32 * rast.width) as usize;
        let drow = (dy as u32 * w) as usize;
        for col in 0..rast.width as i32 {
            let dx = x0 + col;
            if dx < 0 || dx >= w as i32 {
                continue;
            }
            let cov = rast.alpha[srow + col as usize] as f32 / 255.0;
            if cov > 0.0 {
                let k = (drow + dx as usize) * 4;
                let a = color.a * cov;
                img.data[k] = color.r * cov;
                img.data[k + 1] = color.g * cov;
                img.data[k + 2] = color.b * cov;
                img.data[k + 3] = a;
            }
        }
    }
    if angle_deg.abs() > 1e-3 {
        transform_image(&img, center, center, 1.0, 1.0, angle_deg, false, false, 0)
    } else {
        img
    }
}

// ── Blur / DirectionalBlur / Glow / DropShadow ───────────────────────────

/// Быстрое приближение Гауссова размытия (3-проходный разделимый box-blur, O(N)).
pub fn box_blur(src: &Image, rx: f32, ry: f32) -> Image {
    let irx = (rx.max(0.0) * 0.65).round() as i32;
    let iry = (ry.max(0.0) * 0.65).round() as i32;
    if irx == 0 && iry == 0 {
        return src.clone();
    }
    let mut cur = src.clone();
    let mut tmp = Image::empty(src.width, src.height);
    for _ in 0..3 {
        if irx > 0 {
            blur_h(&cur, &mut tmp, irx);
            std::mem::swap(&mut cur, &mut tmp);
        }
        if iry > 0 {
            blur_v(&cur, &mut tmp, iry);
            std::mem::swap(&mut cur, &mut tmp);
        }
    }
    cur
}

fn blur_h(src: &Image, dst: &mut Image, r: i32) {
    let w = src.width as i32;
    let inv = 1.0 / (2 * r + 1) as f32;
    dst.data.par_chunks_exact_mut(src.width as usize * 4).enumerate().for_each(|(y, out_row)| {
        let row = &src.data[y * src.width as usize * 4..(y + 1) * src.width as usize * 4];
        let mut sum = [0.0f32; 4];
        for dx in -r..=r {
            let x = dx.clamp(0, w - 1) as usize * 4;
            for c in 0..4 {
                sum[c] += row[x + c];
            }
        }
        for x in 0..w {
            let k = x as usize * 4;
            for c in 0..4 {
                out_row[k + c] = sum[c] * inv;
            }
            let x_rem = (x - r).clamp(0, w - 1) as usize * 4;
            let x_add = (x + r + 1).clamp(0, w - 1) as usize * 4;
            for c in 0..4 {
                sum[c] += row[x_add + c] - row[x_rem + c];
            }
        }
    });
}

fn blur_v(src: &Image, dst: &mut Image, r: i32) {
    let w = src.width as usize;
    let h = src.height as i32;
    let inv = 1.0 / (2 * r + 1) as f32;
    dst.data.par_chunks_exact_mut(w * 4).enumerate().for_each(|(y, out_row)| {
        let y0 = y as i32;
        for x in 0..w {
            let mut sum = [0.0f32; 4];
            for dy in -r..=r {
                let yy = (y0 + dy).clamp(0, h - 1) as usize;
                let k = (yy * w + x) * 4;
                for c in 0..4 {
                    sum[c] += src.data[k + c];
                }
            }
            let k = x * 4;
            for c in 0..4 {
                out_row[k + c] = sum[c] * inv;
            }
        }
    });
}

/// Линейное, радиальное или зум-размытие (`DirectionalBlur`, `RadialBlur`, `ZoomBlur`).
/// `kind`: 0 = Linear, 1/2 = Radial/Centered, 3 = Zoom.
pub fn directional_blur(src: &Image, kind: i32, length: f32, angle_deg: f32, center: (f32, f32)) -> Image {
    if length.abs() < 1e-4 {
        return src.clone();
    }
    let (w, h) = (src.width, src.height);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let samples = 11i32;
    let (sin_a, cos_a) = angle_deg.to_radians().sin_cos();
    let mut out = Image::empty(w, h);
    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let (du, dv) = match kind {
                3 => ((u - center.0) * length, (v - center.1) * length), // Zoom
                1 | 2 => (-(v - center.1) * length, (u - center.0) * length), // Radial
                _ => (cos_a * length, sin_a * length), // Linear
            };
            let mut acc = [0.0f32; 4];
            for s in 0..samples {
                let t = (s as f32 / (samples - 1) as f32) - 0.5;
                let c = src.sample_uv(u + du * t, v + dv * t, 2);
                acc[0] += c.r;
                acc[1] += c.g;
                acc[2] += c.b;
                acc[3] += c.a;
            }
            let inv = 1.0 / samples as f32;
            let k = px * 4;
            row[k] = acc[0] * inv;
            row[k + 1] = acc[1] * inv;
            row[k + 2] = acc[2] * inv;
            row[k + 3] = acc[3] * inv;
        }
    });
    out
}

/// Свечение (`Glow`): размытая копия + аддитивное сложение.
pub fn glow_image(src: &Image, glow_size: f32, glow: f32, gain: f32) -> Image {
    let r = (glow_size * src.width as f32 / 1920.0 * 2.0).max(1.0);
    let blurred = box_blur(src, r, r);
    let strength = (glow * gain).max(0.0);
    let mut out = src.clone();
    out.data
        .par_chunks_exact_mut(4)
        .zip(blurred.data.par_chunks_exact(4))
        .for_each(|(dst, b)| {
            dst[0] = (dst[0] + b[0] * strength).min(1.0);
            dst[1] = (dst[1] + b[1] * strength).min(1.0);
            dst[2] = (dst[2] + b[2] * strength).min(1.0);
            dst[3] = (dst[3] + b[3] * strength * 0.5).min(1.0);
        });
    out
}

/// Тень (`DropShadow` / `Shadow`).
pub fn drop_shadow(src: &Image, dx: f32, dy: f32, blur_px: f32, color: Color4) -> Image {
    let shifted = transform_image(src, (0.5 + dx, 0.5 + dy), (0.5, 0.5), 1.0, 1.0, 0.0, false, false, 0);
    let mut shadow = Image::empty(src.width, src.height);
    shadow.data.par_chunks_exact_mut(4).zip(shifted.data.par_chunks_exact(4)).for_each(|(d, s)| {
        let a = s[3] * color.a;
        d[0] = color.r * s[3];
        d[1] = color.g * s[3];
        d[2] = color.b * s[3];
        d[3] = a;
    });
    let shadow = if blur_px > 0.5 { box_blur(&shadow, blur_px, blur_px) } else { shadow };
    merge_images(&shadow, src, (0.5, 0.5), 1.0, 0.0, 1.0, 1.0, "Normal", "Over", None)
}

// ── Displace ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn displace_image(
    src: &Image,
    map: &Image,
    kind: i32,
    x_offset: f32,
    x_refract: f32,
    y_offset: f32,
    y_refract: f32,
    offset: f32,
    refract: f32,
) -> Image {
    let (w, h) = (src.width, src.height);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let mut out = Image::empty(w, h);
    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let k = (py * w as usize + px) * 4;
            let (du, dv) = if kind == 1 {
                // XY displacement
                let mx = map.data[k] + x_offset;
                let my = map.data[k + 1] + y_offset;
                (mx * x_refract, my * y_refract)
            } else {
                // Radial / luminance gradient displacement
                let lum = 0.299 * map.data[k] + 0.587 * map.data[k + 1] + 0.114 * map.data[k + 2];
                let d = (lum + offset) * refract;
                (d, d)
            };
            let c = src.sample_uv(u + du, v + dv, 2);
            let rk = px * 4;
            row[rk] = c.r;
            row[rk + 1] = c.g;
            row[rk + 2] = c.b;
            row[rk + 3] = c.a;
        }
    });
    out
}

// ── BrightnessContrast & ColorCurves & FastNoise ─────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn brightness_contrast(src: &Image, gain: f32, lift: f32, gamma: f32, contrast: f32, brightness: f32, sat: f32) -> Image {
    let mut out = src.clone();
    let inv_gamma = 1.0 / gamma.max(0.05);
    let c_mul = 1.0 + contrast;
    out.data.par_chunks_exact_mut(4).for_each(|px| {
        let a = px[3];
        if a < 1e-5 {
            return;
        }
        let inv_a = 1.0 / a;
        let mut rgb = [px[0] * inv_a, px[1] * inv_a, px[2] * inv_a];
        for v in &mut rgb {
            let mut x = *v * gain + lift * (1.0 - *v) + brightness;
            x = (x - 0.5) * c_mul + 0.5;
            x = x.clamp(0.0, 1.0).powf(inv_gamma);
            *v = x;
        }
        if (sat - 1.0).abs() > 1e-4 {
            let l = 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2];
            for v in &mut rgb {
                *v = (l + (*v - l) * sat).clamp(0.0, 1.0);
            }
        }
        px[0] = rgb[0] * a;
        px[1] = rgb[1] * a;
        px[2] = rgb[2] * a;
    });
    out
}

pub fn apply_color_curves(src: &Image, luts: &[[f32; 256]; 4]) -> Image {
    let mut out = src.clone();
    out.data.par_chunks_exact_mut(4).for_each(|px| {
        let a = px[3];
        if a < 1e-5 {
            return;
        }
        let inv_a = 1.0 / a;
        let na = luts[3][(a.clamp(0.0, 1.0) * 255.0).round() as usize];
        for c in 0..3 {
            let idx = ((px[c] * inv_a).clamp(0.0, 1.0) * 255.0).round() as usize;
            px[c] = luts[c][idx] * na;
        }
        px[3] = na;
    });
    out
}

/// Процедурный fBm-шум `FastNoise`.
pub fn fast_noise(w: u32, h: u32, scale: f32, seethe: f32, detail: i32, c1: Color4, c2: Color4) -> Image {
    let mut out = Image::empty(w, h);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let octs = detail.clamp(1, 5);
    let hash = |ix: i32, iy: i32| -> f32 {
        let n = (ix.wrapping_mul(374761393) ^ iy.wrapping_mul(668265263) ^ (seethe * 1000.0) as i32) as u32;
        let n = (n ^ (n >> 13)).wrapping_mul(1274126177);
        ((n ^ (n >> 16)) & 0xFFFF) as f32 / 65535.0
    };
    let vnoise = |x: f32, y: f32| -> f32 {
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let fx = x - ix as f32;
        let fy = y - iy as f32;
        let ux = fx * fx * (3.0 - 2.0 * fx);
        let uy = fy * fy * (3.0 - 2.0 * fy);
        let a = hash(ix, iy);
        let b = hash(ix + 1, iy);
        let c = hash(ix, iy + 1);
        let d = hash(ix + 1, iy + 1);
        (a + (b - a) * ux) + ((c + (d - c) * ux) - (a + (b - a) * ux)) * uy
    };
    let sc = scale.max(0.5);
    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let (mut val, mut amp, mut freq, mut norm) = (0.0f32, 1.0f32, sc, 0.0f32);
            for _ in 0..octs {
                val += vnoise(u * freq, v * freq) * amp;
                norm += amp;
                amp *= 0.5;
                freq *= 2.0;
            }
            let c = c1.lerp(c2, (val / norm.max(1e-6)).clamp(0.0, 1.0));
            let k = px * 4;
            row[k] = c.r;
            row[k + 1] = c.g;
            row[k + 2] = c.b;
            row[k + 3] = c.a;
        }
    });
    out
}

/// Проективное преобразование `CornerPositioner` (4 угла).
pub fn corner_positioner(src: &Image, bl: (f32, f32), br: (f32, f32), tl: (f32, f32), tr: (f32, f32)) -> Image {
    let (w, h) = (src.width, src.height);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    // Простое билинейное отображение (быстрый устойчивый fallback для 4 углов)
    let mut out = Image::empty(w, h);
    let min_x = bl.0.min(br.0).min(tl.0).min(tr.0);
    let max_x = bl.0.max(br.0).max(tl.0).max(tr.0);
    let min_y = bl.1.min(br.1).min(tl.1).min(tr.1);
    let max_y = bl.1.max(br.1).max(tl.1).max(tr.1);
    let dx = (max_x - min_x).max(1e-4);
    let dy = (max_y - min_y).max(1e-4);
    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let su = (u - min_x) / dx;
            let sv = (v - min_y) / dy;
            let c = src.sample_uv(su, sv, 0);
            let k = px * 4;
            row[k] = c.r;
            row[k + 1] = c.g;
            row[k + 2] = c.b;
            row[k + 3] = c.a;
        }
    });
    out
}

/// Оптическая дисторсия `LensDistort`.
pub fn lens_distort(src: &Image, k: f32, squeeze: f32) -> Image {
    if k.abs() < 1e-4 && (squeeze - 1.0).abs() < 1e-4 {
        return src.clone();
    }
    let (w, h) = (src.width, src.height);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let mut out = Image::empty(w, h);
    let sq = squeeze.clamp(0.25, 4.0);
    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        let dy = v - 0.5;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let dx = (u - 0.5) * sq;
            let r2 = dx * dx + dy * dy;
            let fac = 1.0 + k * r2;
            let c = src.sample_uv(0.5 + dx * fac / sq, 0.5 + dy * fac, 2);
            let rk = px * 4;
            row[rk] = c.r;
            row[rk + 1] = c.g;
            row[rk + 2] = c.b;
            row[rk + 3] = c.a;
        }
    });
    out
}

/// Пикселизация `MosaicBlur`.
pub fn mosaic_blur(src: &Image, freq: f32) -> Image {
    let cells = freq.clamp(2.0, 512.0);
    let (w, h) = (src.width, src.height);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let mut out = Image::empty(w, h);
    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        let qv = ((v * cells).floor() + 0.5) / cells;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let qu = ((u * cells).floor() + 0.5) / cells;
            let c = src.sample_uv(qu, qv, 2);
            let rk = px * 4;
            row[rk] = c.r;
            row[rk + 1] = c.g;
            row[rk + 2] = c.b;
            row[rk + 3] = c.a;
        }
    });
    out
}

/// Волновое искажение `Waviness`.
pub fn waviness(src: &Image, scale: f32, strength: f32, phase: f32, vertical: bool) -> Image {
    if strength.abs() < 1e-4 {
        return src.clone();
    }
    let (w, h) = (src.width, src.height);
    let wf = w.max(1) as f32;
    let hf = h.max(1) as f32;
    let freq = scale.max(0.5) * std::f32::consts::TAU;
    let mut out = Image::empty(w, h);
    out.data.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(py, row)| {
        let v = 1.0 - (py as f32 + 0.5) / hf;
        for px in 0..w as usize {
            let u = (px as f32 + 0.5) / wf;
            let (su, sv) = if vertical {
                (u, v + (u * freq + phase).sin() * strength * 0.05)
            } else {
                (u + (v * freq + phase).sin() * strength * 0.05, v)
            };
            let c = src.sample_uv(su, sv, 2);
            let rk = px * 4;
            row[rk] = c.r;
            row[rk + 1] = c.g;
            row[rk + 2] = c.b;
            row[rk + 3] = c.a;
        }
    });
    out
}

/// Резкость `Sharpen` (unsharp mask).
pub fn sharpen_image(src: &Image, amount: f32) -> Image {
    if amount.abs() < 1e-3 {
        return src.clone();
    }
    let blur = box_blur(src, 1.5, 1.5);
    let mut out = src.clone();
    out.data.par_chunks_exact_mut(4).zip(blur.data.par_chunks_exact(4)).for_each(|(d, b)| {
        for c in 0..3 {
            d[c] = (d[c] + (d[c] - b[c]) * amount).clamp(0.0, 1.0);
        }
    });
    out
}

/// Яркостный кеер `LumaKeyer`.
pub fn luma_keyer(src: &Image, low: f32, high: f32, invert: bool) -> Image {
    let mut out = src.clone();
    let span = (high - low).max(1e-4);
    out.data.par_chunks_exact_mut(4).for_each(|px| {
        let a = px[3];
        if a < 1e-5 {
            return;
        }
        let lum = (0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2]) / a;
        let mut m = ((lum - low) / span).clamp(0.0, 1.0);
        if invert {
            m = 1.0 - m;
        }
        px[0] *= m;
        px[1] *= m;
        px[2] *= m;
        px[3] *= m;
    });
    out
}
