//! Серверные рамки окон: заголовок с кнопками и тень.
//!
//! Растеризуются на CPU (tiny-skia — фигуры, fontdue — текст) в
//! `MemoryRenderBuffer`, который композитор выводит как текстуру. Заголовок
//! перерисовывается только при смене ключа (ширина, текст, фокус, наведение,
//! масштаб, палитра); тень — одна текстура на всё, режется на 9 частей.

use std::sync::Arc;

use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::{Logical, Point, Rectangle, Transform},
};
use synshell_common::config::{Appearance, Decorations, Rgba};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform as SkTransform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Close,
    Maximize,
    Minimize,
    Sticky,
    Above,
    Icon,
}

impl Button {
    fn parse(s: &str) -> Option<Button> {
        Some(match s.trim() {
            "close" | "X" => Button::Close,
            "maximize" | "A" => Button::Maximize,
            "minimize" | "I" => Button::Minimize,
            "sticky" | "S" | "on_all_desktops" => Button::Sticky,
            "above" | "keep_above" | "F" => Button::Above,
            "icon" | "menu" | "M" => Button::Icon,
            _ => return None,
        })
    }
}

/// Что под указателем в области рамки.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoHit {
    Title,
    Button(Button),
}

/// Тема рамок — производная от конфига, общая для всех окон.
#[derive(Debug, Clone)]
pub struct DecoTheme {
    pub height: i32,
    pub font_size: f32,
    pub center_title: bool,
    pub left: Vec<Button>,
    pub right: Vec<Button>,
    pub radius: f32,
    pub active_bg: Rgba,
    pub inactive_bg: Rgba,
    pub active_fg: Rgba,
    pub inactive_fg: Rgba,
    pub accent: Rgba,
    pub danger: Rgba,
    pub border: Rgba,
    pub shadow: bool,
    pub shadow_size: i32,
    pub shadow_opacity: f32,
    /// Сквозной номер темы: меняется при перезагрузке конфига, чтобы
    /// кэши заголовков перерисовались.
    pub generation: u64,
}

impl DecoTheme {
    pub fn from_config(deco: &Decorations, appearance: &Appearance, generation: u64) -> Self {
        let p = appearance.palette();
        let (active_bg, inactive_bg) = appearance.titlebar_colors(deco);
        let (left, right) = match deco.buttons.split_once(':') {
            Some((l, r)) => (l, r),
            None => ("", deco.buttons.as_str()),
        };
        let parse_list = |s: &str| s.split(',').filter_map(Button::parse).collect::<Vec<_>>();
        let fg_for = |bg: Rgba| if bg.luminance() > 0.55 { Rgba::rgb(0x1d, 0x20, 0x26) } else { Rgba::rgb(0xec, 0xee, 0xf2) };
        Self {
            height: deco.title_height.clamp(16, 96),
            font_size: deco.font_size.clamp(6.0, 48.0),
            center_title: deco.title_align != "left",
            left: parse_list(left),
            right: parse_list(right),
            radius: deco.corner_radius.max(0.0),
            active_bg,
            inactive_bg,
            active_fg: fg_for(active_bg),
            inactive_fg: fg_for(inactive_bg).mix(inactive_bg, 0.35),
            accent: p.accent,
            danger: p.danger,
            border: p.border,
            shadow: deco.shadow,
            shadow_size: deco.shadow_size.clamp(0, 128),
            shadow_opacity: deco.shadow_opacity.clamp(0.0, 1.0),
            generation,
        }
    }

    /// Прямоугольники кнопок в логических координатах относительно левого
    /// верхнего угла заголовка шириной `width`.
    pub fn button_rects(&self, width: i32) -> Vec<(Button, Rectangle<i32, Logical>)> {
        let h = self.height;
        let size = h;
        let mut out = Vec::new();
        let mut x = 4;
        for b in &self.left {
            out.push((*b, Rectangle::new((x, 0).into(), (size, h).into())));
            x += size;
        }
        let mut x = width - 4;
        for b in self.right.iter().rev() {
            x -= size;
            out.push((*b, Rectangle::new((x, 0).into(), (size, h).into())));
        }
        out
    }

