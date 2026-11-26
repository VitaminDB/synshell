//! Рамка трансформации на просмотре: выделенный клип в кадре — рамка с
//! учётом положения, масштаба и поворота. Тянуть внутри — сдвиг, за угол —
//! масштаб, за кружок сверху — поворот; щелчок по кадру — выделить клип под
//! курсором. Положение с ключами — путь движения пунктиром. Пока тянут, кадр
//! перерисовывается с новыми значениями; по отпусканию — одна правка
//! (параметр с ключами — ключ в позиции, иначе эффект «Положение и масштаб»).

use std::sync::Mutex;

use serde_json::{json, Map, Value};
use syngui::containers::Positioned;
use syngui::prelude::*;
use syngui::widgets::*;

use super::{ed, St};
use crate::doc::{Clip, Project, Source, TrackKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    Move,
    Scale,
    Rotate,
}

/// Перетаскивание: клип, что тянут, исходная трансформация, начало и
/// текущая точка (пикс. проекта).
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    cid: u64,
    grab: Grab,
    base: Xf,
    from: (f32, f32),
    cur: (f32, f32),
}

/// Трансформация клипа: сдвиг (доли кадра), масштаб, поворот (°).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    x: f64,
    y: f64,
    k: f64,
    rot: f64,
}

/// Рамка клипа в пикселях проекта: центр, размер (с масштабом), поворот.
#[derive(Clone, Copy, Debug)]
struct Geo {
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    rot: f32,
}

impl Geo {
    /// Точка рамки (доли −0.5…0.5 по ширине и высоте) с поворотом.
    fn at(&self, u: f32, v: f32) -> (f32, f32) {
        let (s, c) = self.rot.to_radians().sin_cos();
        let (dx, dy) = (u * self.w, v * self.h);
        (self.cx + dx * c - dy * s, self.cy + dx * s + dy * c)
    }
    fn contains(&self, x: f32, y: f32) -> bool {
        let (s, c) = (-self.rot).to_radians().sin_cos();
        let (dx, dy) = (x - self.cx, y - self.cy);
        let (u, v) = (dx * c - dy * s, dx * s + dy * c);
        u.abs() <= self.w / 2.0 && v.abs() <= self.h / 2.0
    }
}

/// Точка мыши над кадром (пикс. просмотра).
static PT: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));
/// Начальное панорамирование просмотра при Alt+перетаскивании или перетаскивании пустого места в зуме.
static PAN_VIEW0: Mutex<Option<(f32, f32)>> = Mutex::new(None);

fn num(m: &Map<String, Value>, k: &str, d: f64) -> f64 {
    m.get(k).and_then(Value::as_f64).unwrap_or(d)
}

fn xf(c: &Clip) -> Xf {
    match c.effects.iter().find(|e| e.kind == "transform") {
        Some(e) => Xf { x: num(&e.params, "x", 0.0), y: num(&e.params, "y", 0.0), k: num(&e.params, "scale", 1.0), rot: num(&e.params, "rotation", 0.0) },
        None => Xf { x: 0.0, y: 0.0, k: 1.0, rot: 0.0 },
    }
}

/// Рамка клипа `c` (значения на момент `t`) в кадре проекта с учётом родителя.
fn geo(p: &Project, c: &Clip, t: f64) -> Option<Geo> {
    let (w, h) = (p.settings.width as f32, p.settings.height as f32);
    let wt = crate::render::compose::world_transform(p, c, t);
    let (cx, cy) = (w / 2.0 + wt.x * w, h / 2.0 + wt.y * h);
    let k = wt.scale;
    let (bw, bh, cy) = match &c.source {
        Source::Media { media } => {
            let m = p.media(*media)?;
            let (mw, mh) = (m.width.max(1) as f32, m.height.max(1) as f32);
            let fk = crate::render::compose::fit_k(c, w / mw, h / mh) * k;
            (mw * fk, mh * fk, cy)
        }
        Source::Color { .. } => (w * k, h * k, cy),
        Source::Shape { kind, style } => {
            let is_line = matches!(kind.as_str(), "line" | "arrow");
            let wf = num(style, "width", if is_line { 0.45 } else { 0.28 }).clamp(0.01, 2.0) as f32;
            let hf = num(style, "height", if kind == "line" { 0.06 } else if kind == "arrow" { 0.14 } else { 0.28 }).clamp(0.01, 2.0) as f32;
            ((wf * w * k).max(8.0), (hf * h * k).max(8.0), cy)
        }
        Source::Title { text, style } => {
            if style.get("template").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
                (w * k, h * k, cy)
            } else {
                // размер текста — по кеглю и числу знаков (точный обмер — в композиторе)
                let size = num(style, "size", (w.min(h) * 0.075) as f64) as f32 * k;
                let lines: Vec<&str> = text.lines().collect();
                let chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f32;
                let (tw, th) = ((chars * size * 0.56 + size * 0.5).min(w), lines.len().max(1) as f32 * size * 1.25 + size * 0.5);
                let margin = h * 0.07;
                let base = match style.get("position").and_then(Value::as_str).unwrap_or("bottom") {
                    "top" => margin + th / 2.0,
                    "center" | "middle" => h / 2.0,
                    _ => h - margin - th / 2.0,
                };
                (tw, th, base + wt.y * h)
            }
        }
    };
    Some(Geo { cx, cy, w: bw, h: bh, rot: wt.rot })
}

