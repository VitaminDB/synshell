//! Панель «Цвет» (F2): колёса «Тени / Средние / Света» (оттенок — точка в
//! круге, тянуть; яркость — ползунок; 2×щелчок — сброс), кривые — общая и
//! R/G/B (щелчок — точка, тянуть, 2×щелчок по точке — убрать), LUT клипа с
//! силой, сравнение «до/после» в просмотре. Всё — эффект `grade` клипа
//! (команды effect.add/effect.set, правки подряд — один шаг отмены).

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Map, Value};
use syngui::containers::Positioned;
use syngui::prelude::*;
use syngui::widgets::*;

use super::{ed, St};
use crate::doc::Clip;
use crate::render::grade::{eval_curve, points, wheel_color, Grade, CURVE_KEYS, WHEEL_KEYS};

const WHEEL: f32 = 132.0;
const PAD: f32 = 10.0;

/// Точка мыши над холстами панели (по имени холста).
static PT: Mutex<Option<HashMap<&'static str, (f32, f32)>>> = Mutex::new(None);

fn pt(name: &'static str) -> (f32, f32) {
    PT.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(name).copied())).unwrap_or((0.0, 0.0))
}

fn set_pt(name: &'static str, p: (f32, f32)) {
    if let Ok(mut g) = PT.lock() {
        g.get_or_insert_with(HashMap::new).insert(name, p);
    }
}

/// Что тянут на кривой: номер точки.
static CURVE_GRAB: Mutex<Option<usize>> = Mutex::new(None);

/// Клип для коррекции: первый выделенный с картинкой.
pub fn current() -> Option<Clip> {
    let e = ed();
    e.selection
        .iter()
        .filter_map(|id| e.project.find_clip(*id).map(|(ti, ci)| &e.project.tracks[ti].clips[ci]).filter(|_| e.project.find_clip(*id).is_some_and(|(ti, _)| e.project.tracks[ti].kind != crate::doc::TrackKind::Audio)))
        .next()
        .cloned()
}

fn grade_params(c: &Clip) -> (Option<u64>, Map<String, Value>) {
    c.effects.iter().find(|e| e.kind == "grade").map(|e| (Some(e.id), e.params.clone())).unwrap_or((None, Map::new()))
}

/// Записать параметры коррекции клипа (эффект появляется при первой правке).
fn set(st: St, cid: u64, params: Value) {
    let eid = ed().project.clip(cid).and_then(|c| c.effects.iter().find(|e| e.kind == "grade").map(|e| e.id));
    match eid {
        Some(eid) => {
            st.exec("effect.set", json!({"id": eid, "params": params}));
        }
        None => {
            st.exec("effect.add", json!({"ids": [cid], "kind": "grade", "params": params}));
        }
    }
}

fn r3(v: f32) -> f64 {
    ((v as f64) * 1000.0).round() / 1000.0
}

