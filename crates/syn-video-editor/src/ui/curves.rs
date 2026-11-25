//! Панель «Кривые»: график анимированного параметра выделенного клипа по его
//! времени. Ключи — точки: тянуть по времени и значению; участок под
//! позицией — сглаживание чипами (ровно, плавно, разгон, торможение, с
//! перелётом, пружина, отскок, скачком, своя кривая), у своей кривой —
//! ручки Безье. Щелчок по графику — позиция. Всё командами key.*.

use std::sync::Mutex;

use serde_json::json;
use syngui::containers::Positioned;
use syngui::prelude::*;
use syngui::widgets::*;

use super::{ed, St};
use crate::doc::{interp, parse_bezier, Clip, Key};

/// Что тянут: ключ (номер) или ручку Безье участка под позицией.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    Key(usize),
    H1,
    H2,
}

/// Перетаскивание: путь параметра, что тянут, точка мыши (пикс. холста).
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    path: String,
    grab: Grab,
    x: f32,
    y: f32,
}

/// Точка мыши над холстом.
static PT: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

const PL: f32 = 46.0;
const PR: f32 = 14.0;
const PT_: f32 = 14.0;
const PB: f32 = 20.0;

/// Подпись параметра по пути.
pub fn label(c: &Clip, path: &str) -> String {
    match path {
        "style.size" => return "Размер титра".into(),
        "style.color" => return "Цвет титра".into(),
        "style.fill" => return "Заливка".into(),
        "style.stroke" => return "Обводка".into(),
        "style.width" => return "Ширина фигуры".into(),
        "style.height" => return "Высота фигуры".into(),
        "style.radius" => return "Скругление".into(),
        "style.stroke_width" => return "Толщина обводки".into(),
        _ => {}
    }
    if let Some(pkey) = path.strip_prefix("style.p:") {
        if let crate::doc::Source::Title { style, .. } = &c.source {
            if let Some(tp) = style.get("template").and_then(serde_json::Value::as_str) {
                if let Ok(comp) = crate::fusion::load_cached(std::path::Path::new(tp)) {
                    if let Some(lbl) = comp.param_label(pkey) {
                        return lbl;
                    }
                }
            }
        }
        return pkey.into();
    }
    let Some((head, name)) = path.split_once('.') else { return path.into() };
    let kind = head.strip_prefix('e').and_then(|s| s.parse::<u64>().ok()).and_then(|id| c.effects.iter().find(|e| e.id == id)).map(|e| e.kind.as_str()).unwrap_or("");
    super::inspector::effect_params(kind).iter().find(|p| p.0 == name).map(|p| p.1.to_string()).unwrap_or_else(|| path.into())
}

/// Выделенный клип с ключами и выбранный путь.
fn current(st: St) -> Option<(Clip, String)> {
    let e = ed();
    let c = e.selection.first().and_then(|id| e.project.clip(*id))?.clone();
    if c.keys.is_empty() {
        return None;
    }
    let want = st.curve_param.get_untracked();
    let path = want.filter(|p| c.keys.contains_key(p)).unwrap_or_else(|| c.keys.keys().next().unwrap().clone());
    Some((c, path))
}

/// Участок под позицией: номер ключа в его начале.
fn segment(keys: &[Key], lt: f64) -> Option<usize> {
    if keys.len() < 2 {
        return None;
    }
    Some(keys.windows(2).position(|w| lt >= w[0].t && lt < w[1].t).unwrap_or(if lt < keys[0].t { 0 } else { keys.len() - 2 }))
}

/// Пересчёт холста: время ↔ x, значение ↔ y.
#[derive(Clone, Copy)]
struct Map {
    w: f32,
    h: f32,
    dur: f64,
    lo: f64,
    hi: f64,
}

impl Map {
    fn x(&self, t: f64) -> f32 {
        PL + (t / self.dur.max(1e-6)) as f32 * (self.w - PL - PR)
    }
    fn y(&self, v: f64) -> f32 {
        PT_ + (1.0 - ((v - self.lo) / (self.hi - self.lo).max(1e-9)) as f32) * (self.h - PT_ - PB)
    }
    fn t(&self, x: f32) -> f64 {
        (((x - PL) / (self.w - PL - PR).max(1.0)) as f64 * self.dur).clamp(0.0, self.dur)
    }
    fn v(&self, y: f32) -> f64 {
        self.lo + (1.0 - ((y - PT_) / (self.h - PT_ - PB).max(1.0)) as f64) * (self.hi - self.lo)
    }
}

