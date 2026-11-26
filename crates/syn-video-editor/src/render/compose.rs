//! Кадр таймлайна — дерево виджетов syngui. Одно и то же дерево рисуется в
//! панели «Просмотр» (масштаб под панель) и при экспорте (`Offscreen`, масштаб
//! 1), поэтому просмотр совпадает с результатом.
//!
//! Слои — видеодорожки снизу вверх; у клипа: кадр источника (или титр, цвет),
//! затухания, положение и масштаб (`transform`), цвет и размытие (`filter`).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};
use syngui::containers::Positioned;
use syngui::mss::StyleValue;
use syngui::prelude::*;

use crate::doc::{Clip, Effect, Project, Source, TrackKind};

/// Кадр источника клипа (уже с LUT).
pub type Frames = HashMap<u64, syngui::video::RgbaFrame>;

/// Слой кадра: клип, его время на таймлайне (на переходе — зажатое в границы
/// клипа) и переход (вид, доля 0–1, входящий ли клип).
pub struct Layer<'a> {
    pub clip: &'a Clip,
    pub t: f64,
    pub tr: Option<(&'a str, f64, bool)>,
}

/// Все виды переходов: (вид, подпись).
pub const TRANSITIONS: &[(&str, &str)] = &[
    ("dissolve", "Наплыв"),
    ("dip_black", "Через чёрное"),
    ("slide_left", "Сдвиг влево"),
    ("slide_right", "Сдвиг вправо"),
    ("slide_up", "Сдвиг вверх"),
    ("slide_down", "Сдвиг вниз"),
    ("wipe_left", "Шторка слева"),
    ("wipe_right", "Шторка справа"),
    ("zoom", "Зум"),
    ("blur", "Размытие"),
];

/// Видимые клипы видеодорожек в момент `t`, снизу вверх; на переходе — оба
/// клипа склейки (уходящий, затем входящий).
pub fn layers(p: &Project, t: f64) -> Vec<Layer<'_>> {
    let mut out = Vec::new();
    for tr in p.tracks.iter().filter(|tr| tr.kind == TrackKind::Video && !tr.hidden) {
        let clips = &tr.clips;
        let mut done = false;
        for (i, c) in clips.iter().enumerate() {
            let Some(trn) = &c.transition else { continue };
            let d = trn.duration.max(1e-3);
            let (w0, w1) = (c.start - d / 2.0, c.start + d / 2.0);
            if t < w0 || t >= w1 {
                continue;
            }
            let Some(prev) = i.checked_sub(1).map(|j| &clips[j]).filter(|pr| (pr.end() - c.start).abs() < 1e-3) else { continue };
            let q = ((t - w0) / d).clamp(0.0, 1.0);
            out.push(Layer { clip: prev, t: t.min(prev.end() - 1e-3).max(prev.start), tr: Some((trn.kind.as_str(), q, false)) });
            out.push(Layer { clip: c, t: t.max(c.start), tr: Some((trn.kind.as_str(), q, true)) });
            done = true;
            break;
        }
        if !done {
            if let Some(c) = clips.iter().find(|c| c.start <= t + 1e-9 && t < c.end() - 1e-9) {
                out.push(Layer { clip: c, t, tr: None });
            }
        }
    }
    out
}

/// Как переход меняет слой: множитель непрозрачности, сдвиг (доли кадра),
/// множитель масштаба, размытие (пикс. при масштабе 1), видимая полоса по x (доли).
struct TrFx {
    alpha: f64,
    dx: f32,
    dy: f32,
    k: f32,
    blur: f32,
    clip: Option<(f32, f32)>,
}

fn tr_fx(kind: &str, q: f64, incoming: bool) -> TrFx {
    let e = (q * q * (3.0 - 2.0 * q)) as f32; // плавный вход и выход
    let mut f = TrFx { alpha: 1.0, dx: 0.0, dy: 0.0, k: 1.0, blur: 0.0, clip: None };
    match (kind, incoming) {
        ("dissolve", true) => f.alpha = e as f64,
        ("dip_black", false) => f.alpha = (1.0 - 2.0 * q).max(0.0),
        ("dip_black", true) => f.alpha = (2.0 * q - 1.0).max(0.0),
        ("slide_left", false) => f.dx = -e,
        ("slide_left", true) => f.dx = 1.0 - e,
        ("slide_right", false) => f.dx = e,
        ("slide_right", true) => f.dx = e - 1.0,
        ("slide_up", false) => f.dy = -e,
        ("slide_up", true) => f.dy = 1.0 - e,
        ("slide_down", false) => f.dy = e,
        ("slide_down", true) => f.dy = e - 1.0,
        ("wipe_left", true) => f.clip = Some((0.0, e)),
        ("wipe_right", true) => f.clip = Some((1.0 - e, 1.0)),
        ("zoom", false) => {
            f.k = 1.0 + e * 0.6;
            f.alpha = 1.0 - e as f64;
        }
        ("zoom", true) => {
            f.k = 0.7 + 0.3 * e;
            f.alpha = e as f64;
        }
        ("blur", false) => {
            f.blur = e * 24.0;
            f.alpha = 1.0 - e as f64;
        }
        ("blur", true) => {
            f.blur = (1.0 - e) * 24.0;
            f.alpha = e as f64;
        }
        _ => {}
    }
    f
}

fn num(m: &Map<String, Value>, k: &str, d: f64) -> f64 {
    m.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(d)
}

fn effect<'a>(c: &'a Clip, kind: &str) -> Option<&'a Effect> {
    c.effects.iter().find(|e| e.kind == kind)
}

/// Непрозрачность от затуханий в локальное время клипа `lt`.
pub fn fade(c: &Clip, lt: f64, prefix: &str) -> f64 {
    let mut a: f64 = 1.0;
    if let Some(e) = effect(c, &format!("{prefix}fade_in")) {
        let d = num(&e.params, "duration", 0.5).max(1e-3);
        a = a.min(lt / d);
    }
    if let Some(e) = effect(c, &format!("{prefix}fade_out")) {
        let d = num(&e.params, "duration", 0.5).max(1e-3);
        a = a.min((c.duration - lt) / d);
    }
    a.clamp(0.0, 1.0)
}

/// Цепочка `filter` из эффектов цвета, размытия, виньетки.
fn filters(c: &Clip, scale: f32) -> String {
    let mut f = Vec::new();
    // с графом кадра эффекты изображения считает граф (FrameSource)
    if c.graph.is_some() {
        return String::new();
    }
    if let Some(e) = effect(c, "color") {
        let p = &e.params;
        let b = num(p, "brightness", 0.0);
        let k = num(p, "contrast", 0.0);
        let s = num(p, "saturation", 0.0);
        let t = num(p, "temperature", 0.0);
        if b != 0.0 {
            f.push(format!("brightness({:.3})", 1.0 + b));
        }
        if k != 0.0 {
            f.push(format!("contrast({:.3})", 1.0 + k));
        }
        if s != 0.0 {
            f.push(format!("saturate({:.3})", (1.0 + s).max(0.0)));
        }
        if t != 0.0 {
            // тёплый — к оранжевому, холодный — к синему
            f.push(format!("hue-rotate({:.1}deg)", -t * 12.0));
        }
    }
    if let Some(e) = effect(c, "blur") {
        f.push(format!("blur({:.1}px)", num(&e.params, "radius", 6.0) as f32 * scale));
    }
    if let Some(e) = effect(c, "vignette") {
        f.push(format!("vignette({:.2})", num(&e.params, "amount", 0.5)));
    }
    f.join(" ")
}

/// Итоговая трансформация слоя с учётом цепочки родителей (`Clip::parent`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldXf {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub rot: f32,
    pub opacity: f64,
}

/// Трансформация клипа `c` (уже вычисленного через `Clip::at`) в момент таймлайна `t`
/// с учётом всех предков по `parent`.
pub fn world_transform(p: &Project, c: &Clip, t: f64) -> WorldXf {
    world_transform_rec(p, c, t, 0)
}

fn world_transform_rec(p: &Project, c: &Clip, t: f64, depth: usize) -> WorldXf {
    let (mut x, mut y, mut k, mut rot, mut opacity) = (0.0f32, 0.0f32, 1.0f32, 0.0f32, 1.0f64);
    if let Some(e) = effect(c, "transform") {
        let pm = &e.params;
        x = num(pm, "x", 0.0) as f32;
        y = num(pm, "y", 0.0) as f32;
        k = num(pm, "scale", 1.0) as f32;
        rot = num(pm, "rotation", 0.0) as f32;
        opacity = num(pm, "opacity", 1.0);
    }
    if depth < 16 {
        if let Some(pid) = c.parent {
            if let Some(pc) = p.clip(pid) {
                let plt = (t - pc.start).clamp(0.0, pc.duration);
                let pe = pc.at(plt);
                let par = world_transform_rec(p, &pe, t, depth + 1);
                let (pw, ph) = (p.settings.width.max(1) as f32, p.settings.height.max(1) as f32);
                let (s, cs) = par.rot.to_radians().sin_cos();
                let lx = x * pw * par.scale;
                let ly = y * ph * par.scale;
                let wx = par.x * pw + (lx * cs - ly * s);
                let wy = par.y * ph + (lx * s + ly * cs);
                x = wx / pw;
                y = wy / ph;
                k *= par.scale;
                rot += par.rot;
                opacity *= par.opacity;
            }
        }
    }
    WorldXf { x, y, scale: k, rot, opacity }
}

