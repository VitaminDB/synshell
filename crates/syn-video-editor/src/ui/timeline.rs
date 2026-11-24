//! Таймлайн: линейка, дорожки, клипы, позиция воспроизведения. Щелчок по
//! клипу выделяет его, по линейке или пустому месту — ставит позицию.

use serde_json::json;
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::containers::Positioned;
use syngui::DragData;

use super::{ed, St};
use crate::doc::{Source, TrackKind};

const HEAD: f32 = 96.0;
const RULER: f32 = 24.0;
const ROW_V: f32 = 52.0;
const ROW_A: f32 = 38.0;

fn row_h(k: TrackKind) -> f32 {
    if k == TrackKind::Video {
        ROW_V
    } else {
        ROW_A
    }
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

pub fn view(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        let zoom = st.zoom.get();
        let e = ed();
        let p = &e.project;
        // видеодорожки сверху вниз: последняя — верхний слой
        let mut order: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Video).rev().collect();
        order.extend((0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Audio));
        let width = ((p.duration() as f32 + 30.0) * zoom).max(1200.0);
        let height = RULER + order.iter().map(|i| row_h(p.tracks[*i].kind)).sum::<f32>() + 8.0;

        // заголовки дорожек
        let mut heads = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("tl-corner").child(Text::new(crate::doc::tc(e.playhead)).class("tl-time")));
        for &i in &order {
            let t = &p.tracks[i];
            let mut flags = String::new();
            if t.muted {
                flags.push_str(" 🔇");
            }
            if t.hidden {
                flags.push_str(" ◌");
            }
            heads = heads.child(DecoratedBox::new().class(if t.kind == TrackKind::Video { "tl-head tl-head-v" } else { "tl-head tl-head-a" }).child(Text::new(format!("{}{flags}", t.name)).max_lines(1).class("tl-head-text")));
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
        let mut y = RULER;
        for &i in &order {
            let tr = &p.tracks[i];
            let h = row_h(tr.kind);
            field = field.child(Positioned::new(DecoratedBox::new().class("tl-row")).at(0.0, y).dimensions(width, h));
            for c in &tr.clips {
                let (class, name) = match &c.source {
                    Source::Media { media } => (if tr.kind == TrackKind::Audio { "clip clip-audio" } else { "clip clip-video" }, p.media(*media).map(|m| m.name()).unwrap_or_default()),
                    Source::Title { text, .. } => ("clip clip-title", format!("T  {text}")),
                    Source::Color { color } => ("clip clip-color", color.clone()),
                };
                let selected = e.selection.contains(&c.id);
                let fx = if c.effects.is_empty() { String::new() } else { format!("  ✦{}", c.effects.len()) };
                let w = (c.duration as f32 * zoom - 2.0).max(3.0);
                let clip = DecoratedBox::new().class(class).class(if selected { "clip-selected" } else { "" }).child(Text::new(format!("{name}{fx}")).max_lines(1).elide(Elide::End).class("clip-text"));
                // клип можно тащить по таймлайну (payload — id клипа); щелчок — выделить
                let cid = c.id;
                let clip = Draggable::new(DRAG_CLIP, cid.to_string()).label(name.clone()).on_click(move || {
                    st.exec("selection.set", json!({"ids": [cid]}));
                    st.toast.set(None);
                }).child(clip);
                // где курсор на клипе — чтобы при переносе клип не прыгал началом к курсору
                let clip = EventHook::new().on_hover_at(move |p| {
                    if let Some((p, _)) = p {
                        *GRAB.lock().unwrap() = (cid, p.x);
                    }
                }).child(clip);
                field = field.child(
                    Positioned::new(clip)
                    .at(c.start as f32 * zoom + 1.0, y + 3.0)
                    .dimensions(w, h - 6.0),
                );
            }
            y += h;
        }
        // превью броска: клип на дорожке под курсором
        if let Some((hx, hy, dur, label)) = st.drop_hint.get() {
            if let Some((row_y, row_h)) = row_at(&order, p, hy) {
                let x = (hx.max(0.0) / zoom).max(0.0) as f32 * zoom;
                field = field.child(Positioned::new(DecoratedBox::new().class("clip clip-drop").child(Text::new(label).max_lines(1).elide(Elide::End).class("clip-text"))).at(x + 1.0, row_y + 3.0).dimensions((dur as f32 * zoom).max(8.0), row_h - 6.0));
            }
        }
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

/// Щелчок по полю (координаты поля): клип под точкой — выделить; иначе
/// позиция воспроизведения (по линейке и дорожкам) и снятие выделения.
fn click(st: St, x: f32, y: f32) {
    let zoom = st.zoom.get_untracked();
    let t = (x / zoom).max(0.0) as f64;
    st.toast.set(None);
    if y <= RULER {
        st.exec("playhead.set", json!({"at": t}));
        return;
    }
    let hit = {
        let e = ed();
        let p = &e.project;
        let mut order: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Video).rev().collect();
        order.extend((0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Audio));
        let mut top = RULER;
        let mut hit = None;
        for i in order {
            let h = row_h(p.tracks[i].kind);
            if y >= top && y < top + h {
                hit = p.tracks[i].clips.iter().find(|c| c.start <= t && t < c.end()).map(|c| c.id);
                break;
            }
            top += h;
        }
        hit
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
    st.toast.set(None);
}

/// Клип и смещение курсора от его левого края при последнем наведении, пикс.
static GRAB: std::sync::Mutex<(u64, f32)> = std::sync::Mutex::new((0, 0.0));

/// Тип переноса «клип таймлайна» (payload — id клипа).
pub const DRAG_CLIP: &str = "syn-video-editor/clip";

/// Дорожка под точкой поля (`None` — над линейкой или ниже дорожек).
fn track_at(y: f32) -> Option<(u64, TrackKind)> {
    let e = ed();
    let p = &e.project;
    let mut order: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Video).rev().collect();
    order.extend((0..p.tracks.len()).filter(|i| p.tracks[*i].kind == TrackKind::Audio));
    let mut top = RULER;
    for i in order {
        let h = row_h(p.tracks[i].kind);
        if y >= top && y < top + h {
            return Some((p.tracks[i].id, p.tracks[i].kind));
        }
        top += h;
    }
    None
}