    pub fn hit(&self, width: i32, pos: Point<f64, Logical>) -> Option<DecoHit> {
        if pos.x < 0.0 || pos.y < 0.0 || pos.x >= width as f64 || pos.y >= self.height as f64 {
            return None;
        }
        for (b, r) in self.button_rects(width) {
            if r.to_f64().contains(pos) {
                return Some(DecoHit::Button(b));
            }
        }
        Some(DecoHit::Title)
    }
}

// ─── Шрифт ─────────────────────────────────────────────────────────────────

pub struct TitleFont {
    font: Option<fontdue::Font>,
}

impl TitleFont {
    /// Шрифт через `fc-match` (семейство из appearance.font или системный).
    pub fn load(family: &str) -> Self {
        // Вес по шкале fontconfig: medium = 100 (не CSS-овские 500).
        let pattern = if family.trim().is_empty() { "sans-serif:medium".to_string() } else { format!("{family}:medium") };
        let path = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", &pattern])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty());
        let candidates = path.into_iter().chain(
            [
                "/usr/share/fonts/noto/NotoSans-Medium.ttf",
                "/usr/share/fonts/noto/NotoSans-Regular.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
            ]
            .into_iter()
            .map(String::from),
        );
        for p in candidates {
            if let Ok(bytes) = std::fs::read(&p) {
                match fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()) {
                    Ok(font) => {
                        tracing::info!(path = p, "шрифт заголовков окон");
                        return Self { font: Some(font) };
                    }
                    Err(e) => tracing::warn!(path = p, e, "шрифт не читается"),
                }
            }
        }
        tracing::warn!("не найден шрифт для заголовков окон — заголовки без текста");
        Self { font: None }
    }

    /// Ширина строки в пикселях.
    fn measure(&self, text: &str, px: f32) -> f32 {
        let Some(font) = &self.font else { return 0.0 };
        text.chars().map(|c| font.metrics(c, px).advance_width).sum()
    }

    /// Нарисовать строку (белой маской с цветом `color`) в `pixmap` от
    /// базовой линии `baseline` начиная с `x`; обрезка по `max_x`.
    fn draw(&self, pixmap: &mut Pixmap, text: &str, px: f32, x: f32, baseline: f32, max_x: f32, color: Rgba) {
        let Some(font) = &self.font else { return };
        let w = pixmap.width() as i32;
        let h = pixmap.height() as i32;
        let data = pixmap.data_mut();
        let mut pen = x;
        for c in text.chars() {
            let (m, bitmap) = font.rasterize(c, px);
            let gx = (pen + m.xmin as f32).round() as i32;
            let gy = (baseline - m.ymin as f32 - m.height as f32).round() as i32;
            for row in 0..m.height as i32 {
                let py = gy + row;
                if py < 0 || py >= h {
                    continue;
                }
                for col in 0..m.width as i32 {
                    let pxx = gx + col;
                    if pxx < 0 || pxx >= w || pxx as f32 >= max_x {
                        continue;
                    }
                    let cov = bitmap[(row * m.width as i32 + col) as usize] as f32 / 255.0;
                    if cov <= 0.0 {
                        continue;
                    }
                    let a = cov * color.a as f32 / 255.0;
                    let idx = ((py * w + pxx) * 4) as usize;
                    // Премультиплицированное RGBA: src over dst.
                    let src = [color.r as f32 * a, color.g as f32 * a, color.b as f32 * a, 255.0 * a];
                    for k in 0..4 {
                        let d = data[idx + k] as f32;
                        data[idx + k] = (src[k] + d * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            pen += m.advance_width;
        }
    }
}

// ─── Заголовок ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct TitleKey {
    pub width: i32,
    pub scale: i32,
    pub title: String,
    pub app_id: String,
    pub focused: bool,
    pub maximized: bool,
    pub sticky: bool,
    pub above: bool,
    pub hover: Option<Button>,
    pub pressed: Option<Button>,
    pub generation: u64,
}

/// Заголовок одного окна: ключ последней отрисовки + буфер.
#[derive(Default)]
pub struct TitleBar {
    key: Option<TitleKey>,
    buffer: Option<MemoryRenderBuffer>,
}

impl TitleBar {
    pub fn buffer(&mut self, key: TitleKey, theme: &DecoTheme, font: &TitleFont) -> &MemoryRenderBuffer {
        if self.key.as_ref() != Some(&key) || self.buffer.is_none() {
            let pixmap = render_title(&key, theme, font);
            let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
            self.buffer = Some(MemoryRenderBuffer::from_slice(
                pixmap.data(),
                Fourcc::Abgr8888,
                (w, h),
                key.scale,
                Transform::Normal,
                None,
            ));
            self.key = Some(key);
        }
        self.buffer.as_ref().unwrap()
    }
}

fn color(c: Rgba) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(c));
    p.anti_alias = true;
    p
}