pub fn view(st: St) -> impl Widget {
    let body = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let ch = st.color_channel.get();
        let compare = st.color_compare.get();
        let Some(c) = current() else {
            return vec![Box::new(Center::new().child(Text::new("Выделите клип с картинкой: колёса «Тени / Средние / Света», кривые и LUT — первичная цветокоррекция").max_lines(3).class("panel-empty")))];
        };
        let (_, params) = grade_params(&c);
        let g = Grade::from_params(&params);
        let cid = c.id;
        let name = if c.name.trim().is_empty() { format!("Клип c{cid}") } else { format!("c{cid} · {}", c.name.trim()) };
        // узкая панель (телефон): всё столбцом с прокруткой, колёса по ширине
        let size = st.color_curve_size.get();
        let narrow = size.width < 700.0;
        let ws = if narrow { ((size.width - 16.0 - 2.0 * 14.0) / 3.0).clamp(72.0, WHEEL) } else { WHEEL };

        let title = Text::new(name).max_lines(1).class("cg-title");
        let mut head = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if !narrow {
            head = head.child(title).child(DecoratedBox::new().class("grow"));
        }
        let head = head
            .child(Button::new(if compare { "До | После ✓" } else { "До | После" }).class(if compare { "primary-btn" } else { "head-btn" }).on_click(move || st.color_compare.set(!st.color_compare.get_untracked())))
            .child(Button::new("Сбросить").class("head-btn").on_click(move || {
                if let Some(eid) = ed().project.clip(cid).and_then(|c| c.effects.iter().find(|e| e.kind == "grade").map(|e| e.id)) {
                    st.exec("effect.remove", json!({"id": eid}));
                }
            }));

        let mut wheels = Row::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Start);
        for (i, (key, label)) in WHEEL_KEYS.iter().zip(["Тени", "Средние", "Света"]).enumerate() {
            let w = [g.lift, g.gamma, g.gain][i];
            wheels = wheels.child(wheel(st, cid, key, label, w.x, w.y, w.l, ws));
        }

        let mut chips = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new("Кривая:").class("cv-hint"));
        for (i, l) in ["Яркость", "R", "G", "B"].iter().enumerate() {
            let on = i == ch;
            chips = chips.child(GestureDetector::new().cursor(syngui::input::CursorIcon::Pointer).on_click(move || st.color_channel.set(i)).child(DecoratedBox::new().class("cv-chip").class(if on { "cv-chip-on" } else { "" }).child(Text::new(*l).class("cv-chip-text"))));
        }
        if !narrow {
            chips = chips.child(Text::new("щелчок — точка, тянуть, 2×щелчок — убрать").class("cv-hint"));
        }

        let curve = curve_view(st, cid, ch, g.curves.clone(), narrow);

        // LUT клипа
        let lut = c.effects.iter().find(|e| e.kind == "lut").cloned();
        let mut lut_row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new("LUT").class("cg-label"));
        match lut {
            Some(e) => {
                let name = e.params.get("path").and_then(Value::as_str).and_then(|p| std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
                let s = e.params.get("strength").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                let eid = e.id;
                lut_row = lut_row
                    .child(Text::new(name).max_lines(1).class("cg-value"))
                    .child(Text::new("сила").class("cv-hint"))
                    .child(Slider::new().range(0.0, 1.0).value(s).width(140.0).on_change(move |v| {
                        st.exec("effect.set", json!({"id": eid, "params": {"strength": r3(v)}}));
                    }))
                    .child(Button::new("Убрать").class("head-btn").on_click(move || {
                        st.exec("effect.remove", json!({"id": eid}));
                    }));
            }
            None => lut_row = lut_row.child(Text::new("нет").class("cv-hint")),
        }
        lut_row = lut_row.child(DecoratedBox::new().class("grow")).child(Button::new("Подобрать LUT…").class("head-btn").on_click(move || st.open_panel("luts")));

        if narrow {
            let col = Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Text::new(if c.name.trim().is_empty() { format!("Клип c{cid}") } else { format!("c{cid} · {}", c.name.trim()) }).max_lines(1).class("cg-title"))
                .child(head)
                .child(wheels)
                .child(chips)
                .child(curve)
                .child(lut_row);
            return vec![Box::new(ScrollView::new().vertical().child(col))];
        }
        let _ = title;
        let left = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(wheels).child(lut_row);
        let right = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(chips).child(curve);
        vec![Box::new(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(head)
                .child(Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Start).child(left).child(right)),
        )]
    });
    // размер холста кривой — от размера панели (не от содержимого)
    EventHook::new()
        .fill()
        .on_resize(move |s| {
            let cur = st.color_curve_size.get_untracked();
            if (s.width - cur.width).abs() > 1.0 || (s.height - cur.height).abs() > 1.0 {
                st.color_curve_size.set(s);
            }
        })
        .child(DecoratedBox::new().class("color-panel").child(body))
}

