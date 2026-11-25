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
//! - Выделение: щелчок, Ctrl/Shift+щелчок — добавить или убрать, рамка по
//!   пустому месту. Перенос выделенного клипа двигает всё выделение.
//! - Привязка (N): края клипов, позиция и начало притягивают перенос и обрезку.
//! - Масштаб: Ctrl+колесо (вокруг курсора), щипок, ползунок в шапке.

use serde_json::json;
use syngui::containers::Positioned;
use syngui::prelude::*;
use syngui::widgets::*;
use syngui::DragData;

use super::{ed, St};
use crate::doc::{Project, Source, TrackKind};

const HEAD: f32 = 150.0;
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
    /// Линия привязки (x поля).
    snap: Option<f32>,
    /// Остальные клипы переносимой группы (x, y, w, h).
    more: Vec<[f32; 4]>,
}

/// Порог привязки, пикс.
const SNAP_PX: f32 = 8.0;

/// Точки привязки: начало, позиция, края клипов (кроме переносимых).
pub struct Snapper {
    pts: Vec<f64>,
    tol: f64,
}

impl Snapper {
    fn new(st: St, e: &crate::editor::Editor, except: &[u64]) -> Self {
        if !st.snap.get_untracked() {
            return Self { pts: Vec::new(), tol: 0.0 };
        }
        let mut pts = vec![0.0, e.playhead];
        for t in &e.project.tracks {
            for c in t.clips.iter().filter(|c| !except.contains(&c.id)) {
                pts.push(c.start);
                pts.push(c.end());
            }
        }
        Self { pts, tol: (SNAP_PX / st.zoom.get_untracked().max(0.01)) as f64 }
    }

    /// Ближайшая точка к `t` в пределах порога: (сдвиг, точка).
    fn near(&self, t: f64) -> Option<(f64, f64)> {
        self.pts.iter().map(|p| (p - t, *p)).filter(|(d, _)| d.abs() <= self.tol).min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
    }

    /// Точка `t` с привязкой: (новое время, линия привязки).
    fn point(&self, t: f64) -> (f64, Option<f64>) {
        match self.near(t) {
            Some((_, p)) => (p, Some(p)),
            None => (t, None),
        }
    }

    /// Отрезок `[s, s+d)`: к точке тянется начало или конец — что ближе.
    fn span(&self, s: f64, d: f64) -> (f64, Option<f64>) {
        match (self.near(s), self.near(s + d)) {
            (Some(a), Some(b)) if b.0.abs() < a.0.abs() => (s + b.0, Some(b.1)),
            (Some(a), _) => (s + a.0, Some(a.1)),
            (None, Some(b)) => (s + b.0, Some(b.1)),
            (None, None) => (s, None),
        }
    }
}

/// Соседняя склейка до или после `t` (края клипов, начало, конец).
pub fn edit_point(p: &Project, t: f64, forward: bool) -> Option<f64> {
    let mut pts = vec![0.0, p.duration()];
    for tr in &p.tracks {
        for c in &tr.clips {
            pts.push(c.start);
            pts.push(c.end());
        }
    }
    let it = pts.into_iter();
    if forward {
        it.filter(|x| *x > t + 1e-6).min_by(f64::total_cmp)
    } else {
        it.filter(|x| *x < t - 1e-6).max_by(f64::total_cmp)
    }
}

/// Клипы, которые едут вместе с `id`: всё выделение, если в нём есть клипы
/// помимо `id` и его связанной пары (одиночный клип или одна связанная пара
/// переносятся через `clip.move`, чтобы можно было менять дорожку).
fn group(e: &crate::editor::Editor, id: u64) -> Vec<u64> {
    let peer = e.project.linked_peer(id);
    let only_pair = e.selection.iter().all(|&s| s == id || Some(s) == peer);
    if !only_pair && e.selection.contains(&id) {
        let mut out = Vec::with_capacity(e.selection.len() * 2);
        for &s in &e.selection {
            if !out.contains(&s) {
                out.push(s);
            }
            if let Some(lp) = e.project.linked_peer(s) {
                if !out.contains(&lp) {
                    out.push(lp);
                }
            }
        }
        out
    } else {
        vec![id]
    }
}

/// Ручка прокрутки поля: масштаб держит точку под курсором.
fn scroll() -> &'static ScrollHandle {
    static H: std::sync::OnceLock<ScrollHandle> = std::sync::OnceLock::new();
    H.get_or_init(ScrollHandle::new)
}

/// Подпись ползунка масштаба перестраивается, когда масштаб сменили не им.
fn zoom_ui() -> RwSignal<u64> {
    static S: std::sync::OnceLock<RwSignal<u64>> = std::sync::OnceLock::new();
    *S.get_or_init(|| use_signal(0u64))
}

const ZOOM_MIN: f32 = 4.0;
const ZOOM_MAX: f32 = 800.0;

/// Масштаб `z` пикс./с; точка `anchor` (пикс. от левого края видимой
/// области) остаётся на месте; без неё — позиция, если видна, иначе левый край.
pub fn zoom_to(st: St, z: f32, anchor: Option<f32>) {
    zoom_set(st, z, anchor, false);
}

fn zoom_set(st: St, z: f32, anchor: Option<f32>, from_slider: bool) {
    let z = z.clamp(ZOOM_MIN, ZOOM_MAX);
    let z0 = st.zoom.get_untracked();
    if (z - z0).abs() < 1e-3 {
        return;
    }
    let h = scroll();
    let (off, vw) = (h.offset(), h.viewport().width);
    let ax = anchor.unwrap_or_else(|| {
        let px = ed().playhead as f32 * z0 - off.x;
        if (0.0..=vw).contains(&px) {
            px
        } else {
            0.0
        }
    });
    let t = (off.x + ax) / z0;
    st.zoom.set(z);
    h.scroll_to(Point::new((t * z - ax).max(0.0), off.y));
    if !from_slider {
        zoom_ui().update(|v| *v += 1);
    }
}

/// Доля ползунка ↔ масштаб (логарифмически).
fn zoom_frac(z: f32) -> f32 {
    (z / ZOOM_MIN).ln() / (ZOOM_MAX / ZOOM_MIN).ln()
}

fn frac_zoom(f: f32) -> f32 {
    ZOOM_MIN * (ZOOM_MAX / ZOOM_MIN).powf(f.clamp(0.0, 1.0))
}

/// Шапка таймлайна: масштаб и привязка. Вне перестраиваемого поля —
/// ползунок не пересоздаётся посреди протяжки.
fn bar(st: St) -> impl Widget {
    let zoom_row = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = zoom_ui().get();
        let z = st.zoom.get_untracked();
        vec![Box::new(
            Row::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(ToolButton::new(super::icons::ZOOM_OUT).tooltip("Мельче (Ctrl+колесо)").on_click(move || zoom_to(st, st.zoom.get_untracked() / 1.5, None)))
                .child(Slider::new().range(0.0, 1.0).value(zoom_frac(z)).width(160.0).on_change(move |v| zoom_set(st, frac_zoom(v), None, true)))
                .child(ToolButton::new(super::icons::ZOOM_IN).tooltip("Крупнее (Ctrl+колесо)").on_click(move || zoom_to(st, st.zoom.get_untracked() * 1.5, None))),
        )]
    });
    let snap = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let on = st.snap.get();
        vec![Box::new(ToolButton::new(super::icons::SNAP).tooltip(if on { "Привязка включена (N)" } else { "Привязка выключена (N)" }).active(on).on_click(move || st.snap.update(|v| *v = !*v)))]
    });
    DecoratedBox::new().class("tl-bar").child(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new("S — разрез · Delete — удалить · Ctrl+C/V · Alt+←/→ — сдвиг").max_lines(1).elide(Elide::End).class("tl-bar-hint"))
            .child(DecoratedBox::new().class("grow"))
            .child(snap)
            .child(DecoratedBox::new().class("toolbar-sep"))
            .child(zoom_row),
    )
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
        Source::Shape { kind, .. } => format!("◆  {}", crate::doc::shape_title(kind)),
    }
}

