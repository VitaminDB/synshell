//! Кубик со скошенными гранями и фигура из кубиков.

use crate::game::{Material, Shape};
use syngui::core::canvas::CanvasContext;
use syngui::prelude::*;

/// Внешний вид кубика.
#[derive(Clone, Copy, Debug)]
pub struct CubeStyle {
    /// Доля стороны под скос (0.14–0.2 выглядит как в референсе).
    pub bevel: f32,
    /// Смешать все цвета с этим (для «погашенных» фигур и исчезновения).
    pub blend: Option<(Color, f32)>,
    /// Доп. осветление (вспышка при установке/подсветке).
    pub flash: f32,
    /// Материал: меняет рисунок граней.
    pub material: Material,
    /// Зерно узора (сучки дерева, трещины лавы) — обычно позиция клетки.
    pub seed: u32,
    /// Время для анимированных материалов (лава пульсирует, золото блестит).
    pub time: f32,
}

impl Default for CubeStyle {
    fn default() -> Self {
        CubeStyle { bevel: 0.17, blend: None, flash: 0.0, material: Material::Plain, seed: 0, time: 0.0 }
    }
}

impl CubeStyle {
    pub fn with_material(mut self, m: Material, seed: u32, time: f32) -> Self {
        self.material = m;
        self.seed = seed;
        self.time = time;
        self
    }
}

fn hash01(seed: u32, salt: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    (x & 0xFFFF) as f32 / 65535.0
}

impl CubeStyle {
    pub fn dimmed(bg: Color, k: f32) -> Self {
        CubeStyle { blend: Some((bg, k)), ..Default::default() }
    }

    pub fn flash(k: f32) -> Self {
        CubeStyle { flash: k, ..Default::default() }
    }
}

fn tone(base: Color, style: &CubeStyle, light: f32) -> Color {
    let mut c = if light >= 0.0 { base.lighten(light) } else { base.darken(-light) };
    if style.flash > 0.0 {
        c = c.lerp(&Color::WHITE, style.flash);
    }
    if let Some((bg, k)) = style.blend {
        c = c.lerp(&bg, k);
    }
    c
}

/// Нарисовать один кубик в `rect`.
pub fn draw_cube(list: &mut DisplayList, rect: Rect, base: Color, style: &CubeStyle) {
    let w = rect.width();
    let h = rect.height();
    if w < 1.0 || h < 1.0 {
        return;
    }
    let b = (w.min(h) * style.bevel).max(1.5);
    let x0 = rect.origin.x;
    let y0 = rect.origin.y;
    let x1 = x0 + w;
    let y1 = y0 + h;

    let top = tone(base, style, 0.48);
    let left = tone(base, style, 0.18);
    let right = tone(base, style, -0.24);
    let bottom = tone(base, style, -0.46);
    let face_a = tone(base, style, 0.08);
    let face_b = tone(base, style, -0.09);

    let mut cv = CanvasContext::new(Point::new(0.0, 0.0), Size::new(w, h));
    cv.set_anti_alias(0.8);
    cv.set_color(top);
    cv.fill_polygon(&[(x0, y0), (x1, y0), (x1 - b, y0 + b), (x0 + b, y0 + b)]);
    cv.set_color(left);
    cv.fill_polygon(&[(x0, y0), (x0 + b, y0 + b), (x0 + b, y1 - b), (x0, y1)]);
    cv.set_color(right);
    cv.fill_polygon(&[(x1, y0), (x1, y1), (x1 - b, y1 - b), (x1 - b, y0 + b)]);
    cv.set_color(bottom);
    cv.fill_polygon(&[(x0, y1), (x0 + b, y1 - b), (x1 - b, y1 - b), (x1, y1)]);
    cv.flush(list);

    let inner = Rect::new(Point::new(x0 + b, y0 + b), Size::new(w - b * 2.0, h - b * 2.0));
    list.push_gradient_rect(
        inner,
        Gradient::Linear {
            angle_deg: 180.0,
            stops: vec![ColorStop::new(face_a, 0.0), ColorStop::new(face_b, 1.0)],
        },
        [0.0; 4],
    );
    if w > 14.0 {
        // глянец: тонкий блик по верхнему краю грани
        let hl = Rect::new(
            Point::new(inner.origin.x + inner.width() * 0.08, inner.origin.y + inner.height() * 0.07),
            Size::new(inner.width() * 0.84, (inner.height() * 0.09).max(1.0)),
        );
        list.push_rect(hl, tone(base, style, 0.6).with_alpha(0.6), [hl.height() * 0.5; 4]);
        // диагональный отсвет «стекла» в левом верхнем углу грани
        let ix = inner.origin.x;
        let iy = inner.origin.y;
        let iw = inner.width();
        let ih = inner.height();
        let mut cv2 = CanvasContext::new(Point::new(0.0, 0.0), Size::new(w, h));
        cv2.set_anti_alias(0.8);
        cv2.set_color(Color::WHITE.with_alpha(0.10 + style.flash * 0.25));
        cv2.fill_polygon(&[(ix, iy), (ix + iw * 0.55, iy), (ix, iy + ih * 0.55)]);
        cv2.flush(list);
        // яркая кромка сверху и тень снизу — объём
        list.push_rect(Rect::new(Point::new(x0 + b, y0), Size::new(w - b * 2.0, 1.0)), tone(base, style, 0.7).with_alpha(0.7), [0.0; 4]);
        list.push_rect(Rect::new(Point::new(x0 + b, y1 - 1.0), Size::new(w - b * 2.0, 1.0)), Color::BLACK.with_alpha(0.25), [0.0; 4]);
    }
    draw_material(list, inner, base, style);
}