fn blend_type(s: &str) -> syngui::effects::BlendModeType {
    use syngui::effects::BlendModeType as B;
    match s {
        "multiply" => B::Multiply,
        "screen" => B::Screen,
        "overlay" => B::Overlay,
        "add" => B::Add,
        "darken" => B::Darken,
        "lighten" => B::Lighten,
        "soft_light" => B::SoftLight,
        "hard_light" => B::HardLight,
        "difference" => B::Difference,
        _ => B::Normal,
    }
}

/// Параметры эффекта матового стекла (`effect:glass`).
#[derive(Clone, Debug)]
struct GlassParams {
    blur: f32,
    tint: Option<String>,
    tint_opacity: f32,
    border_opacity: f32,
    highlight: f32,
    shadow: f32,
    saturation: f32,
    brightness: f32,
}

fn glass_params(c: &Clip) -> Option<GlassParams> {
    let e = effect(c, "glass")?;
    let pm = &e.params;
    let tint = pm
        .get("tint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("none"))
        .map(str::to_string);
    Some(GlassParams {
        blur: num(pm, "blur", 18.0).clamp(0.0, 80.0) as f32,
        tint,
        tint_opacity: num(pm, "tint_opacity", 0.18).clamp(0.0, 1.0) as f32,
        border_opacity: num(pm, "border_opacity", 0.48).clamp(0.0, 1.0) as f32,
        highlight: num(pm, "highlight", 0.30).clamp(0.0, 1.0) as f32,
        shadow: num(pm, "shadow", 0.28).clamp(0.0, 1.0) as f32,
        saturation: num(pm, "saturation", 1.20).clamp(0.0, 2.5) as f32,
        brightness: num(pm, "brightness", 0.04).clamp(-0.5, 0.5) as f32,
    })
}

/// GPU-эффекты слоя `syngui`: маска (`effect:mask`) и режим наложения (`Clip::blend`).
fn layer_gpu_effect(c: &Clip) -> Option<syngui::effects::Effect> {
    use syngui::effects::{BlendModeType, Effect as GpuEffect};
    let mut chain: Vec<GpuEffect> = Vec::new();
    if let Some(e) = effect(c, "mask") {
        let pm = &e.params;
        let ellipse = pm.get("shape").and_then(Value::as_str).unwrap_or("rect") == "ellipse";
        let cx = num(pm, "x", 0.0) as f32;
        let cy = num(pm, "y", 0.0) as f32;
        let mw = num(pm, "width", 0.6).clamp(0.02, 2.0) as f32;
        let mh = num(pm, "height", 0.6).clamp(0.02, 2.0) as f32;
        let radius = num(pm, "radius", 0.0).clamp(0.0, 0.5) as f32;
        let feather = num(pm, "feather", 0.0).clamp(0.0, 0.5) as f32;
        let invert = pm.get("invert").and_then(Value::as_bool).unwrap_or(false);
        chain.push(GpuEffect::ShapeMask { ellipse, center: [cx, cy], size: [mw, mh], radius, feather, invert });
    }
    let bm = blend_type(&c.blend);
    if bm != BlendModeType::Normal {
        chain.push(GpuEffect::BlendMode { mode: bm });
    }
    match chain.len() {
        0 => None,
        1 => Some(chain.remove(0)),
        _ => Some(GpuEffect::Chain(chain)),
    }
}

/// Множитель кадра медиа: `contain` — целиком в кадре, `cover` — заполнить кадр.
pub fn fit_k(c: &Clip, kw: f32, kh: f32) -> f32 {
    if c.fit == "cover" {
        kw.max(kh)
    } else {
        kw.min(kh)
    }
}

/// Кадр в момент `t` размером `w × h` (масштаб `scale` от размера проекта).
pub fn frame_widget(p: &Project, t: f64, frames: &Frames, scale: f32) -> Box<dyn Widget> {
    let w = p.settings.width as f32 * scale;
    let h = p.settings.height as f32 * scale;
    let mut stack = Stack::new().clip(true);
    let bg_str = p.settings.bg.trim();
    if let Some(bg_col) = if bg_str.is_empty() { Some(Color::from_srgb(0, 0, 0, 1.0)) } else { parse_col(bg_str, 1.0) } {
        stack = stack.child(Positioned::new(DecoratedBox::new().style("background-color", bg_col)).at(0.0, 0.0).dimensions(w, h));
    }
    for l in layers(p, t) {
        // Переход Fusion (C5): оба клипа сведены в один готовый кадр в `frames[c.id]`
        if let Some(("fusion", _, incoming)) = l.tr {
            if !incoming {
                continue;
            }
            if let Some(f) = frames.get(&l.clip.id) {
                let ptr = Arc::as_ptr(&f.rgba) as usize;
                let img = Image::from_rgba_shared(format!("ftr{}:{:.4}:{ptr:x}", l.clip.id, f.pts), f.width, f.height, f.rgba.clone())
                    .fit(ImageFit::Fill)
                    .placeholder(false);
                stack = stack.child(Positioned::new(img).at(0.0, 0.0).dimensions(w, h));
                continue;
            }
        }
        let lt = l.t - l.clip.start;
        // ключевые кадры — значения на этот момент
        let ce = l.clip.at(lt);
        let c: &Clip = &ce;
        let wxf = world_transform(p, c, l.t);
        let mut alpha = fade(c, lt, "") * wxf.opacity;
        // положение и масштаб: x, y — доли кадра от центра, scale, rotation, opacity
        let (mut x, mut y, mut k, rot) = (wxf.x, wxf.y, wxf.scale, wxf.rot);
        let fx = l.tr.map(|(kind, q, inc)| tr_fx(kind, q, inc));
        let mut filter = filters(c, scale);
        if let Some(f) = &fx {
            alpha *= f.alpha;
            x += f.dx;
            y += f.dy;
            k *= f.k;
            if f.blur > 0.0 {
                filter = format!("{filter} blur({:.1}px)", f.blur * scale).trim().to_string();
            }
        }
        let gpu_fx = layer_gpu_effect(c);
        let glass = glass_params(c);
        let layer: Box<dyn Widget> = match &c.source {
            Source::Media { .. } => {
                let Some(f) = frames.get(&c.id) else { continue };
                // вписать кадр (contain) в кадр проекта или заполнить его (cover)
                let fk = fit_k(c, w / f.width as f32, h / f.height as f32) * k;
                let (iw, ih) = (f.width as f32 * fk, f.height as f32 * fk);
                // адрес буфера в ключе: обработанный кадр (коррекция, LUT, граф) — новая текстура,
                // иначе после правки коррекции просмотр показывал прежнюю из кэша
                let ptr = Arc::as_ptr(&f.rgba) as usize;
                let img = Image::from_rgba_shared(format!("clip{}:{:.4}:{ptr:x}", c.id, f.pts), f.width, f.height, f.rgba.clone()).fit(ImageFit::Fill).placeholder(false);
                let mut db = DecoratedBox::new().child(img);
                if let Some(ef) = gpu_fx {
                    db = db.layer_effect(ef);
                }
                let mut b = db.style("opacity", alpha as f32);
                if !filter.is_empty() {
                    b = b.style("filter", StyleValue::String(filter.clone()));
                }
                if rot != 0.0 {
                    b = b.style("transform", StyleValue::String(format!("rotate({rot}deg)")));
                }
                Box::new(Positioned::new(b).at((w - iw) / 2.0 + x * w, (h - ih) / 2.0 + y * h).dimensions(iw, ih))
            }
            Source::Color { color } => {
                let col = syngui::mss::MssColor::parse(color).map(|m| Color::from_srgb(m.r, m.g, m.b, m.a as f32 / 255.0 * alpha as f32)).unwrap_or(Color::from_srgb(0, 0, 0, alpha as f32));
                let mut db = DecoratedBox::new();
                if let Some(ef) = gpu_fx {
                    db = db.layer_effect(ef);
                }
                let mut b = db.style("background-color", col).style("transform", StyleValue::String(format!("rotate({rot}deg) scale({k})")));
                if !filter.is_empty() {
                    b = b.style("filter", StyleValue::String(filter.clone()));
                }
                let plate: Box<dyn Widget> = Box::new(Positioned::new(b).at(x * w, y * h).dimensions(w, h));
                // незаполненное место шаблона — плашка с подписью
                match c.slot.as_ref().filter(|s| !s.filled) {
                    Some(s) => {
                        let mut st = Map::new();
                        // подпись сверху: по центру обычно стоит текст шаблона
                        st.insert("position".into(), Value::from("top"));
                        st.insert("color".into(), Value::from("#ffffff90"));
                        st.insert("box".into(), Value::Bool(true));
                        st.insert("size".into(), Value::from((p.settings.width.min(p.settings.height) as f64) * 0.04));
                        st.insert("in".into(), Value::from("none"));
                        st.insert("out".into(), Value::from("none"));
                        let label = format!("＋ {}", if s.label.is_empty() { "ваш файл" } else { &s.label });
                        Box::new(Stack::new().child(plate).child(title(&label, &st, lt, c.duration, alpha as f32, (x, y, 1.0, 0.0), w, h, scale, None)))
                    }
                    None => plate,
                }
            }
            Source::Title { text, style } => {
                if let Some(f) = frames.get(&c.id) {
                    let fk = (w / f.width as f32).min(h / f.height as f32) * k;
                    let (iw, ih) = (f.width as f32 * fk, f.height as f32 * fk);
                    let ptr = Arc::as_ptr(&f.rgba) as usize;
                    let img = Image::from_rgba_shared(format!("tpl{}:{:.4}:{ptr:x}", c.id, f.pts), f.width, f.height, f.rgba.clone()).fit(ImageFit::Fill).placeholder(false);
                    let mut db = DecoratedBox::new().child(img);
                    if let Some(ef) = gpu_fx {
                        db = db.layer_effect(ef);
                    }
                    let mut b = db.style("opacity", alpha as f32);
                    if !filter.is_empty() {
                        b = b.style("filter", StyleValue::String(filter.clone()));
                    }
                    if rot != 0.0 {
                        b = b.style("transform", StyleValue::String(format!("rotate({rot}deg)")));
                    }
                    Box::new(Positioned::new(b).at((w - iw) / 2.0 + x * w, (h - ih) / 2.0 + y * h).dimensions(iw, ih))
                } else {
                    title(text, style, lt, c.duration, alpha as f32, (x, y, k, rot), w, h, scale, gpu_fx)
                }
            }
            Source::Shape { kind, style } => shape_widget(kind, style, alpha as f32, (x, y, k, rot), w, h, scale, &filter, gpu_fx, glass.as_ref()),
        };
        // шторка: видна полоса кадра [x0, x1] (доли ширины)
        let layer: Box<dyn Widget> = match fx.as_ref().and_then(|f| f.clip) {
            Some((x0, x1)) => {
                let inner = Stack::new().child(Positioned::new(Stack::new().child(layer)).at(-x0 * w, 0.0).dimensions(w, h));
                Box::new(Positioned::new(DecoratedBox::new().clip(true).child(inner)).at(x0 * w, 0.0).dimensions(((x1 - x0) * w).max(0.0), h))
            }
            None => layer,
        };
        stack = stack.child(layer);
    }
    Box::new(DecoratedBox::new().child(stack).style("width", w).style("height", h))
}

fn parse_col(s: &str, alpha: f32) -> Option<Color> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") || s.eq_ignore_ascii_case("transparent") {
        return None;
    }
    syngui::mss::MssColor::parse(s).map(|m| Color::from_srgb(m.r, m.g, m.b, (m.a as f32 / 255.0 * alpha).clamp(0.0, 1.0)))
}