/// Видимые слои в момент `t` сверху вниз: (клип на момент, исходный клип).
fn visible(p: &Project, t: f64) -> Vec<(Clip, u64)> {
    let mut v: Vec<(Clip, u64)> = Vec::new();
    for tr in p.tracks.iter().filter(|tr| tr.kind == TrackKind::Video && !tr.hidden) {
        if let Some(c) = tr.clips.iter().find(|c| c.start <= t && t < c.end()) {
            v.push((c.at(t - c.start).into_owned(), c.id));
        }
    }
    v.reverse();
    v
}

/// Трансформация после перетаскивания (с учётом поворота/масштаба родителя).
fn dragged(p: &Project, c: &Clip, d: &Drag, g0: &Geo, t: f64) -> Xf {
    let (w, h) = (p.settings.width as f32, p.settings.height as f32);
    let (par_k, par_rot) = match c.parent.and_then(|pid| p.clip(pid)) {
        Some(pc) => {
            let par = crate::render::compose::world_transform(p, &pc.at((t - pc.start).clamp(0.0, pc.duration)), t);
            (par.scale.max(0.01), par.rot)
        }
        None => (1.0, 0.0),
    };
    let mut xf = d.base;
    match d.grab {
        Grab::Move => {
            let (dx, dy) = (d.cur.0 - d.from.0, d.cur.1 - d.from.1);
            let (s, cs) = (-par_rot).to_radians().sin_cos();
            let lx = (dx * cs - dy * s) / par_k;
            let ly = (dx * s + dy * cs) / par_k;
            xf.x = d.base.x + (lx / w) as f64;
            xf.y = d.base.y + (ly / h) as f64;
        }
        Grab::Scale => {
            let r0 = (d.from.0 - g0.cx).hypot(d.from.1 - g0.cy).max(1.0);
            let r1 = (d.cur.0 - g0.cx).hypot(d.cur.1 - g0.cy);
            xf.k = (d.base.k * (r1 / r0) as f64).clamp(0.05, 10.0);
        }
        Grab::Rotate => {
            let a0 = (d.from.1 - g0.cy).atan2(d.from.0 - g0.cx);
            let a1 = (d.cur.1 - g0.cy).atan2(d.cur.0 - g0.cx);
            let mut r = d.base.rot + (a1 - a0).to_degrees() as f64;
            // к 15° с шагом, если близко
            let snap = (r / 15.0).round() * 15.0;
            if (r - snap).abs() < 2.0 {
                r = snap;
            }
            xf.rot = ((r + 180.0).rem_euclid(360.0)) - 180.0;
        }
    }
    xf.x = (xf.x * 1000.0).round() / 1000.0;
    xf.y = (xf.y * 1000.0).round() / 1000.0;
    xf.k = (xf.k * 1000.0).round() / 1000.0;
    xf.rot = (xf.rot * 10.0).round() / 10.0;
    xf
}

/// Подставить трансформацию в клип проекта (предпросмотр): ключи этих
/// параметров на время перетаскивания убираются.
pub fn apply_preview(p: &mut Project, d: &Drag, t: f64) {
    let Some(c) = p.clip(d.cid).cloned() else { return };
    let ce = c.at(t - c.start).into_owned();
    let Some(g0) = geo(p, &ce, t) else { return };
    let x = dragged(p, &ce, d, &g0, t);
    let id = p.alloc();
    let cm = p.clip_mut(d.cid).unwrap();
    let eid = match cm.effects.iter().find(|e| e.kind == "transform") {
        Some(e) => e.id,
        None => {
            cm.effects.push(crate::doc::Effect { id, kind: "transform".into(), params: Map::new() });
            id
        }
    };
    for name in ["x", "y", "scale", "rotation"] {
        cm.keys.remove(&format!("e{eid}.{name}"));
    }
    let e = cm.effects.iter_mut().find(|e| e.id == eid).unwrap();
    e.params.insert("x".into(), json!(x.x));
    e.params.insert("y".into(), json!(x.y));
    e.params.insert("scale".into(), json!(x.k));
    e.params.insert("rotation".into(), json!(x.rot));
}

