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

/// Видимые клипы видеодорожек в момент `t`, снизу вверх.
pub fn layers(p: &Project, t: f64) -> Vec<&Clip> {
    p.tracks
        .iter()
        .filter(|tr| tr.kind == TrackKind::Video && !tr.hidden)
        .filter_map(|tr| tr.clips.iter().find(|c| c.start <= t + 1e-9 && t < c.end() - 1e-9))
        .collect()
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

/// Кадр в момент `t` размером `w × h` (масштаб `scale` от размера проекта).
pub fn frame_widget(p: &Project, t: f64, frames: &Frames, scale: f32) -> Box<dyn Widget> {
    let w = p.settings.width as f32 * scale;
    let h = p.settings.height as f32 * scale;
    let mut stack = Stack::new().clip(true);
    stack = stack.child(Positioned::new(DecoratedBox::new().style("background-color", Color::from_srgb(0, 0, 0, 1.0))).at(0.0, 0.0).dimensions(w, h));
    for c in layers(p, t) {
        let lt = t - c.start;
        let mut alpha = fade(c, lt, "");
        // положение и масштаб: x, y — доли кадра от центра, scale, rotation, opacity
        let (mut x, mut y, mut k, mut rot) = (0.0f32, 0.0f32, 1.0f32, 0.0f32);
        if let Some(e) = effect(c, "transform") {
            let pm = &e.params;
            x = num(pm, "x", 0.0) as f32;
            y = num(pm, "y", 0.0) as f32;
            k = num(pm, "scale", 1.0) as f32;
            rot = num(pm, "rotation", 0.0) as f32;
            alpha *= num(pm, "opacity", 1.0);
        }
        let filter = filters(c, scale);
        let layer: Box<dyn Widget> = match &c.source {
            Source::Media { .. } => {
                let Some(f) = frames.get(&c.id) else { continue };
                // вписать кадр (contain) в кадр проекта
                let fk = (w / f.width as f32).min(h / f.height as f32) * k;
                let (iw, ih) = (f.width as f32 * fk, f.height as f32 * fk);
                let img = Image::from_rgba_shared(format!("clip{}:{:.4}", c.id, f.pts), f.width, f.height, f.rgba.clone()).fit(ImageFit::Fill).placeholder(false);
                let mut b = DecoratedBox::new().child(img).style("opacity", alpha as f32);
                if !filter.is_empty() {
                    b = b.style("filter", StyleValue::String(filter.clone()));
                }
                if rot != 0.0 {
                    b = b.style("transform", StyleValue::String(format!("rotate({rot}deg)")));
                }
                Box::new(Positioned::new(b).at((w - iw) / 2.0 + x * w, (h - ih) / 2.0 + y * h).dimensions(iw, ih))
            }
            Source::Color { color } => {
                let col = syngui::mss::MssColor::parse(color).map(|m| Color::from_srgb(m.r, m.g, m.b, m.a as f32 / 255.0)).unwrap_or(Color::from_srgb(0, 0, 0, 1.0));
                Box::new(Positioned::new(DecoratedBox::new().style("background-color", col).style("opacity", alpha as f32)).at(0.0, 0.0).dimensions(w, h))
            }
            Source::Title { text, style } => title(text, style, lt, c.duration, alpha as f32, (x, y, k), w, h, scale),
        };
        stack = stack.child(layer);
    }
    Box::new(DecoratedBox::new().child(stack).style("width", w).style("height", h))
}

/// Титр: размер, цвет, положение, подложка; появление и исчезание (`in`/`out`:
/// fade, slide, pop, none) за `anim` секунд.
#[allow(clippy::too_many_arguments)]
fn title(text: &str, st: &Map<String, Value>, lt: f64, dur: f64, alpha: f32, tr: (f32, f32, f32), w: f32, h: f32, scale: f32) -> Box<dyn Widget> {
    let size = num(st, "size", (w.min(h) / scale) as f64 * 0.075) as f32 * scale * tr.2;
    let color = st.get("color").and_then(Value::as_str).and_then(syngui::mss::MssColor::parse).map(|m| Color::from_srgb(m.r, m.g, m.b, m.a as f32 / 255.0)).unwrap_or(Color::from_srgb(255, 255, 255, 1.0));
    let pos = st.get("position").and_then(Value::as_str).unwrap_or("bottom");
    let anim = num(st, "anim", 0.4).max(0.0);
    let kind_in = st.get("in").and_then(Value::as_str).unwrap_or("fade");
    let kind_out = st.get("out").and_then(Value::as_str).unwrap_or("fade");
    // фаза анимации: 0 — нет, 1 — полностью видно
    let pin = if anim > 0.0 { (lt / anim).clamp(0.0, 1.0) } else { 1.0 };
    let pout = if anim > 0.0 { ((dur - lt) / anim).clamp(0.0, 1.0) } else { 1.0 };
    let ease = |p: f64| 1.0 - (1.0 - p).powi(3);
    let (mut a, mut dy, mut k) = (alpha, 0.0f32, 1.0f32);
    for (kind, p) in [(kind_in, pin), (kind_out, pout)] {
        let e = ease(p) as f32;
        match kind {
            "fade" => a *= e,
            "slide" => {
                a *= e;
                dy += (1.0 - e) * size * 0.8;
            }
            "pop" => {
                a *= e;
                k *= 0.6 + 0.4 * e;
            }
            _ => {}
        }
    }
    let font = (size * k).max(1.0);
    let mut txt = Text::new(text.to_string()).max_lines(4).style("color", color).style("font-size", font).style("font-weight", 700.0f32).style("text-align", StyleValue::String("center".into()));
    if let Some(f) = st.get("font").and_then(Value::as_str) {
        txt = txt.style("font-family", StyleValue::String(f.to_string()));
    }
    let boxed = st.get("box").and_then(Value::as_bool).unwrap_or(false);
    let mut pill = DecoratedBox::new().child(txt).style("padding", font * 0.25);
    if boxed {
        pill = pill.style("background-color", Color::from_srgb(0, 0, 0, 0.55)).style("border-radius", font * 0.25);
    }
    let main = match pos {
        "top" => MainAxisAlignment::Start,
        "center" | "middle" => MainAxisAlignment::Center,
        _ => MainAxisAlignment::End,
    };
    let col = Column::new().main_axis_alignment(main).cross_axis_alignment(CrossAxisAlignment::Stretch).child(pill);
    let margin = h * 0.07;
    let inner = DecoratedBox::new().child(col).style("padding", margin).style("opacity", a);
    Box::new(Positioned::new(inner).at(tr.0 * w, tr.1 * h + dy).dimensions(w, h))
}

/// Кадры источников для момента `t`: по читателю на клип (подряд — быстро),
/// LUT клипа применяется здесь.
pub struct FrameSource {
    readers: HashMap<u64, syngui::video::FrameReader>,
    stills: HashMap<u64, syngui::video::RgbaFrame>,
    max: (u32, u32),
    /// Нейрофильтры клипов (`interpolate`, `upscale`) — при экспорте.
    pub neural: bool,
    interp: HashMap<u64, Option<super::neural::Interp>>,
    upscale: HashMap<u64, Option<super::neural::Upscale>>,
    /// Что не удалось (нет CUDA, моделей) — для отчёта экспорта.
    pub notes: Vec<String>,
}

impl FrameSource {
    /// Кадры вписываются в `max_w × max_h` (размер проекта или панели).
    pub fn new(max_w: u32, max_h: u32) -> Self {
        Self { readers: HashMap::new(), stills: HashMap::new(), max: (max_w, max_h), neural: false, interp: HashMap::new(), upscale: HashMap::new(), notes: Vec::new() }
    }

    pub fn frames(&mut self, p: &Project, t: f64) -> Frames {
        let mut out = Frames::new();
        for c in layers(p, t) {
            let Some(mid) = c.media() else { continue };
            let Some(m) = p.media(mid) else { continue };
            let path = m.path.display().to_string();
            let frame = if m.kind == crate::doc::MediaKind::Image {
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
                    if let Ok(cur) = r.frame_at(src) {
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
                if let (true, Some(e), Some(fr)) = (self.neural, effect(c, "upscale"), f.as_ref()) {
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
            if let Some(e) = effect(c, "lut") {
                if let Some(lp) = e.params.get("path").and_then(Value::as_str) {
                    if let Ok(lut) = super::lut::load(lp) {
                        let mut px = (*f.rgba).clone();
                        lut.apply(&mut px, num(&e.params, "strength", 1.0) as f32);
                        f.rgba = Arc::new(px);
                    }
                }
            }
            out.insert(c.id, f);
        }
        // читатели клипов, которых больше нет на таймлайне
        self.readers.retain(|id, _| p.find_clip(*id).is_some());
        out
    }
}