fn combine_gpu_fx(a: syngui::effects::Effect, b: Option<syngui::effects::Effect>) -> syngui::effects::Effect {
    match b {
        None => a,
        Some(syngui::effects::Effect::Chain(mut v)) => {
            v.insert(0, a);
            syngui::effects::Effect::Chain(v)
        }
        Some(other) => syngui::effects::Effect::Chain(vec![a, other]),
    }
}

/// Векторная фигура: прямоугольник, круг/овал, треугольник, звезда, линия, стрелка.
/// С `effect:glass` прямоугольник и овал рисуются матовым стеклом (GPU backdrop blur
/// с SDF-обрезкой по скруглению и повороту, бликом, кромкой и мягкой тенью).
#[allow(clippy::too_many_arguments)]
fn shape_widget(kind: &str, st: &Map<String, Value>, alpha: f32, tr: (f32, f32, f32, f32), w: f32, h: f32, scale: f32, filter: &str, gpu_fx: Option<syngui::effects::Effect>, glass: Option<&GlassParams>) -> Box<dyn Widget> {
    let is_line = matches!(kind, "line" | "arrow");
    let wf = num(st, "width", if is_line { 0.45 } else { 0.28 }).clamp(0.01, 2.0) as f32;
    let hf = num(st, "height", if kind == "line" { 0.06 } else if kind == "arrow" { 0.14 } else { 0.28 }).clamp(0.01, 2.0) as f32;
    let sw = (wf * w * tr.2).max(2.0);
    let sh = (hf * h * tr.2).max(2.0);
    let fill_str = st.get("fill").and_then(Value::as_str).unwrap_or(if is_line { "none" } else { "#3b82f6" });
    let stroke_str = st.get("stroke").and_then(Value::as_str).unwrap_or(if is_line { "#ffffff" } else { "none" });
    let fill_col = parse_col(fill_str, alpha);
    let stroke_col = parse_col(stroke_str, alpha);
    let stroke_w = (num(st, "stroke_width", if is_line { 10.0 } else { 0.0 }).max(0.0) as f32) * scale * tr.2;
    let radius = num(st, "radius", 0.08).clamp(0.0, 0.5) as f32;
    let tips = (num(st, "points", 5.0).round() as usize).clamp(3, 12);

    // Матовое стекло (glassmorphism) на прямоугольнике или овале
    if let Some(g) = glass {
        if matches!(kind, "rect" | "ellipse") {
            let ellipse = kind == "ellipse";
            let corner_frac = if ellipse { 0.5 } else { radius };
            let r_px = corner_frac * sw.min(sh);
            let tint_src = g.tint.as_deref().unwrap_or(if fill_str.eq_ignore_ascii_case("none") { "#ffffff" } else { fill_str });
            let (tr_r, tr_g, tr_b) = syngui::mss::MssColor::parse(tint_src).map(|m| (m.r, m.g, m.b)).unwrap_or((255, 255, 255));
            let base_a = (g.tint_opacity * alpha).clamp(0.0, 1.0);
            let hi = g.highlight * alpha;
            // плавный диагональный стеклянный блик: сверху светлее, к низу мягче
            let top_r = (tr_r as f32 + (255.0 - tr_r as f32) * (0.45 * g.highlight)).round() as u8;
            let top_g = (tr_g as f32 + (255.0 - tr_g as f32) * (0.45 * g.highlight)).round() as u8;
            let top_b = (tr_b as f32 + (255.0 - tr_b as f32) * (0.45 * g.highlight)).round() as u8;
            let top_a = (base_a + 0.18 * hi).clamp(0.0, 0.92);
            let mid_a = (base_a + 0.04 * hi).clamp(0.0, 0.90);
            let bot_a = (base_a * 0.82).clamp(0.0, 0.88);
            let grad = syngui::core::Gradient::Linear {
                angle_deg: 165.0,
                stops: vec![
                    syngui::core::ColorStop::new(Color::from_srgb(top_r, top_g, top_b, top_a), 0.0),
                    syngui::core::ColorStop::new(Color::from_srgb(tr_r, tr_g, tr_b, mid_a), 0.42),
                    syngui::core::ColorStop::new(Color::from_srgb(tr_r, tr_g, tr_b, bot_a), 1.0),
                ],
            };
            let rim_src = if stroke_str.eq_ignore_ascii_case("none") || stroke_str.is_empty() { "#ffffff" } else { stroke_str };
            let (sr, sg, sb) = syngui::mss::MssColor::parse(rim_src).map(|m| (m.r, m.g, m.b)).unwrap_or((255, 255, 255));
            let rim_w = if stroke_w > 0.0 { stroke_w } else { (1.5 * scale * tr.2).max(1.0) };
            let rim_a = (g.border_opacity * alpha).clamp(0.0, 1.0);
            let mut shadows: Vec<String> = Vec::new();
            if g.shadow > 0.001 && alpha > 0.001 {
                let sa = (0.34 * g.shadow * alpha).clamp(0.0, 0.8);
                shadows.push(format!("0 {:.1}px {:.1}px rgba(0, 0, 0, {sa:.3})", 10.0 * scale * tr.2, 28.0 * scale * tr.2));
            }
            if hi > 0.001 {
                let dy = (1.5 * scale * tr.2).max(1.0);
                let br = (3.5 * scale * tr.2).max(1.5);
                shadows.push(format!("inset 0 {dy:.1}px {br:.1}px rgba(255, 255, 255, {:.3})", (0.55 * hi).clamp(0.0, 0.85)));
                shadows.push(format!("inset 0 -{dy:.1}px {br:.1}px rgba(0, 0, 0, {:.3})", (0.14 * hi).clamp(0.0, 0.4)));
            }
            let blur_px = (g.blur * scale * tr.2).max(0.0);
            let mut base_box = DecoratedBox::new();
            if tr.3.abs() < 1e-4 {
                let bg_fx = syngui::effects::Effect::BackdropGlass {
                    radius: blur_px,
                    size: [1.0, 1.0],
                    corner_radius: corner_frac,
                    ellipse,
                    rotation: 0.0,
                    opacity: alpha.clamp(0.0, 1.0),
                    saturation: g.saturation,
                    brightness: g.brightness,
                };
                base_box = base_box.layer_effect(combine_gpu_fx(bg_fx, gpu_fx.clone()));
            }
            let mut card = base_box
                .style("width", sw)
                .style("height", sh)
                .style("border-radius", r_px)
                .style("background-gradient", StyleValue::Gradient(grad));
            if rim_a > 0.001 && rim_w > 0.0 {
                card = card.style("border-width", rim_w).style("border-color", Color::from_srgb(sr, sg, sb, rim_a));
            }
            if !shadows.is_empty() {
                card = card.style("box-shadow", StyleValue::String(shadows.join(", ")));
            }
            if !filter.is_empty() {
                card = card.style("filter", StyleValue::String(filter.to_string()));
            }
            if tr.3.abs() < 1e-4 {
                return Box::new(Positioned::new(card).at((w - sw) / 2.0 + tr.0 * w, (h - sh) / 2.0 + tr.1 * h).dimensions(sw, sh));
            }
            // при повороте: внешний контейнер осевой, SDF в BackdropGlass повёрнут на тот же угол
            let rad = tr.3.to_radians();
            let (ca, sa) = (rad.cos().abs(), rad.sin().abs());
            let bw = (sw * ca + sh * sa).max(sw);
            let bh = (sw * sa + sh * ca).max(sh);
            let card = card.style("transform", StyleValue::String(format!("rotate({}deg)", tr.3)));
            let inner = Stack::new().child(Positioned::new(card).at((bw - sw) * 0.5, (bh - sh) * 0.5).dimensions(sw, sh));
            let bg_fx = syngui::effects::Effect::BackdropGlass {
                radius: blur_px,
                size: [(sw / bw).clamp(0.01, 1.0), (sh / bh).clamp(0.01, 1.0)],
                corner_radius: corner_frac,
                ellipse,
                rotation: rad,
                opacity: alpha.clamp(0.0, 1.0),
                saturation: g.saturation,
                brightness: g.brightness,
            };
            let wrap = DecoratedBox::new().layer_effect(combine_gpu_fx(bg_fx, gpu_fx)).child(inner).style("width", bw).style("height", bh);
            return Box::new(Positioned::new(wrap).at((w - bw) / 2.0 + tr.0 * w, (h - bh) / 2.0 + tr.1 * h).dimensions(bw, bh));
        }
    }
    let pad = if stroke_w > 0.0 { stroke_w * 0.6 + 1.0 } else { 1.0 };
    let (cw, ch) = (sw + pad * 2.0, sh + pad * 2.0);
    let kind_s = kind.to_string();

    let canvas = syngui::widgets::Canvas::new(move |cx, _| {
        let (x0, y0) = (pad, pad);
        match kind_s.as_str() {
            "ellipse" => {
                let (rx, ry) = (sw * 0.5, sh * 0.5);
                let (cx0, cy0) = (x0 + rx, y0 + ry);
                let pts: Vec<(f32, f32)> = (0..64)
                    .map(|i| {
                        let a = std::f32::consts::TAU * i as f32 / 64.0;
                        (cx0 + rx * a.cos(), cy0 + ry * a.sin())
                    })
                    .collect();
                if let Some(c) = fill_col {
                    cx.set_color(c);
                    cx.fill_polygon(&pts);
                }
                if let (Some(c), true) = (stroke_col, stroke_w > 0.0) {
                    cx.set_color(c);
                    cx.set_stroke_width(stroke_w);
                    cx.stroke_path(&pts, true);
                }
            }
            "triangle" => {
                let pts = [(x0 + sw * 0.5, y0), (x0 + sw, y0 + sh), (x0, y0 + sh)];
                if let Some(c) = fill_col {
                    cx.set_color(c);
                    cx.fill_polygon(&pts);
                }
                if let (Some(c), true) = (stroke_col, stroke_w > 0.0) {
                    cx.set_color(c);
                    cx.set_stroke_width(stroke_w);
                    cx.stroke_path(&pts, true);
                }
            }
            "star" => {
                let (rx, ry) = (sw * 0.5, sh * 0.5);
                let (cx0, cy0) = (x0 + rx, y0 + ry);
                let n = tips * 2;
                let pts: Vec<(f32, f32)> = (0..n)
                    .map(|i| {
                        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / n as f32;
                        let r = if i % 2 == 0 { 1.0 } else { 0.42 };
                        (cx0 + rx * r * a.cos(), cy0 + ry * r * a.sin())
                    })
                    .collect();
                if let Some(c) = fill_col {
                    cx.set_color(c);
                    cx.fill_polygon_concave(&pts);
                }
                if let (Some(c), true) = (stroke_col, stroke_w > 0.0) {
                    cx.set_color(c);
                    cx.set_stroke_width(stroke_w);
                    cx.stroke_path(&pts, true);
                }
            }
            "line" => {
                let col = stroke_col.or(fill_col).unwrap_or(Color::from_srgb(255, 255, 255, alpha.clamp(0.0, 1.0)));
                cx.set_color(col);
                cx.set_stroke_width(stroke_w.max(2.0));
                cx.draw_line(x0, y0 + sh * 0.5, x0 + sw, y0 + sh * 0.5);
            }
            "arrow" => {
                let col = stroke_col.or(fill_col).unwrap_or(Color::from_srgb(255, 255, 255, alpha.clamp(0.0, 1.0)));
                let cy0 = y0 + sh * 0.5;
                let head_w = (sw * 0.28).min(sh * 0.9).max(8.0);
                let shaft_end = (x0 + sw - head_w * 0.85).max(x0);
                cx.set_color(col);
                cx.set_stroke_width(stroke_w.max(2.0));
                cx.draw_line(x0, cy0, shaft_end, cy0);
                let head = [(x0 + sw, cy0), (x0 + sw - head_w, y0), (x0 + sw - head_w, y0 + sh)];
                cx.fill_polygon(&head);
            }
            _ => {
                let r = radius * sw.min(sh);
                if let Some(c) = fill_col {
                    cx.set_color(c);
                    if r > 0.5 {
                        cx.fill_rounded_rect(x0, y0, sw, sh, r);
                    } else {
                        cx.fill_rect(x0, y0, sw, sh);
                    }
                }
                if let (Some(c), true) = (stroke_col, stroke_w > 0.0) {
                    cx.set_color(c);
                    cx.set_stroke_width(stroke_w);
                    if r > 0.5 {
                        // скруглённая обводка контуром
                        let mut pts = Vec::with_capacity(36);
                        let corners = [
                            (x0 + sw - r, y0 + r, -std::f32::consts::FRAC_PI_2, 0.0),
                            (x0 + sw - r, y0 + sh - r, 0.0, std::f32::consts::FRAC_PI_2),
                            (x0 + r, y0 + sh - r, std::f32::consts::FRAC_PI_2, std::f32::consts::PI),
                            (x0 + r, y0 + r, std::f32::consts::PI, std::f32::consts::PI * 1.5),
                        ];
                        for (ccx, ccy, a0, a1) in corners {
                            for j in 0..=8 {
                                let a = a0 + (a1 - a0) * j as f32 / 8.0;
                                pts.push((ccx + r * a.cos(), ccy + r * a.sin()));
                            }
                        }
                        cx.stroke_path(&pts, true);
                    } else {
                        cx.draw_rect(x0, y0, sw, sh);
                    }
                }
            }
        }
    })
    .size(cw, ch);

    let mut db = DecoratedBox::new().child(canvas);
    if let Some(ef) = gpu_fx {
        db = db.layer_effect(ef);
    }
    let mut b = db.style("width", cw).style("height", ch);
    if !filter.is_empty() {
        b = b.style("filter", StyleValue::String(filter.to_string()));
    }
    if tr.3 != 0.0 {
        b = b.style("transform", StyleValue::String(format!("rotate({}deg)", tr.3)));
    }
    Box::new(Positioned::new(b).at((w - cw) / 2.0 + tr.0 * w, (h - ch) / 2.0 + tr.1 * h).dimensions(cw, ch))
}