/// Ключи с учётом текущего перетаскивания (предпросмотр до отпускания).
fn preview(keys: &[Key], m: &Map, lt: f64, fps: f64, d: &Drag) -> Vec<Key> {
    let mut k = keys.to_vec();
    let frame = 1.0 / fps.max(1.0);
    match d.grab {
        Grab::Key(i) if i < k.len() => {
            let lo = if i > 0 { k[i - 1].t + frame } else { 0.0 };
            let hi = if i + 1 < k.len() { k[i + 1].t - frame } else { m.dur };
            k[i].t = ((m.t(d.x).clamp(lo, hi.max(lo))) / frame).round() * frame;
            if k[i].v.is_number() {
                k[i].v = json!(m.v(d.y));
            }
        }
        Grab::H1 | Grab::H2 => {
            if let Some(i) = segment(&k, lt) {
                let (a, b) = (&k[i], &k[i + 1]);
                let (va, vb) = (a.v.as_f64().unwrap_or(0.0), b.v.as_f64().unwrap_or(1.0));
                let fx = ((m.t(d.x) - a.t) / (b.t - a.t).max(1e-9)).clamp(0.0, 1.0);
                let fy = if (vb - va).abs() > 1e-9 { (m.v(d.y) - va) / (vb - va) } else { 0.5 };
                let mut bz = parse_bezier(&a.ease).unwrap_or([0.25, 0.1, 0.25, 1.0]);
                if d.grab == Grab::H1 {
                    bz[0] = fx;
                    bz[1] = fy;
                } else {
                    bz[2] = fx;
                    bz[3] = fy;
                }
                k[i].ease = format!("bezier({:.3},{:.3},{:.3},{:.3})", bz[0], bz[1], bz[2], bz[3]);
            }
        }
        _ => {}
    }
    k
}

