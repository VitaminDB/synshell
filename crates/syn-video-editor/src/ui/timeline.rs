//! Таймлайн: линейка, дорожки, клипы, позиция воспроизведения.
//!
//! - Щелчок по клипу — выделить, по пустому месту — позиция.
//! - Перенос медиа и клипов: клип ложится, где отпустили; если место занято —
//!   на ближайшую свободную дорожку выше, а нет такой — на новую (как в
//!   CapCut, без затирания). Строки «+» над видео и под звуком — новая дорожка.
//!   Пока тянете — превью: клип на дорожке, куда он встанет (отдельный слой,
//!   поле с областью броска не пересоздаётся).
//! - Края клипа тянутся мышью (обрезка); видна полупрозрачная полная длина
//!   источника — обрезанное можно вернуть.

use serde_json::json;
use syngui::containers::Positioned;
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::DragData;

use super::{ed, St};
use crate::doc::{Project, Source, TrackKind};

const HEAD: f32 = 96.0;
const RULER: f32 = 24.0;
const ROW_V: f32 = 52.0;
const ROW_A: f32 = 38.0;
/// Строка «новая дорожка» над видео и под звуком.
const NEW_ROW: f32 = 22.0;
/// Ширина ручки обрезки на краю клипа, пикс.
const EDGE: f32 = 7.0;

fn row_h(k: TrackKind) -> f32 {
    if k == TrackKind::Video {
        ROW_V
    } else {
        ROW_A
    }
}

/// Строки поля сверху вниз.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RowKind {
    NewVideo,
    Track(usize),
    NewAudio,
}

/// Раскладка поля: (строка, верх, высота).
fn rows(p: &Project) -> Vec<(RowKind, f32, f32)> {
    let mut v = Vec::new();
    let mut y = RULER;
    v.push((RowKind::NewVideo, y, NEW_ROW));
    y += NEW_ROW;
    // видео сверху вниз: последняя дорожка — верхний слой
    let mut order: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Video).rev().collect();
    order.extend((0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Audio));
    for i in order {
        let h = row_h(p.tracks[i].kind);
        v.push((RowKind::Track(i), y, h));
        y += h;
    }
    v.push((RowKind::NewAudio, y, NEW_ROW));
    v
}

fn row_under(rows: &[(RowKind, f32, f32)], y: f32) -> Option<(RowKind, f32, f32)> {
    rows.iter().copied().find(|(_, top, h)| y >= *top && y < top + h)
}

/// Куда встанет клип.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dest {
    Track(u64),
    New(TrackKind),
}

/// Свободно ли на дорожке `[s, s+d)` (клип `except` не в счёт).
fn free(p: &Project, ti: usize, s: f64, d: f64, except: Option<u64>) -> bool {
    p.tracks[ti].clips.iter().all(|c| Some(c.id) == except || c.end() <= s + 1e-6 || c.start >= s + d - 1e-6)
}

/// Дорожка для клипа вида `kind` длиной `d` с `s`, брошенного на высоте `y`:
/// под курсором, если свободно; иначе ближайшая свободная выше; иначе новая.
fn resolve(p: &Project, kind: TrackKind, y: f32, s: f64, d: f64, except: Option<u64>) -> Dest {
    let r = rows(p);
    let hovered = row_under(&r, y).map(|x| x.0);
    match hovered {
        Some(RowKind::NewVideo) if kind == TrackKind::Video => return Dest::New(kind),
        Some(RowKind::NewAudio) if kind == TrackKind::Audio => return Dest::New(kind),
        _ => {}
    }
    let same: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == kind).collect();
    let start = match hovered {
        Some(RowKind::Track(i)) if p.tracks[i].kind == kind => same.iter().position(|x| *x == i).unwrap_or(0),
        _ => 0,
    };
    for &ti in same.iter().skip(start) {
        if free(p, ti, s, d, except) {
            return Dest::Track(p.tracks[ti].id);
        }
    }
    Dest::New(kind)
}

/// Превью броска: где нарисовать клип (поле), подпись, новая ли дорожка.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: String,
    new_track: bool,
}

