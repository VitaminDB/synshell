//! Поверхность контроллера: слой Overlay на весь экран (поверх полноэкранной игры), касания
//! принимают только элементы (область ввода), остальное — игре. Каждый палец ведёт свой элемент
//! ([`MultiTouch`]); synwm отдаёт пальцы на этом слое без жестов края и сдвига стола.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use syngui::prelude::*;
use syngui::containers::{MultiTouch, Positioned, Stack, StackFit, TouchPhase, TouchPoint};
use syngui::mss::StyleValue;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceId, SurfaceSpec};
use synshell_common::haptics::{self, Feedback};

use crate::layout::{Axis, Bind, Kind, Layout};
use crate::output;

/// Ручка «спрятать/показать» (вверху по центру): сторона, px.
const HANDLE: f32 = 40.0;
/// Удержание ручки дольше — не сворачивать (позже — редактор раскладки).
const HANDLE_HOLD_MS: u128 = 600;
/// Запас вокруг элемента, который ещё ловит палец (доля размера).
const HIT_SLOP: f32 = 0.12;
/// Ход ручки стика — доля радиуса основания.
const KNOB: f32 = 0.42;

#[derive(Clone, Copy)]
pub struct Pad {
    /// Режим «Контроллер»: поверхность есть.
    pub shown: RwSignal<bool>,
    /// Свёрнут до ручки (временно спрятан).
    pub folded: RwSignal<bool>,
    pub layout_id: RwSignal<String>,
    pub layout: RwSignal<Layout>,
}

impl Pad {
    pub fn new(layout_id: &str) -> Self {
        Self {
            shown: use_signal(false),
            folded: use_signal(false),
            layout_id: use_signal(layout_id.to_string()),
            layout: use_signal(crate::layout::load(layout_id)),
        }
    }

    pub fn set_layout(&self, id: &str) {
        self.layout_id.set(id.to_string());
        self.layout.set(crate::layout::load(id));
    }
}

/// Состояние элемента для отрисовки.
#[derive(Clone, Copy)]
struct Rt {
    pressed: RwSignal<bool>,
    /// Ручка стика: левый верхний угол.
    knob: RwSignal<Point>,
    /// Крестовина: нажатое направление.
    dir: RwSignal<(i32, i32)>,
}

enum Target {
    Handle,
    Element(usize),
}

struct Finger {
    target: Target,
    /// Где палец коснулся.
    origin: (f32, f32),
    center: (f32, f32),
    radius: f32,
    last: (f32, f32),
    start: Instant,
    moved: bool,
}