fn render_title(key: &TitleKey, theme: &DecoTheme, font: &TitleFont) -> Pixmap {
    let s = key.scale.max(1) as f32;
    let w = (key.width.max(1) as f32 * s).round() as u32;
    let h = (theme.height as f32 * s).round() as u32;
    let mut pm = Pixmap::new(w.max(1), h.max(1)).expect("pixmap");
    let bg = if key.focused { theme.active_bg } else { theme.inactive_bg };
    let fg = if key.focused { theme.active_fg } else { theme.inactive_fg };

    // Фон с верхними скруглениями (у развёрнутого — прямой).
    let r = if key.maximized { 0.0 } else { (theme.radius * s).min(h as f32 / 2.0) };
    let (wf, hf) = (w as f32, h as f32);
    let mut pb = PathBuilder::new();
    pb.move_to(0.0, hf);
    pb.line_to(0.0, r);
    if r > 0.0 {
        pb.quad_to(0.0, 0.0, r, 0.0);
    }
    pb.line_to(wf - r, 0.0);
    if r > 0.0 {
        pb.quad_to(wf, 0.0, wf, r);
    }
    pb.line_to(wf, hf);
    pb.close();
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint(bg), FillRule::Winding, SkTransform::identity(), None);
        // Тонкая светлая кромка сверху — объём, как у Breeze.
        let mut edge = paint(fg.with_alpha(0.07));
        edge.anti_alias = true;
        let stroke = Stroke { width: s, ..Default::default() };
        pm.stroke_path(&path, &edge, &stroke, SkTransform::identity(), None);
    }

    // Кнопки.
    let rects = theme.button_rects(key.width);
    let mut title_left = 10.0 * s;
    let mut title_right = wf - 10.0 * s;
    for (b, rect) in &rects {
        let rx = rect.loc.x as f32 * s;
        let rw = rect.size.w as f32 * s;
        if rx < wf / 2.0 {
            title_left = title_left.max(rx + rw + 4.0 * s);
        } else {
            title_right = title_right.min(rx - 4.0 * s);
        }
        draw_button(&mut pm, *b, rx, rw, hf, s, key, theme, fg, font);
    }

    // Текст заголовка.
    let px = theme.font_size * s;
    let avail = (title_right - title_left).max(0.0);
    let mut text = key.title.clone();
    if font.measure(&text, px) > avail {
        while !text.is_empty() && font.measure(&format!("{text}…"), px) > avail {
            text.pop();
        }
        text.push('…');
    }
    let tw = font.measure(&text, px);
    let x = if theme.center_title {
        ((wf - tw) / 2.0).clamp(title_left, (title_right - tw).max(title_left))
    } else {
        title_left
    };
    let baseline = hf / 2.0 + px * 0.36;
    font.draw(&mut pm, &text, px, x, baseline, title_right, fg);
    pm
}