/// Колесо: круг оттенков, точка, ползунок яркости.
fn wheel(st: St, cid: u64, key: &'static str, label: &'static str, x: f32, y: f32, l: f32, ws: f32) -> impl Widget {
    let r = ws / 2.0 - 6.0;
    let (cx0, cy0) = (ws / 2.0, ws / 2.0);
    let painter = Canvas::new(move |cx, _| {
        // круг оттенков: к краю насыщеннее
        for ri in 0..9 {
            let rr = r * (ri as f32 + 0.5) / 9.0;
            let steps = 12 + ri * 8;
            for a in 0..steps {
                let ang = a as f32 / steps as f32 * std::f32::consts::TAU;
                let (wx, wy) = (ang.cos() * rr / r, ang.sin() * rr / r);
                let c = wheel_color(wx, wy);
                cx.set_color(Color::from_srgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 0.9));
                cx.fill_circle(cx0 + wx * r, cy0 - wy * r, r / 9.0 * 0.9 + 1.5);
            }
        }
        cx.set_stroke_width(1.0);
        cx.set_color(Color::from_srgb(255, 255, 255, 0.25));
        cx.draw_line(cx0 - r, cy0, cx0 + r, cy0);
        cx.draw_line(cx0, cy0 - r, cx0, cy0 + r);
        // точка
        let (px, py) = (cx0 + x * r, cy0 - y * r);
        cx.set_color(Color::from_srgb(20, 22, 30, 1.0));
        cx.fill_circle(px, py, 7.0);
        cx.set_color(Color::from_srgb(255, 255, 255, 1.0));
        cx.fill_circle(px, py, 5.0);
    })
    .size(ws, ws);
    let to_wheel = move |(mx, my): (f32, f32)| {
        let (mut wx, mut wy) = ((mx - cx0) / r, -(my - cy0) / r);
        let d = wx.hypot(wy);
        if d > 1.0 {
            wx /= d;
            wy /= d;
        }
        (wx, wy)
    };
    let put = move |p: (f32, f32)| {
        let (wx, wy) = to_wheel(p);
        set(st, cid, json!({format!("{key}_x"): r3(wx), format!("{key}_y"): r3(wy)}));
    };
    let surface = GestureDetector::new()
        .cursor(syngui::input::CursorIcon::Crosshair)
        .on_double_click(move || set(st, cid, json!({format!("{key}_x"): 0.0, format!("{key}_y"): 0.0})))
        .on_pan_start(move |_| put(pt(key)))
        .on_pan_update(move |_| put(pt(key)))
        .child(DecoratedBox::new().style("width", ws).style("height", ws));
    let area = EventHook::new()
        .on_hover_at(move |p| {
            if let Some((p, _)) = p {
                set_pt(key, (p.x, p.y));
            }
        })
        .child(Stack::new().child(painter).child(Positioned::new(surface).at(0.0, 0.0).dimensions(ws, ws)));
    let info = format!("{label}  {:+.2}", l);
    Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(info).class("cg-label"))
        .child(area)
        .child(Slider::new().range(-1.0, 1.0).value(l).width(ws).on_change(move |v| set(st, cid, json!({key: r3(v)}))))
}

fn curve_color(ch: usize) -> Color {
    match ch {
        1 => Color::from_srgb(239, 68, 68, 1.0),
        2 => Color::from_srgb(74, 222, 128, 1.0),
        3 => Color::from_srgb(96, 165, 250, 1.0),
        _ => Color::from_srgb(230, 232, 240, 1.0),
    }
}