/// Виды анимации титра (появление и исчезание): (код, подпись).
pub const TITLE_ANIMS: &[(&str, &str)] = &[
    ("fade", "Появление"),
    ("slide", "Сдвиг"),
    ("pop", "Выпрыгивание"),
    ("scatter", "Разброс"),
    ("type", "Печать"),
    ("none", "Без анимации"),
];

/// Единица анимации титра: (код, подпись).
pub const TITLE_UNITS: &[(&str, &str)] = &[
    ("all", "Целиком"),
    ("char", "По символам"),
    ("word", "По словам"),
    ("line", "По строкам"),
];

/// Эффективная единица анимации: для «разброса» и «печати» при `all`
/// автоматически включается посимвольный режим.
pub fn title_unit<'a>(by: &'a str, kind_in: &str, kind_out: &str) -> &'a str {
    match by {
        "char" | "word" | "line" => by,
        _ if matches!(kind_in, "type" | "scatter") || matches!(kind_out, "type" | "scatter") => "char",
        _ => "all",
    }
}

/// Детерминированный псевдослучайный поток [0, 1) для «разброса».
fn hash01(seed: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / ((1u64 << 24) as f32)
}

/// Мягкий перенос строк по словам для посимвольной/пословной раскладки.
pub fn wrap_lines(text: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split('\n') {
        if max_chars == 0 || raw.chars().count() <= max_chars {
            out.push(raw.to_string());
            continue;
        }
        let mut line = String::new();
        let mut len = 0usize;
        for word in raw.split(' ') {
            let wlen = word.chars().count();
            if len > 0 && len + 1 + wlen > max_chars {
                out.push(std::mem::take(&mut line));
                len = 0;
            }
            if len > 0 {
                line.push(' ');
                len += 1;
            }
            line.push_str(word);
            len += wlen;
        }
        out.push(line);
    }
    out
}

