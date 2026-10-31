//! Растр страницы на процессоре (tiny-skia): бумага, шаблон, слои с
//! непрозрачностью, штрихи (объединение отпечатков одной заливкой —
//! без двойного наложения у полупрозрачных), фигуры, картинки.
//!
//! Один и тот же код рисует окно холста ([`render_region`]) — по частям,
//! только изменённую область — и экспорт PNG ([`export_png`]).

use std::collections::HashMap;

use tiny_skia::{
    BlendMode, Color as SkColor, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect as SkRect,
    Stroke as SkStroke, Transform,
};

use super::brush::{self, Dab};
use super::model::{BBox, Brush, ImageItem, InkDoc, InkPage, Item, Rgba, Shape, ShapeKind, Stroke, Template, P2};

/// Как рисовать.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterOpts {
    /// Сглаживание краёв. На E Ink в ч/б режиме без него чётче.
    pub anti_alias: bool,
    /// Рисовать бумагу и шаблон (в оверлее — нет).
    pub paper: bool,
    /// Цвет шаблонных линий.
    pub rule_color: Rgba,
}

impl Default for RasterOpts {
    fn default() -> Self {
        Self { anti_alias: true, paper: true, rule_color: Rgba([150, 160, 175, 255]) }
    }
}

/// Раскодированные картинки (ключ ассета → pixmap).
#[derive(Default)]
pub struct ImageCache {
    map: HashMap<String, Option<Pixmap>>,
}

impl ImageCache {
    pub fn get(&mut self, doc: &InkDoc, key: &str) -> Option<&Pixmap> {
        if !self.map.contains_key(key) {
            let pm = doc.asset(key).and_then(|a| decode(&a.bytes));
            self.map.insert(key.to_string(), pm);
        }
        self.map.get(key).and_then(|o| o.as_ref())
    }
}

/// Байты картинки → pixmap (премультиплицированный).
pub fn decode(bytes: &[u8]) -> Option<Pixmap> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut pm = Pixmap::new(w, h)?;
    for (dst, src) in pm.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(pm)
}

pub(crate) fn sk(c: Rgba) -> SkColor {
    SkColor::from_rgba8(c.0[0], c.0[1], c.0[2], c.0[3])
}

fn paint(c: Rgba, aa: bool) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(sk(c));
    p.anti_alias = aa;
    p
}

/// Нарисовать область `region` страницы (единицы страницы) в `pm`, где
/// пиксель (0,0) — точка `region.x0,y0`, масштаб `scale` px на единицу.
/// `skip` — элементы, которые сейчас рисует кто-то другой (лассо тащит).
pub fn render_region(
    pm: &mut Pixmap,
    doc: &InkDoc,
    page: &InkPage,
    region: BBox,
    scale: f32,
    opts: &RasterOpts,
    images: &mut ImageCache,
    skip: &dyn Fn(u64) -> bool,
) {
    let tf = Transform::from_row(scale, 0.0, 0.0, scale, -region.x0 * scale, -region.y0 * scale);
    if opts.paper {
        pm.fill(sk(page.paper.unwrap_or(Rgba::WHITE)));
        draw_template(pm, doc, page, region, tf, scale, opts, images);
    } else {
        pm.fill(SkColor::TRANSPARENT);
    }
    let cull = region.inflate(2.0);
    for layer in page.layers.iter().filter(|l| l.visible) {
        let visible: Vec<&Item> =
            layer.items.iter().filter(|i| !skip(i.id()) && i.bbox().intersects(&cull)).collect();
        if visible.is_empty() {
            continue;
        }
        if layer.opacity < 0.999 {
            let Some(mut tmp) = Pixmap::new(pm.width(), pm.height()) else { continue };
            draw_items(&mut tmp, doc, &visible, tf, scale, opts.anti_alias, images);
            let pp = PixmapPaint { opacity: layer.opacity.clamp(0.0, 1.0), ..PixmapPaint::default() };
            pm.draw_pixmap(0, 0, tmp.as_ref(), &pp, Transform::identity(), None);
        } else {
            draw_items(pm, doc, &visible, tf, scale, opts.anti_alias, images);
        }
    }
}

/// Элементы слоя: сначала выделители (они «под текстом»), потом остальное.
fn draw_items(pm: &mut Pixmap, doc: &InkDoc, items: &[&Item], tf: Transform, scale: f32, aa: bool, images: &mut ImageCache) {
    for pass in 0..2 {
        for it in items {
            let hl = matches!(it, Item::Stroke(s) if s.brush == Brush::Highlighter);
            if (pass == 0) != hl {
                continue;
            }
            draw_item(pm, doc, it, tf, scale, aa, images);
        }
    }
}