/// Холст кривой выбранного канала.
fn curve_view(st: St, cid: u64, ch: usize, curves: [Vec<[f32; 2]>; 4], narrow: bool) -> impl Widget {
    // панель: шапка ~34, колёса 3×(WHEEL+16) слева, чипы ~28, отступы
    let size = st.color_curve_size.get();
    let (side, h) = if narrow {
        // столбцом: на всю ширину, высота — как у квадрата, но не выше 220
        let w = (size.width - 20.0).max(140.0);
        (w, (w * 0.6).clamp(120.0, 220.0))
    } else {
        let w = (size.width - 3.0 * (WHEEL + 14.0) - 16.0 - 24.0).max(140.0);
        let h = (size.height - 34.0 - 28.0 - 16.0 - 26.0).clamp(90.0, 420.0);
        (w.min(h * 1.8).max(140.0), h)
    };
    let (iw, ih) = (side - 2.0 * PAD, h - 2.0 * PAD);
    let to_px = move |p: [f32; 2]| (PAD + p[0] * iw, PAD + (1.0 - p[1]) * ih);
    let from_px = move |(x, y): (f32, f32)| [((x - PAD) / iw).clamp(0.0, 1.0), (1.0 - (y - PAD) / ih).clamp(0.0, 1.0)];
    let pts = curves[ch].clone();
    let all = curves.clone();
    let painter = Canvas::new(move |cx, _| {
        cx.set_stroke_width(1.0);
        cx.set_color(Color::from_srgb(255, 255, 255, 0.07));
        for i in 0..=4 {
            let f = i as f32 / 4.0;
            cx.draw_line(PAD + f * iw, PAD, PAD + f * iw, PAD + ih);
            cx.draw_line(PAD, PAD + f * ih, PAD + iw, PAD + f * ih);
        }
        cx.set_color(Color::from_srgb(255, 255, 255, 0.18));
        cx.draw_line(PAD, PAD + ih, PAD + iw, PAD);
        // другие каналы — бледно
        for (k, c) in all.iter().enumerate() {
            if k == ch || c.is_empty() {
                continue;
            }
            let col = curve_color(k);
            cx.set_color(Color::from_srgb((col.r * 255.0) as u8, (col.g * 255.0) as u8, (col.b * 255.0) as u8, 0.35));
            let line: Vec<(f32, f32)> = (0..=100).map(|i| to_px([i as f32 / 100.0, eval_curve(c, i as f32 / 100.0)])).collect();
            cx.draw_polyline(&line);
        }
        cx.set_color(curve_color(ch));
        cx.set_stroke_width(2.0);
        let line: Vec<(f32, f32)> = (0..=200).map(|i| to_px([i as f32 / 200.0, eval_curve(&pts, i as f32 / 200.0)])).collect();
        cx.draw_polyline(&line);
        for p in &pts {
            let (x, y) = to_px(*p);
            cx.set_color(Color::from_srgb(20, 22, 30, 1.0));
            cx.fill_circle(x, y, 6.0);
            cx.set_color(curve_color(ch));
            cx.fill_circle(x, y, 4.5);
        }
    })
    .size(side, h);
    let key = CURVE_KEYS[ch];
    let base = curves[ch].clone();
    let near = move |pts: &[[f32; 2]], m: (f32, f32)| pts.iter().position(|p| {
        let (x, y) = to_px(*p);
        (x - m.0).hypot(y - m.1) <= 10.0
    });
    let write = move |pts: Vec<[f32; 2]>| {
        let v: Vec<Value> = pts.iter().map(|p| json!([r3(p[0]), r3(p[1])])).collect();
        set(st, cid, json!({key: v}));
    };
    let (b1, b2, b3) = (base.clone(), base.clone(), base.clone());
    let surface = GestureDetector::new()
        .cursor(syngui::input::CursorIcon::Crosshair)
        .on_click(move || {
            let m = pt("curve");
            if near(&b1, m).is_none() {
                let mut p = b1.clone();
                p.push(from_px(m));
                write(points(Some(&json!(p))));
            }
        })
        .on_double_click(move || {
            let m = pt("curve");
            if let Some(i) = near(&b2, m) {
                let mut p = b2.clone();
                p.remove(i);
                write(p);
            }
        })
        .on_pan_start(move |_| {
            let m = pt("curve");
            let i = near(&b3, m).or_else(|| {
                // тянуть с пустого места — новая точка
                let mut p = b3.clone();
                p.push(from_px(m));
                write(points(Some(&json!(p))));
                None
            });
            *CURVE_GRAB.lock().unwrap_or_else(|e| e.into_inner()) = i;
        })
        .on_pan_update(move |_| {
            let m = from_px(pt("curve"));
            let Some(c) = ed().project.clip(cid).cloned() else { return };
            let cur = points(grade_params(&c).1.get(key));
            let mut grab = CURVE_GRAB.lock().unwrap_or_else(|e| e.into_inner());
            let i = match *grab {
                Some(i) if i < cur.len() => i,
                // точка, поставленная в начале перетаскивания, — ближайшая
                _ => match cur.iter().enumerate().min_by(|a, b| ((a.1[0] - m[0]).abs()).total_cmp(&(b.1[0] - m[0]).abs())).map(|x| x.0) {
                    Some(i) => i,
                    None => return,
                },
            };
            let lo = if i > 0 { cur[i - 1][0] + 0.01 } else { 0.0 };
            let hi = if i + 1 < cur.len() { cur[i + 1][0] - 0.01 } else { 1.0 };
            let mut p = cur.clone();
            p[i] = [m[0].clamp(lo, hi.max(lo)), m[1]];
            *grab = Some(i);
            drop(grab);
            write(p);
        })
        .on_pan_end(move |_| *CURVE_GRAB.lock().unwrap_or_else(|e| e.into_inner()) = None)
        .child(DecoratedBox::new().style("width", side).style("height", h));
    let stack = Stack::new().child(painter).child(Positioned::new(surface).at(0.0, 0.0).dimensions(side, h));
    EventHook::new()
        .on_hover_at(move |p| {
            if let Some((p, _)) = p {
                set_pt("curve", (p.x, p.y));
            }
        })
        .child(DecoratedBox::new().class("cv-area").child(stack))
}