pub fn view(st: St) -> impl Widget {
    let field_view = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.rev.get();
        // готовы ленты кадров или волны — перерисовать клипы
        let _ = st.thumbs.get();
        let zoom = st.zoom.get();
        let e = ed();
        let p = &e.project;
        let layout = rows(p);
        let width = ((p.duration() as f32 + 30.0) * zoom).max(1200.0);
        let height = layout.last().map(|(_, t, h)| t + h).unwrap_or(RULER) + 8.0;

        // заголовки дорожек
        let mut heads = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("tl-corner").child(Reactive::new(move || -> Vec<Box<dyn Widget>> {
            vec![Box::new(Text::new(crate::doc::tc(st.ph.get())).class("tl-time"))]
        })));
        let _ = st.rename_track.get();
        for (row, _, h) in &layout {
            let text = match row {
                RowKind::NewVideo => "+ видео",
                RowKind::NewAudio => "+ звук",
                RowKind::Track(i) => {
                    heads = heads.child(track_head(st, &p.tracks[*i], *h));
                    continue;
                }
            };
            heads = heads.child(DecoratedBox::new().class("tl-head tl-head-new").child(Text::new(text).max_lines(1).class("tl-head-text")).style("height", *h));
        }
        // заголовки принимают перенос дорожки: порядок
        let heads = DropArea::new().accept_types(vec![DRAG_TRACK.to_string()]).on_drop_positioned(move |info: syngui::widgets::overlay::DropInfo| drop_track(st, &info.data, info.local_position.y)).child(heads);

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
                    Source::Shape { .. } => "clip clip-shape",
                };
                let name = clip_name(p, c);
                let selected = e.selection.contains(&c.id);
                let mut marks = String::new();
                if let Some(pid) = c.parent {
                    marks.push_str(&format!("  ↳c{pid}"));
                }
                if p.linked_peer(c.id).is_some() {
                    marks.push_str("  ⇄");
                }
                if !c.blend.is_empty() && c.blend != "normal" {
                    marks.push_str(&format!("  [{}]", c.blend));
                }
                if !c.effects.is_empty() {
                    marks.push_str(&format!("  ✦{}", c.effects.len()));
                }
                let w = (c.duration as f32 * zoom - 2.0).max(3.0);
                let body = DecoratedBox::new().clip(true).child(clip_content(p, tr.kind, c, format!("{name}{marks}"), w, h - 6.0, zoom)).class(class).class(if selected { "clip-selected" } else { "" }).class(if (tr.hidden && tr.kind == TrackKind::Video) || (tr.muted && tr.kind == TrackKind::Audio) { "clip-off" } else { "" });
                // клип можно тащить по таймлайну (payload — id клипа); щелчок — выделить
                let cid = c.id;
                let body = Draggable::new(DRAG_CLIP, cid.to_string()).label(name.clone()).on_click(move || select_click(st, cid)).child(body);
                // где курсор на клипе — чтобы при переносе клип не прыгал началом к курсору
                let body = EventHook::new().on_hover_at(move |p| {
                    if let Some((p, _)) = p {
                        *GRAB.lock().unwrap() = (cid, p.x);
                    }
                }).child(body);
                let x = c.start as f32 * zoom + 1.0;
                field = field.child(Positioned::new(body).at(x, y + 3.0).dimensions(w, h - 6.0));
                // ключевые кадры — ромбики у нижнего края клипа
                let mut kt: Vec<f64> = c.keys.values().flatten().map(|k| k.t).filter(|t| (0.0..=c.duration).contains(t)).collect();
                kt.sort_by(f64::total_cmp);
                kt.dedup_by(|a, b| ((*a - *b) * zoom as f64).abs() < 6.0);
                for t in kt {
                    field = field.child(Positioned::new(Text::new("\u{25C6}").class("clip-key")).at(x + t as f32 * zoom - 5.0, y + h - 17.0));
                }
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
        // переходы: плашка на склейке, края тянутся (длительность), щелчок — в инспектор
        for (row, y, h) in &layout {
            let RowKind::Track(i) = row else { continue };
            for c in &p.tracks[*i].clips {
                let Some(t) = &c.transition else { continue };
                let cid = c.id;
                let x0 = ((c.start - t.duration / 2.0) as f32 * zoom).max(0.0);
                let tw = (t.duration as f32 * zoom).max(10.0);
                let label = t.title();
                let marker = GestureDetector::new()
                    .on_click(move || {
                        st.exec("selection.set", json!({"ids": [cid]}));
                        st.dock.activate("inspector");
                    })
                    .child(DecoratedBox::new().class("tl-transition").child(Center::new().child(Text::new(format!("◇ {label}")).max_lines(1).elide(Elide::End).class("tl-transition-text"))));
                let (my, mh) = (y + h * 0.5 - 11.0, 22.0);
                field = field.child(Positioned::new(marker).at(x0, my).dimensions(tw, mh));
                for left in [true, false] {
                    let hx = if left { x0 } else { x0 + tw - 6.0 };
                    let handle = GestureDetector::new()
                        .cursor(syngui::input::CursorIcon::ColResize)
                        .on_pan_update(move |u: syngui::widgets::PanUpdate| st.tr_edit.set(Some((cid, if left { -u.total.x } else { u.total.x }))))
                        .on_pan_end(move |_| finish_tr(st))
                        .child(DecoratedBox::new().class("tl-transition-edge"));
                    field = field.child(Positioned::new(handle).at(hx, my).dimensions(6.0, mh));
                }
            }
        }
        // протяжка по линейке: зажать и вести — позиция за курсором
        let ruler_scrub = EventHook::new()
            .on_hover_at(move |p| {
                if let Some((p, _)) = p {
                    *RULER_X.lock().unwrap() = p.x;
                    if SCRUB.load(std::sync::atomic::Ordering::Relaxed) {
                        st.seek_live((p.x / st.zoom.get_untracked()) as f64);
                    }
                }
            })
            .child(
                GestureDetector::new()
                    .cursor(syngui::input::CursorIcon::ColResize)
                    .on_pan_start(move |_| {
                        SCRUB.store(true, std::sync::atomic::Ordering::Relaxed);
                        st.seek_live((*RULER_X.lock().unwrap() / st.zoom.get_untracked()) as f64);
                    })
                    .on_pan_end(move |_| SCRUB.store(false, std::sync::atomic::Ordering::Relaxed))
                    .on_click(move || st.seek_live((*RULER_X.lock().unwrap() / st.zoom.get_untracked()) as f64))
                    .child(DecoratedBox::new().class("tl-ruler-hit")),
            );
        field = field.child(Positioned::new(ruler_scrub).at(0.0, 0.0).dimensions(width, RULER));
        // слой превью (бросок, обрезка) и позиция воспроизведения — свои Reactive,
        // поле не перестраивается (протяжка, воспроизведение)
        field = field.child(overlay(st));
        field = field.child(playhead(st, height));
        drop(e);

        // точка мыши над полем: за ней идёт перетаскиваемая головка курсора
        let field = EventHook::new()
            .on_hover_at(move |p| {
                if let Some((p, _)) = p {
                    *FIELD_X.lock().unwrap() = p.x;
                    *FIELD_Y.lock().unwrap() = p.y;
                    if PH_DRAG.load(std::sync::atomic::Ordering::Relaxed) {
                        st.seek_live((p.x / st.zoom.get_untracked()) as f64);
                    }
                    if let Some([x0, y0, _, _]) = st.band.get_untracked() {
                        st.band.set(Some([x0, y0, p.x, p.y]));
                    }
                }
            })
            .child(field);
        // по пустому месту: щелчок — позиция, протяжка — рамка выделения
        let surface = GestureDetector::new()
            .on_click_with_bounds(move |pt: Point, b: Rect| click(st, pt.x - b.origin.x, pt.y - b.origin.y))
            .on_pan_start(move |_| {
                let (x, y) = (*FIELD_X.lock().unwrap(), *FIELD_Y.lock().unwrap());
                if y > RULER {
                    st.band.set(Some([x, y, x, y]));
                }
            })
            .on_pan_end(move |_| finish_band(st))
            .on_secondary_click(move |pt: Point| open_menu(st, pt))
            // на телефоне — долгое нажатие
            .on_long_press(move |pt: Point| open_menu(st, pt))
            .child(DecoratedBox::new().child(field).style("width", width).style("height", height));
        // бросок медиа из медиатеки, шаблона из галереи или клипа: время и дорожка — по точке
        let surface = DropArea::new()
            .accept_types(vec![
                super::panels::DRAG_MEDIA.to_string(),
                super::templates::DRAG_TEMPLATE.to_string(),
                super::templates::DRAG_TRANSITION.to_string(),
                DRAG_CLIP.to_string(),
            ])
            // над дорожками вместо призрака — клип нужной длины в месте броска
            .hide_ghost()
            .on_drag_over(move |info: syngui::widgets::overlay::DropInfo| hint(st, &info.data, info.local_position.x, info.local_position.y))
            .on_drag_leave(move || st.drop_hint.set(None))
            .on_drop_positioned(move |info: syngui::widgets::overlay::DropInfo| {
                st.drop_hint.set(None);
                on_drop(st, &info.data, info.local_position.x, info.local_position.y)
            })
            .child(surface);

        // масштаб: Ctrl+колесо вокруг курсора, щипок
        let sv = ScrollView::new().both().handle(scroll().clone()).child(surface);
        let sv = GestureDetector::new()
            .on_pinch(move |u: syngui::widgets::PinchUpdate| {
                let mut z0 = PINCH_Z0.lock().unwrap();
                let base = *z0.get_or_insert(st.zoom.get_untracked());
                zoom_to(st, base * u.scale, Some(*VIEW_X.lock().unwrap()));
            })
            .on_pinch_end(move || *PINCH_Z0.lock().unwrap() = None)
            .child(sv);
        let sv = EventHook::new()
            .on_hover_at(move |p| {
                if let Some((p, _)) = p {
                    *VIEW_X.lock().unwrap() = p.x;
                }
            })
            .on_wheel(move |_dx, dy, _pos| {
                if !syngui::input::current_modifiers().ctrl || dy == 0.0 {
                    return false;
                }
                let k = 1.15f32.powf(dy / 40.0);
                zoom_to(st, st.zoom.get_untracked() * k, Some(*VIEW_X.lock().unwrap()));
                true
            })
            .child(sv);

        vec![Box::new(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .class("timeline")
                .child(DecoratedBox::new().child(heads).class("tl-heads").style("width", HEAD))
                .child(DecoratedBox::new().class("grow").child(sv)),
        )]
    });
    Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(bar(st)).child(DecoratedBox::new().class("grow").child(field_view))
}