pub fn draw_item(pm: &mut Pixmap, doc: &InkDoc, it: &Item, tf: Transform, scale: f32, aa: bool, images: &mut ImageCache) {
    match it {
        Item::Stroke(s) => draw_stroke(pm, s, tf, scale, aa),
        Item::Shape(s) => draw_shape(pm, s, tf, aa),
        Item::Image(i) => draw_image(pm, doc, i, tf, images),
        // Текст рисует холст поверх растра (шрифты — у syngui).
        Item::Text(_) => {}
    }
}

/// Штрих: все отпечатки и перемычки одной заливкой «ненулевым» правилом.
pub fn draw_stroke(pm: &mut Pixmap, s: &Stroke, tf: Transform, scale: f32, aa: bool) {
    let dabs = brush::dabs(s);
    if dabs.is_empty() {
        return;
    }
    // Карандаш: тон меняется по штриху — рисуем кусками одного тона.
    if s.brush == Brush::Pencil {
        draw_dabs_toned(pm, &dabs, tf, scale, aa);
        return;
    }
    let Some(path) = dabs_path(&dabs, scale) else { return };
    let mut p = paint(dabs[0].color, aa);
    p.blend_mode = BlendMode::SourceOver;
    pm.fill_path(&path, &p, FillRule::Winding, tf, None);
}

/// Путь-объединение отпечатков.
pub fn dabs_path(dabs: &[Dab], scale: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let mut add_poly = |pts: &[P2]| {
        if let Some(f) = pts.first() {
            pb.move_to(f.x, f.y);
            for q in &pts[1..] {
                pb.line_to(q.x, q.y);
            }
            pb.close();
        }
    };
    // Круг на каждом отпечатке: между перемычками соседних отрезков на
    // изгибе остаётся клин, видный тонкой щелью у широких кистей.
    let n = dabs.len();
    for i in 0..n {
        add_poly(&brush::circle(dabs[i].p, dabs[i].r, brush::circle_segments(dabs[i].r, scale)));
        if i + 1 < n {
            match brush::bridge(&dabs[i], &dabs[i + 1]) {
                Some(q) => add_poly(&q),
                // Один круг внутри другого — перемычки нет, нужен больший круг.
                None => {
                    let d = if dabs[i].r >= dabs[i + 1].r { &dabs[i] } else { &dabs[i + 1] };
                    add_poly(&brush::circle(d.p, d.r, brush::circle_segments(d.r, scale)));
                }
            }
        }
    }
    pb.finish()
}

fn draw_dabs_toned(pm: &mut Pixmap, dabs: &[Dab], tf: Transform, scale: f32, aa: bool) {
    let mut start = 0;
    while start < dabs.len() {
        let c = dabs[start].color;
        let mut end = start + 1;
        while end < dabs.len() && tone_close(dabs[end].color, c) {
            end += 1;
        }
        // Куски перекрываются на один отпечаток — без щелей.
        let from = start.saturating_sub(1);
        let to = (end + 1).min(dabs.len());
        if let Some(path) = dabs_path(&dabs[from..to], scale) {
            pm.fill_path(&path, &paint(c, aa), FillRule::Winding, tf, None);
        }
        start = end;
    }
}

fn tone_close(a: Rgba, b: Rgba) -> bool {
    (0..3).all(|i| (a.0[i] as i32 - b.0[i] as i32).abs() <= 6)
}

pub fn draw_shape(pm: &mut Pixmap, s: &Shape, tf: Transform, aa: bool) {
    let outline = s.outline();
    if outline.len() < 2 {
        return;
    }
    let closed = s.kind.closed();
    let mut pb = PathBuilder::new();
    pb.move_to(outline[0].x, outline[0].y);
    for p in &outline[1..] {
        pb.line_to(p.x, p.y);
    }
    if closed {
        pb.close();
    }
    let Some(path) = pb.finish() else { return };
    if let Some(fill) = s.fill.filter(|_| closed) {
        pm.fill_path(&path, &paint(fill, aa), FillRule::Winding, tf, None);
    }
    let st = SkStroke { width: s.width.max(0.3), line_cap: LineCap::Round, line_join: LineJoin::Round, ..SkStroke::default() };
    pm.stroke_path(&path, &paint(s.color, aa), &st, tf, None);
    if s.kind == ShapeKind::Arrow && outline.len() >= 2 {
        let head = arrow_head(outline[outline.len() - 2], outline[outline.len() - 1], s.width);
        let mut pb = PathBuilder::new();
        pb.move_to(head[0].x, head[0].y);
        pb.line_to(head[1].x, head[1].y);
        pb.line_to(head[2].x, head[2].y);
        pb.close();
        if let Some(path) = pb.finish() {
            pm.fill_path(&path, &paint(s.color, aa), FillRule::Winding, tf, None);
        }
    }
}