#[allow(clippy::too_many_arguments)]
fn draw_button(
    pm: &mut Pixmap,
    b: Button,
    x: f32,
    w: f32,
    h: f32,
    s: f32,
    key: &TitleKey,
    theme: &DecoTheme,
    fg: Rgba,
    font: &TitleFont,
) {
    let cx = x + w / 2.0;
    let cy = h / 2.0;
    let hovered = key.hover == Some(b);
    let pressed = key.pressed == Some(b);
    let radius = (h * 0.30).min(11.0 * s);

    if b == Button::Icon {
        // Значок приложения: буква в кружке цвета акцента.
        let letter = key
            .app_id
            .rsplit('.')
            .next()
            .and_then(|s| s.chars().next())
            .or_else(|| key.title.chars().next())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        if let Some(circle) = PathBuilder::from_circle(cx, cy, radius * 1.05) {
            pm.fill_path(&circle, &paint(theme.accent), FillRule::Winding, SkTransform::identity(), None);
        }
        let px = radius * 1.3;
        let tw = font.measure(&letter, px);
        font.draw(pm, &letter, px, cx - tw / 2.0, cy + px * 0.36, f32::MAX, theme.accent.contrast_fg());
        return;
    }

    let active_toggle = (b == Button::Sticky && key.sticky) || (b == Button::Above && key.above);
    let circle_color = if b == Button::Close && (hovered || pressed) {
        Some(theme.danger.mix(Rgba::rgb(0, 0, 0), if pressed { 0.2 } else { 0.0 }))
    } else if pressed {
        Some(fg.with_alpha(0.28))
    } else if hovered || active_toggle {
        Some(fg.with_alpha(0.16))
    } else {
        None
    };
    if let (Some(c), Some(circle)) = (circle_color, PathBuilder::from_circle(cx, cy, radius)) {
        pm.fill_path(&circle, &paint(c), FillRule::Winding, SkTransform::identity(), None);
    }
    let glyph = if b == Button::Close && (hovered || pressed) { Rgba::rgb(255, 255, 255) } else { fg };
    let mut p = paint(glyph);
    p.anti_alias = true;
    let stroke = Stroke { width: 1.35 * s, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
    let g = radius * 0.42;
    let mut pb = PathBuilder::new();
    match b {
        Button::Close => {
            pb.move_to(cx - g, cy - g);
            pb.line_to(cx + g, cy + g);
            pb.move_to(cx + g, cy - g);
            pb.line_to(cx - g, cy + g);
        }
        Button::Minimize => {
            pb.move_to(cx - g, cy - g * 0.35);
            pb.line_to(cx, cy + g * 0.55);
            pb.line_to(cx + g, cy - g * 0.35);
        }
        Button::Maximize => {
            if key.maximized {
                // «Восстановить» — ромб.
                pb.move_to(cx, cy - g);
                pb.line_to(cx + g, cy);
                pb.line_to(cx, cy + g);
                pb.line_to(cx - g, cy);
                pb.close();
            } else {
                pb.move_to(cx - g, cy + g * 0.35);
                pb.line_to(cx, cy - g * 0.55);
                pb.line_to(cx + g, cy + g * 0.35);
            }
        }
        Button::Sticky => {
            if let Some(c) = PathBuilder::from_circle(cx, cy, g * 0.45) {
                pm.fill_path(&c, &p, FillRule::Winding, SkTransform::identity(), None);
            }
            return;
        }
        Button::Above => {
            pb.move_to(cx - g, cy + g * 0.2);
            pb.line_to(cx, cy - g * 0.7);
            pb.line_to(cx + g, cy + g * 0.2);
            pb.move_to(cx - g, cy + g * 0.8);
            pb.line_to(cx + g, cy + g * 0.8);
        }
        Button::Icon => {}
    }
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &p, &stroke, SkTransform::identity(), None);
    }
}

// ─── Тень ──────────────────────────────────────────────────────────────────

/// Тень: текстура размытого скруглённого прямоугольника, который режется на
/// 9 частей (углы — как есть, стороны и центр растягиваются).
pub struct Shadow {
    key: Option<(i32, i32, u32, i32)>,
    pub buffer: Option<MemoryRenderBuffer>,
    /// Размер угловой части (логические px) = size + radius.
    pub corner: i32,
    /// Отступ тени за пределы окна.
    pub size: i32,
}