/// Щелчок по клипу: выделить (со связанным звуком/видео, если не зажат Alt);
/// с Ctrl/Shift — добавить или убрать из выделения.
fn select_click(st: St, cid: u64) {
    let m = syngui::input::current_modifiers();
    let peer = if m.alt { None } else { ed().project.linked_peer(cid) };
    let ids = if m.ctrl || m.shift {
        let mut s = ed().selection.clone();
        match s.iter().position(|x| *x == cid) {
            Some(i) => {
                s.remove(i);
                if let Some(pid) = peer {
                    s.retain(|x| *x != pid);
                }
            }
            None => {
                s.push(cid);
                if let Some(pid) = peer {
                    if !s.contains(&pid) {
                        s.push(pid);
                    }
                }
            }
        }
        s
    } else if let Some(pid) = peer {
        vec![cid, pid]
    } else {
        vec![cid]
    };
    st.exec("selection.set", json!({"ids": ids}));
}

/// Рамка отпущена: выделить клипы, которые она задела (с Ctrl/Shift — добавить).
fn finish_band(st: St) {
    let Some([x0, y0, x1, y1]) = st.band.get_untracked() else { return };
    st.band.set(None);
    let (xa, xb, ya, yb) = (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1));
    if xb - xa < 3.0 && yb - ya < 3.0 {
        return;
    }
    let zoom = st.zoom.get_untracked();
    let m = syngui::input::current_modifiers();
    let ids: Vec<u64> = {
        let e = ed();
        let p = &e.project;
        let mut ids: Vec<u64> = if m.ctrl || m.shift { e.selection.clone() } else { Vec::new() };
        for (row, y, h) in rows(p) {
            let RowKind::Track(i) = row else { continue };
            if y + h < ya || y > yb {
                continue;
            }
            for c in &p.tracks[i].clips {
                let (cx0, cx1) = (c.start as f32 * zoom, c.end() as f32 * zoom);
                if cx1 >= xa && cx0 <= xb && !ids.contains(&c.id) {
                    ids.push(c.id);
                }
            }
        }
        ids
    };
    st.exec("selection.set", json!({"ids": ids}));
}

/// Точка мыши над видимой областью поля (для масштаба вокруг курсора).
static VIEW_X: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
static FIELD_Y: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
/// Масштаб в начале щипка.
static PINCH_Z0: std::sync::Mutex<Option<f32>> = std::sync::Mutex::new(None);