/// Узор материала поверх центральной грани.
fn draw_material(list: &mut DisplayList, inner: Rect, base: Color, style: &CubeStyle) {
    let iw = inner.width();
    let ih = inner.height();
    if iw < 8.0 {
        return;
    }
    let ix = inner.origin.x;
    let iy = inner.origin.y;
    match style.material {
        Material::Plain => {}
        Material::Wood => {
            // волокна: 3 тёмные полосы разной толщины + сучок
            let dark = tone(base, style, -0.35).with_alpha(0.55);
            for k in 0..3 {
                let fy = iy + ih * (0.22 + 0.28 * k as f32 + 0.06 * (hash01(style.seed, k) - 0.5));
                let th = (ih * 0.045).max(1.0) * (0.7 + 0.6 * hash01(style.seed, 10 + k));
                list.push_rect(Rect::new(Point::new(ix + iw * 0.04, fy), Size::new(iw * 0.92, th)), dark, [th; 4]);
            }
            if hash01(style.seed, 99) > 0.55 {
                let kx = ix + iw * (0.25 + 0.5 * hash01(style.seed, 7));
                let ky = iy + ih * (0.3 + 0.4 * hash01(style.seed, 8));
                let kr = iw * 0.11;
                list.push_rect(Rect::new(Point::new(kx - kr, ky - kr * 0.7), Size::new(kr * 2.0, kr * 1.4)), tone(base, style, -0.4).with_alpha(0.7), [kr; 4]);
                list.push_rect(Rect::new(Point::new(kx - kr * 0.45, ky - kr * 0.3), Size::new(kr * 0.9, kr * 0.6)), tone(base, style, -0.15).with_alpha(0.8), [kr; 4]);
            }
        }
        Material::Crystal => {
            // стекло: белая диагональная полоса и мерцающая искра
            let mut cv = CanvasContext::new(Point::new(0.0, 0.0), Size::new(iw, ih));
            cv.set_anti_alias(0.8);
            cv.set_color(Color::WHITE.with_alpha(0.28));
            cv.fill_polygon(&[(ix + iw * 0.55, iy), (ix + iw * 0.8, iy), (ix + iw * 0.25, iy + ih), (ix, iy + ih)]);
            cv.flush(list);
            let tw = ((style.time * 3.0 + hash01(style.seed, 3) * 6.28).sin() * 0.5 + 0.5).powi(3);
            let sx = ix + iw * (0.6 + 0.25 * hash01(style.seed, 4));
            let sy = iy + ih * (0.2 + 0.3 * hash01(style.seed, 5));
            let ss = iw * 0.09 * (0.6 + tw);
            list.push_rect(Rect::new(Point::new(sx - ss, sy - ss * 0.18), Size::new(ss * 2.0, ss * 0.36)), Color::WHITE.with_alpha(0.9 * tw), [ss; 4]);
            list.push_rect(Rect::new(Point::new(sx - ss * 0.18, sy - ss), Size::new(ss * 0.36, ss * 2.0)), Color::WHITE.with_alpha(0.9 * tw), [ss; 4]);
        }
        Material::Gold => {
            // металл: бегущая полоса блеска
            let phase = (style.time * 0.35 + hash01(style.seed, 1) * 0.5) % 1.0;
            let bx = ix - iw * 0.4 + iw * 1.8 * phase;
            let mut cv = CanvasContext::new(Point::new(0.0, 0.0), Size::new(iw, ih));
            cv.set_anti_alias(1.0);
            cv.set_color(Color::WHITE.with_alpha(0.35));
            let bw = iw * 0.22;
            let pts = [(bx, iy), (bx + bw, iy), (bx + bw - iw * 0.35, iy + ih), (bx - iw * 0.35, iy + ih)];
            // клип полосы по грани: простой способ — рисовать только если внутри
            if bx > ix - bw && bx < ix + iw + iw * 0.35 {
                let clipped: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (x.clamp(ix, ix + iw), y)).collect();
                cv.fill_polygon(&clipped);
            }
            cv.flush(list);
            list.push_rect(Rect::new(Point::new(ix + iw * 0.12, iy + ih * 0.55), Size::new(iw * 0.76, (ih * 0.06).max(1.0))), tone(base, style, -0.3).with_alpha(0.35), [2.0; 4]);
        }
        Material::Lava => {
            // трещины с пульсирующим свечением
            let pulse = ((style.time * 2.2 + hash01(style.seed, 2) * 6.28).sin() * 0.5 + 0.5) * 0.5 + 0.5;
            let hot = Color::from_hex("#ff7a2f").lerp(&Color::from_hex("#ffd166"), pulse * 0.5).with_alpha(0.55 + 0.45 * pulse);
            let glow = Color::from_hex("#ff5a1f").with_alpha(0.10 + 0.14 * pulse);
            list.push_rect(inner, glow, [3.0; 4]);
            let w = (iw * 0.06).max(1.2);
            let pts1 = vec![
                [ix + iw * 0.1, iy + ih * (0.3 + 0.2 * hash01(style.seed, 20))],
                [ix + iw * 0.4, iy + ih * (0.5 + 0.15 * hash01(style.seed, 21))],
                [ix + iw * 0.55, iy + ih * (0.25 + 0.2 * hash01(style.seed, 22))],
                [ix + iw * 0.92, iy + ih * (0.45 + 0.2 * hash01(style.seed, 23))],
            ];
            list.push_line_strip(pts1, hot, w);
            let pts2 = vec![
                [ix + iw * (0.4 + 0.2 * hash01(style.seed, 24)), iy + ih * 0.08],
                [ix + iw * (0.45 + 0.15 * hash01(style.seed, 25)), iy + ih * 0.45],
                [ix + iw * (0.3 + 0.3 * hash01(style.seed, 26)), iy + ih * 0.92],
            ];
            list.push_line_strip(pts2, hot.with_alpha(hot.a * 0.8), w * 0.8);
        }
    }
}