impl Default for Shadow {
    fn default() -> Self {
        Self { key: None, buffer: None, corner: 0, size: 0 }
    }
}

impl Shadow {
    pub fn update(&mut self, theme: &DecoTheme, radius: i32, scale: i32) {
        let key = (theme.shadow_size, radius, (theme.shadow_opacity * 1000.0) as u32, scale);
        if self.key == Some(key) && self.buffer.is_some() {
            return;
        }
        self.key = Some(key);
        self.size = theme.shadow_size;
        self.corner = theme.shadow_size + radius.max(1);
        let s = scale.max(1);
        let size = theme.shadow_size * s;
        let corner = self.corner * s;
        // Текстура: 2 угла + 1 пиксель середины.
        let dim = (corner * 2 + s) as usize;
        let mut alpha = vec![0f32; dim * dim];
        // Скруглённый прямоугольник «окна» внутри текстуры.
        let inset = size as f32;
        let r = (radius * s) as f32;
        let (x0, y0, x1, y1) = (inset, inset, dim as f32 - inset, dim as f32 - inset);
        for y in 0..dim {
            for x in 0..dim {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let dx = (x0 + r - px).max(px - (x1 - r)).max(0.0);
                let dy = (y0 + r - py).max(py - (y1 - r)).max(0.0);
                let inside = if dx > 0.0 && dy > 0.0 {
                    (dx * dx + dy * dy).sqrt() <= r
                } else {
                    px >= x0 && px <= x1 && py >= y0 && py <= y1
                };
                if inside {
                    alpha[y * dim + x] = 1.0;
                }
            }
        }
        // Три прохода box blur ≈ гауссово размытие.
        let radius_blur = ((size as f32) / 2.2).max(1.0) as usize;
        for _ in 0..3 {
            box_blur(&mut alpha, dim, radius_blur);
        }
        let opacity = theme.shadow_opacity;
        let mut rgba = vec![0u8; dim * dim * 4];
        for (i, a) in alpha.iter().enumerate() {
            // Смещение тени вниз: верхняя кромка светлее.
            let y = i / dim;
            let bias = if (y as f32) < dim as f32 / 2.0 { 0.75 } else { 1.0 };
            let v = (a * opacity * bias * 255.0).round().clamp(0.0, 255.0) as u8;
            rgba[i * 4 + 3] = v; // чёрный премультиплицированный: RGB = 0
        }
        self.buffer = Some(MemoryRenderBuffer::from_slice(
            &rgba,
            Fourcc::Abgr8888,
            (dim as i32, dim as i32),
            s,
            Transform::Normal,
            None,
        ));
    }
}

fn box_blur(data: &mut [f32], dim: usize, r: usize) {
    let mut tmp = vec![0f32; data.len()];
    let norm = 1.0 / (2 * r + 1) as f32;
    // Горизонталь.
    for y in 0..dim {
        let row = &data[y * dim..(y + 1) * dim];
        let mut acc: f32 = (0..=r).map(|i| row[i.min(dim - 1)]).sum::<f32>() + row[0] * r as f32;
        for x in 0..dim {
            tmp[y * dim + x] = acc * norm;
            let add = row[(x + r + 1).min(dim - 1)];
            let sub = row[x.saturating_sub(r)];
            acc += add - sub;
        }
    }
    // Вертикаль.
    for x in 0..dim {
        let mut acc: f32 = (0..=r).map(|i| tmp[i.min(dim - 1) * dim + x]).sum::<f32>() + tmp[x] * r as f32;
        for y in 0..dim {
            data[y * dim + x] = acc * norm;
            let add = tmp[(y + r + 1).min(dim - 1) * dim + x];
            let sub = tmp[y.saturating_sub(r) * dim + x];
            acc += add - sub;
        }
    }
}

/// Общий шрифт заголовков (перезагружается при смене `appearance.font`).
pub type SharedFont = Arc<TitleFont>;