/// Превью поверх поля: клип в месте броска; при обрезке — полная длина
/// источника полупрозрачно и новые края.
fn overlay(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let mut out: Vec<Box<dyn Widget>> = Vec::new();
        let line_h = rows(&ed().project).last().map(|(_, t, h)| t + h).unwrap_or(RULER);
        if let Some(hn) = st.drop_hint.get() {
            let class = if hn.new_track { "clip clip-drop clip-drop-new" } else { "clip clip-drop" };
            out.push(Box::new(Positioned::new(DecoratedBox::new().class(class).child(Text::new(hn.label).max_lines(1).elide(Elide::End).class("clip-text"))).at(hn.x, hn.y).dimensions(hn.w, hn.h)));
            for [x, y, w, h] in hn.more {
                out.push(Box::new(Positioned::new(DecoratedBox::new().class("clip clip-drop")).at(x, y).dimensions(w, h)));
            }
            if let Some(sx) = hn.snap {
                out.push(Box::new(Positioned::new(DecoratedBox::new().class("tl-snap")).at(sx, 0.0).dimensions(1.0, line_h)));
            }
        }
        if let Some([x0, y0, x1, y1]) = st.band.get() {
            out.push(Box::new(Positioned::new(DecoratedBox::new().class("tl-band")).at(x0.min(x1), y0.min(y1)).dimensions((x1 - x0).abs().max(1.0), (y1 - y0).abs().max(1.0))));
        }
        if let Some((cid, dx)) = st.tr_edit.get() {
            let zoom = st.zoom.get_untracked();
            let e = ed();
            let p = &e.project;
            let lay = rows(p);
            if let (Some(c), Some((ti, _))) = (p.clip(cid), p.find_clip(cid)) {
                if let (Some(t), Some((_, y, h))) = (&c.transition, lay.iter().copied().find(|(r, _, _)| *r == RowKind::Track(ti))) {
                    let d = tr_duration(p, c, t.duration, (dx / zoom) as f64);
                    out.push(Box::new(Positioned::new(DecoratedBox::new().class("tl-transition tl-transition-edit").child(Center::new().child(Text::new(format!("{d:.2} с")).class("tl-transition-text")))).at(((c.start - d / 2.0) as f32 * zoom).max(0.0), y + h * 0.5 - 11.0).dimensions((d as f32 * zoom).max(10.0), 22.0)));
                }
            }
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
                    let (a, b, snap) = trimmed(p, c, left, (dx / zoom) as f64, &Snapper::new(st, &e, &[cid]));
                    if let Some(t) = snap {
                        out.push(Box::new(Positioned::new(DecoratedBox::new().class("tl-snap")).at(t as f32 * zoom, 0.0).dimensions(1.0, line_h)));
                    }
                    out.push(Box::new(Positioned::new(DecoratedBox::new().class("clip clip-trim").child(Text::new(format!("{} — {}", crate::doc::tc(a), crate::doc::tc(b))).max_lines(1).class("clip-text"))).at(a as f32 * zoom + 1.0, y + 3.0).dimensions(((b - a) as f32 * zoom).max(3.0), h - 6.0)));
                    // если есть связанный звук/видео — показать его синхронную обрезку
                    if let Some(pid) = p.linked_peer(cid) {
                        if let (Some(pc), Some((pti, _))) = (p.clip(pid), p.find_clip(pid)) {
                            if let Some((_, py, ph)) = lay.iter().copied().find(|(r, _, _)| *r == RowKind::Track(pti)) {
                                let pa = (pc.start + (a - c.start)).max(0.0);
                                let pb = (pc.end() + (b - c.end())).max(pa + 0.01);
                                out.push(Box::new(Positioned::new(DecoratedBox::new().class("clip clip-trim")).at(pa as f32 * zoom + 1.0, py + 3.0).dimensions(((pb - pa) as f32 * zoom).max(3.0), ph - 6.0)));
                            }
                        }
                    }
                }
            }
        }
        out
    })
}

/// Края клипа после сдвига края на `dt` секунд (в пределах источника и
/// соседей, с привязкой) и точка привязки.
fn trimmed(p: &Project, c: &crate::doc::Clip, left: bool, dt: f64, sn: &Snapper) -> (f64, f64, Option<f64>) {
    let edge = if left { c.start + dt } else { c.end() + dt };
    let (t, snap) = sn.point(edge);
    let (a, b) = trimmed_raw(p, c, left, t - if left { c.start } else { c.end() });
    let hit = snap.filter(|s| (if left { a } else { b } - s).abs() < 1e-6);
    (a, b, hit)
}

fn trimmed_raw(p: &Project, c: &crate::doc::Clip, left: bool, dt: f64) -> (f64, f64) {
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
        let (a, b, _) = trimmed(&e.project, c, left, (dx / zoom) as f64, &Snapper::new(st, &e, &[cid]));
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
        Some(id) => select_click(st, id),
        None => {
            st.exec("playhead.set", json!({"at": t}));
            let m = syngui::input::current_modifiers();
            if !m.ctrl && !m.shift {
                st.exec("selection.set", json!({"ids": []}));
            }
        }
    }
}

/// Клип и смещение курсора от его левого края при последнем наведении, пикс.
static GRAB: std::sync::Mutex<(u64, f32)> = std::sync::Mutex::new((0, 0.0));

/// Тип переноса «клип таймлайна» (payload — id клипа).
pub const DRAG_CLIP: &str = "syn-video-editor/clip";

/// Что переносится.
struct Dragged {
    kind: TrackKind,
    /// Начало (с привязкой).
    start: f64,
    dur: f64,
    label: String,
    /// Переносимый клип (при переносе клипа).
    clip: Option<u64>,
    /// Вся группа (выделение) — больше одного клипа: едут вместе.
    group: Vec<u64>,
    snap: Option<f64>,
    /// Видео из медиатеки со звуком — при броске займёт ещё и звуковую дорожку.
    with_audio: bool,
}

fn dragged(st: St, data: &DragData, x: f32) -> Option<Dragged> {
    let zoom = st.zoom.get_untracked();
    let e = ed();
    let p = &e.project;
    if data.drag_type == super::templates::DRAG_TEMPLATE {
        let path = std::path::Path::new(data.payload.trim());
        let name = path.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_else(|| "Шаблон".into());
        let d = super::templates::template_duration(path, p.settings.fps);
        let (s, snap) = Snapper::new(st, &e, &[]).span((x / zoom).max(0.0) as f64, d);
        return Some(Dragged {
            kind: TrackKind::Video,
            start: p.snap(s.max(0.0)),
            dur: d,
            label: format!("T  {name}"),
            clip: None,
            group: Vec::new(),
            snap,
            with_audio: false,
        });
    }
    let id = data.payload.trim().parse::<u64>().ok()?;
    if data.drag_type == super::panels::DRAG_MEDIA {
        let m = p.media(id)?;
        let kind = if m.kind == crate::doc::MediaKind::Audio { TrackKind::Audio } else { TrackKind::Video };
        let with_audio = m.kind == crate::doc::MediaKind::Video && m.has_audio;
        let d = if m.duration > 0.0 { m.duration } else { 5.0 };
        let (s, snap) = Snapper::new(st, &e, &[]).span((x / zoom).max(0.0) as f64, d);
        Some(Dragged { kind, start: p.snap(s.max(0.0)), dur: d, label: m.name(), clip: None, group: Vec::new(), snap, with_audio })
    } else if data.drag_type == DRAG_CLIP {
        let c = p.clip(id)?;
        let (ti, _) = p.find_clip(id)?;
        let (gid, gx) = *GRAB.lock().unwrap();
        let x = if gid == id { x - gx } else { x };
        let group = group(&e, id);
        let mut snap_except = group.clone();
        if let Some(pid) = p.linked_peer(id) {
            if !snap_except.contains(&pid) {
                snap_except.push(pid);
            }
        }
        let (mut s, snap) = Snapper::new(st, &e, &snap_except).span((x / zoom).max(0.0) as f64, c.duration);
        // группа не уходит левее нуля
        let lead = group.iter().filter_map(|g| p.clip(*g)).map(|g| c.start - g.start).fold(0.0, f64::max);
        s = s.max(lead);
        Some(Dragged { kind: p.tracks[ti].kind, start: p.snap(s.max(0.0)), dur: c.duration, label: clip_name(p, c), clip: Some(id), group, snap, with_audio: false })
    } else {
        None
    }
}