/// Шаг делений линейки, с: чтобы подписи не слипались.
fn tick_step(zoom: f32) -> f64 {
    for s in [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0] {
        if s as f32 * zoom >= 70.0 {
            return s;
        }
    }
    1200.0
}

fn label(t: f64) -> String {
    let s = t.round() as i64;
    if t.fract().abs() > 1e-6 && t < 60.0 {
        format!("{t:.1}")
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn clip_name(p: &Project, c: &crate::doc::Clip) -> String {
    match &c.source {
        Source::Media { media } => p.media(*media).map(|m| m.name()).unwrap_or_default(),
        Source::Title { text, .. } => format!("T  {text}"),
        Source::Color { color } => color.clone(),
    }
}

pub fn view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let zoom = st.zoom.get();
        let e = ed();
        let p = &e.project;
        let layout = rows(p);
        let width = ((p.duration() as f32 + 30.0) * zoom).max(1200.0);
        let height = layout.last().map(|(_, t, h)| t + h).unwrap_or(RULER) + 8.0;

        // заголовки дорожек
        let mut heads = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("tl-corner").child(Text::new(crate::doc::tc(e.playhead)).class("tl-time")));
        for (row, _, h) in &layout {
            let (text, class) = match row {
                RowKind::NewVideo => ("+ видео".to_string(), "tl-head tl-head-new"),
                RowKind::NewAudio => ("+ звук".to_string(), "tl-head tl-head-new"),
                RowKind::Track(i) => {
                    let t = &p.tracks[*i];
                    let mut flags = String::new();
                    if t.muted {
                        flags.push_str(" 🔇");
                    }
                    if t.hidden {
                        flags.push_str(" ◌");
                    }
                    (format!("{}{flags}", t.name), if t.kind == TrackKind::Video { "tl-head tl-head-v" } else { "tl-head tl-head-a" })
                }
            };
            heads = heads.child(DecoratedBox::new().class(class).child(Text::new(text).max_lines(1).class("tl-head-text")).style("height", *h));
        }

        // поле: линейка, дорожки, клипы
        let mut field = Stack::new();
        field = field.child(Positioned::new(DecoratedBox::new().class("tl-ruler")).at(0.0, 0.0).dimensions(width, RULER));
        let step = tick_step(zoom);
        let mut t = 0.0;
        while (t as f32) * zoom < width {
            let x = t as f32 * zoom;
            field = field.child(Positioned::new(DecoratedBox::new().class("tl-tick")).at(x, RULER - 8.0).dimensions(1.0, 8.0));
            field = field.child(Positioned::new(Text::new(label(t)).class("tl-tick-text")).at(x + 3.0, 3.0));
            t += step;
        }
        for (row, y, h) in &layout {
            let (y, h) = (*y, *h);
            let i = match row {
                RowKind::Track(i) => *i,
                _ => {
                    field = field.child(Positioned::new(DecoratedBox::new().class("tl-row-new")).at(0.0, y).dimensions(width, h));
                    continue;
                }
            };
            let tr = &p.tracks[i];
            field = field.child(Positioned::new(DecoratedBox::new().class("tl-row")).at(0.0, y).dimensions(width, h));
            for c in &tr.clips {
                let class = match &c.source {
                    Source::Media { .. } => {
                        if tr.kind == TrackKind::Audio {
                            "clip clip-audio"
                        } else {
                            "clip clip-video"
                        }
                    }
                    Source::Title { .. } => "clip clip-title",
                    Source::Color { .. } => "clip clip-color",
                };
                let name = clip_name(p, c);
                let selected = e.selection.contains(&c.id);
                let mut marks = String::new();
                if !c.effects.is_empty() {
                    marks.push_str(&format!("  ✦{}", c.effects.len()));
                }
                if c.transition.is_some() {
                    marks.insert_str(0, "◇ ");
                }
                let w = (c.duration as f32 * zoom - 2.0).max(3.0);
                let body = DecoratedBox::new().class(class).class(if selected { "clip-selected" } else { "" }).child(Text::new(format!("{name}{marks}")).max_lines(1).elide(Elide::End).class("clip-text"));
                // клип можно тащить по таймлайну (payload — id клипа); щелчок — выделить
                let cid = c.id;
                let body = Draggable::new(DRAG_CLIP, cid.to_string()).label(name.clone()).on_click(move || {
                    st.exec("selection.set", json!({"ids": [cid]}));
                }).child(body);
                // где курсор на клипе — чтобы при переносе клип не прыгал началом к курсору
                let body = EventHook::new().on_hover_at(move |p| {
                    if let Some((p, _)) = p {
                        *GRAB.lock().unwrap() = (cid, p.x);
                    }
                }).child(body);
                let x = c.start as f32 * zoom + 1.0;
                field = field.child(Positioned::new(body).at(x, y + 3.0).dimensions(w, h - 6.0));
                // ручки обрезки на краях (поверх клипа — ловят перетаскивание первыми)
                for left in [true, false] {
                    let hx = if left { x } else { x + w - EDGE };
                    let handle = GestureDetector::new()
                        .cursor(syngui::input::CursorIcon::ColResize)
                        .on_pan_update(move |u: syngui::widgets::PanUpdate| st.trim.set(Some((cid, left, u.total.x))))
                        .on_pan_end(move |_| finish_trim(st))
                        .child(DecoratedBox::new().class("clip-edge"));
                    field = field.child(Positioned::new(handle).at(hx, y + 3.0).dimensions(EDGE.min(w / 2.0), h - 6.0));
                }
            }
        }
        // слой превью (бросок, обрезка) — свой Reactive, поле не перестраивается
        field = field.child(overlay(st));
        // позиция воспроизведения
        let px = e.playhead as f32 * zoom;
        field = field.child(Positioned::new(DecoratedBox::new().class("tl-playhead")).at(px, 0.0).dimensions(2.0, height));
        field = field.child(Positioned::new(DecoratedBox::new().class("tl-playhead-cap")).at(px - 5.0, 0.0).dimensions(12.0, 10.0));
        drop(e);

        let surface = GestureDetector::new()
            .on_click_with_bounds(move |pt: Point, b: Rect| click(st, pt.x - b.origin.x, pt.y - b.origin.y))
            .child(DecoratedBox::new().child(field).style("width", width).style("height", height));
        // бросок медиа из медиатеки или клипа: время и дорожка — по точке
        let surface = DropArea::new()
            .accept_types(vec![super::panels::DRAG_MEDIA.to_string(), DRAG_CLIP.to_string()])
            // над дорожками вместо призрака — клип нужной длины в месте броска
            .hide_ghost()
            .on_drag_over(move |info: syngui::widgets::overlay::DropInfo| hint(st, &info.data, info.local_position.x, info.local_position.y))
            .on_drag_leave(move || st.drop_hint.set(None))
            .on_drop_positioned(move |info: syngui::widgets::overlay::DropInfo| {
                st.drop_hint.set(None);
                on_drop(st, &info.data, info.local_position.x, info.local_position.y)
            })
            .child(surface);

        vec![Box::new(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .class("timeline")
                .child(DecoratedBox::new().child(heads).class("tl-heads").style("width", HEAD))
                .child(DecoratedBox::new().class("grow").child(ScrollView::new().horizontal().vertical().child(surface))),
        )]
    })
}

