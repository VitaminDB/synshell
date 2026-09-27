//! Док (`[[panel]] mode = "dock"`): значки приложений, разделов и папок в
//! ряду «рыбий глаз» (`syngui::widgets::Fisheye`) — значок под курсором
//! плавно вырастает над полосой, соседи расступаются, как в macOS и Latte
//! Dock. Индикаторы окон, «прыжки» при запуске, частицы, 3D-эффекты
//! наведения, подпись над значком, автоскрытие и «умное» скрытие, когда
//! полосу перекрывает окно.
//!
//! Поверхность — на всю длину края и толще полосы на запас под увеличение и
//! подпись; область ввода — только полоса (над ней указатель проходит к
//! окнам), при наведении — полоса с увеличенными значками.
//!
//! Внешний вид — MSS: `.dock-root.dock-<край>.dock-style-<стиль>`,
//! `.dock-items` (полоса, `magnification*`, `background-rotate-x` для 3D-полки),
//! `.dock-item` (+ `-running`, `-active`, `-launching`, `-urgent`),
//! `.dock-icon`, `.dock-dot`, `.dock-label`, `.dock-fx-hover-*`/`.dock-fx-launch-*`
//! (частицы `particle-*`). Подробно — docs/DOCK.md.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use syndesktop_common::config::{Applet, Edge, Panel};
use syndesktop_common::ipc::WindowInfo;
use syngui::core::sync::Mutex;
use syngui::input::MouseButton;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::containers::Positioned;
use syngui::widgets::{EventHook, Fisheye, ParticleEmitter};
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, OutputInfo, SurfaceHooks, SurfaceId, SurfaceSpec};

use crate::ctx::{PopupKind, ShellCtx};
use crate::launchers::{self, Launchable};
use crate::panel::PanelCtx;
use crate::ui::{icon, mi, InputArea};

/// Геометрия дока в логических px.
#[derive(Clone, Copy, Debug)]
struct Geo {
    icon: f32,
    /// Сторона квадрата значка с полями (место под индикатор).
    item: f32,
    /// Толщина полосы.
    bar: f32,
    /// Отступ плавающей полосы от края экрана.
    gap: f32,
    /// Запас над полосой: увеличение, подпись, частицы.
    headroom: f32,
}

impl Geo {
    fn of(panel: &Panel) -> Self {
        let icon = panel.dock.icon_size.clamp(16, 256) as f32;
        let pad = (icon * 0.16).round().max(4.0);
        let item = icon + pad * 2.0;
        let bar = item + 12.0;
        let gap = if panel.floating { 8.0 } else { 0.0 };
        let zoom = panel.dock.zoom.max(1.9);
        // Сверху/снизу подпись — над значком, слева/справа — сбоку (шире).
        let label = if panel.edge.is_vertical() { 190.0 } else { 52.0 };
        let headroom = (item * (zoom - 1.0) + label).ceil();
        Self { icon, item, bar, gap, headroom }
    }

    fn thickness(&self) -> f32 {
        self.gap + self.bar + self.headroom
    }
}

fn edge_name(e: Edge) -> &'static str {
    match e {
        Edge::Top => "top",
        Edge::Bottom => "bottom",
        Edge::Left => "left",
        Edge::Right => "right",
    }
}

fn spec_for(panel: &Panel, out: &OutputInfo, geo: &Geo) -> SurfaceSpec {
    let (anchor, size) = match panel.edge {
        Edge::Bottom => (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, (0, geo.thickness() as u32)),
        Edge::Top => (Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, (0, geo.thickness() as u32)),
        Edge::Left => (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (geo.thickness() as u32, 0)),
        Edge::Right => (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (geo.thickness() as u32, 0)),
    };
    let hides = panel.autohide || panel.dock.intellihide;
    let exclusive = if panel.exclusive && !hides { (geo.gap + geo.bar) as i32 } else { 0 };
    SurfaceSpec {
        namespace: "syndesktop-dock".into(),
        layer: Layer::Top,
        anchor,
        size,
        margin: [0; 4],
        exclusive_zone: exclusive,
        keyboard: KeyboardInteractivity::None,
        output: Some(out.name.clone()),
        auto_size: false,
        clear_color: [0.0; 4],
    }
}