thread_local! {
    static SURFACE: std::cell::Cell<Option<SurfaceId>> = const { std::cell::Cell::new(None) };
    static RT: RefCell<Vec<Rt>> = const { RefCell::new(Vec::new()) };
    static FINGERS: RefCell<HashMap<u64, Finger>> = RefCell::new(HashMap::new());
    /// Сколько пальцев держит каждый элемент.
    static HOLD: RefCell<HashMap<usize, u32>> = RefCell::new(HashMap::new());
    /// Залипшие кнопки-переключатели.
    static TOGGLED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

fn spec() -> SurfaceSpec {
    SurfaceSpec {
        namespace: "syngamepad".into(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
        size: (0, 0),
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::None,
        ..Default::default()
    }
}

fn handle_rect(w: f32, _h: f32) -> [f32; 4] {
    [w / 2.0 - HANDLE / 2.0, 6.0, HANDLE, HANDLE]
}

fn contains(r: [f32; 4], p: (f32, f32), slop: f32) -> bool {
    let (dx, dy) = (r[2] * slop, r[3] * slop);
    p.0 >= r[0] - dx && p.0 <= r[0] + r[2] + dx && p.1 >= r[1] - dy && p.1 <= r[1] + r[3] + dy
}

fn rt(i: usize) -> Option<Rt> {
    RT.with(|r| r.borrow().get(i).copied())
}

/// Отпустить всё (свернули, сменили раскладку, спрятали).
fn release_all() {
    FINGERS.with(|f| f.borrow_mut().clear());
    HOLD.with(|h| h.borrow_mut().clear());
    TOGGLED.with(|t| t.borrow_mut().clear());
    RT.with(|r| {
        for x in r.borrow().iter() {
            x.pressed.set(false);
            x.dir.set((0, 0));
        }
    });
    output::release_all();
}

pub fn install(pad: Pad) {
    // Элементы сменились — новое состояние отрисовки.
    create_effect(move || {
        let layout = pad.layout.get();
        release_all();
        let list: Vec<Rt> = layout.elements.iter().map(|_| Rt { pressed: use_signal(false), knob: use_signal(Point::zero()), dir: use_signal((0, 0)) }).collect();
        RT.with(|r| *r.borrow_mut() = list);
    });
    // Поверхность есть в режиме «Контроллер».
    create_effect(move || {
        let shown = pad.shown.get();
        match (shown, SURFACE.with(|s| s.get())) {
            (true, None) => {
                output::connect();
                let id = syngui_layer::create_surface(spec(), move || Box::new(view(pad)));
                SURFACE.with(|s| s.set(Some(id)));
                syngui_layer::set_input_region(id, Some(Vec::new()));
            }
            (false, Some(id)) => {
                SURFACE.with(|s| s.set(None));
                release_all();
                syngui_layer::close_surface(id);
                output::disconnect();
            }
            _ => {}
        }
    });
    // Касания — только элементы и ручка; свёрнутый — только ручка.
    create_effect(move || {
        let size = syngui::viewport::viewport_size().get();
        let folded = pad.folded.get();
        let layout = pad.layout.get();
        let _ = pad.shown.get();
        let Some(id) = SURFACE.with(|s| s.get()) else { return };
        let (w, h) = (size.width, size.height);
        let mut rects = vec![handle_rect(w, h)];
        if !folded {
            rects.extend(layout.elements.iter().map(|e| e.rect(w, h)).map(|r| {
                let (dx, dy) = (r[2] * HIT_SLOP, r[3] * HIT_SLOP);
                [r[0] - dx, r[1] - dy, r[2] + 2.0 * dx, r[3] + 2.0 * dy]
            }));
        }
        let ints = rects.into_iter().map(|r| [r[0].floor() as i32, r[1].floor() as i32, r[2].ceil() as i32, r[3].ceil() as i32]).collect();
        log::debug!("область ввода {w}×{h}: {ints:?}");
        syngui_layer::set_input_region(id, Some(ints));
    });
}

fn view(pad: Pad) -> impl Widget {
    let visuals = Reactive::new(move || -> Vec<Box<dyn Widget>> {
        let size = syngui::viewport::viewport_size().get();
        let folded = pad.folded.get();
        let layout = pad.layout.get();
        let (w, h) = (size.width, size.height);
        let mut out: Vec<Box<dyn Widget>> = Vec::new();
        if !folded {
            for (i, e) in layout.elements.iter().enumerate() {
                let Some(st) = rt(i) else { continue };
                out.push(element_view(e, st, w, h, layout.opacity));
            }
        }
        let [hx, hy, hw, hh] = handle_rect(w, h);
        let handle = DecoratedBox::new()
            .child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(if folded { crate::icons::GAMEPAD } else { crate::icons::FOLD }).class("gp-handle-icon"))
                    .class("gp-fill"),
            )
            .class("gp-handle")
            .style("width", StyleValue::px(hw))
            .style("height", StyleValue::px(hh));
        out.push(Box::new(Positioned::new(handle).at(hx, hy)));
        out
    });
    MultiTouch::new()
        .on_touch(move |t| on_touch(pad, t))
        .child(Stack::new().fit(StackFit::Expand).child(visuals))
}