/// Рамка, ручки, путь и поверхность для мыши поверх кадра `fw × fh` (масштаб `k`).
pub fn overlay(st: St, p: &Project, t: f64, k: f32, fw: f32, fh: f32) -> Vec<Box<dyn Widget>> {
    let mut out: Vec<Box<dyn Widget>> = Vec::new();
    let sel = ed().selection.first().copied();
    let layers = visible(p, t);
    let target = sel.and_then(|id| layers.iter().find(|(_, src)| *src == id)).map(|(c, _)| c.clone());
    let (pw, ph) = (p.settings.width as f32, p.settings.height as f32);
    if let Some(c) = &target {
        if let Some(g) = geo(p, c, t) {
            // путь движения: центр по времени клипа, если сдвиг анимирован
            let base = p.clip(c.id).cloned();
            let mut path: Vec<(f32, f32)> = Vec::new();
            let mut dots: Vec<(f32, f32)> = Vec::new();
            if let Some(b) = &base {
                let eid = b.effects.iter().find(|e| e.kind == "transform").map(|e| e.id);
                if let Some(eid) = eid {
                    let (kx, ky) = (format!("e{eid}.x"), format!("e{eid}.y"));
                    if b.keys.contains_key(&kx) || b.keys.contains_key(&ky) {
                        for i in 0..=120 {
                            let lt = b.duration * i as f64 / 120.0;
                            if let Some(gg) = geo(p, &b.at(lt), b.start + lt) {
                                path.push((gg.cx * k, gg.cy * k));
                            }
                        }
                        let mut kt: Vec<f64> = b.keys.get(&kx).into_iter().chain(b.keys.get(&ky)).flatten().map(|k| k.t).collect();
                        kt.sort_by(f64::total_cmp);
                        kt.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
                        for lt in kt {
                            if let Some(gg) = geo(p, &b.at(lt), b.start + lt) {
                                dots.push((gg.cx * k, gg.cy * k));
                            }
                        }
                    }
                }
            }
            let corners: Vec<(f32, f32)> = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].iter().map(|(u, v)| {
                let (x, y) = g.at(*u, *v);
                (x * k, y * k)
            }).collect();
            let top = g.at(0.0, -0.5);
            let rot_h = {
                let (s, c) = g.rot.to_radians().sin_cos();
                let d = 26.0 / k;
                ((top.0 + d * s) * k, (top.1 - d * c) * k)
            };
            let accent = Color::from_srgb(96, 165, 250, 1.0);
            let painter = Canvas::new(move |cx, _| {
                if path.len() > 1 {
                    cx.set_color(Color::from_srgb(255, 213, 79, 0.7));
                    cx.set_stroke_width(1.5);
                    cx.draw_polyline(&path);
                    for (x, y) in &dots {
                        cx.fill_circle(*x, *y, 3.5);
                    }
                }
                cx.set_color(accent);
                cx.set_stroke_width(1.5);
                let mut ring = corners.clone();
                ring.push(corners[0]);
                cx.draw_polyline(&ring);
                let tc = ((corners[0].0 + corners[1].0) / 2.0, (corners[0].1 + corners[1].1) / 2.0);
                cx.draw_line(tc.0, tc.1, rot_h.0, rot_h.1);
                for (x, y) in &corners {
                    cx.set_color(Color::from_srgb(255, 255, 255, 1.0));
                    cx.fill_rect(x - 4.5, y - 4.5, 9.0, 9.0);
                    cx.set_color(accent);
                    cx.draw_rect(x - 4.5, y - 4.5, 9.0, 9.0);
                }
                cx.set_color(Color::from_srgb(255, 255, 255, 1.0));
                cx.fill_circle(rot_h.0, rot_h.1, 5.5);
                cx.set_color(accent);
                cx.stroke_circle(rot_h.0, rot_h.1, 5.5);
            })
            .size(fw, fh);
            out.push(Box::new(Positioned::new(painter).at(0.0, 0.0).dimensions(fw, fh)));
        }
    }
    // мышь: ручки, сдвиг, щелчок — выделить слой под курсором
    let pc = p.clone();
    let tgt = target.clone();
    let layers2 = layers.clone();
    let surface = GestureDetector::new()
        .on_click(move || {
            let (x, y) = *PT.lock().unwrap();
            let (x, y) = (x / k, y / k);
            let hit = layers2.iter().find(|(c, _)| geo(&pc, c, t).is_some_and(|g| g.contains(x, y))).map(|(_, id)| *id);
            match hit {
                Some(id) => {
                    st.exec("selection.set", json!({"ids": [id]}));
                }
                None => {
                    st.exec("selection.set", json!({"ids": []}));
                }
            }
        })
        .on_pan_start({
            let pc = p.clone();
            move |_| {
                *PAN_VIEW0.lock().unwrap() = None;
                let alt = syngui::input::current_modifiers().alt;
                if alt && st.viewer_zoom.get_untracked().is_some() {
                    *PAN_VIEW0.lock().unwrap() = Some(st.viewer_pan.get_untracked());
                    return;
                }
                let grab = if let Some(c) = &tgt {
                    if let Some(g) = geo(&pc, c, t) {
                        let (x, y) = *PT.lock().unwrap();
                        let (px, py) = (x / k, y / k);
                        let near = |a: (f32, f32)| (a.0 * k - x).hypot(a.1 * k - y) <= 10.0;
                        let top = g.at(0.0, -0.5);
                        let (s, cs) = g.rot.to_radians().sin_cos();
                        let d = 26.0 / k;
                        let rot_h = (top.0 + d * s, top.1 - d * cs);
                        let g_kind = if near(rot_h) {
                            Some(Grab::Rotate)
                        } else if [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].iter().any(|(u, v)| near(g.at(*u, *v))) {
                            Some(Grab::Scale)
                        } else if g.contains(px, py) {
                            Some(Grab::Move)
                        } else {
                            None
                        };
                        g_kind.map(|gk| (c.id, gk, xf(c), (px, py)))
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some((cid, gk, base, pt)) = grab {
                    st.xf_drag.set(Some(Drag { cid, grab: gk, base, from: pt, cur: pt }));
                } else if st.viewer_zoom.get_untracked().is_some() {
                    *PAN_VIEW0.lock().unwrap() = Some(st.viewer_pan.get_untracked());
                }
            }
        })
        .on_pan_update(move |u: syngui::widgets::PanUpdate| {
            if let Some((x0, y0)) = *PAN_VIEW0.lock().unwrap() {
                st.viewer_pan.set((x0 + u.total.x, y0 + u.total.y));
                return;
            }
            if let Some(mut d) = st.xf_drag.get_untracked() {
                let (x, y) = *PT.lock().unwrap();
                d.cur = (x / k, y / k);
                st.xf_drag.set(Some(d));
            }
        })
        .on_pan_end(move |_| {
            if PAN_VIEW0.lock().unwrap().take().is_some() {
                return;
            }
            finish(st);
        })
        .child(DecoratedBox::new().style("width", fw).style("height", fh));
    let surface = EventHook::new()
        .on_hover_at(move |p| {
            if let Some((p, _)) = p {
                *PT.lock().unwrap() = (p.x, p.y);
            }
        })
        .child(surface);
    out.push(Box::new(Positioned::new(surface).at(0.0, 0.0).dimensions(fw, fh)));
    let _ = (pw, ph);
    out
}

/// Отпустили: записать трансформацию одним шагом отмены.
fn finish(st: St) {
    let Some(d) = st.xf_drag.get_untracked() else { return };
    st.xf_drag.set(None);
    let (p, t) = {
        let e = ed();
        (e.project.clone(), e.playhead)
    };
    let Some(c) = p.clip(d.cid).cloned() else { return };
    let ce = c.at(t - c.start).into_owned();
    let Some(g0) = geo(&p, &ce, t) else { return };
    let x = dragged(&p, &ce, &d, &g0, t);
    if x == d.base {
        return;
    }
    let changed: Vec<(&str, f64)> = [("x", x.x, d.base.x), ("y", x.y, d.base.y), ("scale", x.k, d.base.k), ("rotation", x.rot, d.base.rot)].into_iter().filter(|(_, a, b)| (a - b).abs() > 1e-9).map(|(n, a, _)| (n, a)).collect();
    let eid = c.effects.iter().find(|e| e.kind == "transform").map(|e| e.id);
    ed().begin_group(match d.grab {
        Grab::Move => "Сдвиг",
        Grab::Scale => "Масштаб",
        Grab::Rotate => "Поворот",
    });
    let mut plain = Map::new();
    for (name, v) in changed {
        let keyed = eid.is_some_and(|e| c.keys.contains_key(&format!("e{e}.{name}")));
        if keyed {
            st.exec("key.set", json!({"id": d.cid, "param": format!("transform.{name}"), "value": v}));
        } else {
            plain.insert(name.into(), json!(v));
        }
    }
    if !plain.is_empty() {
        match eid {
            Some(e) => {
                st.exec("effect.set", json!({"id": e, "params": plain}));
            }
            None => {
                st.exec("effect.add", json!({"ids": [d.cid], "kind": "transform", "params": plain}));
            }
        }
    }
    ed().end_group();
    st.rev.set(ed().revision);
}