/// Состояние одного дока.
#[derive(Clone)]
struct DockState {
    pc: PanelCtx,
    geo: Geo,
    edge: Edge,
    /// Указатель над поверхностью.
    hovered: RwSignal<bool>,
    hidden: RwSignal<bool>,
    /// Подпись: имя и прямоугольник увеличенного значка.
    label: RwSignal<Option<(String, Rect)>>,
    /// Последний значок под указателем: (номер в ряду, прямоугольник).
    last_hover: Arc<StdMutex<Option<(usize, Rect)>>>,
    /// Границы полосы (координаты поверхности).
    bar_slot: Arc<Mutex<Rect>>,
}

impl DockState {
    /// Якорь всплывающего окна у значка номер `slot` в ряду.
    fn anchor_for(&self, slot: usize, fallback: Rect) -> crate::ctx::PopupAnchor {
        let r = match *self.last_hover.lock().unwrap() {
            Some((i, r)) if i == slot => r,
            _ => fallback,
        };
        self.pc.anchor(r)
    }
}

/// Создать док номер `index` на выводе.
pub fn create(ctx: ShellCtx, index: usize, panel: &Panel, out: &OutputInfo) -> (SurfaceId, u64) {
    let key = crate::panel::next_key();
    let geo = Geo::of(panel);
    let spec = spec_for(panel, out, &geo);
    crate::panel::register(key, &out.name, panel.edge, &spec);
    let pc = PanelCtx {
        key,
        index,
        output: out.name.clone(),
        edge: panel.edge,
        vertical: panel.edge.is_vertical(),
        size: geo.bar as u32,
    };
    let st = DockState {
        pc: pc.clone(),
        geo,
        edge: panel.edge,
        hovered: use_signal(false),
        hidden: use_signal(panel.autohide),
        label: use_signal(None),
        last_hover: Arc::new(StdMutex::new(None)),
        bar_slot: Arc::new(Mutex::new(Rect::zero())),
    };
    let id_cell: Arc<StdMutex<Option<SurfaceId>>> = Arc::new(StdMutex::new(None));
    let hide_timer: Arc<StdMutex<Option<u64>>> = Arc::new(StdMutex::new(None));

    let hooks = {
        let out_resize = out.clone();
        let st = st.clone();
        let autohide = panel.autohide;
        let intellihide = panel.dock.intellihide;
        let hide_timer = hide_timer.clone();
        SurfaceHooks {
            on_resize: Some(Box::new(move |w, h| crate::panel::resized(key, &out_resize, w, h))),
            on_pointer: Some(Box::new(move |inside| {
                st.hovered.set(inside);
                if !inside {
                    st.label.set(None);
                }
                if let Some(t) = hide_timer.lock().unwrap().take() {
                    syngui_layer::cancel_timer(t);
                }
                if inside {
                    st.hidden.set(false);
                } else if autohide || intellihide {
                    let st = st.clone();
                    let t = syngui_layer::add_timer(Duration::from_millis(700), move || {
                        // Пока открыто окно дока (стек, меню) — не прятать.
                        if ShellCtx::get().popup.get_untracked().is_some() || st.hovered.get_untracked() {
                            return Some(Duration::from_millis(400));
                        }
                        if autohide || overlapped(&st) {
                            st.hidden.set(true);
                        }
                        None
                    });
                    *hide_timer.lock().unwrap() = Some(t);
                }
            })),
            ..Default::default()
        }
    };

    let panel_c = panel.clone();
    let st_view = st.clone();
    let id = syngui_layer::create_surface_with(spec, hooks, move || {
        let st = st_view;
        let panel = panel_c;
        Box::new(crate::ui::rx(move || {
            let ctx = ShellCtx::get();
            let editing = ctx.editing.get() == Some(st.pc.index);
            let hidden = st.hidden.get() && !editing;
            Box::new(root(&panel, &st, editing, hidden)) as Box<dyn Widget>
        }))
    });
    *id_cell.lock().unwrap() = Some(id);
    start_region_timer(id, st, panel.autohide, panel.dock.intellihide);
    let _ = ctx;
    (id, key)
}

