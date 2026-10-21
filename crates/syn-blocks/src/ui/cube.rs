//! Кубик со скошенными гранями и фигура из кубиков.
//!
//! Два вида: **Стандарт** — цветные грани, узоры материалов (`draw_material`);
//! **Люкс** — отделка (`finish`): готовый спрайт блока из атласа (форма,
//! свет и тень запечены офлайн), поверх — анимация материала. Спрайты
//! рисуются отдельным проходом (`draw_cubes`), чтобы батчер собрал их в один вызов.

use super::finish::{Anim, Finish, Shade};
use super::finish::hash01;
use crate::game::{Material, Shape};
use syngui::core::canvas::CanvasContext;
use syngui::prelude::*;
use syngui::render::TextureId;

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
    /// Цвет свечения узора (трещины магмы в режиме блоков); `None` — родной.
    pub accent: Option<Color>,
    /// Отделка «Люкса» (текстура + свой характер бликов).
    pub finish: Option<&'static Finish>,
    /// Атлас отделок; `None` — рисовать отделку цветом.
    pub texture: Option<TextureId>,
    /// Бонусная фигура — искра в углу (в «Люксе»).
    pub bonus: bool,
}

impl Default for CubeStyle {
    fn default() -> Self {
        CubeStyle { bevel: 0.17, blend: None, flash: 0.0, material: Material::Plain, seed: 0, time: 0.0, accent: None, finish: None, texture: None, bonus: false }
    }
}

impl CubeStyle {
    pub fn with_material(mut self, m: Material, seed: u32, time: f32) -> Self {
        self.material = m;
        self.seed = seed;
        self.time = time;
        self
    }

    /// Материал, цвет и акцент из `Look` (режимы блоков, отделки).
    pub fn with_look(self, look: &super::world::Look, seed: u32, time: f32) -> Self {
        let mut s = self.with_material(look.material, seed, time);
        s.accent = look.accent;
        s.finish = look.finish;
        s.bonus = look.bonus;
        s
    }

    pub fn with_texture(mut self, t: Option<TextureId>) -> Self {
        self.texture = t;
        self
    }

    pub fn dimmed(bg: Color, k: f32) -> Self {
        CubeStyle { blend: Some((bg, k)), ..Default::default() }
    }

    pub fn flash(k: f32) -> Self {
        CubeStyle { flash: k, ..Default::default() }
    }