pub fn view(st: St) -> impl Widget {
    let size = use_signal(Size::new(600.0, 220.0));
    let head = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let _ = st.ph.get();
        let _ = st.curve_param.get();
        let Some((c, path)) = current(st) else { return Vec::new() };
        let lt = (ed().playhead - c.start).clamp(0.0, c.duration);
        let mut params = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for p in c.keys.keys() {
            let on = *p == path;
            let pc = p.clone();
            params = params.child(GestureDetector::new().cursor(syngui::input::CursorIcon::Pointer).on_click(move || st.curve_param.set(Some(pc.clone()))).child(DecoratedBox::new().class("cv-chip").class(if on { "cv-chip-on" } else { "" }).child(Text::new(label(&c, p)).class("cv-chip-text"))));
        }
        let keys = &c.keys[&path];
        let mut eases = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some(i) = segment(keys, lt) {
            let cur = keys[i].ease.clone();
            let at = c.start + keys[i].t;
            eases = eases.child(Text::new(format!("Участок {}–{}:", crate::doc::tc(c.start + keys[i].t), crate::doc::tc(c.start + keys[i + 1].t))).class("cv-hint"));
            let mut list: Vec<(String, String)> = crate::doc::EASES.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect();
            list.push((if cur.starts_with("bezier") { cur.clone() } else { "bezier(0.25,0.1,0.25,1)".into() }, "Своя".into()));
            for (k, l) in list {
                let on = k == cur || (k.starts_with("bezier") && cur.starts_with("bezier"));
                let (pp, kk) = (path.clone(), k.clone());
                let cid = c.id;
                eases = eases.child(GestureDetector::new().cursor(syngui::input::CursorIcon::Pointer).on_click(move || {
                    st.exec("key.set", json!({"id": cid, "param": pp, "at": at, "ease": kk}));
                }).child(DecoratedBox::new().class("cv-chip").class(if on { "cv-chip-on" } else { "" }).child(Text::new(l).class("cv-chip-text"))));
            }
        }
        vec![Box::new(Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Start).child(params).child(eases))]
    });
    let canvas = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let _ = st.ph.get();
        let _ = st.curve_param.get();
        let drag = st.curve_drag.get();
        let sz = size.get();
        let Some((c, path)) = current(st) else {
            return vec![Box::new(Center::new().child(Text::new("Выделите клип с ключевыми кадрами — ◇ у параметра в инспекторе или команда key.set").max_lines(3).class("panel-empty")))];
        };
        let fps = ed().project.settings.fps;
        let lt = (ed().playhead - c.start).clamp(0.0, c.duration);
        let base = c.keys[&path].clone();
        let numeric = base.iter().all(|k| k.v.is_number());
        // пределы значений по ключам и по самой кривой (перелёт, пружина)
        let span = |keys: &[Key]| {
            let mut lo = f64::MAX;
            let mut hi = f64::MIN;
            for i in 0..=200 {
                if let Some(v) = interp(keys, c.duration * i as f64 / 200.0).as_f64() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            for k in keys {
                if let Some(v) = k.v.as_f64() {
                    lo = lo.min(v);
                    hi = hi.max(v);
                }
            }
            if lo > hi {
                (0.0, 1.0)
            } else if (hi - lo).abs() < 1e-9 {
                (lo - 1.0, hi + 1.0)
            } else {
                let pad = (hi - lo) * 0.12;
                (lo - pad, hi + pad)
            }
        };
        let (lo, hi) = span(&base);
        let m = Map { w: sz.width, h: sz.height, dur: c.duration, lo, hi };
        let keys = match &drag {
            Some(d) if d.path == path => preview(&base, &m, lt, fps, d),
            _ => base.clone(),
        };
        let seg = segment(&keys, lt);
        let (w, h) = (sz.width, sz.height);
        let keys_c = keys.clone();
        let accent = Color::from_srgb(96, 165, 250, 1.0);
        let painter = Canvas::new(move |cx, _| {
            cx.set_stroke_width(1.0);
            cx.set_color(Color::from_srgb(255, 255, 255, 0.07));
            for i in 0..=4 {
                let y = PT_ + (h - PT_ - PB) * i as f32 / 4.0;
                cx.draw_line(PL, y, w - PR, y);
            }
            if numeric {
                // кривая
                let pts: Vec<(f32, f32)> = (0..=300).map(|i| {
                    let t = m.dur * i as f64 / 300.0;
                    (m.x(t), m.y(interp(&keys_c, t).as_f64().unwrap_or(0.0)))
                }).collect();
                cx.set_color(accent);
                cx.set_stroke_width(2.0);
                cx.draw_polyline(&pts);
                // ручки своей кривой у участка под позицией
                if let Some(i) = seg {
                    if let Some(bz) = parse_bezier(&keys_c[i].ease) {
                        let (a, b) = (&keys_c[i], &keys_c[i + 1]);
                        let (va, vb) = (a.v.as_f64().unwrap_or(0.0), b.v.as_f64().unwrap_or(0.0));
                        let p1 = (m.x(a.t + bz[0] * (b.t - a.t)), m.y(va + bz[1] * (vb - va)));
                        let p2 = (m.x(a.t + bz[2] * (b.t - a.t)), m.y(va + bz[3] * (vb - va)));
                        cx.set_stroke_width(1.0);
                        cx.set_color(Color::from_srgb(255, 213, 79, 0.8));
                        cx.draw_line(m.x(a.t), m.y(va), p1.0, p1.1);
                        cx.draw_line(m.x(b.t), m.y(vb), p2.0, p2.1);
                        cx.fill_circle(p1.0, p1.1, 4.5);
                        cx.fill_circle(p2.0, p2.1, 4.5);
                    }
                }
            } else {
                // цвет: полоса с переходом и точки ключей
                let y = h / 2.0;
                for i in 0..200 {
                    let t = m.dur * i as f64 / 200.0;
                    let col = interp(&keys_c, t).as_str().map(Color::from_hex).unwrap_or(Color::from_srgb(255, 255, 255, 1.0));
                    cx.set_color(col);
                    let x0 = m.x(t);
                    cx.fill_rect(x0, y - 10.0, (w - PL - PR) / 200.0 + 1.0, 20.0);
                }
            }
            for k in &keys_c {
                let y = if numeric { m.y(k.v.as_f64().unwrap_or(0.0)) } else { h / 2.0 };
                cx.set_color(Color::from_srgb(20, 22, 30, 1.0));
                cx.fill_circle(m.x(k.t), y, 6.0);
                cx.set_color(Color::from_srgb(255, 213, 79, 1.0));
                cx.fill_circle(m.x(k.t), y, 4.5);
            }
            // позиция
            cx.set_color(Color::from_srgb(239, 68, 68, 0.9));
            cx.set_stroke_width(1.5);
            cx.draw_line(m.x(lt), PT_ - 6.0, m.x(lt), h - PB + 4.0);
        })
        .size(w, h);
        let mut stack = Stack::new().child(painter);
        if numeric {
            for (v, y) in [(hi, PT_), (lo, h - PB)] {
                stack = stack.child(Positioned::new(Text::new(format!("{v:.2}")).class("cv-axis")).at(4.0, y - 7.0));
            }
        }
        stack = stack.child(Positioned::new(Text::new(format!("{} · {}", label(&c, &path), crate::doc::tc(c.duration))).class("cv-axis")).at(PL, h - PB + 4.0));
        // взаимодействие: ключи и ручки тянутся, щелчок — позиция
        let cid = c.id;
        let start = c.start;
        let path2 = path.clone();
        let base2 = base.clone();
        let surface = GestureDetector::new()
            .on_click(move || {
                let (x, _) = *PT.lock().unwrap();
                st.seek_live(start + m.t(x));
            })
            .on_pan_start(move |_| {
                let (x, y) = *PT.lock().unwrap();
                let near = |px: f32, py: f32| (px - x).hypot(py - y) <= 10.0;
                let mut grab = None;
                if let Some(i) = segment(&base2, lt) {
                    if let Some(bz) = parse_bezier(&base2[i].ease) {
                        let (a, b) = (&base2[i], &base2[i + 1]);
                        let (va, vb) = (a.v.as_f64().unwrap_or(0.0), b.v.as_f64().unwrap_or(0.0));
                        if near(m.x(a.t + bz[0] * (b.t - a.t)), m.y(va + bz[1] * (vb - va))) {
                            grab = Some(Grab::H1);
                        } else if near(m.x(a.t + bz[2] * (b.t - a.t)), m.y(va + bz[3] * (vb - va))) {
                            grab = Some(Grab::H2);
                        }
                    }
                }
                if grab.is_none() {
                    grab = base2.iter().position(|k| near(m.x(k.t), if k.v.is_number() { m.y(k.v.as_f64().unwrap_or(0.0)) } else { h / 2.0 })).map(Grab::Key);
                }
                if let Some(g) = grab {
                    st.curve_drag.set(Some(Drag { path: path2.clone(), grab: g, x, y }));
                }
            })
            .on_pan_update(move |_| {
                if let Some(mut d) = st.curve_drag.get_untracked() {
                    let (x, y) = *PT.lock().unwrap();
                    d.x = x;
                    d.y = y;
                    st.curve_drag.set(Some(d));
                }
            })
            .on_pan_end(move |_| finish(st, cid, start, m, lt, fps))
            .child(DecoratedBox::new().style("width", w).style("height", h));
        stack = stack.child(Positioned::new(surface).at(0.0, 0.0).dimensions(w, h));
        vec![Box::new(stack)]
    });
    let area = EventHook::new()
        .fill()
        .on_resize(move |s| size.set(s))
        .on_hover_at(move |p| {
            if let Some((p, _)) = p {
                *PT.lock().unwrap() = (p.x, p.y);
            }
        })
        .child(DecoratedBox::new().class("cv-area").child(canvas));
    Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("curves").child(head).child(DecoratedBox::new().class("grow").child(area))
}