/// Превью поверх поля: клип в месте броска; при обрезке — полная длина
/// источника полупрозрачно и новые края.
fn overlay(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let mut out: Vec<Box<dyn Widget>> = Vec::new();
        if let Some(hn) = st.drop_hint.get() {
            let class = if hn.new_track { "clip clip-drop clip-drop-new" } else { "clip clip-drop" };
            out.push(Box::new(Positioned::new(DecoratedBox::new().class(class).child(Text::new(hn.label).max_lines(1).elide(Elide::End).class("clip-text"))).at(hn.x, hn.y).dimensions(hn.w, hn.h)));
        }
        if let Some((cid, left, dx)) = st.trim.get() {
            let zoom = st.zoom.get_untracked();
            let e = ed();
            let p = &e.project;
            let lay = rows(p);
            if let (Some(c), Some((ti, _))) = (p.clip(cid), p.find_clip(cid)) {
                if let Some((_, y, h)) = lay.iter().copied().find(|(r, _, _)| *r == RowKind::Track(ti)) {
                    let src_dur = c.media().and_then(|m| p.media(m)).map(|m| m.duration).filter(|d| *d > 0.0);
                    // полная длина источника — полупрозрачно
                    if let Some(sd) = src_dur {
                        let s0 = c.start - c.offset / c.speed;
                        let s1 = s0 + sd / c.speed;
                        out.push(Box::new(Positioned::new(DecoratedBox::new().class("clip-source")).at(s0 as f32 * zoom + 1.0, y + 3.0).dimensions(((s1 - s0) as f32 * zoom).max(3.0), h - 6.0)));
                    }
                    let (a, b) = trimmed(p, c, left, (dx / zoom) as f64);
                    out.push(Box::new(Positioned::new(DecoratedBox::new().class("clip clip-trim").child(Text::new(format!("{} — {}", crate::doc::tc(a), crate::doc::tc(b))).max_lines(1).class("clip-text"))).at(a as f32 * zoom + 1.0, y + 3.0).dimensions(((b - a) as f32 * zoom).max(3.0), h - 6.0)));
                }
            }
        }
        out
    })
}