    fn textured(&self) -> bool {
        self.finish.is_some() && self.texture.is_some()
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

/// Один кубик для пакетной отрисовки.
#[derive(Clone, Copy, Debug)]
pub struct CubeDraw {
    pub rect: Rect,
    pub base: Color,
    pub style: CubeStyle,
}

/// Нарисовать кубики в два прохода: сначала текстурные квады (один батч),
/// затем скосы, объём и блики всех кубиков.
pub fn draw_cubes(list: &mut DisplayList, draws: &[CubeDraw]) {
    for d in draws {
        if d.style.textured() {
            draw_face_texture(list, d.rect, d.base, &d.style);
        }
    }
    for d in draws {
        if d.style.textured() {
            draw_finish_overlay(list, d.rect, d.base, &d.style, true);
        } else {
            draw_cube(list, d.rect, d.base, &d.style);
        }
    }
}

/// Нарисовать один кубик в `rect` (без пакетирования).
pub fn draw_cube(list: &mut DisplayList, rect: Rect, base: Color, style: &CubeStyle) {
    if style.textured() {
        draw_face_texture(list, rect, base, style);
        draw_finish_overlay(list, rect, base, style, true);
        return;
    }
    if style.finish.is_some() {
        draw_finish_overlay(list, rect, base, style, false);
        return;
    }
    draw_plain_cube(list, rect, base, style);
}

/// Кубик «Стандарта»: 4 грани, глянец, узор материала.
fn draw_plain_cube(list: &mut DisplayList, rect: Rect, base: Color, style: &CubeStyle) {
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

// ── отделки «Люкса» ──────────────────────────────────────────────────────
//
// Блок — готовый спрайт из атласа (форма, свет, тень запечены в
// tools/build_atlas.py): один квад на кубик, у красящихся отделок — второй
// квад с белыми бликами поверх окрашенной базы. Никаких скосов на лету;
// поверх — только вспышка, приглушение, анимация материала и искра бонуса.

/// Спрайт отделки (проход 1).
fn draw_face_texture(list: &mut DisplayList, rect: Rect, base: Color, style: &CubeStyle) {
    let (Some(f), Some(tex)) = (style.finish, style.texture) else { return };
    let mut tint = if f.tint { base.lerp(&Color::WHITE, 0.08) } else { Color::WHITE };
    let mut gloss_a = 1.0;
    if let Some((bg, k)) = style.blend {
        tint = tint.lerp(&bg, k * 0.75);
        gloss_a *= 1.0 - k * 0.8;
    }
    let (uv_base, uv_gloss) = super::finish::uv(f, style.seed);
    let sr = super::finish::sprite_rect(rect);
    list.push_image(sr, tex, uv_base, tint);
    if let Some(g) = uv_gloss {
        list.push_image(sr, tex, g, Color::WHITE.with_alpha(gloss_a));
    }
}

/// Вспышка, приглушение, анимация и искра бонуса (проход 2). При
/// `textured == false` (атласа нет) блок рисуется цветом отделки.
fn draw_finish_overlay(list: &mut DisplayList, rect: Rect, base: Color, style: &CubeStyle, textured: bool) {
    let Some(f) = style.finish else { return };
    let w = rect.width();
    let h = rect.height();
    if w < 1.0 || h < 1.0 {
        return;
    }
    let radius = super::finish::corner(f, rect);
    let white = Color::WHITE;
    let black = Color::BLACK;
    let (x0, y0) = (rect.origin.x, rect.origin.y);

    if !textured {
        // без атласа: скруглённый блок с тенью, объёмом и бликом цветом отделки
        list.push_rect(Rect::new(Point::new(x0, y0 + h * 0.06), rect.size), black.with_alpha(0.35), radius);
        list.push_gradient_rect(
            rect,
            Gradient::Linear { angle_deg: 160.0, stops: vec![ColorStop::new(tone(base, style, 0.22), 0.0), ColorStop::new(tone(base, style, -0.18), 1.0)] },
            radius,
        );
        let hl = Rect::new(Point::new(x0 + w * 0.12, y0 + h * 0.08), Size::new(w * 0.5, (h * 0.16).max(1.0)));
        list.push_rect(hl, white.with_alpha(match f.shade { Shade::Metal | Shade::Glass => 0.45, _ => 0.18 }), [hl.height() * 0.5; 4]);
        list.push_outline(inset_rect(rect, 0.5), white.with_alpha(0.18), 1.0, 0.0, radius);
    }

    // область под анимацию — внутри скруглений
    let ins = (radius[0] * 0.55).max(1.0);
    let inner = Rect::new(Point::new(x0 + ins, y0 + ins), Size::new((w - ins * 2.0).max(1.0), (h - ins * 2.0).max(1.0)));
    let (ix, iy, iw, ih) = (inner.origin.x, inner.origin.y, inner.width(), inner.height());
    let accent = style.accent.unwrap_or(white);

    if w > 12.0 {
        match f.anim {
            Anim::Sheen => {
                // бегущая полоса блеска
                let phase = (style.time * 0.28 + hash01(style.seed, 1)) % 1.0;
                let bx = ix - iw * 0.5 + iw * 2.0 * phase;
                let bw = iw * 0.18;
                if bx > ix - bw && bx < ix + iw + iw * 0.4 {
                    let mut cv = CanvasContext::new(Point::new(0.0, 0.0), Size::new(w, h));
                    cv.set_anti_alias(1.0);
                    cv.set_color(white.with_alpha(0.28));
                    let pts = [(bx, iy), (bx + bw, iy), (bx + bw - iw * 0.4, iy + ih), (bx - iw * 0.4, iy + ih)];
                    let clipped: Vec<(f32, f32)> = pts.iter().map(|&(x, y)| (x.clamp(ix, ix + iw), y)).collect();
                    cv.fill_polygon(&clipped);
                    cv.flush(list);
                }
            }
            Anim::Sparkle => {
                // редкие тонкие искры с мягким ореолом (не на каждом кубике сразу)
                for k in 0..2u32 {
                    let tw = ((style.time * (1.6 + 0.7 * k as f32) + hash01(style.seed, 3 + k) * 6.28).sin() * 0.5 + 0.5).powi(8);
                    if tw < 0.04 {
                        continue;
                    }
                    let sx = ix + iw * (0.15 + 0.7 * hash01(style.seed, 5 + k));
                    let sy = iy + ih * (0.15 + 0.7 * hash01(style.seed, 8 + k));
                    let ss = iw * 0.10 * (0.4 + tw);
                    list.push_rect(Rect::new(Point::new(sx - ss * 0.6, sy - ss * 0.6), Size::new(ss * 1.2, ss * 1.2)), white.with_alpha(0.25 * tw), [ss * 0.6; 4]);
                    list.push_rect(Rect::new(Point::new(sx - ss, sy - ss * 0.08), Size::new(ss * 2.0, ss * 0.16)), white.with_alpha(0.9 * tw), [ss; 4]);
                    list.push_rect(Rect::new(Point::new(sx - ss * 0.08, sy - ss), Size::new(ss * 0.16, ss * 2.0)), white.with_alpha(0.9 * tw), [ss; 4]);
                }
            }
            Anim::Pulse => {
                // пульс свечения: кромка и лёгкая заливка цветом акцента
                let pulse = ((style.time * 2.2 + hash01(style.seed, 2) * 6.28).sin() * 0.5 + 0.5) * 0.5 + 0.5;
                let strong = f.shade == Shade::Glow;
                list.push_outline(inset_rect(rect, 1.0), accent.with_alpha((if strong { 0.35 } else { 0.15 }) + 0.35 * pulse), (w * 0.045).max(1.0), 0.0, radius);
                list.push_rect(rect, accent.with_alpha((if strong { 0.04 } else { 0.03 }) + 0.08 * pulse), radius);
            }
            Anim::Iridescent => {
                let t = style.time * 0.25 + hash01(style.seed, 4);
                let hue = |o: f32| {
                    let a = (t + o) * std::f32::consts::TAU;
                    Color::rgb(0.5 + 0.5 * a.sin(), 0.5 + 0.5 * (a + 2.09).sin(), 0.5 + 0.5 * (a + 4.19).sin())
                };
                list.push_gradient_rect(
                    rect,
                    Gradient::Linear {
                        angle_deg: 35.0 + 20.0 * (t * 3.0).sin(),
                        stops: vec![ColorStop::new(hue(0.0).with_alpha(0.20), 0.0), ColorStop::new(hue(0.33).with_alpha(0.20), 0.5), ColorStop::new(hue(0.66).with_alpha(0.20), 1.0)],
                    },
                    radius,
                );
            }
            Anim::None => {}
        }
        // бонусная фигура: искра в углу
        if style.bonus {
            let tw = ((style.time * 3.0 + hash01(style.seed, 11) * 6.28).sin() * 0.5 + 0.5).powi(2);
            let sx = ix + iw * 0.8;
            let sy = iy + ih * 0.2;
            let ss = iw * 0.13 * (0.7 + 0.5 * tw);
            let gold = Color::from_hex("#ffe27a");
            list.push_rect(Rect::new(Point::new(sx - ss * 1.1, sy - ss * 1.1), Size::new(ss * 2.2, ss * 2.2)), gold.with_alpha(0.25 + 0.2 * tw), [ss * 1.1; 4]);
            list.push_rect(Rect::new(Point::new(sx - ss, sy - ss * 0.2), Size::new(ss * 2.0, ss * 0.4)), white.with_alpha(0.85 + 0.15 * tw), [ss; 4]);
            list.push_rect(Rect::new(Point::new(sx - ss * 0.2, sy - ss), Size::new(ss * 0.4, ss * 2.0)), white.with_alpha(0.85 + 0.15 * tw), [ss; 4]);
        }
    }

    // вспышка и приглушение — по силуэту
    if style.flash > 0.001 {
        list.push_rect(rect, white.with_alpha((style.flash * 0.7).min(1.0)), radius);
    }
    if let Some((bg, k)) = style.blend {
        if k > 0.001 && !textured {
            list.push_rect(rect, bg.with_alpha((k * 0.9).min(1.0)), radius);
        }
    }
}

fn inset_rect(r: Rect, d: f32) -> Rect {
    Rect::new(Point::new(r.origin.x + d, r.origin.y + d), Size::new((r.width() - d * 2.0).max(0.0), (r.height() - d * 2.0).max(0.0)))
}

/// Узор материала поверх центральной грани («Стандарт»).
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
            if style.accent.is_some() {
                // режим «металл»: шлифовка и заклёпки по углам
                let lh = (ih * 0.025).max(0.7);
                for k in 0..5 {
                    let y = iy + ih * (0.12 + 0.18 * k as f32);
                    let col = if k % 2 == 0 { Color::WHITE.with_alpha(0.16) } else { Color::BLACK.with_alpha(0.12) };
                    list.push_rect(Rect::new(Point::new(ix + iw * 0.05, y), Size::new(iw * 0.9, lh)), col, [0.0; 4]);
                }
                let rr = (iw * 0.07).max(1.2);
                for (fx, fy) in [(0.14, 0.14), (0.86, 0.14), (0.14, 0.86), (0.86, 0.86)] {
                    let (x, y) = (ix + iw * fx, iy + ih * fy);
                    list.push_rect(Rect::new(Point::new(x - rr, y - rr), Size::new(rr * 2.0, rr * 2.0)), tone(base, style, -0.45), [rr; 4]);
                    list.push_rect(Rect::new(Point::new(x - rr * 0.7, y - rr * 0.8), Size::new(rr * 1.2, rr * 1.2)), tone(base, style, 0.55), [rr; 4]);
                }
            }
        }
        Material::Lava => {
            // трещины с пульсирующим свечением
            let pulse = ((style.time * 2.2 + hash01(style.seed, 2) * 6.28).sin() * 0.5 + 0.5) * 0.5 + 0.5;
            let (hot, glow0) = match style.accent {
                Some(a) => (a.lerp(&Color::WHITE, pulse * 0.2), a),
                None => (Color::from_hex("#ff7a2f").lerp(&Color::from_hex("#ffd166"), pulse * 0.5), Color::from_hex("#ff5a1f")),
            };
            let hot = hot.with_alpha(0.55 + 0.45 * pulse);
            let glow = glow0.with_alpha(if style.accent.is_some() { 0.04 + 0.06 * pulse } else { 0.10 + 0.14 * pulse });
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

/// Прямоугольники кубиков фигуры: `origin` — левый-верх рамки, `cell` — шаг
/// сетки, `gap` — зазор, `scale` — масштаб относительно центра рамки.
pub fn piece_rects(shape: &Shape, origin: Point, cell: f32, gap: f32, scale: f32) -> Vec<Rect> {
    let cx = origin.x + shape.w as f32 * cell * 0.5;
    let cy = origin.y + shape.h as f32 * cell * 0.5;
    shape
        .cells()
        .iter()
        .map(|&(r, c)| {
            let x = origin.x + c as f32 * cell + gap;
            let y = origin.y + r as f32 * cell + gap;
            let s = cell - gap * 2.0;
            let sx = cx + (x - cx) * scale;
            let sy = cy + (y - cy) * scale;
            Rect::new(Point::new(sx, sy), Size::new(s * scale, s * scale))
        })
        .collect()
}

/// Фигура из кубиков (в два прохода, если есть текстура).
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
    let draws: Vec<CubeDraw> = piece_rects(shape, origin, cell, gap, scale)
        .into_iter()
        .enumerate()
        .map(|(i, rect)| CubeDraw { rect, base, style: CubeStyle { seed: style.seed.wrapping_add(i as u32 * 31), ..*style } })
        .collect();
    draw_cubes(list, &draws);
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