/// Наконечник стрелки: кончик и два крыла.
pub fn arrow_head(from: P2, to: P2, width: f32) -> [P2; 3] {
    let d = to.sub(from);
    let len = d.len().max(1e-4);
    let u = d.scale(1.0 / len);
    let size = (width * 4.0).max(10.0);
    let base = to.sub(u.scale(size));
    let n = P2::new(-u.y, u.x).scale(size * 0.5);
    [to, base.add(n), base.sub(n)]
}

fn draw_image(pm: &mut Pixmap, doc: &InkDoc, im: &ImageItem, tf: Transform, images: &mut ImageCache) {
    let Some(src) = images.get(doc, &im.asset) else { return };
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    if sw < 1.0 || sh < 1.0 {
        return;
    }
    let c = P2::new(im.x + im.w / 2.0, im.y + im.h / 2.0);
    // Картинка: масштаб в w×h, поворот вокруг центра, потом вид.
    let t = Transform::from_scale(im.w / sw, im.h / sh)
        .post_translate(im.x, im.y)
        .post_concat(Transform::from_rotate_at(im.angle.to_degrees(), c.x, c.y))
        .post_concat(tf);
    let pp = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
    pm.draw_pixmap(0, 0, src.as_ref(), &pp, t, None);
}

#[allow(clippy::too_many_arguments)]
fn draw_template(
    pm: &mut Pixmap,
    doc: &InkDoc,
    page: &InkPage,
    region: BBox,
    tf: Transform,
    scale: f32,
    opts: &RasterOpts,
    images: &mut ImageCache,
) {
    let (pw, ph) = page.dims().unwrap_or((region.x1.max(1.0), region.y1.max(1.0)));
    let infinite = page.size.is_infinite();
    // Область линий: у страницы — сама страница, у холста — видимое.
    let area = if infinite { region } else { BBox::new(0.0, 0.0, pw, ph) };
    let line_w = (1.0 / scale).max(0.6).max(1.0 / scale);
    // Линии шаблона — без сглаживания: ровно в пиксель (и tiny-skia не
    // любит сглаженные прямоугольники тоньше пикселя).
    let mut fill = paint(opts.rule_color, false);
    // Линия — целыми пикселями: начало в пиксель, толщина не меньше пикселя.
    let snap = |v: f32, o: f32| ((v - o) * scale).floor() / scale + o;
    let thick = |w: f32| ((w * scale).round().max(1.0)) / scale;
    let hline = |pm: &mut Pixmap, y: f32, x0: f32, x1: f32, w: f32, p: &Paint| {
        let w = thick(w);
        if let Some(r) = SkRect::from_xywh(x0, snap(y, region.y0), x1 - x0, w) {
            pm.fill_rect(r, p, tf, None);
        }
    };
    let vline = |pm: &mut Pixmap, x: f32, y0: f32, y1: f32, w: f32, p: &Paint| {
        let w = thick(w);
        if let Some(r) = SkRect::from_xywh(snap(x, region.x0), y0, w, y1 - y0) {
            pm.fill_rect(r, p, tf, None);
        }
    };
    let first = |from: f32, step: f32| (from / step).floor() * step;
    match &page.template {
        Template::Blank => {}
        Template::Lined { step } | Template::Cornell { step } => {
            let step = step.max(8.0);
            let top = if infinite { first(region.y0, step) } else { step * 3.0 };
            let mut y = top.max(if infinite { f32::MIN } else { step * 3.0 });
            while y < area.y1.min(region.y1 + step) {
                if y >= region.y0 - step {
                    hline(pm, y, area.x0, area.x1, line_w, &fill);
                }
                y += step;
            }
            if !infinite {
                let margin = if matches!(page.template, Template::Cornell { .. }) { pw * 0.3 } else { step * 2.5 };
                let red = paint(Rgba([220, 120, 120, 255]), false);
                vline(pm, margin, 0.0, ph, line_w * 1.2, &red);
                if matches!(page.template, Template::Cornell { .. }) {
                    let bottom = ph * 0.78;
                    if let Some(r) = SkRect::from_xywh(0.0, bottom, pw, ph - bottom) {
                        pm.fill_rect(r, &paint(page.paper.unwrap_or(Rgba::WHITE), false), tf, None);
                    }
                    hline(pm, bottom, 0.0, pw, line_w * 1.6, &fill);
                }
            }
        }
        Template::Grid { step } => {
            let step = step.max(6.0);
            let mut y = first(area.y0.max(region.y0), step);
            while y <= area.y1.min(region.y1) {
                hline(pm, y, area.x0.max(region.x0), area.x1.min(region.x1), line_w, &fill);
                y += step;
            }
            let mut x = first(area.x0.max(region.x0), step);
            while x <= area.x1.min(region.x1) {
                vline(pm, x, area.y0.max(region.y0), area.y1.min(region.y1), line_w, &fill);
                x += step;
            }
        }
        Template::Dots { step } => {
            let step = step.max(6.0);
            let r = (1.1 / scale).max(0.9);
            let mut y = first(area.y0.max(region.y0), step);
            while y <= area.y1.min(region.y1) {
                let mut x = first(area.x0.max(region.x0), step);
                while x <= area.x1.min(region.x1) {
                    if let Some(rc) = SkRect::from_xywh(x - r, y - r, r * 2.0, r * 2.0) {
                        pm.fill_rect(rc, &fill, tf, None);
                    }
                    x += step;
                }
                y += step;
            }
        }
        Template::Music { step } => {
            let step = step.max(4.0);
            fill.set_color(sk(Rgba([110, 115, 125, 255])));
            let group = step * 4.0 + step * 5.0; // стан + промежуток
            let mut top = if infinite { first(region.y0, group) } else { step * 8.0 };
            while top < area.y1.min(region.y1 + group) {
                for k in 0..5 {
                    let y = top + k as f32 * step;
                    if !infinite && y > ph - step * 4.0 {
                        break;
                    }
                    hline(pm, y, area.x0 + if infinite { 0.0 } else { step * 4.0 }, area.x1 - if infinite { 0.0 } else { step * 4.0 }, line_w, &fill);
                }
                top += group;
            }
        }
        Template::Image { asset } => {
            if let Some(src) = images.get(doc, asset) {
                let (sw, sh) = (src.width() as f32, src.height() as f32);
                let t = Transform::from_scale(pw / sw.max(1.0), ph / sh.max(1.0)).post_concat(tf);
                let pp = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
                pm.draw_pixmap(0, 0, src.as_ref(), &pp, t, None);
            }
        }
    }
}