/// Края клипа после сдвига края на `dt` секунд (в пределах источника и соседей).
fn trimmed(p: &Project, c: &crate::doc::Clip, left: bool, dt: f64) -> (f64, f64) {
    let src_dur = c.media().and_then(|m| p.media(m)).map(|m| m.duration).filter(|d| *d > 0.0);
    let (ti, _) = p.find_clip(c.id).unwrap();
    let prev_end = p.tracks[ti].clips.iter().filter(|o| o.id != c.id && o.end() <= c.start + 1e-6).map(|o| o.end()).fold(0.0, f64::max);
    let next_start = p.tracks[ti].clips.iter().filter(|o| o.id != c.id && o.start >= c.end() - 1e-6).map(|o| o.start).fold(f64::MAX, f64::min);
    let min_len = 1.0 / p.settings.fps.max(1.0);
    if left {
        let lo = (c.start - c.offset / c.speed).max(prev_end).max(0.0);
        (p.snap((c.start + dt).clamp(lo, c.end() - min_len)), c.end())
    } else {
        let hi = src_dur.map(|d| c.start + (d - c.offset) / c.speed).unwrap_or(f64::MAX).min(next_start);
        (c.start, p.snap((c.end() + dt).clamp(c.start + min_len, hi)))
    }
}

fn finish_trim(st: St) {
    let Some((cid, left, dx)) = st.trim.get_untracked() else { return };
    st.trim.set(None);
    let zoom = st.zoom.get_untracked();
    let args = {
        let e = ed();
        let Some(c) = e.project.clip(cid) else { return };
        let (a, b) = trimmed(&e.project, c, left, (dx / zoom) as f64);
        if left {
            json!({"id": cid, "start": a})
        } else {
            json!({"id": cid, "end": b})
        }
    };
    st.exec("clip.trim", args);
}

/// Щелчок по полю (координаты поля): клип под точкой — выделить; иначе
/// позиция воспроизведения и снятие выделения.
fn click(st: St, x: f32, y: f32) {
    let zoom = st.zoom.get_untracked();
    let t = (x / zoom).max(0.0) as f64;
    if y <= RULER {
        st.exec("playhead.set", json!({"at": t}));
        return;
    }
    let hit = {
        let e = ed();
        let p = &e.project;
        match row_under(&rows(p), y) {
            Some((RowKind::Track(i), _, _)) => p.tracks[i].clips.iter().find(|c| c.start <= t && t < c.end()).map(|c| c.id),
            _ => None,
        }
    };
    match hit {
        Some(id) => {
            st.exec("selection.set", json!({"ids": [id]}));
        }
        None => {
            st.exec("playhead.set", json!({"at": t}));
            st.exec("selection.set", json!({"ids": []}));
        }
    }
}