/// Полоса дока перекрыта окном (умное скрытие).
fn overlapped(st: &DockState) -> bool {
    let ctx = ShellCtx::get();
    let bar = *st.bar_slot.lock().unwrap_or_else(|e| e.into_inner());
    if bar.size.width <= 0.0 {
        return false;
    }
    let (ox, oy) = crate::panel::origin(st.pc.key);
    let Some(out) = ctx.comp_outputs.get_untracked().into_iter().find(|o| o.name == st.pc.output) else { return false };
    let ws = ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index);
    let bx0 = bar.origin.x + ox;
    let by0 = bar.origin.y + oy;
    let (bx1, by1) = (bx0 + bar.size.width, by0 + bar.size.height);
    ctx.windows.get_untracked().iter().any(|w: &WindowInfo| {
        if w.minimized || !(Some(w.workspace) == ws || w.sticky) || w.output.as_deref().is_some_and(|o| o != st.pc.output) {
            return false;
        }
        let [x, y, ww, hh] = w.geometry;
        let (x0, y0) = ((x - out.geometry[0]) as f32, (y - out.geometry[1]) as f32);
        let (x1, y1) = (x0 + ww as f32, y0 + hh as f32);
        x0 < bx1 && x1 > bx0 && y0 < by1 && y1 > by0
    })
}

/// Область ввода по состоянию дока и умное скрытие — раз в 120 мс.
fn start_region_timer(id: SurfaceId, st: DockState, autohide: bool, intellihide: bool) {
    let last: Arc<StdMutex<Option<Vec<[i32; 4]>>>> = Arc::new(StdMutex::new(None));
    let mut tick = 0u32;
    syngui_layer::add_timer(Duration::from_millis(120), move || {
        let ctx = ShellCtx::get();
        // Док разобран (перечитан конфиг, сменились мониторы) — таймер не нужен.
        if !crate::panel::alive(st.pc.key) {
            return None;
        }
        tick += 1;
        if intellihide && !autohide && tick % 3 == 0 {
            let busy = st.hovered.get_untracked() || ctx.popup.get_untracked().is_some();
            let over = overlapped(&st);
            if over && !busy && !st.hidden.get_untracked() {
                st.hidden.set(true);
            } else if !over && st.hidden.get_untracked() {
                st.hidden.set(false);
            }
        }
        let bar = *st.bar_slot.lock().unwrap_or_else(|e| e.into_inner());
        let editing = ctx.editing.get_untracked() == Some(st.pc.index);
        let (sw, sh) = surface_size(&st);
        let region = if st.hidden.get_untracked() && !editing {
            // Спрятан: полоска у края на всю длину.
            let t = 3;
            Some(vec![match st.edge {
                Edge::Bottom => [0, sh - t, sw, t],
                Edge::Top => [0, 0, sw, t],
                Edge::Left => [0, 0, t, sh],
                Edge::Right => [sw - t, 0, t, sh],
            }])
        } else if bar.size.width <= 0.0 {
            None
        } else if editing || ctx.popup.get_untracked().is_some() {
            None
        } else {
            let grow = if st.hovered.get_untracked() { st.geo.headroom } else { 0.0 };
            let (x, y, w, h) = (bar.origin.x - 4.0, bar.origin.y - 4.0, bar.size.width + 8.0, bar.size.height + 8.0);
            // Вместе с отступом до края: курсор, прижатый к краю, — над доком.
            let r = match st.edge {
                Edge::Bottom => [x, y - grow, w, sh as f32 - y + grow],
                Edge::Top => [x, 0.0, w, y + h + grow],
                Edge::Left => [0.0, y, x + w + grow, h],
                Edge::Right => [x - grow, y, sw as f32 - x + grow, h],
            };
            Some(vec![r.map(|v| v.round() as i32)])
        };
        let mut l = last.lock().unwrap();
        if *l != region {
            *l = region.clone();
            syngui_layer::set_input_region(id, region);
        }
        Some(Duration::from_millis(120))
    });
}