/// Ближайшая склейка клипов встык на видеодорожке для перетаскиваемого перехода Fusion:
/// `(clip_id, row_y, row_h, cut_time, max_dur)`.
fn nearest_cut(p: &Project, x_sec: f64, y: f32) -> Option<(u64, f32, f32, f64, f64)> {
    let lay = rows(p);
    let hover_ti = match row_under(&lay, y) {
        Some((RowKind::Track(ti), _, _)) => Some(ti),
        _ => None,
    };
    let mut best: Option<(f64, u64, f32, f32, f64, f64)> = None;
    for (row, ry, rh) in &lay {
        let RowKind::Track(ti) = *row else { continue };
        let tr = &p.tracks[ti];
        if tr.kind != TrackKind::Video {
            continue;
        }
        let penalty = if hover_ti == Some(ti) { 0.0 } else { 10.0 };
        for w in tr.clips.windows(2) {
            if (w[0].end() - w[1].start).abs() < 1e-3 {
                let cut_t = w[1].start;
                let max_d = w[0].duration.min(w[1].duration);
                let score = (cut_t - x_sec).abs() + penalty;
                if best.is_none_or(|(bs, ..)| score < bs) {
                    best = Some((score, w[1].id, *ry, *rh, cut_t, max_d));
                }
            }
        }
    }
    best.map(|(_, cid, ry, rh, cut_t, max_d)| (cid, ry, rh, cut_t, max_d))
}

/// Превью броска: клип на той дорожке, куда он действительно встанет.
fn hint(st: St, data: &DragData, x: f32, y: f32) {
    if data.drag_type == super::templates::DRAG_TRANSITION {
        let zoom = st.zoom.get_untracked();
        let next = {
            let e = ed();
            let p = &e.project;
            nearest_cut(p, (x / zoom).max(0.0) as f64, y).map(|(_, ry, rh, cut_t, max_d)| {
                let path = std::path::Path::new(data.payload.trim());
                let name = path.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_else(|| "Переход".into());
                let d = 0.8f64.min(max_d).max(1.0 / p.settings.fps.max(1.0));
                let x0 = ((cut_t - d / 2.0).max(0.0)) as f32 * zoom;
                let w = (d as f32 * zoom).max(20.0);
                Hint {
                    x: x0,
                    y: ry + rh * 0.5 - 12.0,
                    w,
                    h: 24.0,
                    label: format!("◇ {name}"),
                    new_track: false,
                    snap: Some(cut_t as f32 * zoom),
                    more: Vec::new(),
                }
            })
        };
        if st.drop_hint.get_untracked() != next {
            st.drop_hint.set(next);
        }
        return;
    }
    let Some(dr) = dragged(st, data, x) else { return };
    let (kind, s, d, label, except) = (dr.kind, dr.start, dr.dur, dr.label.clone(), dr.clip);
    let zoom = st.zoom.get_untracked();
    let snap = dr.snap.map(|t| t as f32 * zoom);
    // группа: каждый клип на своей дорожке со сдвигом
    if dr.group.len() > 1 {
        let next = {
            let e = ed();
            let p = &e.project;
            let lay = rows(p);
            let dt = s - except.and_then(|id| p.clip(id)).map(|c| c.start).unwrap_or(s);
            let mut rects = Vec::new();
            for g in &dr.group {
                let (Some(c), Some((ti, _))) = (p.clip(*g), p.find_clip(*g)) else { continue };
                let Some((_, ry, rh)) = lay.iter().copied().find(|(r, _, _)| *r == RowKind::Track(ti)) else { continue };
                rects.push([(c.start + dt) as f32 * zoom + 1.0, ry + 2.0, (c.duration as f32 * zoom).max(8.0), rh - 4.0]);
            }
            let Some(first) = rects.first().copied() else { return };
            Hint { x: first[0], y: first[1], w: first[2], h: first[3], label: format!("{} клипа · {:+.2} с", dr.group.len(), dt), new_track: false, snap, more: rects[1..].to_vec() }
        };
        if st.drop_hint.get_untracked().as_ref() != Some(&next) {
            st.drop_hint.set(Some(next));
        }
        return;
    }
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
        let mut more = Vec::new();
        if dr.with_audio {
            // превью звуковой дорожки для переносимого видео со звуком
            let (ay, ah) = match resolve(p, TrackKind::Audio, 0.0, s, d, None) {
                Dest::Track(tid) => {
                    let ti = p.tracks.iter().position(|t| t.id == tid).unwrap_or(0);
                    let (_, ay, ah) = lay.iter().copied().find(|(r, _, _)| *r == RowKind::Track(ti)).unwrap_or((RowKind::NewAudio, RULER, NEW_ROW));
                    (ay, ah)
                }
                Dest::New(_) => {
                    let (_, ay, ah) = lay.iter().copied().find(|(r, _, _)| *r == RowKind::NewAudio).unwrap();
                    (ay, ah)
                }
            };
            more.push([s as f32 * zoom + 1.0, ay + 2.0, (d as f32 * zoom).max(8.0), ah - 4.0]);
        } else if let Some(cid) = except {
            // превью связанного парного клипа (видео ↔ звук)
            if let Some(pid) = p.linked_peer(cid) {
                if let (Some(c), Some(pc), Some((pti, _))) = (p.clip(cid), p.clip(pid), p.find_clip(pid)) {
                    if let Some((_, py, ph)) = lay.iter().copied().find(|(r, _, _)| *r == RowKind::Track(pti)) {
                        let dt = s - c.start;
                        more.push([((pc.start + dt).max(0.0)) as f32 * zoom + 1.0, py + 2.0, (pc.duration as f32 * zoom).max(8.0), ph - 4.0]);
                    }
                }
            }
        }
        Hint { x: s as f32 * zoom + 1.0, y: ry + 2.0, w: (d as f32 * zoom).max(8.0), h: rh - 4.0, label, new_track, snap, more }
    };
    if st.drop_hint.get_untracked().as_ref() != Some(&next) {
        st.drop_hint.set(Some(next));
    }
}