/// Отпустили: ключ — key.move, ручка — новое сглаживание участка.
fn finish(st: St, cid: u64, start: f64, m: Map, lt: f64, fps: f64) {
    let Some(d) = st.curve_drag.get_untracked() else { return };
    st.curve_drag.set(None);
    let Some(c) = ed().project.clip(cid).cloned() else { return };
    let Some(base) = c.keys.get(&d.path).cloned() else { return };
    let moved = preview(&base, &m, lt, fps, &d);
    match d.grab {
        Grab::Key(i) if i < base.len() => {
            let (old, new) = (&base[i], &moved[i]);
            let mut args = json!({"id": cid, "param": d.path, "from": start + old.t, "to": start + new.t});
            if new.v != old.v {
                args["value"] = match new.v.as_f64() {
                    Some(v) => json!((v * 1000.0).round() / 1000.0),
                    None => new.v.clone(),
                };
            }
            if (new.t - old.t).abs() > 1e-9 || new.v != old.v {
                st.exec("key.move", args);
            }
        }
        Grab::H1 | Grab::H2 => {
            if let Some(i) = segment(&base, lt) {
                st.exec("key.set", json!({"id": cid, "param": d.path, "at": start + base[i].t, "ease": moved[i].ease}));
            }
        }
        _ => {}
    }
}