fn surface_size(st: &DockState) -> (i32, i32) {
    let (ow, oh) = crate::manager::output_size(Some(&st.pc.output));
    let t = st.geo.thickness() as i32;
    match st.edge {
        Edge::Bottom | Edge::Top => (ow as i32, t),
        Edge::Left | Edge::Right => (t, oh as i32),
    }
}

// ─── Виджеты ────────────────────────────────────────────────────────────────

fn root(panel: &Panel, st: &DockState, editing: bool, hidden: bool) -> impl Widget {
    let d = &panel.dock;
    let edge = edge_name(st.edge);
    let mut cls = format!(
        "dock-root dock-{edge} dock-style-{} dock-ind-{} dock-hover-{} dock-launch-{}",
        d.style, d.indicator, d.hover_effect, d.launch_animation
    );
    if panel.floating {
        cls.push_str(" dock-floating");
    }
    if hidden {
        cls.push_str(" dock-hidden");
    }
    if editing {
        cls.push_str(" dock-editing");
    }
    let items = items_row(panel, st, editing);
    // Правый клик по полосе мимо значков (или по апплету без своего меню) —
    // меню дока.
    let st_menu = st.clone();
    let bar = InputArea::new(EventHook::new().report_bounds(st.bar_slot.clone()).child(items))
        .buttons(&[MouseButton::Right])
        .on_click(move |_, _, _| {
            let r = *st_menu.bar_slot.lock().unwrap_or_else(|e| e.into_inner());
            ShellCtx::get().open_popup(PopupKind::PanelMenu(st_menu.pc.index), st_menu.pc.anchor(r));
        });
    let slide = DecoratedBox::new().child(bar).class("dock-slide");
    let gap = StyleValue::px(st.geo.gap);
    let area: Box<dyn Widget> = match st.edge {
        Edge::Bottom => Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::End)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(slide)
                .style("padding-bottom", gap),
        ),
        Edge::Top => Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(slide)
                .style("padding-top", gap),
        ),
        Edge::Left => Box::new(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::Start)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(slide)
                .style("padding-left", gap),
        ),
        Edge::Right => Box::new(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::End)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(slide)
                .style("padding-right", gap),
        ),
    };
    let mut stack = Stack::new().fit(StackFit::Expand).child(area);
    if d.labels && !editing {
        let st2 = st.clone();
        stack = stack.child(crate::ui::rx(move || label_view(&st2)));
    }
    DecoratedBox::new().child(stack).class(cls)
}

/// Подпись над (сбоку от) увеличенным значком.
fn label_view(st: &DockState) -> Box<dyn Widget> {
    let Some((name, r)) = st.label.get() else {
        return Box::new(DecoratedBox::new().class("dock-label-none"));
    };
    const W: f32 = 260.0;
    let text = DecoratedBox::new().child(Text::new(name).max_lines(1).class("dock-label-text")).class("dock-label");
    let (x, y, align) = match st.edge {
        Edge::Bottom => (r.origin.x + r.size.width / 2.0 - W / 2.0, r.origin.y - 40.0, MainAxisAlignment::Center),
        Edge::Top => (r.origin.x + r.size.width / 2.0 - W / 2.0, r.origin.y + r.size.height + 10.0, MainAxisAlignment::Center),
        Edge::Left => (r.origin.x + r.size.width + 12.0, r.origin.y + r.size.height / 2.0 - 16.0, MainAxisAlignment::Start),
        Edge::Right => (r.origin.x - 12.0 - W, r.origin.y + r.size.height / 2.0 - 16.0, MainAxisAlignment::End),
    };
    let row = Row::new().main_axis_alignment(align).child(text).style("width", StyleValue::px(W));
    Box::new(Positioned::new(row).at(x, y))
}