fn element_view(e: &crate::layout::Element, st: Rt, w: f32, h: f32, opacity: f32) -> Box<dyn Widget> {
    let [x, y, ew, eh] = e.rect(w, h);
    let label = e.label.clone();
    let kind = e.kind;
    let body: Box<dyn Widget> = match kind {
        Kind::Button | Kind::Trackpad => Box::new(Reactive::new(move || {
            let pressed = st.pressed.get();
            let class = match (kind, pressed) {
                (Kind::Trackpad, false) => "gp-pad",
                (Kind::Trackpad, true) => "gp-pad gp-pressed",
                (_, false) => "gp-btn",
                (_, true) => "gp-btn gp-pressed",
            };
            vec![Box::new(DecoratedBox::new()
                .child(
                    Row::new()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(label_view(&label))
                        .class("gp-fill"),
                )
                .class(class)
                .style("border-radius", StyleValue::px(if kind == Kind::Trackpad { 18.0 } else { eh / 2.0 }))
                .style("width", StyleValue::px(ew))
                .style("height", StyleValue::px(eh))
                .style("opacity", if pressed { (opacity + 0.3).min(1.0) } else { opacity })) as Box<dyn Widget>]
        })),
        Kind::Stick => {
            let k = eh * KNOB * 1.25;
            let base = DecoratedBox::new()
                .class("gp-stick")
                .style("border-radius", StyleValue::px(eh / 2.0))
                .style("width", StyleValue::px(ew))
                .style("height", StyleValue::px(eh))
                .style("opacity", opacity);
            st.knob.set(Point::new(ew / 2.0 - k / 2.0, eh / 2.0 - k / 2.0));
            let knob = Reactive::new(move || {
                let pressed = st.pressed.get();
                vec![Box::new(DecoratedBox::new()
                    .class(if pressed { "gp-knob gp-pressed" } else { "gp-knob" })
                    .style("border-radius", StyleValue::px(k / 2.0))
                    .style("width", StyleValue::px(k))
                    .style("height", StyleValue::px(k))
                    .style("opacity", if pressed { (opacity + 0.3).min(1.0) } else { opacity })) as Box<dyn Widget>]
            });
            Box::new(
                Stack::new()
                    .child(base)
                    .child(Positioned::new(knob).offset_signal(st.knob))
                    .style("width", StyleValue::px(ew))
                    .style("height", StyleValue::px(eh)),
            )
        }
        Kind::Dpad => Box::new(Reactive::new(move || {
            let (dx, dy) = st.dir.get();
            let arrow = |glyph: &'static str, on: bool| -> Box<dyn Widget> {
                Box::new(
                    DecoratedBox::new()
                        .child(
                            Row::new()
                                .main_axis_alignment(MainAxisAlignment::Center)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Icon::new(glyph).class("gp-arrow"))
                                .class("gp-fill"),
                        )
                        .class(if on { "gp-dir gp-pressed" } else { "gp-dir" })
                        .style("width", StyleValue::px(eh / 3.0))
                        .style("height", StyleValue::px(eh / 3.0)),
                )
            };
            let blank = || -> Box<dyn Widget> { Box::new(DecoratedBox::new().style("width", StyleValue::px(eh / 3.0)).style("height", StyleValue::px(eh / 3.0))) };
            let row = |a: Box<dyn Widget>, b: Box<dyn Widget>, c: Box<dyn Widget>| Row::new().child(a).child(b).child(c);
            vec![Box::new(DecoratedBox::new()
                .child(
                    Column::new()
                        .child(row(blank(), arrow(crate::icons::UP, dy < 0), blank()))
                        .child(row(arrow(crate::icons::LEFT, dx < 0), blank(), arrow(crate::icons::RIGHT, dx > 0)))
                        .child(row(blank(), arrow(crate::icons::DOWN, dy > 0), blank())),
                )
                .class("gp-dpad")
                .style("border-radius", StyleValue::px(eh / 2.0))
                .style("width", StyleValue::px(ew))
                .style("height", StyleValue::px(eh))
                .style("opacity", if dx != 0 || dy != 0 { (opacity + 0.3).min(1.0) } else { opacity })) as Box<dyn Widget>]
        })),
    };
    Box::new(Positioned::new(body).at(x, y))
}