/// Бросок на таймлайн: медиа — новый клип, клип — перенос; дорожка — по `resolve`.
fn on_drop(st: St, data: &DragData, x: f32, y: f32) {
    if data.drag_type == super::templates::DRAG_TRANSITION {
        let zoom = st.zoom.get_untracked();
        let hit = {
            let e = ed();
            nearest_cut(&e.project, (x / zoom).max(0.0) as f64, y)
        };
        let Some((cid, _, _, cut_t, _)) = hit else {
            st.toast.set(Some("Перетащите переход на склейку двух клипов встык на видеодорожке".into()));
            return;
        };
        if st.exec("transition.set", json!({"id": cid, "kind": "fusion", "template": data.payload.trim()})).is_some() {
            st.exec("selection.set", json!({"ids": [cid]}));
            st.seek_live(cut_t);
            st.dock.activate("inspector");
        }
        return;
    }
    let Some(dr) = dragged(st, data, x) else { return };
    let (kind, s, d, except) = (dr.kind, dr.start, dr.dur, dr.clip);
    // группа едет по времени, каждый клип на своей дорожке
    if dr.group.len() > 1 {
        let c0 = except.and_then(|id| ed().project.clip(id).map(|c| c.start));
        if let Some(c0) = c0 {
            if (s - c0).abs() > 1e-6 {
                st.exec("clip.shift", json!({"ids": dr.group, "by": s - c0}));
            }
        }
        return;
    }
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
    if data.drag_type == super::templates::DRAG_TEMPLATE {
        let path = std::path::Path::new(data.payload.trim());
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(o) = st.exec("template.apply", json!({"name": name, "path": data.payload.trim(), "at": s, "duration": d, "track": tid})) {
            if let Some(cid) = o.data.get("id").and_then(|v| v.as_u64()) {
                st.exec("selection.set", json!({"ids": [cid]}));
                st.dock.activate("inspector");
            }
        }
        return;
    }
    let Ok(id) = data.payload.trim().parse::<u64>() else { return };
    if data.drag_type == super::panels::DRAG_MEDIA {
        st.exec("clip.add", json!({"media": id, "at": s, "track": tid}));
    } else {
        st.exec("clip.move", json!({"id": id, "at": s, "track": tid}));
        let sel = match ed().project.linked_peer(id) {
            Some(pid) => vec![id, pid],
            None => vec![id],
        };
        st.exec("selection.set", json!({"ids": sel}));
    }
}

/// Протяжка по линейке идёт; последняя точка курсора над линейкой (пикс. поля).
static SCRUB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static RULER_X: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
/// Головку курсора тянут; точка мыши над полем (пикс.).
static PH_DRAG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FIELD_X: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);

/// Курсор позиции: линия и головка; головку и линию можно тащить.
fn playhead(st: St, height: f32) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let px = st.ph.get() as f32 * st.zoom.get_untracked();
        let grab = GestureDetector::new()
            .cursor(syngui::input::CursorIcon::ColResize)
            .on_pan_start(move |_| {
                PH_DRAG.store(true, std::sync::atomic::Ordering::Relaxed);
                st.seek_live((*FIELD_X.lock().unwrap() / st.zoom.get_untracked()) as f64);
            })
            .on_pan_end(move |_| PH_DRAG.store(false, std::sync::atomic::Ordering::Relaxed))
            .child(DecoratedBox::new().class("tl-playhead-hit"));
        vec![
            Box::new(Positioned::new(DecoratedBox::new().class("tl-playhead")).at(px, 0.0).dimensions(2.0, height)),
            Box::new(Positioned::new(DecoratedBox::new().class("tl-playhead-cap")).at(px - 5.0, 0.0).dimensions(12.0, 12.0)),
            // зона захвата шире линии: головка и вся линия
            Box::new(Positioned::new(grab).at(px - 6.0, 0.0).dimensions(14.0, height)),
        ]
    })
}

/// Содержимое блока клипа: раскадровка (видеодорожка) или волна (звуковая
/// дорожка — в том числе звук, вынесенный из видеофайла), подпись поверх.
fn clip_content(p: &Project, track_kind: TrackKind, c: &crate::doc::Clip, label: String, w: f32, h: f32, zoom: f32) -> Box<dyn Widget> {
    let mut stack = Stack::new().fit(StackFit::Expand);
    if let Some(m) = c.media().and_then(|id| p.media(id)) {
        if track_kind == TrackKind::Audio || m.kind == crate::doc::MediaKind::Audio {
            if let Some(pk) = super::wave::peaks(&m.path).filter(|v| !v.is_empty()) {
                let (offset, speed) = (c.offset, c.speed);
                let canvas = Canvas::new(move |cx, _| {
                    cx.set_color(Color::from_srgb(255, 255, 255, 0.55));
                    cx.set_stroke_width(1.0);
                    let mid = h / 2.0;
                    let mut x = 0.0f32;
                    while x < w {
                        // пик на отрезке источника под этими двумя пикселями
                        let a = ((offset + (x / zoom) as f64 * speed) * super::wave::RATE) as usize;
                        let b = ((offset + ((x + 2.0) / zoom) as f64 * speed) * super::wave::RATE) as usize;
                        let v = pk.get(a..b.max(a + 1).min(pk.len())).map(|s| s.iter().copied().fold(0.0, f32::max)).unwrap_or(0.0);
                        let half = (v * (h / 2.0 - 2.0)).max(0.5);
                        cx.draw_line(x, mid - half, x, mid + half);
                        x += 2.0;
                    }
                })
                .size(w, h);
                stack = stack.child(canvas);
            }
        } else if let Some(strip) = super::thumbs::strip(&m.path).filter(|s| !s.is_empty()) {
            // кадр по высоте блока, по времени источника в его левом крае
            let f0 = &strip[0];
            let tw = (h * f0.width as f32 / f0.height.max(1) as f32).max(12.0);
            let n = ((w / tw).ceil() as usize).clamp(1, 400);
            let mut row = Row::new().gap(0.0);
            for i in 0..n {
                let src = c.offset + (i as f64 * tw as f64 / zoom as f64) * c.speed;
                let k = if m.duration > 0.0 { ((src / m.duration) * strip.len() as f64) as usize } else { 0 }.min(strip.len() - 1);
                let f = &strip[k];
                row = row.child(DecoratedBox::new().child(Image::from_rgba_shared(format!("thumb:{}:{k}", m.id), f.width, f.height, f.rgba.clone()).fit(ImageFit::Cover).placeholder(false)).style("width", tw).style("height", h).class("clip-frame"));
            }
            stack = stack.child(row);
        }
    }
    stack = stack.child(Column::new().cross_axis_alignment(CrossAxisAlignment::Start).child(DecoratedBox::new().class("clip-label").child(Text::new(label).max_lines(1).elide(Elide::End).class("clip-text"))));
    Box::new(stack)
}

/// Длительность перехода после сдвига края на `dt` с (край тянет обе стороны:
/// переход по центру склейки); не длиннее половины соседних клипов ×2.
fn tr_duration(p: &Project, c: &crate::doc::Clip, d0: f64, dt: f64) -> f64 {
    let (ti, ci) = p.find_clip(c.id).unwrap();
    let prev = ci.checked_sub(1).map(|j| p.tracks[ti].clips[j].duration).unwrap_or(0.0);
    let max = prev.min(c.duration).max(0.05);
    let fps = p.settings.fps.max(1.0);
    (((d0 + 2.0 * dt).clamp(1.0 / fps, max)) * fps).round() / fps
}

fn finish_tr(st: St) {
    let Some((cid, dx)) = st.tr_edit.get_untracked() else { return };
    st.tr_edit.set(None);
    let zoom = st.zoom.get_untracked();
    let args = {
        let e = ed();
        let Some(c) = e.project.clip(cid) else { return };
        let Some(t) = &c.transition else { return };
        let new_d = tr_duration(&e.project, c, t.duration, (dx / zoom) as f64);
        if let Some(tpl) = &t.template {
            json!({"id": cid, "kind": t.kind, "template": tpl, "duration": new_d})
        } else {
            json!({"id": cid, "kind": t.kind, "duration": new_d})
        }
    };
    st.exec("transition.set", args);
    st.exec("selection.set", json!({"ids": [cid]}));
}

// ── контекстное меню ────────────────────────────────────────────────────