/// Что стоит в ряду дока на месте номер `slot`.
#[derive(Clone)]
struct SlotInfo {
    /// Имя для подписи (пусто — без подписи).
    name: String,
}

/// Ряд значков.
fn items_row(panel: &Panel, st: &DockState, editing: bool) -> impl Widget {
    let st2 = st.clone();
    let applets = panel.applets.clone();
    let dock = panel.dock.clone();
    crate::ui::rx(move || {
        let ctx = ShellCtx::get();
        let windows = ctx.windows.get();
        let launching = ctx.launching.get();
        let bursts = ctx.bursts.get();
        let _ = ctx.focused.get();
        let vertical = st2.edge.is_vertical();
        let cross = match st2.edge {
            Edge::Bottom | Edge::Right => CrossAxisAlignment::End,
            _ => CrossAxisAlignment::Start,
        };
        let mut slots: Vec<SlotInfo> = Vec::new();
        let mut fe = Fisheye::new()
            .vertical(vertical)
            .cross_axis_alignment(cross)
            .overflow(true)
            .zoom(if editing { 1.0 } else { dock.zoom })
            .range(dock.zoom_range)
            .gap(4.0)
            .class(if editing { "dock-items dock-items-editing" } else { "dock-items" });
        let pinned: Vec<String> = applets.iter().filter(|a| a.kind == "app").map(|a| Launchable::from_applet(a).app_id).collect();
        let env = ItemEnv { st: st2.clone(), windows: &windows, launching: &launching, bursts: &bursts, dock: &dock };
        for (ai, a) in applets.iter().enumerate() {
            match a.kind.as_str() {
                "taskbar" => {
                    if editing {
                        let w = placeholder_item(&env, "\u{F088}", "Открытые окна");
                        fe = fe.child(crate::edit::item_frame(&st2.pc, ai, w, true));
                        slots.push(SlotInfo { name: String::new() });
                        continue;
                    }
                    for app in running_apps(&windows, &pinned, a.bool_or("all_workspaces", false)) {
                        let slot = slots.len();
                        let entry = crate::xdg::app_by_id(&app);
                        let l = match &entry {
                            Some(e) => Launchable::from_entry(e),
                            None => Launchable {
                                app_id: app.clone(),
                                name: app.clone(),
                                icon: crate::xdg::window_icon(&app),
                                glyph: None,
                                command: app.clone(),
                                terminal: false,
                            },
                        };
                        slots.push(SlotInfo { name: l.name.clone() });
                        fe = fe.child(app_item(&env, slot, &l, ItemOrigin::Running(app.clone())));
                    }
                }
                kind => {
                    let slot = slots.len();
                    let (w, name) = match kind {
                        "app" => {
                            let l = Launchable::from_applet(a);
                            (app_item(&env, slot, &l, ItemOrigin::Applet(ai)), l.name.clone())
                        }
                        "group" | "folder" => (stack_item(&env, slot, a, ai), launchers::stack_title(a)),
                        "separator" => (separator(&env), String::new()),
                        "spacer" => (Box::new(DecoratedBox::new().class("dock-spacer")) as Box<dyn Widget>, String::new()),
                        "launcher" => (launcher_item(&env, slot, a), "Приложения".to_string()),
                        _ => (applet_item(&env, slot, a, ai), String::new()),
                    };
                    slots.push(SlotInfo { name });
                    fe = fe.child(crate::edit::item_frame(&st2.pc, ai, w, editing));
                }
            }
        }
        if editing {
            fe = fe.child(crate::edit::edit_controls(&st2.pc));
            slots.push(SlotInfo { name: String::new() });
        }
        let st3 = st2.clone();
        fe = fe.on_hover(move |h| {
            *st3.last_hover.lock().unwrap() = h;
            let label = h.and_then(|(i, r)| slots.get(i).filter(|s| !s.name.is_empty()).map(|s| (s.name.clone(), r)));
            if st3.label.get_untracked() != label {
                st3.label.set(label);
            }
        });
        Box::new(fe) as Box<dyn Widget>
    })
}