/// Подпись: значок Material Icons (символы личной области Unicode) или текст.
fn label_view(label: &str) -> Box<dyn Widget> {
    let icon = !label.is_empty() && label.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c));
    if icon {
        Box::new(Icon::new(label.to_string()).class("gp-icon"))
    } else {
        Box::new(Text::new(label.to_string()).class("gp-label"))
    }
}

fn on_touch(pad: Pad, t: TouchPoint) -> bool {
    log::debug!("касание {t:?}");
    let (w, h) = (t.size.width, t.size.height);
    let p = (t.position.x, t.position.y);
    match t.phase {
        TouchPhase::Down => {
            if contains(handle_rect(w, h), p, 0.2) {
                FINGERS.with(|f| f.borrow_mut().insert(t.id, Finger { target: Target::Handle, origin: p, center: p, radius: 1.0, last: p, start: Instant::now(), moved: false }));
                return true;
            }
            if pad.folded.get_untracked() {
                return false;
            }
            let layout = pad.layout.get_untracked();
            let Some((i, e)) = layout.elements.iter().enumerate().rev().find(|(_, e)| contains(e.rect(w, h), p, HIT_SLOP)) else { return false };
            let r = e.rect(w, h);
            let center = if e.kind == Kind::Stick && e.floating { p } else { (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0) };
            let finger = Finger { target: Target::Element(i), origin: p, center, radius: r[3] / 2.0, last: p, start: Instant::now(), moved: false };
            FINGERS.with(|f| f.borrow_mut().insert(t.id, finger));
            if layout.haptics {
                haptics::play(Feedback::Key);
            }
            press(&layout, i, true);
            track(&layout, i, t.id, p, w, h);
            true
        }
        TouchPhase::Move => {
            let layout = pad.layout.get_untracked();
            let Some(i) = FINGERS.with(|f| {
                let mut f = f.borrow_mut();
                let fi = f.get_mut(&t.id)?;
                if (p.0 - fi.origin.0).abs() + (p.1 - fi.origin.1).abs() > 8.0 {
                    fi.moved = true;
                }
                match fi.target {
                    Target::Element(i) => Some(i),
                    Target::Handle => None,
                }
            }) else {
                return true;
            };
            track(&layout, i, t.id, p, w, h);
            true
        }
        TouchPhase::Up | TouchPhase::Cancel => {
            let Some(f) = FINGERS.with(|f| f.borrow_mut().remove(&t.id)) else { return true };
            match f.target {
                Target::Handle => {
                    if t.phase == TouchPhase::Up && f.start.elapsed().as_millis() < HANDLE_HOLD_MS {
                        release_all();
                        pad.folded.set(!pad.folded.get_untracked());
                    }
                }
                Target::Element(i) => {
                    let layout = pad.layout.get_untracked();
                    if let Some(e) = layout.elements.get(i) {
                        // Тачпад: короткое касание без движения — щелчок.
                        if e.kind == Kind::Trackpad && t.phase == TouchPhase::Up && !f.moved && f.start.elapsed().as_millis() < 250 {
                            let b = Bind::parse(if e.bind.is_empty() { "mouse:left" } else { &e.bind });
                            output::button(&b, true);
                            output::button(&b, false);
                        }
                        match e.kind {
                            Kind::Stick | Kind::Dpad => {
                                let still = FINGERS.with(|fs| fs.borrow().values().any(|x| matches!(x.target, Target::Element(j) if j == i)));
                                if !still {
                                    if let Some(st) = rt(i) {
                                        let [_, _, ew, eh] = e.rect(w, h);
                                        let k = eh * KNOB * 1.25;
                                        st.knob.set(Point::new(ew / 2.0 - k / 2.0, eh / 2.0 - k / 2.0));
                                        st.dir.set((0, 0));
                                    }
                                    stick_output(e, i, 0.0, 0.0);
                                }
                            }
                            _ => {}
                        }
                    }
                    press(&layout, i, false);
                }
            }
            true
        }
    }
}