/// Бросок на таймлайн: медиа — новый клип, клип — перенос.
fn on_drop(st: St, data: &DragData, x: f32, y: f32) {
    let t = (x / st.zoom.get_untracked()).max(0.0) as f64;
    let track = track_at(y);
    let Ok(id) = data.payload.trim().parse::<u64>() else { return };
    if data.drag_type == super::panels::DRAG_MEDIA {
        let kind = ed().project.media(id).map(|m| m.kind);
        let want = if kind == Some(crate::doc::MediaKind::Audio) { TrackKind::Audio } else { TrackKind::Video };
        let mut args = json!({"media": id, "at": t});
        if let Some((tid, k)) = track.filter(|(_, k)| *k == want) {
            let _ = k;
            args["track"] = json!(tid);
        }
        st.exec("clip.add", args);
    } else if data.drag_type == DRAG_CLIP {
        let (gid, gx) = *GRAB.lock().unwrap();
        let t = if gid == id { ((x - gx) / st.zoom.get_untracked()).max(0.0) as f64 } else { t };
        let mut args = json!({"id": id, "at": t});
        let kind = {
            let e = ed();
            e.project.find_clip(id).map(|(ti, _)| e.project.tracks[ti].kind)
        };
        if let Some((tid, _)) = track.filter(|(_, k)| Some(*k) == kind) {
            args["track"] = json!(tid);
        }
        st.exec("clip.move", args);
        st.exec("selection.set", json!({"ids": [id]}));
    }
    st.toast.set(None);
}

/// Верх и высота дорожки под `y` поля.
fn row_at(order: &[usize], p: &crate::doc::Project, y: f32) -> Option<(f32, f32)> {
    let mut top = RULER;
    for &i in order {
        let h = row_h(p.tracks[i].kind);
        if y >= top && y < top + h {
            return Some((top, h));
        }
        top += h;
    }
    None
}

/// Превью броска: длина клипа и подпись; x — начало (с учётом места захвата клипа).
fn hint(st: St, data: &DragData, x: f32, y: f32) {
    let Ok(id) = data.payload.trim().parse::<u64>() else { return };
    let (dur, label, x) = {
        let e = ed();
        if data.drag_type == super::panels::DRAG_MEDIA {
            match e.project.media(id) {
                Some(m) => (if m.duration > 0.0 { m.duration } else { 5.0 }, m.name(), x),
                None => return,
            }
        } else {
            match e.project.clip(id) {
                Some(c) => {
                    let (gid, gx) = *GRAB.lock().unwrap();
                    let name = if c.name.is_empty() { c.media().and_then(|m| e.project.media(m)).map(|m| m.name()).unwrap_or_default() } else { c.name.clone() };
                    (c.duration, name, if gid == id { x - gx } else { x })
                }
                None => return,
            }
        }
    };
    let cur = st.drop_hint.get_untracked();
    let next = Some((x, y, dur, label));
    if cur != next {
        st.drop_hint.set(next);
    }
}