/// Приложения с окнами, у которых нет закреплённого значка, — по порядку
/// появления окон.
fn running_apps(windows: &[WindowInfo], pinned: &[String], all: bool) -> Vec<String> {
    let ctx = ShellCtx::get();
    let ws = ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index);
    let mut out: Vec<String> = Vec::new();
    for w in windows {
        if w.skip_taskbar || w.app_id.is_empty() {
            continue;
        }
        if !all && !(ws.is_none() || w.sticky || Some(w.workspace) == ws) {
            continue;
        }
        let id = launchers::desktop_id_of(&w.app_id);
        if pinned.iter().any(|p| launchers::window_is_app(w, p)) || out.contains(&id) {
            continue;
        }
        out.push(id);
    }
    out
}

struct ItemEnv<'a> {
    st: DockState,
    windows: &'a [WindowInfo],
    launching: &'a [(String, u64)],
    bursts: &'a std::collections::HashMap<String, u32>,
    dock: &'a syndesktop_common::config::Dock,
}

#[derive(Clone)]
enum ItemOrigin {
    /// Закреплённый значок — апплет номер.
    Applet(usize),
    /// Работающее приложение без значка (из `taskbar`).
    Running(String),
}

/// Квадрат значка: частицы наведения и запуска, 3D-эффекты (MSS),
/// индикатор окон.
fn item_box(env: &ItemEnv, kind: &str, state: &str, icon_w: Box<dyn Widget>, windows: usize, active: bool, burst_key: &str) -> impl Widget {
    let g = env.st.geo;
    let token = env.bursts.get(burst_key).copied().unwrap_or(0);
    let hover_fx = ParticleEmitter::new()
        .class(format!("dock-fx dock-fx-hover dock-fx-hover-{}", env.dock.hover_particles))
        .child(DecoratedBox::new().child(icon_w).class("dock-icon"));
    let launch_fx = ParticleEmitter::new()
        .burst_token(token)
        .class(format!("dock-fx dock-fx-launch dock-fx-launch-{}", env.dock.launch_particles))
        .child(hover_fx);
    let dots = match env.dock.indicator.as_str() {
        "none" => 0,
        "dots" => windows.min(4),
        _ => windows.min(1),
    };
    let mut dot_row = Flex::new()
        .direction(if env.st.edge.is_vertical() { FlexDirection::Column } else { FlexDirection::Row })
        .gap(3.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("dock-dots");
    for i in 0..dots {
        dot_row = dot_row.child(DecoratedBox::new().class(if active && i == 0 { "dock-dot dock-dot-active" } else { "dock-dot" }));
    }
    // Индикатор — у края экрана.
    let indicator: Box<dyn Widget> = match env.st.edge {
        Edge::Bottom => Box::new(Column::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(dot_row)),
        Edge::Top => Box::new(Column::new().main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Center).child(dot_row)),
        Edge::Left => Box::new(Row::new().main_axis_alignment(MainAxisAlignment::Start).cross_axis_alignment(CrossAxisAlignment::Center).child(dot_row)),
        Edge::Right => Box::new(Row::new().main_axis_alignment(MainAxisAlignment::End).cross_axis_alignment(CrossAxisAlignment::Center).child(dot_row)),
    };
    let mut cls = format!("dock-item dock-item-{kind}");
    if !state.is_empty() {
        cls.push(' ');
        cls.push_str(state);
    }
    DecoratedBox::new()
        .child(Stack::new().fit(StackFit::Expand).child(crate::ui::vcenter(launch_fx)).child(indicator))
        .class(cls)
        .style("width", StyleValue::px(g.item))
        .style("height", StyleValue::px(g.item))
}