/// Палец лёг на элемент или ушёл с него.
fn press(layout: &Layout, i: usize, down: bool) {
    let Some(e) = layout.elements.get(i) else { return };
    let n = HOLD.with(|h| {
        let mut h = h.borrow_mut();
        let n = h.entry(i).or_insert(0);
        if down {
            *n += 1;
        } else {
            *n = n.saturating_sub(1);
        }
        *n
    });
    let st = rt(i);
    match e.kind {
        Kind::Button => {
            let bind = Bind::parse(&e.bind);
            if e.toggle {
                if down && n == 1 {
                    let on = TOGGLED.with(|t| {
                        let mut t = t.borrow_mut();
                        if !t.remove(&i) {
                            t.insert(i);
                            true
                        } else {
                            false
                        }
                    });
                    output::button(&bind, on);
                    if let Some(st) = st {
                        st.pressed.set(on);
                    }
                }
                return;
            }
            if (down && n == 1) || (!down && n == 0) {
                output::button(&bind, down);
            }
            if let Some(st) = st {
                st.pressed.set(n > 0);
            }
        }
        Kind::Stick => {
            if let Some(st) = st {
                st.pressed.set(n > 0);
            }
        }
        Kind::Trackpad => {
            if let Some(st) = st {
                st.pressed.set(n > 0);
            }
        }
        Kind::Dpad => {}
    }
}

/// Палец ведёт элемент: стик, крестовина, тачпад.
fn track(layout: &Layout, i: usize, id: u64, p: (f32, f32), w: f32, h: f32) {
    let Some(e) = layout.elements.get(i) else { return };
    let Some((center, radius, last)) = FINGERS.with(|f| {
        let mut f = f.borrow_mut();
        let fi = f.get_mut(&id)?;
        let last = fi.last;
        fi.last = p;
        Some((fi.center, fi.radius, last))
    }) else {
        return;
    };
    match e.kind {
        Kind::Stick => {
            let travel = radius * (1.0 - KNOB * 0.6);
            let (mut vx, mut vy) = ((p.0 - center.0) / travel, (p.1 - center.1) / travel);
            let len = (vx * vx + vy * vy).sqrt();
            if len > 1.0 {
                vx /= len;
                vy /= len;
            }
            if let Some(st) = rt(i) {
                let r = e.rect(w, h);
                let k = r[3] * KNOB * 1.25;
                // Плавающий стик: ручка под пальцем там, где коснулись (сдвиг центра от середины).
                let (ox, oy) = (center.0 - (r[0] + r[2] / 2.0), center.1 - (r[1] + r[3] / 2.0));
                st.knob.set(Point::new(r[2] / 2.0 + ox + vx * travel - k / 2.0, r[3] / 2.0 + oy + vy * travel - k / 2.0));
            }
            stick_output(e, i, vx, vy);
        }
        Kind::Dpad => {
            let (vx, vy) = ((p.0 - center.0) / (radius * 0.6), (p.1 - center.1) / (radius * 0.6));
            let d = |v: f32| if v <= -0.4 { -1 } else if v >= 0.4 { 1 } else { 0 };
            if let Some(st) = rt(i) {
                st.dir.set((d(vx), d(vy)));
            }
            stick_output(e, i, vx.clamp(-1.0, 1.0), vy.clamp(-1.0, 1.0));
        }
        Kind::Trackpad => {
            let k = 1.6 * e.sensitivity.max(0.1);
            output::mouse_move((p.0 - last.0) * k, (p.1 - last.1) * k);
        }
        Kind::Button => {}
    }
}

fn stick_output(e: &crate::layout::Element, i: usize, x: f32, y: f32) {
    match Axis::parse(e.kind, &e.bind) {
        Axis::Mouse => output::mouse_axis(i, x, y, e.sensitivity.max(0.1)),
        a => output::axis(i, a, x, y),
    }
}