/// Правый щелчок по полю (точка окна): клип под курсором выделяется (если он
/// не в выделении), меню — по нему; по пустому месту — меню места.
fn open_menu(st: St, at_window: Point) {
    let (x, y) = (*FIELD_X.lock().unwrap(), *FIELD_Y.lock().unwrap());
    let zoom = st.zoom.get_untracked();
    let t = (x / zoom).max(0.0) as f64;
    let hit = {
        let e = ed();
        let p = &e.project;
        match row_under(&rows(p), y) {
            Some((RowKind::Track(i), _, _)) => p.tracks[i].clips.iter().find(|c| c.start <= t && t < c.end()).map(|c| (c.id, e.selection.contains(&c.id))),
            _ => None,
        }
    };
    match hit {
        Some((id, selected)) => {
            if !selected {
                let sel = match ed().project.linked_peer(id) {
                    Some(pid) => vec![id, pid],
                    None => vec![id],
                };
                st.exec("selection.set", json!({"ids": sel}));
            }
            st.menu_at.set(MenuAt::Field(id, t));
        }
        None => st.menu_at.set(MenuAt::Field(0, t)),
    }
    st.menu_pos.set(at_window);
    st.menu_open.set(true);
}

const SPEEDS: &[f64] = &[0.25, 0.5, 1.0, 1.5, 2.0, 4.0];

fn menu_items(st: St) -> Vec<MenuItem> {
    use super::icons;
    let cid = match st.menu_at.get() {
        MenuAt::Field(c, _) => c,
        MenuAt::Track(tid) => return track_items(tid),
    };
    let e = ed();
    let p = &e.project;
    let paste = MenuItem::new("paste", "Вставить сюда").shortcut("Ctrl+V").disabled(e.clipboard.is_empty());
    let Some(c) = (cid != 0).then(|| p.clip(cid)).flatten() else {
        let shapes: Vec<MenuItem> = crate::doc::SHAPE_KINDS.iter().map(|(k, l)| MenuItem::new(format!("shape:{k}"), *l)).collect();
        return vec![
            paste,
            MenuItem::new("title", "Титр здесь").icon(icons::TITLE),
            MenuItem::new("shape", "Фигура здесь").icon(icons::SHAPE).children(shapes),
            MenuItem::new("split_all", "Разрезать всё здесь").icon(icons::CUT),
        ];
    };
    let n = e.selection.len().max(1);
    let many = if n > 1 { format!(" ({n})") } else { String::new() };
    let media = c.media().and_then(|m| p.media(m));
    let is_audio_track = p.find_clip(cid).map(|(ti, _)| p.tracks[ti].kind == TrackKind::Audio).unwrap_or(false);
    let linked_peer = p.linked_peer(cid);
    let has_sound = (media.is_some_and(|m| m.has_audio || m.kind == crate::doc::MediaKind::Audio) && c.volume > 0.0)
        || linked_peer.and_then(|pid| p.clip(pid)).is_some_and(|pc| pc.volume > 0.0);
    let mut v = vec![
        MenuItem::new("split_here", "Разрезать здесь").icon(icons::CUT),
        MenuItem::new("split_ph", "Разрезать по позиции").shortcut("S"),
        MenuItem::separator(),
        MenuItem::new("copy", format!("Копировать{many}")).shortcut("Ctrl+C"),
        paste,
        MenuItem::new("delete", format!("Удалить{many}")).shortcut("Delete"),
        MenuItem::new("ripple", format!("Удалить со сдвигом{many}")).shortcut("Shift+Delete"),
        MenuItem::separator(),
    ];
    if !is_audio_track {
        let mut tr: Vec<MenuItem> = crate::render::compose::TRANSITIONS.iter().map(|(k, l)| MenuItem::new(format!("tr:{k}"), *l)).collect();
        if c.transition.is_some() {
            tr.push(MenuItem::separator());
            tr.push(MenuItem::new("tr:-", "Убрать переход"));
        }
        v.push(MenuItem::new("tr", "Переход в начале").children(tr));
    }
    let kinds: Vec<&str> = if is_audio_track {
        vec!["audio_fade_in", "audio_fade_out"]
    } else {
        let mut k = vec!["fade_in", "fade_out", "color", "blur", "sharpen", "vignette", "transform", "mask", "glass"];
        if media.is_some() {
            k.extend(["upscale", "interpolate"]);
        }
        if has_sound {
            k.extend(["audio_fade_in", "audio_fade_out"]);
        }
        k
    };
    v.push(MenuItem::new("fx", format!("Эффект{many}")).children(kinds.iter().map(|k| MenuItem::new(format!("fx:{k}"), super::inspector::effect_title(k))).collect()));
    if media.is_some() {
        v.push(MenuItem::new("speed", "Скорость").children(SPEEDS.iter().map(|s| MenuItem::new(format!("speed:{s}"), format!("{s}×{}", if (c.speed - s).abs() < 1e-6 { "  ✓" } else { "" }))).collect()));
    }
    if linked_peer.is_some() {
        v.push(MenuItem::new("detach", "Отвязать звук").icon(super::icons::AUDIO));
    } else if !is_audio_track && media.is_some_and(|m| m.kind == crate::doc::MediaKind::Video && m.has_audio) && c.volume > 0.0 {
        v.push(MenuItem::new("detach", "Отделить звук").icon(super::icons::AUDIO));
    }
    v.push(MenuItem::separator());
    v.push(MenuItem::new("props", "Свойства…"));
    v
}

fn menu_run(st: St, id: &str) {
    st.menu_open.set(false);
    let (cid, t) = match st.menu_at.get_untracked() {
        MenuAt::Field(c, t) => (c, t),
        MenuAt::Track(tid) => return track_run(st, tid, id),
    };
    let sel = ed().selection.clone();
    let ids = if sel.is_empty() && cid != 0 { vec![cid] } else { sel };
    match id {
        "paste" => {
            st.exec("clip.paste", json!({"at": t}));
        }
        "title" => {
            st.exec("title.add", json!({"text": "Заголовок", "at": t}));
        }
        "split_all" => {
            let under: Vec<u64> = ed().project.tracks.iter().flat_map(|tr| tr.clips.iter()).filter(|c| c.start < t - 1e-6 && c.end() > t + 1e-6).map(|c| c.id).collect();
            if !under.is_empty() {
                st.exec("clip.split", json!({"at": t, "ids": under}));
            }
        }
        "split_here" => {
            st.exec("clip.split", json!({"at": t, "ids": [cid]}));
        }
        "split_ph" => {
            st.exec("clip.split", json!({}));
        }
        "copy" => {
            st.exec("clip.copy", json!({"ids": ids}));
        }
        "delete" => {
            st.exec("clip.remove", json!({"ids": ids}));
        }
        "ripple" => {
            st.exec("clip.remove", json!({"ids": ids, "ripple": true}));
        }
        "detach" => {
            st.exec("clip.detach_audio", json!({"ids": [cid]}));
        }
        "props" => {
            st.exec("selection.set", json!({"ids": [cid]}));
            st.dock.activate("inspector");
        }
        "tr:-" => {
            st.exec("transition.remove", json!({"id": cid}));
        }
        _ => {
            if let Some(k) = id.strip_prefix("shape:") {
                st.exec("shape.add", json!({"kind": k, "at": t}));
            } else if let Some(k) = id.strip_prefix("tr:") {
                let d = ed().project.clip(cid).and_then(|c| c.transition.as_ref().map(|t| t.duration)).unwrap_or(0.6);
                st.exec("transition.set", json!({"id": cid, "kind": k, "duration": d}));
            } else if let Some(k) = id.strip_prefix("fx:") {
                st.exec("effect.add", json!({"ids": ids, "kind": k}));
                st.dock.activate("inspector");
            } else if let Some(s) = id.strip_prefix("speed:").and_then(|s| s.parse::<f64>().ok()) {
                let mut seen = Vec::new();
                for c in &ids {
                    let peer_seen = ed().project.linked_peer(*c).is_some_and(|p| seen.contains(&p));
                    if !peer_seen {
                        seen.push(*c);
                        st.exec("clip.set", json!({"id": c, "speed": s}));
                    }
                }
            }
        }
    }
}