fn app_item(env: &ItemEnv, slot: usize, l: &Launchable, origin: ItemOrigin) -> Box<dyn Widget> {
    let g = env.st.geo;
    let mine = launchers::windows_of(&l.app_id, env.windows);
    let key = l.key();
    let mut state = String::new();
    if !mine.is_empty() {
        state.push_str("dock-item-running");
    }
    let active = mine.iter().any(|w| w.focused);
    if active {
        state.push_str(" dock-item-active");
    }
    if !mine.is_empty() && mine.iter().all(|w| w.minimized) {
        state.push_str(" dock-item-minimized");
    }
    if mine.iter().any(|w| w.urgent) {
        state.push_str(" dock-item-urgent");
    }
    if env.launching.iter().any(|(k, _)| *k == key) {
        state.push_str(" dock-item-launching");
    }
    let icon_w = launchers::icon_widget(&l.icon, &l.glyph, "dock-icon-img", g.icon);
    let body = item_box(env, "app", state.trim(), icon_w, mine.len(), active, &key);
    // Окна сворачиваются в этот значок.
    let bounds = Arc::new(Mutex::new(Rect::zero()));
    for w in &mine {
        crate::applets::taskbar::track(w.id, &env.st.pc, bounds.clone());
    }
    let st = env.st.clone();
    let (l1, l2, l3) = (l.clone(), l.clone(), l.app_id.clone());
    let panel = env.st.pc.index;
    Box::new(
        EventHook::new().report_bounds(bounds).child(
            InputArea::new(body)
                .pointer()
                .on_click(move |b, _, r| {
                    let ctx = ShellCtx::get();
                    match b {
                        MouseButton::Left => launchers::activate_or_launch(ctx, &l1),
                        MouseButton::Middle => launchers::launch(ctx, &l2),
                        MouseButton::Right => {
                            let kind = match &origin {
                                ItemOrigin::Applet(i) => PopupKind::ItemMenu { panel, index: *i },
                                ItemOrigin::Running(app) => PopupKind::AppMenu { panel, app: app.clone() },
                            };
                            ctx.open_popup(kind, st.anchor_for(slot, r));
                        }
                        _ => {}
                    }
                })
                .on_wheel(move |dy| launchers::cycle_windows(ShellCtx::get(), &l3, dy)),
        ),
    )
}

fn stack_item(env: &ItemEnv, slot: usize, a: &Applet, index: usize) -> Box<dyn Widget> {
    let g = env.st.geo;
    let icon_w = launchers::stack_icon(a, g.icon, "dock-icon-img");
    let key = format!("{}:{index}", env.st.pc.index);
    let open_kind = PopupKind::Stack { panel: env.st.pc.index, index, hover: false };
    let is_open = ShellCtx::get().popup.get_untracked().is_some_and(|p| matches!(p.kind, PopupKind::Stack { panel, index: i, .. } if panel == env.st.pc.index && i == index));
    let body = item_box(env, &a.kind, if is_open { "dock-item-open" } else { "" }, icon_w, 0, false, &key);
    let st = env.st.clone();
    let st2 = env.st.clone();
    let hover_open = a.str_or("open", "click") == "hover";
    let panel = env.st.pc.index;
    let timer: Arc<StdMutex<Option<u64>>> = Default::default();
    Box::new(
        InputArea::new(body)
            .pointer()
            .on_click(move |b, _, r| {
                let ctx = ShellCtx::get();
                match b {
                    MouseButton::Left => {
                        ctx.burst(&key);
                        ctx.open_popup(open_kind.clone(), st.anchor_for(slot, r));
                    }
                    MouseButton::Right => ctx.open_popup(PopupKind::ItemMenu { panel, index }, st.anchor_for(slot, r)),
                    _ => {}
                }
            })
            .on_hover(move |inside| {
                if hover_open {
                    let rect = st2.last_hover.lock().unwrap().filter(|(i, _)| *i == slot).map(|(_, r)| r);
                    launchers::hover_open_stack(&timer, inside, panel, index, &st2.pc, rect);
                }
            }),
    )
}