/// Премультиплицированный pixmap → обычный RGBA (для текстуры и PNG).
pub fn demultiply_into(pm: &Pixmap, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(pm.data().len());
    for px in pm.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
}

/// Скопировать прямоугольник `src` в `dst` в точку (x, y) с заменой пикселей.
pub fn blit(dst: &mut Pixmap, src: &Pixmap, x: i32, y: i32) {
    let pp = PixmapPaint { blend_mode: BlendMode::Source, ..PixmapPaint::default() };
    dst.draw_pixmap(x, y, src.as_ref(), &pp, Transform::identity(), None);
}

/// Страница целиком в PNG (`scale` px на единицу). У бесконечного холста —
/// всё нарисованное с полем 24 единицы.
pub fn export_png(doc: &InkDoc, page: usize, scale: f32, opts: &RasterOpts) -> Option<Vec<u8>> {
    let pm = render_page(doc, page, scale, opts)?;
    pm.encode_png().ok()
}

/// Страница целиком в pixmap.
pub fn render_page(doc: &InkDoc, page: usize, scale: f32, opts: &RasterOpts) -> Option<Pixmap> {
    let pg = doc.pages.get(page)?;
    let region = page_extent(pg);
    let w = (region.w() * scale).ceil().max(1.0) as u32;
    let h = (region.h() * scale).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(w.min(16384), h.min(16384))?;
    let mut images = ImageCache::default();
    render_region(&mut pm, doc, pg, region, scale, opts, &mut images, &|_| false);
    Some(pm)
}

/// Что занимает страница: лист или, у холста, нарисованное с полем.
pub fn page_extent(pg: &InkPage) -> BBox {
    match pg.dims() {
        Some((w, h)) => BBox::new(0.0, 0.0, w, h),
        None => {
            let b = pg.bbox();
            if b.is_empty() {
                BBox::new(0.0, 0.0, 800.0, 600.0)
            } else {
                b.inflate(24.0)
            }
        }
    }
}

/// Слои без бумаги и шаблона, прозрачный фон: для заливки и пипетки.
pub fn render_ink_only(doc: &InkDoc, page: &InkPage, region: BBox, scale: f32) -> Option<Pixmap> {
    let w = (region.w() * scale).ceil().max(1.0) as u32;
    let h = (region.h() * scale).ceil().max(1.0) as u32;
    let mut pm = Pixmap::new(w.min(8192), h.min(8192))?;
    let opts = RasterOpts { paper: false, anti_alias: true, ..RasterOpts::default() };
    let mut images = ImageCache::default();
    render_region(&mut pm, doc, page, region, scale, &opts, &mut images, &|_| false);
    Some(pm)
}