/// Фрагмент строки титра: текст и номер анимируемой единицы (`None` — пробел).
struct Piece {
    text: String,
    idx: Option<usize>,
}

fn tokenize_line(line: &str, unit: &str, next: &mut usize) -> Vec<Piece> {
    match unit {
        "line" => {
            if line.trim().is_empty() {
                vec![Piece { text: "\u{00A0}".into(), idx: None }]
            } else {
                let i = *next;
                *next += 1;
                vec![Piece { text: line.to_string(), idx: Some(i) }]
            }
        }
        "word" => {
            let mut out = Vec::new();
            let mut cur = String::new();
            let mut ws: Option<bool> = None;
            let flush = |cur: &mut String, is_ws: bool, out: &mut Vec<Piece>, next: &mut usize| {
                if cur.is_empty() {
                    return;
                }
                let s = std::mem::take(cur);
                if is_ws {
                    out.push(Piece { text: s.replace(' ', "\u{00A0}"), idx: None });
                } else {
                    let i = *next;
                    *next += 1;
                    out.push(Piece { text: s, idx: Some(i) });
                }
            };
            for ch in line.chars() {
                let is_ws = ch.is_whitespace();
                if ws.is_some_and(|prev| prev != is_ws) {
                    flush(&mut cur, ws.unwrap(), &mut out, next);
                }
                cur.push(ch);
                ws = Some(is_ws);
            }
            if let Some(is_ws) = ws {
                flush(&mut cur, is_ws, &mut out, next);
            }
            if out.is_empty() {
                out.push(Piece { text: "\u{00A0}".into(), idx: None });
            }
            out
        }
        _ => {
            let mut out = Vec::new();
            for ch in line.chars() {
                if ch.is_whitespace() {
                    out.push(Piece { text: "\u{00A0}".into(), idx: None });
                } else {
                    let i = *next;
                    *next += 1;
                    out.push(Piece { text: ch.to_string(), idx: Some(i) });
                }
            }
            if out.is_empty() {
                out.push(Piece { text: "\u{00A0}".into(), idx: None });
            }
            out
        }
    }
}

/// Прогресс элемента `idx` из `total` при общей фазе `p` (0–1) и разбеге `stagger` (0–1).
pub fn unit_progress(p: f64, idx: usize, total: usize, stagger: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    if total <= 1 || stagger <= 1e-6 {
        return p;
    }
    let span = stagger.clamp(0.0, 0.95);
    let ud = (1.0 - span).max(0.05);
    let start = (idx as f64 / (total - 1) as f64) * span;
    ((p - start) / ud).clamp(0.0, 1.0)
}

/// Состояние одной анимированной единицы (символа, слова или строки).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitState {
    pub alpha: f32,
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub rot: f32,
    pub cursor: bool,
}

/// Расчёт трансформации и прозрачности единицы `idx` из `total`.
#[allow(clippy::too_many_arguments)]
pub fn title_unit_state(lt: f64, dur: f64, anim: f64, kind_in: &str, kind_out: &str, ease_kind: &str, stagger: f64, idx: usize, total: usize, font: f32) -> UnitState {
    let pin = if anim > 0.0 { (lt / anim).clamp(0.0, 1.0) } else { 1.0 };
    let pout = if anim > 0.0 { ((dur - lt) / anim).clamp(0.0, 1.0) } else { 1.0 };
    let mut s = UnitState { alpha: 1.0, dx: 0.0, dy: 0.0, scale: 1.0, rot: 0.0, cursor: false };
    for (kind, p, is_out) in [(kind_in, pin, false), (kind_out, pout, true)] {
        match kind {
            "none" => {}
            "type" => {
                let shown = if is_out {
                    if p <= 1e-9 { 0 } else { ((p * total as f64).ceil() as usize).min(total) }
                } else if p >= 1.0 - 1e-9 {
                    total
                } else {
                    ((p * total as f64 + 1e-9).floor() as usize).min(total)
                };
                if idx >= shown {
                    s.alpha = 0.0;
                }
                // курсор в позиции следующей печатаемой/стираемой единицы (без сдвига раскладки)
                if idx == shown && shown < total && ((!is_out && p > 0.0) || (is_out && p < 1.0)) {
                    s.cursor = true;
                }
            }
            "fade" => {
                let u = unit_progress(p, idx, total, stagger);
                let e = crate::doc::ease(ease_kind, u).clamp(0.0, 1.0) as f32;
                s.alpha *= e;
            }
            "slide" => {
                let u = unit_progress(p, idx, total, stagger);
                let e = crate::doc::ease(ease_kind, u) as f32;
                s.alpha *= e.clamp(0.0, 1.0);
                let dir = if is_out { -1.0 } else { 1.0 };
                s.dy += (1.0 - e) * font * 0.8 * dir;
            }
            "pop" => {
                let u = unit_progress(p, idx, total, stagger);
                let e = crate::doc::ease(ease_kind, u) as f32;
                s.alpha *= e.clamp(0.0, 1.0);
                s.scale *= (0.3 + 0.7 * e).max(0.01);
            }
            "scatter" => {
                let u = unit_progress(p, idx, total, stagger);
                let e = crate::doc::ease(ease_kind, u) as f32;
                s.alpha *= e.clamp(0.0, 1.0);
                let base = idx as u64 * 3 + if is_out { 1000 } else { 1 };
                let hx = hash01(base) * 2.0 - 1.0;
                let hy = hash01(base + 1) * 2.0 - 1.0;
                let hr = hash01(base + 2) * 2.0 - 1.0;
                s.dx += (1.0 - e) * hx * font * 2.0;
                s.dy += (1.0 - e) * hy * font * 1.5;
                s.rot += (1.0 - e) * hr * 42.0;
                s.scale *= (0.55 + 0.45 * e).max(0.01);
            }
            _ => {}
        }
    }
    s
}