fn launcher_item(env: &ItemEnv, slot: usize, a: &Applet) -> Box<dyn Widget> {
    let g = env.st.geo;
    let glyph = a.str("icon").map(String::from);
    let icon_w = match glyph.as_deref() {
        Some(i) if i.chars().count() == 1 => launchers::icon_widget(&None, &Some(i.to_string()), "dock-icon-img", g.icon),
        Some(i) => launchers::icon_widget(&crate::xdg::lookup_icon(i), &Some(mi::APPS.to_string()).filter(|_| crate::xdg::lookup_icon(i).is_none()), "dock-icon-img", g.icon),
        None => match crate::xdg::lookup_icon("start-here").or_else(|| crate::xdg::lookup_icon("applications-all")) {
            Some(p) => launchers::icon_widget(&Some(p), &None, "dock-icon-img", g.icon),
            None => launchers::icon_widget(&None, &Some(mi::APPS.to_string()), "dock-icon-img", g.icon),
        },
    };
    let body = item_box(env, "launcher", "", icon_w, 0, false, "launcher");
    let st = env.st.clone();
    Box::new(InputArea::new(body).pointer().on_click(move |b, _, r| {
        if b == MouseButton::Left {
            ShellCtx::get().open_popup(PopupKind::Launcher, st.anchor_for(slot, r));
        }
    }))
}

/// Разделитель: тонкая черта поперёк ряда.
fn separator(env: &ItemEnv) -> Box<dyn Widget> {
    let g = env.st.geo;
    let (w, h) = if env.st.edge.is_vertical() { (g.item, 12.0) } else { (12.0, g.item) };
    Box::new(
        DecoratedBox::new()
            .child(crate::ui::vcenter(DecoratedBox::new().class(if env.st.edge.is_vertical() { "dock-separator-line dock-separator-h" } else { "dock-separator-line" })))
            .class("dock-separator")
            .style("width", StyleValue::px(w))
            .style("height", StyleValue::px(h)),
    )
}

/// Прочие апплеты (часы, громкость, лоток…) — в квадрате значка.
fn applet_item(env: &ItemEnv, slot: usize, a: &Applet, index: usize) -> Box<dyn Widget> {
    let g = env.st.geo;
    let (w, h) = match a.kind.as_str() {
        "clock" | "workspaces" | "tray" | "command" | "cpu" | "memory" if !env.st.edge.is_vertical() => (0.0, g.item),
        _ => (g.item, g.item),
    };
    let mut b = DecoratedBox::new()
        .child(crate::ui::vcenter(crate::applets::build(a, &env.st.pc, index)))
        .class(format!("dock-item dock-applet dock-applet-{}", a.kind))
        .style("height", StyleValue::px(h));
    if w > 0.0 {
        b = b.style("width", StyleValue::px(w));
    }
    // Апплет сам правый клик не берёт — меню этого апплета на доке.
    let st = env.st.clone();
    let panel = env.st.pc.index;
    Box::new(InputArea::new(b).buttons(&[MouseButton::Right]).on_click(move |_, _, r| {
        ShellCtx::get().open_popup(PopupKind::ItemMenu { panel, index }, st.anchor_for(slot, r));
    }))
}

/// Место апплета `taskbar` в режиме редактирования (сами окна — не
/// переставляются).
fn placeholder_item(env: &ItemEnv, glyph: &str, name: &str) -> Box<dyn Widget> {
    let g = env.st.geo;
    Box::new(
        DecoratedBox::new()
            .child(
                Column::new()
                    .gap(2.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(glyph).class("dock-placeholder-icon"))
                    .child(Text::new(name.to_string()).max_lines(2).class("dock-placeholder-label")),
            )
            .class("dock-item dock-placeholder")
            .style("width", StyleValue::px(g.item * 1.6))
            .style("height", StyleValue::px(g.item)),
    )
}