/// Плоская «тень»-подложка под кубиком (дёшево: один прямоугольник).
pub fn draw_cube_shadow(list: &mut DisplayList, rect: Rect, alpha: f32, dy: f32) {
    let r = Rect::new(Point::new(rect.origin.x, rect.origin.y + dy), rect.size);
    list.push_rect(r, Color::from_srgb(0, 0, 0, alpha), [rect.width() * 0.12; 4]);
}

/// Фигура из кубиков: `origin` — левый-верх рамки, `cell` — шаг сетки,
/// `gap` — зазор между кубиками, `scale` — масштаб относительно центра рамки.
#[allow(clippy::too_many_arguments)]
pub fn draw_piece(
    list: &mut DisplayList,
    shape: &Shape,
    origin: Point,
    cell: f32,
    gap: f32,
    scale: f32,
    base: Color,
    style: &CubeStyle,
) {
    let cx = origin.x + shape.w as f32 * cell * 0.5;
    let cy = origin.y + shape.h as f32 * cell * 0.5;
    for &(r, c) in shape.cells() {
        let x = origin.x + c as f32 * cell + gap;
        let y = origin.y + r as f32 * cell + gap;
        let s = cell - gap * 2.0;
        // масштаб относительно центра фигуры
        let sx = cx + (x - cx) * scale;
        let sy = cy + (y - cy) * scale;
        draw_cube(list, Rect::new(Point::new(sx, sy), Size::new(s * scale, s * scale)), base, style);
    }
}

/// Тень под всей фигурой (при перетаскивании): два плоских тёмных слоя без
/// размытия — `push_shadow` на GL (Android) сливается в батч со свечением поля
/// и рисуется его цветом, поэтому здесь только обычные прямоугольники.
pub fn draw_piece_shadow(list: &mut DisplayList, shape: &Shape, origin: Point, cell: f32, gap: f32, alpha: f32, dy: f32) {
    let s = cell - gap * 2.0;
    let spread = cell * 0.10;
    for &(r, c) in shape.cells() {
        let x = origin.x + c as f32 * cell + gap;
        let y = origin.y + r as f32 * cell + gap + dy;
        let outer = Rect::new(Point::new(x - spread, y - spread), Size::new(s + spread * 2.0, s + spread * 2.0));
        list.push_rect(outer, Color::from_srgb(0, 0, 0, alpha * 0.35), [s * 0.3; 4]);
        list.push_rect(Rect::new(Point::new(x, y), Size::new(s, s)), Color::from_srgb(0, 0, 0, alpha * 0.6), [s * 0.2; 4]);
    }
}