fn unit_transform(s: &UnitState) -> Option<String> {
    let mut parts = Vec::new();
    if s.dx.abs() > 0.05 || s.dy.abs() > 0.05 {
        parts.push(format!("translate({:.2}px, {:.2}px)", s.dx, s.dy));
    }
    if s.rot.abs() > 0.05 {
        parts.push(format!("rotate({:.2}deg)", s.rot));
    }
    if (s.scale - 1.0).abs() > 1e-3 {
        parts.push(format!("scale({:.3})", s.scale));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Титр: размер, цвет, положение, подложка; появление и исчезание (`in`/`out`:
/// fade, slide, pop, scatter, type, none) целиком или по символам/словам/строкам
/// (`by`: all, char, word, line) за `anim` секунд с разбегом `stagger` и кривой `ease`.
#[allow(clippy::too_many_arguments)]
fn title(text: &str, st: &Map<String, Value>, lt: f64, dur: f64, alpha: f32, tr: (f32, f32, f32, f32), w: f32, h: f32, scale: f32, gpu_fx: Option<syngui::effects::Effect>) -> Box<dyn Widget> {
    let size = num(st, "size", (w.min(h) / scale) as f64 * 0.075) as f32 * scale * tr.2;
    let (cr, cg, cb, ca) = st.get("color").and_then(Value::as_str).and_then(syngui::mss::MssColor::parse).map(|m| (m.r, m.g, m.b, m.a as f32 / 255.0)).unwrap_or((255, 255, 255, 1.0));
    let color_a = |k: f32| Color::from_srgb(cr, cg, cb, (ca * k).clamp(0.0, 1.0));
    let pos = st.get("position").and_then(Value::as_str).unwrap_or("bottom");
    let anim = num(st, "anim", 0.4).max(0.0);
    let kind_in = st.get("in").and_then(Value::as_str).unwrap_or("fade");
    let kind_out = st.get("out").and_then(Value::as_str).unwrap_or("fade");
    let by = st.get("by").and_then(Value::as_str).unwrap_or("all");
    let unit = title_unit(by, kind_in, kind_out);
    let stagger = num(st, "stagger", 0.65).clamp(0.0, 1.0);
    let ease_kind = st.get("ease").and_then(Value::as_str).filter(|s| crate::doc::valid_ease(s)).unwrap_or("out");
    let font_fam = st.get("font").and_then(Value::as_str);
    if let Some(f) = font_fam {
        crate::fonts::ensure(f);
    }
    let boxed = st.get("box").and_then(Value::as_bool).unwrap_or(false);
    let margin = h * 0.07;

    let make_text = |s: String, fsize: f32, max_lines: usize, col: Color| {
        let mut t = Text::new(s).max_lines(max_lines).style("color", col).style("font-size", fsize).style("font-weight", 700.0f32).style("text-align", StyleValue::String("center".into()));
        if let Some(f) = font_fam {
            t = t.style("font-family", StyleValue::String(f.to_string()));
        }
        t
    };

    let (content, a, dy): (Box<dyn Widget>, f32, f32) = if unit == "all" {
        // быстрый путь: анимация всего титра целиком
        let pin = if anim > 0.0 { (lt / anim).clamp(0.0, 1.0) } else { 1.0 };
        let pout = if anim > 0.0 { ((dur - lt) / anim).clamp(0.0, 1.0) } else { 1.0 };
        let (mut a, mut dy, mut k) = (alpha, 0.0f32, 1.0f32);
        for (kind, p) in [(kind_in, pin), (kind_out, pout)] {
            let e = crate::doc::ease(ease_kind, p) as f32;
            match kind {
                "fade" => a *= e.clamp(0.0, 1.0),
                "slide" => {
                    a *= e.clamp(0.0, 1.0);
                    dy += (1.0 - e) * size * 0.8;
                }
                "pop" => {
                    a *= e.clamp(0.0, 1.0);
                    k *= (0.6 + 0.4 * e).max(0.01);
                }
                _ => {}
            }
        }
        let font = (size * k).max(1.0);
        (Box::new(make_text(text.to_string(), font, 4, color_a(a))), a, dy)
    } else {
        // посимвольно / по словам / по строкам: все элементы в раскладке (без прыжков)
        let font = size.max(1.0);
        let max_chars = ((w - 2.0 * margin - font * 0.5) / (font * 0.58)).floor().max(8.0) as usize;
        let lines = wrap_lines(text, max_chars);
        let mut next = 0usize;
        let tokenized: Vec<Vec<Piece>> = lines.iter().map(|l| tokenize_line(l, unit, &mut next)).collect();
        let total = next;
        if total == 0 {
            (Box::new(make_text(text.to_string(), font, 4, color_a(alpha))), alpha, 0.0)
        } else {
            let mut col = Column::new().gap(font * 0.12).cross_axis_alignment(CrossAxisAlignment::Center);
            for pieces in tokenized {
                let mut row = Row::new().gap(0.0).main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center);
                for piece in pieces {
                    let w: Box<dyn Widget> = match piece.idx {
                        None => Box::new(make_text(piece.text, font, 1, Color::from_srgb(0, 0, 0, 0.0))),
                        Some(idx) => {
                            let s = title_unit_state(lt, dur, anim, kind_in, kind_out, ease_kind, stagger, idx, total, font);
                            let t = make_text(piece.text, font, 1, color_a(s.alpha * alpha));
                            let b: Box<dyn Widget> = match unit_transform(&s) {
                                Some(xf) => Box::new(DecoratedBox::new().child(t).style("transform", StyleValue::String(xf))),
                                None => Box::new(t),
                            };
                            if s.cursor {
                                let cw = (font * 0.07).max(1.5);
                                let caret = DecoratedBox::new().style("background-color", color_a(alpha));
                                Box::new(Stack::new().child(b).child(Positioned::new(caret).at(0.0, 0.0).dimensions(cw, font * 1.05)))
                            } else {
                                b
                            }
                        }
                    };
                    row = row.child(w);
                }
                col = col.child(row);
            }
            (Box::new(col), alpha, 0.0)
        }
    };

    let pad = size.max(1.0) * 0.25;
    let mut pill = DecoratedBox::new().child(content).style("padding", pad);
    if boxed {
        pill = pill.style("background-color", Color::from_srgb(0, 0, 0, 0.55 * a.clamp(0.0, 1.0))).style("border-radius", pad);
    }
    // поворот — вокруг самого текста, а не кадра
    if tr.3 != 0.0 {
        pill = pill.style("transform", StyleValue::String(format!("rotate({}deg)", tr.3)));
    }
    let main = match pos {
        "top" => MainAxisAlignment::Start,
        "center" | "middle" => MainAxisAlignment::Center,
        _ => MainAxisAlignment::End,
    };
    let col = Column::new().main_axis_alignment(main).cross_axis_alignment(CrossAxisAlignment::Center).child(pill);
    let mut inner_box = DecoratedBox::new().child(col);
    if let Some(ef) = gpu_fx {
        inner_box = inner_box.layer_effect(ef);
    }
    let inner = inner_box.style("padding", margin).style("opacity", a);
    Box::new(Positioned::new(inner).at(tr.0 * w, tr.1 * h + dy).dimensions(w, h))
}

/// Кадры источников для момента `t`: по читателю на клип (подряд — быстро),
/// LUT клипа и исполнение шаблонов Fusion применяются здесь.
pub struct FrameSource {
    readers: HashMap<u64, syngui::video::FrameReader>,
    stills: HashMap<u64, syngui::video::RgbaFrame>,
    /// Кэш последнего отрендеренного кадра Fusion-шаблона на клип: `clip_id -> (key_hash, frame)`.
    fusion_last: HashMap<u64, (u64, syngui::video::RgbaFrame)>,
    /// Последний кадр Lottie у клипа: (номер кадра, размер) → кадр.
    lottie_last: HashMap<u64, ((i64, (u32, u32)), syngui::video::RgbaFrame)>,
    max: (u32, u32),
    /// Нейрофильтры клипов (`interpolate`, `upscale`) — при экспорте.
    pub neural: bool,
    interp: HashMap<u64, Option<super::neural::Interp>>,
    upscale: HashMap<u64, Option<super::neural::Upscale>>,
    /// Что не удалось (нет CUDA, моделей) — для отчёта экспорта.
    pub notes: Vec<String>,
    /// Сравнение «до/после» в просмотре: коррекция и LUT — только правее
    /// этой доли ширины кадра (экспорт не трогает).
    pub compare: Option<f32>,
}

impl FrameSource {
    /// Кадры вписываются в `max_w × max_h` (размер проекта или панели).
    pub fn new(max_w: u32, max_h: u32) -> Self {
        Self { readers: HashMap::new(), stills: HashMap::new(), fusion_last: HashMap::new(), lottie_last: HashMap::new(), max: (max_w, max_h), neural: false, interp: HashMap::new(), upscale: HashMap::new(), notes: Vec::new(), compare: None }
    }

    /// Граф кадра клипа (D1) на кадре источника; нейроувеличение — общим слотом клипа.
    fn eval_graph(&mut self, cid: u64, g: &crate::framegraph::FrameGraph, f: &syngui::video::RgbaFrame) -> syngui::video::RgbaFrame {
        let scale = f.height as f32 / 1080.0;
        let neural_on = self.neural;
        let slots = &mut self.upscale;
        let notes = &mut self.notes;
        let mut neural = |n: &crate::framegraph::GNode, im: &crate::fusion::img::Image| -> Option<crate::fusion::img::Image> {
            if !neural_on {
                return None;
            }
            let hint = n.params.get("model").and_then(Value::as_str).unwrap_or("").to_string();
            let slot = slots.entry(cid).or_insert_with(|| super::neural::Upscale::open(&hint).map_err(|e| notes.push(format!("c{cid}: увеличение пропущено — {e}"))).ok());
            let up = slot.as_mut()?;
            let fr = syngui::video::RgbaFrame { width: im.width, height: im.height, pts: 0.0, rgba: Arc::new(im.to_rgba8()) };
            match up.apply(&fr) {
                Ok(v) => Some(crate::fusion::img::Image::from_rgba8(v.width, v.height, &v.rgba)),
                Err(e) => {
                    notes.push(format!("c{cid}: увеличение — {e}"));
                    *slot = None;
                    None
                }
            }
        };
        let im = g.eval(f.width, f.height, &f.rgba, scale, &mut neural);
        syngui::video::RgbaFrame { width: im.width, height: im.height, pts: f.pts, rgba: Arc::new(im.to_rgba8()) }
    }

    pub fn frames(&mut self, p: &Project, t: f64) -> Frames {
        let mut out = Frames::new();
        let active_layers = layers(p, t);
        for l in &active_layers {
            let (c, t) = (l.clip, l.t);
            // Шаблон Fusion (`.setting`) в Source::Title — исполняем граф Fusion
            let lt = (t - c.start).clamp(0.0, c.duration);
            let c_at = c.at(lt);
            if let Source::Title { text, style } = &c_at.source {
                if let Some(tpl_path) = style.get("template").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    if let Ok(comp) = crate::fusion::load_cached(std::path::Path::new(tpl_path)) {
                        // В просмотре ограничиваем размер до 960×540 для отзывчивости; на экспорте и frame.export — полное разрешение
                        let (lim_w, lim_h) = if !self.neural && self.max.0 == 1280 && self.max.1 == 1280 {
                            (960u32, 540u32)
                        } else {
                            self.max
                        };
                        let pw = p.settings.width.max(2) as f32;
                        let ph = p.settings.height.max(2) as f32;
                        let k = (lim_w as f32 / pw).min(lim_h as f32 / ph).min(1.0);
                        let fw = (((pw * k).round() as u32) & !1).max(2);
                        let fh = (((ph * k).round() as u32) & !1).max(2);
                        let font_ov = style.get("font").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
                        let frame_idx = (lt * p.settings.fps.max(1.0)).round() as i64;
                        let sig = {
                            use std::hash::{Hash, Hasher};
                            let mut h = std::collections::hash_map::DefaultHasher::new();
                            tpl_path.hash(&mut h);
                            text.hash(&mut h);
                            font_ov.hash(&mut h);
                            frame_idx.hash(&mut h);
                            fw.hash(&mut h);
                            fh.hash(&mut h);
                            c.duration.to_bits().hash(&mut h);
                            // Хэшируем все переопределения параметров шаблона (`p:*`, включая анимированные ключами)
                            for (sk, sv) in style {
                                if sk.starts_with(crate::fusion::STYLE_PREFIX) {
                                    sk.hash(&mut h);
                                    sv.to_string().hash(&mut h);
                                }
                            }
                            h.finish()
                        };
                        let frame = if let Some((k0, f0)) = self.fusion_last.get(&c.id).filter(|(k0, _)| *k0 == sig) {
                            let _ = k0;
                            f0.clone()
                        } else {
                            let ov = crate::fusion::overrides_from_title(&comp, text, style);
                            let f = crate::fusion::eval::render(&comp, fw, fh, lt, c.duration, p.settings.fps, &[], &ov);
                            self.fusion_last.insert(c.id, (sig, f.clone()));
                            f
                        };
                        out.insert(c.id, frame);
                    }
                }
                continue;
            }
            let Some(mid) = c.media() else { continue };
            let Some(m) = p.media(mid) else { continue };
            let path = m.path.display().to_string();
            let frame = if m.kind == crate::doc::MediaKind::Lottie {
                // время источника, как у видео (скорость, сдвиг), по кругу; кадр анимации — в кэше
                let src = c.offset + (t - c.start) * c.speed;
                match crate::lottie::load(&m.path) {
                    Ok(comp) => {
                        let fi = crate::lottie::frame_at(&comp, src).round() as i64;
                        let sig = (fi, self.max);
                        match self.lottie_last.get(&c.id).filter(|(k, _)| *k == sig) {
                            Some((_, f)) => Some(f.clone()),
                            None => {
                                let f = crate::lottie::render(&comp, src, self.max);
                                self.lottie_last.insert(c.id, (sig, f.clone()));
                                Some(f)
                            }
                        }
                    }
                    Err(e) => {
                        self.notes.push(format!("c{}: {e}", c.id));
                        None
                    }
                }
            } else if m.kind == crate::doc::MediaKind::Image {
                if !self.stills.contains_key(&mid) {
                    if let Ok(img) = image::open(&m.path) {
                        let img = img.to_rgba8();
                        let (iw, ih) = img.dimensions();
                        self.stills.insert(mid, syngui::video::RgbaFrame { width: iw, height: ih, pts: 0.0, rgba: Arc::new(img.into_raw()) });
                    }
                }
                self.stills.get(&mid).cloned()
            } else {
                let src = c.offset + (t - c.start) * c.speed;
                let r = match self.readers.entry(c.id) {
                    std::collections::hash_map::Entry::Occupied(o) => Some(o.into_mut()),
                    std::collections::hash_map::Entry::Vacant(v) => syngui::video::FrameReader::open(&path, self.max.0, self.max.1).ok().map(|r| v.insert(r)),
                };
                let mut f = r.as_ref().map(|_| ()).and(None::<syngui::video::RgbaFrame>);
                if let Some(r) = r {
                    let rife = self.neural && effect(c, "interpolate").is_some();
                    // с RIFE — последний кадр до момента (между ним и следующим строится промежуточный)
                    if let Ok(cur) = if rife { r.frame_floor(src) } else { r.frame_at(src) } {
                        let mut out = cur.clone();
                        if self.neural && effect(c, "interpolate").is_some() && src > cur.pts + 1e-4 {
                            if let Some(next) = r.peek_next() {
                                let notes = &mut self.notes;
                                let slot = self.interp.entry(c.id).or_insert_with(|| super::neural::Interp::open().map_err(|e| notes.push(format!("c{}: плавность RIFE пропущена — {e}", c.id))).ok());
                                if let Some(ip) = slot.as_mut() {
                                    match ip.between(&cur, &next, src) {
                                        Ok(v) => out = v,
                                        Err(e) => {
                                            self.notes.push(format!("c{}: RIFE — {e}", c.id));
                                            *slot = None;
                                        }
                                    }
                                }
                            }
                        }
                        f = Some(out);
                    }
                }
                if let (true, Some(e), Some(fr), true) = (self.neural, effect(c, "upscale"), f.as_ref(), c.graph.is_none()) {
                    let hint = e.params.get("model").and_then(Value::as_str).unwrap_or("").to_string();
                    let notes = &mut self.notes;
                    let slot = self.upscale.entry(c.id).or_insert_with(|| super::neural::Upscale::open(&hint).map_err(|err| notes.push(format!("c{}: увеличение пропущено — {err}", c.id))).ok());
                    // увеличивать, только пока кадр меньше кадра проекта
                    if fr.width < self.max.0 && fr.height < self.max.1 {
                        if let Some(up) = slot.as_mut() {
                            match up.apply(fr) {
                                Ok(v) => f = Some(v),
                                Err(err) => {
                                    self.notes.push(format!("c{}: увеличение — {err}", c.id));
                                    *slot = None;
                                }
                            }
                        }
                    }
                }
                f
            };
            let Some(mut f) = frame else { continue };
            if let Some(g) = &c.graph {
                f = self.eval_graph(c.id, g, &f);
                out.insert(c.id, f);
                continue;
            }
            let from_x = self.compare.map(|k| ((f.width as f32 * k.clamp(0.0, 1.0)) as usize).min(f.width as usize)).unwrap_or(0);
            // первичная коррекция (колёса, кривые) — до LUT
            if let Some(e) = effect(c, "grade") {
                let g = super::grade::Grade::from_params(&e.params);
                if !g.is_identity() {
                    let mut px = (*f.rgba).clone();
                    super::grade::apply(&g.tables(), &mut px, f.width as usize, from_x);
                    f.rgba = Arc::new(px);
                }
            }
            if let Some(e) = effect(c, "lut") {
                if let Some(lp) = e.params.get("path").and_then(Value::as_str) {
                    if let Ok(lut) = super::lut::load(lp) {
                        let mut px = (*f.rgba).clone();
                        lut.apply_from(&mut px, f.width as usize, from_x, num(&e.params, "strength", 1.0) as f32);
                        f.rgba = Arc::new(px);
                    }
                }
            }
            out.insert(c.id, f);
        }
        // Переходы Fusion (C5): сводим уходящий и входящий клип через граф `.setting`
        for pair in active_layers.windows(2) {
            let (prev_l, cur_l) = (&pair[0], &pair[1]);
            let Some(("fusion", q, true)) = cur_l.tr else { continue };
            let Some(("fusion", _, false)) = prev_l.tr else { continue };
            let Some(trn) = cur_l.clip.transition.as_ref().filter(|x| x.is_fusion()) else { continue };
            let Some(tpl_path) = trn.template.as_deref() else { continue };
            let Ok(comp) = crate::fusion::load_cached(std::path::Path::new(tpl_path)) else { continue };

            let (lim_w, lim_h) = if !self.neural && self.max.0 == 1280 && self.max.1 == 1280 {
                (960u32, 540u32)
            } else {
                self.max
            };
            let pw = p.settings.width.max(2) as f32;
            let ph = p.settings.height.max(2) as f32;
            let k = (lim_w as f32 / pw).min(lim_h as f32 / ph).min(1.0);
            let fw = (((pw * k).round() as u32) & !1).max(2);
            let fh = (((ph * k).round() as u32) & !1).max(2);

            let frame_idx = (q * trn.duration * p.settings.fps.max(1.0)).round() as i64;
            let prev_pts = out.get(&prev_l.clip.id).map(|f| f.pts.to_bits()).unwrap_or(0);
            let cur_pts = out.get(&cur_l.clip.id).map(|f| f.pts.to_bits()).unwrap_or(0);
            let sig = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                "ftr".hash(&mut h);
                tpl_path.hash(&mut h);
                prev_l.clip.id.hash(&mut h);
                cur_l.clip.id.hash(&mut h);
                prev_pts.hash(&mut h);
                cur_pts.hash(&mut h);
                frame_idx.hash(&mut h);
                fw.hash(&mut h);
                fh.hash(&mut h);
                trn.duration.to_bits().hash(&mut h);
                for (sk, sv) in &trn.params {
                    sk.hash(&mut h);
                    sv.to_string().hash(&mut h);
                }
                h.finish()
            };
            let key_id = cur_l.clip.id ^ 0x5452_414E_5349_5400;
            let rendered = if let Some((_, f0)) = self.fusion_last.get(&key_id).filter(|(k0, _)| *k0 == sig) {
                f0.clone()
            } else {
                let bg_col = fusion_bg_color(&p.settings.bg);
                let img_a = clip_to_fusion_image(prev_l.clip, out.get(&prev_l.clip.id), fw, fh, bg_col);
                let img_b = clip_to_fusion_image(cur_l.clip, out.get(&cur_l.clip.id), fw, fh, bg_col);
                let ov = crate::fusion::overrides_from_title(&comp, "", &trn.params);
                let mut f = crate::fusion::eval::render(&comp, fw, fh, q * trn.duration, trn.duration, p.settings.fps, &[img_a, img_b], &ov);
                f.pts = t;
                self.fusion_last.insert(key_id, (sig, f.clone()));
                f
            };
            out.insert(cur_l.clip.id, rendered);
        }
        // читатели и кэш клипов, которых больше нет на таймлайне
        self.readers.retain(|id, _| p.find_clip(*id).is_some());
        self.fusion_last.retain(|id, _| p.find_clip(*id).is_some() || p.find_clip(*id ^ 0x5452_414E_5349_5400).is_some());
        out
    }
}

