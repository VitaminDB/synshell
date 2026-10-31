//! Сетка для ГПУ: живой штрих под пером, предпросмотр лассо и фигур.
//! Готовые элементы живут в растре страницы ([`super::raster`]); здесь —
//! только то, что меняется на каждое событие пера, поэтому строится заново
//! целиком (это дёшево: один штрих).
//!
//! Координаты — единицы страницы; вид (масштаб, сдвиг) задаёт
//! `push_transform` холста. Края сглажены «бахромой» шириной `feather`
//! (в единицах страницы — пиксель экрана, делённый на масштаб).

use syngui::core::Color;
use syngui::render::Vertex;

use super::brush::{self, Dab};
use super::model::{Item, Rgba, Shape, ShapeKind, Stroke, P2};

const ZERO2: [f32; 2] = [0.0, 0.0];
const ZERO4: [f32; 4] = [0.0; 4];

#[derive(Default, Clone, Debug)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    fn v(&mut self, p: P2, c: [f32; 4]) -> u32 {
        self.vertices.push(Vertex::new([p.x, p.y], ZERO2, c, ZERO4));
        (self.vertices.len() - 1) as u32
    }

    /// Круг с бахромой.
    pub fn disc(&mut self, c: P2, r: f32, color: [f32; 4], feather: f32, px: f32) {
        let seg = brush::circle_segments(r, px);
        let clear = [color[0], color[1], color[2], 0.0];
        let center = self.v(c, color);
        let base = self.vertices.len() as u32;
        for i in 0..seg {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            let (s, co) = a.sin_cos();
            self.v(P2::new(c.x + co * r, c.y + s * r), color);
            self.v(P2::new(c.x + co * (r + feather), c.y + s * (r + feather)), clear);
        }
        for i in 0..seg as u32 {
            let j = (i + 1) % seg as u32;
            let (ri, fi, rj, fj) = (base + i * 2, base + i * 2 + 1, base + j * 2, base + j * 2 + 1);
            self.indices.extend_from_slice(&[center, ri, rj, ri, fi, fj, ri, fj, rj]);
        }
    }

    /// Перемычка между отпечатками с бахромой по длинным сторонам.
    pub fn bridge(&mut self, a: &Dab, b: &Dab, ca: [f32; 4], cb: [f32; 4], feather: f32) {
        let d = b.p.sub(a.p);
        let len = d.len();
        if len < 1e-4 {
            return;
        }
        let n = P2::new(-d.y / len, d.x / len);
        let clear = |c: [f32; 4]| [c[0], c[1], c[2], 0.0];
        let i0 = self.v(a.p.add(n.scale(a.r + feather)), clear(ca));
        let i1 = self.v(a.p.add(n.scale(a.r)), ca);
        let i2 = self.v(a.p.sub(n.scale(a.r)), ca);
        let i3 = self.v(a.p.sub(n.scale(a.r + feather)), clear(ca));
        let j0 = self.v(b.p.add(n.scale(b.r + feather)), clear(cb));
        let j1 = self.v(b.p.add(n.scale(b.r)), cb);
        let j2 = self.v(b.p.sub(n.scale(b.r)), cb);
        let j3 = self.v(b.p.sub(n.scale(b.r + feather)), clear(cb));
        self.indices.extend_from_slice(&[i0, j0, i1, i1, j0, j1, i1, j1, i2, i2, j1, j2, i2, j2, i3, i3, j2, j3]);
    }

    /// Ломаная постоянной ширины (фигуры, рамка лассо).
    pub fn polyline(&mut self, pts: &[P2], closed: bool, width: f32, color: [f32; 4], feather: f32, px: f32) {
        if pts.len() < 2 {
            return;
        }
        let r = width / 2.0;
        let dab = |p: P2| Dab { p, r, color: Rgba::BLACK };
        let n = pts.len();
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            self.bridge(&dab(a), &dab(b), color, color, feather);
        }
        for p in pts {
            self.disc(*p, r, color, feather, px);
        }
    }

    /// Пунктир (рамка лассо, выделение).
    pub fn dashed(&mut self, pts: &[P2], closed: bool, width: f32, dash: f32, color: [f32; 4], feather: f32) {
        let n = pts.len();
        if n < 2 {
            return;
        }
        let r = width / 2.0;
        let dab = |p: P2| Dab { p, r, color: Rgba::BLACK };
        let segs = if closed { n } else { n - 1 };
        let mut phase = 0.0f32;
        for i in 0..segs {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let len = a.dist(b);
            let mut t = 0.0;
            while t < len {
                let on = (phase / dash) as i64 % 2 == 0;
                let step = (dash - phase % dash).min(len - t);
                if on {
                    let p0 = a.lerp(b, t / len);
                    let p1 = a.lerp(b, (t + step) / len);
                    self.bridge(&dab(p0), &dab(p1), color, color, feather);
                }
                t += step;
                phase += step;
            }
        }
    }

    /// Заливка выпуклого/звёздного многоугольника веером из центра.
    pub fn fill_fan(&mut self, pts: &[P2], color: [f32; 4]) {
        if pts.len() < 3 {
            return;
        }
        let c = pts.iter().fold(P2::new(0.0, 0.0), |s, p| s.add(*p)).scale(1.0 / pts.len() as f32);
        let ci = self.v(c, color);
        let base = self.vertices.len() as u32;
        for p in pts {
            self.v(*p, color);
        }
        let n = pts.len() as u32;
        for i in 0..n {
            self.indices.extend_from_slice(&[ci, base + i, base + (i + 1) % n]);
        }
    }
}

pub fn color_of(c: Rgba) -> [f32; 4] {
    Color::from_srgb(c.0[0], c.0[1], c.0[2], c.alpha()).to_array()
}

/// Отпечатки → сетка.
pub fn dabs_mesh(out: &mut Mesh, dabs: &[Dab], feather: f32, px: f32) {
    let n = dabs.len();
    for i in 0..n {
        let c = color_of(dabs[i].color);
        out.disc(dabs[i].p, dabs[i].r, c, feather, px);
        if i + 1 < n {
            out.bridge(&dabs[i], &dabs[i + 1], c, color_of(dabs[i + 1].color), feather);
        }
    }
}

pub fn stroke_mesh(out: &mut Mesh, s: &Stroke, feather: f32, px: f32) {
    dabs_mesh(out, &brush::dabs(s), feather, px);
}

pub fn shape_mesh(out: &mut Mesh, s: &Shape, feather: f32, px: f32) {
    let outline = s.outline();
    if let Some(f) = s.fill.filter(|_| s.kind.closed()) {
        out.fill_fan(&outline, color_of(f));
    }
    out.polyline(&outline, s.kind.closed(), s.width.max(0.5), color_of(s.color), feather, px);
    if s.kind == ShapeKind::Arrow && outline.len() >= 2 {
        let head = super::raster::arrow_head(outline[outline.len() - 2], outline[outline.len() - 1], s.width);
        out.fill_fan(&head, color_of(s.color));
    }
}

/// Элемент целиком (кроме текста и картинок — их рисует холст сам).
pub fn item_mesh(out: &mut Mesh, it: &Item, feather: f32, px: f32) {
    match it {
        Item::Stroke(s) => stroke_mesh(out, s, feather, px),
        Item::Shape(s) => shape_mesh(out, s, feather, px),
        Item::Image(_) | Item::Text(_) => {}
    }
}