/// Одно меню на окно (вне перестраиваемого поля): правка проекта не
/// закрывает открытое меню.
pub fn menu(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let _ = st.menu_at.get();
        let items = menu_items(st);
        vec![Box::new(PopupMenu::new().items(items).is_open(st.menu_open).position(st.menu_pos).min_width(230.0).on_select(move |id: &str| menu_run(st, id)))]
    })
}

/// По чему открыто контекстное меню.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAt {
    /// Поле: клип (0 — пустое место) и время.
    Field(u64, f64),
    /// Заголовок дорожки.
    Track(u64),
}

fn track_items(tid: u64) -> Vec<MenuItem> {
    use super::icons;
    let e = ed();
    let p = &e.project;
    let Some(ti) = p.tracks.iter().position(|t| t.id == tid) else { return Vec::new() };
    let t = &p.tracks[ti];
    let same: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == t.kind).collect();
    // выше на таймлайне: у видео — позже в списке
    let (top, bottom) = if t.kind == TrackKind::Video { (same.last() == Some(&ti), same.first() == Some(&ti)) } else { (same.first() == Some(&ti), same.last() == Some(&ti)) };
    let mut v = vec![
        MenuItem::new("tk:rename", "Переименовать"),
        MenuItem::new("tk:mute", if t.muted { "Включить звук" } else { "Выключить звук" }).icon(if t.muted { icons::VOLUME_UP } else { icons::VOLUME_OFF }),
    ];
    if t.kind == TrackKind::Video {
        v.push(MenuItem::new("tk:hide", if t.hidden { "Показать" } else { "Скрыть" }).icon(if t.hidden { icons::VISIBLE } else { icons::HIDDEN }));
    }
    v.extend([
        MenuItem::separator(),
        MenuItem::new("tk:up", "Выше").disabled(top),
        MenuItem::new("tk:down", "Ниже").disabled(bottom),
        MenuItem::separator(),
        MenuItem::new("tk:remove", if t.clips.is_empty() { "Удалить дорожку".to_string() } else { format!("Удалить дорожку (клипов: {})", t.clips.len()) }).disabled(!t.clips.is_empty()),
    ]);
    v
}

fn track_run(st: St, tid: u64, id: &str) {
    let (muted, hidden) = match ed().project.track(tid) {
        Some(t) => (t.muted, t.hidden),
        None => return,
    };
    match id {
        "tk:rename" => st.rename_track.set(Some(tid)),
        "tk:mute" => {
            st.exec("track.set", json!({"id": tid, "muted": !muted}));
        }
        "tk:hide" => {
            st.exec("track.set", json!({"id": tid, "hidden": !hidden}));
        }
        "tk:up" => {
            st.exec("track.move", json!({"id": tid, "by": -1}));
        }
        "tk:down" => {
            st.exec("track.move", json!({"id": tid, "by": 1}));
        }
        "tk:remove" => {
            st.exec("track.remove", json!({"id": tid}));
        }
        _ => {}
    }
}

/// Тип переноса «дорожка» (payload — id дорожки): порядок перетаскиванием заголовка.
pub const DRAG_TRACK: &str = "syn-video-editor/track";

/// Заголовок дорожки: название (двойной щелчок — переименовать), звук,
/// видимость; правый щелчок — меню; перетаскивание — порядок.
fn track_head(st: St, t: &crate::doc::Track, h: f32) -> Box<dyn Widget> {
    use super::icons;
    let tid = t.id;
    let (muted, hidden) = (t.muted, t.hidden);
    let name: Box<dyn Widget> = if st.rename_track.get_untracked() == Some(tid) {
        Box::new(
            TextField::new()
                .text(t.name.clone())
                .autofocus(true)
                .select_range(0, t.name.len())
                .on_submit(move |s: &str| {
                    st.rename_track.set(None);
                    let s = s.trim().to_string();
                    if !s.is_empty() {
                        st.exec("track.set", json!({"id": tid, "name": s}));
                    }
                })
                .on_escape(move || st.rename_track.set(None))
                .class("tl-head-edit"),
        )
    } else {
        Box::new(Text::new(t.name.clone()).max_lines(1).elide(Elide::End).class("tl-head-text"))
    };
    let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(name));
    row = row.child(ToolButton::new(if muted { icons::VOLUME_OFF } else { icons::VOLUME_UP }).tooltip(if muted { "Включить звук дорожки" } else { "Выключить звук дорожки" }).active(muted).on_click(move || {
        st.exec("track.set", json!({"id": tid, "muted": !muted}));
    }));
    if t.kind == TrackKind::Video {
        row = row.child(ToolButton::new(if hidden { icons::HIDDEN } else { icons::VISIBLE }).tooltip(if hidden { "Показать дорожку" } else { "Скрыть дорожку" }).active(hidden).on_click(move || {
            st.exec("track.set", json!({"id": tid, "hidden": !hidden}));
        }));
    }
    let class = if t.kind == TrackKind::Video { "tl-head tl-head-v" } else { "tl-head tl-head-a" };
    let body = DecoratedBox::new().class(class).class(if muted || hidden { "tl-head-off" } else { "" }).child(row).style("height", h);
    let open = move |pt: Point| {
        st.menu_at.set(MenuAt::Track(tid));
        st.menu_pos.set(pt);
        st.menu_open.set(true);
    };
    // долгое нажатие тут не ловим: оно мешало бы перетаскиванию заголовка
    Box::new(Draggable::new(DRAG_TRACK, tid.to_string()).label(t.name.clone()).on_double_click(move || st.rename_track.set(Some(tid))).child(GestureDetector::new().on_secondary_click(open).child(body)))
}

/// Бросок заголовка дорожки на строку той же группы — встать на её место.
fn drop_track(st: St, data: &DragData, y: f32) {
    let Ok(tid) = data.payload.trim().parse::<u64>() else { return };
    let to = {
        let e = ed();
        let p = &e.project;
        let Some(kind) = p.track(tid).map(|t| t.kind) else { return };
        let Some((RowKind::Track(j), _, _)) = row_under(&rows(p), y) else { return };
        if p.tracks[j].kind != kind {
            return;
        }
        let mut order: Vec<usize> = (0..p.tracks.len()).filter(|i| p.tracks[*i].kind == kind).collect();
        if kind == TrackKind::Video {
            order.reverse();
        }
        order.iter().position(|i| *i == j)
    };
    if let Some(to) = to {
        st.exec("track.move", json!({"id": tid, "to": to}));
    }
}