/// Клип и смещение курсора от его левого края при последнем наведении, пикс.
static GRAB: std::sync::Mutex<(u64, f32)> = std::sync::Mutex::new((0, 0.0));

/// Тип переноса «клип таймлайна» (payload — id клипа).
pub const DRAG_CLIP: &str = "syn-video-editor/clip";

/// Что переносится: вид дорожки, начало, длина, подпись, клип (при переносе клипа).
fn dragged(st: St, data: &DragData, x: f32) -> Option<(TrackKind, f64, f64, String, Option<u64>)> {
    let id = data.payload.trim().parse::<u64>().ok()?;
    let zoom = st.zoom.get_untracked();
    let e = ed();
    let p = &e.project;
    if data.drag_type == super::panels::DRAG_MEDIA {
        let m = p.media(id)?;
        let kind = if m.kind == crate::doc::MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
        let d = if m.duration > 0.0 { m.duration } else { 5.0 };
        Some((kind, p.snap((x / zoom).max(0.0) as f64), d, m.name(), None))
    } else if data.drag_type == DRAG_CLIP {
        let c = p.clip(id)?;
        let (ti, _) = p.find_clip(id)?;
        let (gid, gx) = *GRAB.lock().unwrap();
        let x = if gid == id { x - gx } else { x };
        Some((p.tracks[ti].kind, p.snap((x / zoom).max(0.0) as f64), c.duration, clip_name(p, c), Some(id)))
    } else {
        None
    }
}

/// Превью броска: клип на той дорожке, куда он действительно встанет.
fn hint(st: St, data: &DragData, x: f32, y: f32) {
    let Some((kind, s, d, label, except)) = dragged(st, data, x) else { return };
    let zoom = st.zoom.get_untracked();
    let next = {
        let e = ed();
        let p = &e.project;
        let lay = rows(p);
        let (ry, rh, new_track) = match resolve(p, kind, y, s, d, except) {
            Dest::Track(tid) => {
                let ti = p.tracks.iter().position(|t| t.id == tid).unwrap_or(0);
                let (_, ry, rh) = lay.iter().copied().find(|(r, _, _)| *r == RowKind::Track(ti)).unwrap_or((RowKind::NewVideo, RULER, NEW_ROW));
                (ry, rh, false)
            }
            Dest::New(k) => {
                let want = if k == TrackKind::Video { RowKind::NewVideo } else { RowKind::NewAudio };
                let (_, ry, rh) = lay.iter().copied().find(|(r, _, _)| *r == want).unwrap();
                (ry, rh, true)
            }
        };
        Hint { x: s as f32 * zoom + 1.0, y: ry + 2.0, w: (d as f32 * zoom).max(8.0), h: rh - 4.0, label, new_track }
    };
    if st.drop_hint.get_untracked().as_ref() != Some(&next) {
        st.drop_hint.set(Some(next));
    }
}

/// Бросок на таймлайн: медиа — новый клип, клип — перенос; дорожка — по `resolve`.
fn on_drop(st: St, data: &DragData, x: f32, y: f32) {
    let Some((kind, s, d, _, except)) = dragged(st, data, x) else { return };
    let dest = {
        let e = ed();
        resolve(&e.project, kind, y, s, d, except)
    };
    let tid = match dest {
        Dest::Track(t) => t,
        Dest::New(k) => match st.exec("track.add", json!({"kind": k.as_str()})) {
            Some(o) => o.data["id"].as_u64().unwrap_or(0),
            None => return,
        },
    };
    let Ok(id) = data.payload.trim().parse::<u64>() else { return };
    if data.drag_type == super::panels::DRAG_MEDIA {
        st.exec("clip.add", json!({"media": id, "at": s, "track": tid}));
    } else {
        st.exec("clip.move", json!({"id": id, "at": s, "track": tid}));
        st.exec("selection.set", json!({"ids": [id]}));
    }
}