fn fusion_bg_color(bg: &str) -> crate::fusion::Color4 {
    let s = bg.trim();
    if s.eq_ignore_ascii_case("transparent") || s.eq_ignore_ascii_case("none") {
        return crate::fusion::Color4::TRANSPARENT;
    }
    if let Some(m) = syngui::mss::MssColor::parse(if s.is_empty() { "#000000" } else { s }) {
        crate::fusion::Color4::from_straight(m.r as f32 / 255.0, m.g as f32 / 255.0, m.b as f32 / 255.0, m.a as f32 / 255.0)
    } else {
        crate::fusion::Color4::from_straight(0.0, 0.0, 0.0, 1.0)
    }
}

/// Подготовить кадр клипа к входу перехода Fusion (`fw × fh`, с вписыванием `contain`).
fn clip_to_fusion_image(c: &Clip, frame: Option<&syngui::video::RgbaFrame>, fw: u32, fh: u32, bg: crate::fusion::Color4) -> Arc<crate::fusion::Image> {
    use crate::fusion::{Color4, Image as FImg};
    if let Some(fr) = frame {
        if fr.width == fw && fr.height == fh {
            return Arc::new(FImg::from_rgba8(fw, fh, &fr.rgba));
        }
        let mut out = FImg::solid(fw, fh, bg);
        let sw = fr.width.max(1) as f32;
        let sh = fr.height.max(1) as f32;
        let k = fit_k(c, fw as f32 / sw, fh as f32 / sh);
        let (dw, dh) = (sw * k, sh * k);
        let (ox, oy) = ((fw as f32 - dw) / 2.0, (fh as f32 - dh) / 2.0);
        let src = &fr.rgba;
        for y in 0..fh {
            let v = (y as f32 + 0.5 - oy) / dh;
            if !(0.0..1.0).contains(&v) {
                continue;
            }
            let sy = (v * sh).clamp(0.0, sh - 1.0) as u32;
            for x in 0..fw {
                let u = (x as f32 + 0.5 - ox) / dw;
                if !(0.0..1.0).contains(&u) {
                    continue;
                }
                let sx = (u * sw).clamp(0.0, sw - 1.0) as u32;
                let si = ((sy * fr.width + sx) * 4) as usize;
                let a = src[si + 3] as f32 / 255.0;
                let sc = Color4::from_straight(src[si] as f32 / 255.0, src[si + 1] as f32 / 255.0, src[si + 2] as f32 / 255.0, a);
                let di = ((y * fw + x) * 4) as usize;
                let inv = 1.0 - sc.a;
                out.data[di] = sc.r + out.data[di] * inv;
                out.data[di + 1] = sc.g + out.data[di + 1] * inv;
                out.data[di + 2] = sc.b + out.data[di + 2] * inv;
                out.data[di + 3] = sc.a + out.data[di + 3] * inv;
            }
        }
        return Arc::new(out);
    }
    if let Source::Color { color } = &c.source {
        if let Some(m) = syngui::mss::MssColor::parse(color) {
            return Arc::new(FImg::solid(fw, fh, Color4::from_straight(m.r as f32 / 255.0, m.g as f32 / 255.0, m.b as f32 / 255.0, m.a as f32 / 255.0)));
        }
    }
    Arc::new(FImg::solid(fw, fh, bg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_unit_defaults_and_overrides() {
        assert_eq!(title_unit("all", "fade", "fade"), "all");
        assert_eq!(title_unit("all", "type", "none"), "char");
        assert_eq!(title_unit("all", "none", "scatter"), "char");
        assert_eq!(title_unit("word", "type", "none"), "word");
        assert_eq!(title_unit("line", "slide", "fade"), "line");
    }

    #[test]
    fn wrap_and_tokenize_units() {
        assert_eq!(wrap_lines("один два три четыре", 10), vec!["один два", "три четыре"]);
        assert_eq!(wrap_lines("A\n\nB", 20), vec!["A", "", "B"]);

        let mut n = 0;
        let chars = tokenize_line("А Б", "char", &mut n);
        assert_eq!(n, 2);
        assert_eq!(chars.len(), 3);
        assert_eq!(chars[1].text, "\u{00A0}");
        assert_eq!(chars[1].idx, None);

        let mut n = 0;
        let words = tokenize_line("привет  мир", "word", &mut n);
        assert_eq!(n, 2);
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].idx, Some(0));
        assert_eq!(words[1].idx, None);
        assert_eq!(words[2].idx, Some(1));

        let mut n = 0;
        let line = tokenize_line("целая строка", "line", &mut n);
        assert_eq!(n, 1);
        assert_eq!(line[0].idx, Some(0));
    }

    #[test]
    fn stagger_and_modes_behave_smoothly() {
        // разбег: первый элемент опережает последний при появлении
        let a = title_unit_state(0.3, 3.0, 1.0, "slide", "none", "out", 0.65, 0, 5, 60.0);
        let b = title_unit_state(0.3, 3.0, 1.0, "slide", "none", "out", 0.65, 4, 5, 60.0);
        assert!(a.alpha > b.alpha && a.dy < b.dy);

        // разброс: в середине есть смещение и поворот, после anim — покой
        let mid = title_unit_state(0.25, 3.0, 1.0, "scatter", "none", "out", 0.5, 2, 6, 60.0);
        assert!(mid.dx.abs() > 0.1 || mid.dy.abs() > 0.1);
        assert!(mid.rot.abs() > 0.1);
        let rest = title_unit_state(1.5, 3.0, 1.0, "scatter", "none", "out", 0.5, 2, 6, 60.0);
        assert_eq!((rest.alpha, rest.dx, rest.dy, rest.rot, rest.scale), (1.0, 0.0, 0.0, 0.0, 1.0));

        // печать: на половине — половина символов видна, на следующем стоит курсор
        let s0 = title_unit_state(0.5, 3.0, 1.0, "type", "none", "linear", 0.65, 0, 4, 60.0);
        let s1 = title_unit_state(0.5, 3.0, 1.0, "type", "none", "linear", 0.65, 1, 4, 60.0);
        let s2 = title_unit_state(0.5, 3.0, 1.0, "type", "none", "linear", 0.65, 2, 4, 60.0);
        let s3 = title_unit_state(0.5, 3.0, 1.0, "type", "none", "linear", 0.65, 3, 4, 60.0);
        assert_eq!((s0.alpha, s1.alpha, s2.alpha, s3.alpha), (1.0, 1.0, 0.0, 0.0));
        assert!(!s0.cursor && !s1.cursor && s2.cursor && !s3.cursor);

        // печать при исчезании: стирает с конца
        let e0 = title_unit_state(2.5, 3.0, 1.0, "none", "type", "linear", 0.65, 0, 4, 60.0);
        let e3 = title_unit_state(2.5, 3.0, 1.0, "none", "type", "linear", 0.65, 3, 4, 60.0);
        assert_eq!((e0.alpha, e3.alpha), (1.0, 0.0));
    }
}
