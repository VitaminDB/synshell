//! Панели `[[panel]]`: layer-поверхность на краю вывода, внутри — апплеты.
//! Горизонтальные и вертикальные, во всю длину или частью края, плавающие
//! (отступ + скругление, как в Plasma 6), с автоскрытием. Плавающая панель
//! умеет «отлипать» (`defloat`): при развёрнутом окне или окне у самой
//! панели она прижимается к краю во всю длину, как адаптивная панель Plasma.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use synshell_common::config::{Edge, Panel};
use syngui::core::sync::Mutex;
use syngui::prelude::*;
use syngui::widgets::EventHook;
use syngui_layer::{Anchor, KeyboardInteractivity, Layer, OutputInfo, SurfaceHooks, SurfaceId, SurfaceSpec};

use crate::ctx::{PopupAnchor, ShellCtx};

/// Отступ плавающей панели от краёв.
const FLOAT_GAP: i32 = 8;
/// Толщина полоски, за которую «вытягивается» скрытая панель.
const HIDDEN_STRIP: u32 = 3;

/// Что знают апплеты о своей панели.
#[derive(Clone)]
pub struct PanelCtx {
    pub key: u64,
    /// Номер `[[panel]]` в конфиге (для правок из режима редактирования).
    pub index: usize,
    pub output: String,
    pub edge: Edge,
    pub vertical: bool,
    /// Толщина панели.
    pub size: u32,
}

impl PanelCtx {
    /// Якорь всплывающего окна у прямоугольника апплета (координаты поверхности панели).
    pub fn anchor(&self, r: Rect) -> PopupAnchor {
        let (rect, attached) = PANELS.with(|p| {
            p.borrow()
                .get(&self.key)
                .map(|rt| (rt.anchor_rect(r), rt.attached))
                .unwrap_or(([r.origin.x, r.origin.y, r.size.width, r.size.height], false))
        });
        PopupAnchor { output: Some(self.output.clone()), rect: Some(rect), edge: self.edge, attached }
    }

    /// Границы апплета номер `index` (для якоря всплывающего окна,
    /// открытого не кликом: наведением, из меню).
    pub fn item_slot(&self, index: usize) -> Arc<Mutex<Rect>> {
        let slot = Arc::new(Mutex::new(Rect::zero()));
        PANELS.with(|p| {
            if let Some(rt) = p.borrow_mut().get_mut(&self.key) {
                rt.items.entry(index).or_insert_with(|| slot.clone()).clone()
            } else {
                slot
            }
        })
    }

    pub fn bounds_of(&self, index: usize) -> Option<Rect> {
        PANELS.with(|p| {
            let p = p.borrow();
            let slot = p.get(&self.key)?.items.get(&index)?.clone();
            let r = *slot.lock().unwrap_or_else(|e| e.into_inner());
            (r.size.width > 0.0).then_some(r)
        })
    }

    /// Границы видимой полосы (док): по ним меню встаёт «по центру дока».
    pub fn set_bar_slot(&self, slot: Arc<Mutex<Rect>>) {
        PANELS.with(|p| {
            if let Some(rt) = p.borrow_mut().get_mut(&self.key) {
                rt.bar = Some(slot);
            }
        });
    }

    /// Запомнить границы апплета (для открытия его окна с клавиатуры).
    pub fn bounds_slot(&self, kind: &str) -> Arc<Mutex<Rect>> {
        let slot = Arc::new(Mutex::new(Rect::zero()));
        PANELS.with(|p| {
            if let Some(rt) = p.borrow_mut().get_mut(&self.key) {
                rt.applets.entry(kind.to_string()).or_insert_with(|| slot.clone()).clone()
            } else {
                slot
            }
        })
    }
}

pub(crate) struct PanelRt {
    output: String,
    edge: Edge,
    /// Положение поверхности на выводе (логическое).
    origin: (f32, f32),
    applets: HashMap<String, Arc<Mutex<Rect>>>,
    /// Границы апплетов по номеру.
    items: HashMap<usize, Arc<Mutex<Rect>>>,
    spec: SurfaceSpec,
    /// Панель прижата к краю: всплывающие окна примыкают к ней.
    attached: bool,
    /// Границы видимой части (док — его полоса значков), иначе — по апплетам.
    bar: Option<Arc<Mutex<Rect>>>,
    /// Показать спрятанную автоскрытием панель (док) — по жесту от края
    /// на телефоне или команде; `true` — была спрятана и показана.
    reveal: Option<Rc<dyn Fn() -> bool>>,
}

/// Завести учёт панели (док ведёт его так же): якоря окон считаются от
/// положения поверхности.
pub(crate) fn register(key: u64, output: &str, edge: Edge, spec: &SurfaceSpec, attached: bool) {
    PANELS.with(|p| {
        p.borrow_mut().insert(
            key,
            PanelRt {
                output: output.to_string(),
                edge,
                origin: (0.0, 0.0),
                applets: HashMap::new(),
                items: HashMap::new(),
                spec: spec.clone(),
                attached,
                bar: None,
                reveal: None,
            },
        )
    });
}

/// Задать, как показать спрятанную панель (см. [`reveal_hidden`]).
pub(crate) fn set_reveal(key: u64, f: Rc<dyn Fn() -> bool>) {
    PANELS.with(|p| {
        if let Some(rt) = p.borrow_mut().get_mut(&key) {
            rt.reveal = Some(f);
        }
    });
}

/// Показать спрятанные автоскрытием панели и доки на краю `edge` (все —
/// без края). Они спрячутся сами через `autohide_delay`. `true` — хоть
/// одна была спрятана: жест от края потрачен на неё, а не на действие.
pub fn reveal_hidden(edge: Option<Edge>) -> bool {
    // Замыкания зовутся вне заимствования: они перенастраивают поверхность
    // и правят PANELS сами.
    let fs: Vec<Rc<dyn Fn() -> bool>> = PANELS.with(|p| {
        p.borrow().values().filter(|rt| edge.is_none_or(|e| rt.edge == e)).filter_map(|rt| rt.reveal.clone()).collect()
    });
    let mut any = false;
    for f in fs {
        any |= f();
    }
    any
}

/// Автоскрытие панели: показ полной толщины и скрытие в полоску у края
/// через `autohide_delay` после ухода указателя (отрыва пальца).
#[derive(Clone)]
struct AutoHide {
    id: Arc<std::sync::Mutex<Option<SurfaceId>>>,
    key: u64,
    hidden: RwSignal<bool>,
    defloated: RwSignal<bool>,
    panel: Panel,
    out: OutputInfo,
    timer: Arc<std::sync::Mutex<Option<u64>>>,
}

impl AutoHide {
    fn cancel(&self) {
        if let Some(t) = self.timer.lock().unwrap().take() {
            syngui_layer::cancel_timer(t);
        }
    }

    fn apply(&self, hidden: bool) {
        let Some(id) = *self.id.lock().unwrap() else { return };
        self.hidden.set(hidden);
        let s = spec_for(&self.panel, &self.out, hidden, self.defloated.get_untracked());
        PANELS.with(|p| {
            if let Some(rt) = p.borrow_mut().get_mut(&self.key) {
                rt.spec = s.clone();
            }
        });
        syngui_layer::reconfigure_surface(id, s);
    }

    fn show(&self) {
        self.cancel();
        if self.hidden.get_untracked() {
            self.apply(false);
        }
    }

    fn arm_hide(&self) {
        self.cancel();
        let me = self.clone();
        let t = syngui_layer::add_timer(Duration::from_millis(self.panel.autohide_delay.max(100) as u64), move || {
            if !alive(me.key) {
                return None;
            }
            // Пока открыто окно апплета — не прятать.
            if ShellCtx::get().popup.get_untracked().is_some() {
                return Some(Duration::from_millis(500));
            }
            me.apply(true);
            None
        });
        *self.timer.lock().unwrap() = Some(t);
    }

    /// Показать по жесту: без указателя над панелью она спрячется сама.
    fn reveal(&self) -> bool {
        if !self.hidden.get_untracked() {
            return false;
        }
        self.show();
        self.arm_hide();
        true
    }
}

impl PanelRt {
    /// Прямоугольник-якорь в координатах вывода. У прижатой панели он
    /// растянут до её края по толщине: карточка окна ляжет вплотную к
    /// панели и перетечёт в неё, а не повиснет у кнопки апплета.
    fn anchor_rect(&self, r: Rect) -> [f32; 4] {
        let (ox, oy) = self.origin;
        if !self.attached {
            return [r.origin.x + ox, r.origin.y + oy, r.size.width, r.size.height];
        }
        let (w, h) = (self.spec.size.0 as f32, self.spec.size.1 as f32);
        match self.edge {
            Edge::Top | Edge::Bottom => [r.origin.x + ox, oy, r.size.width, h],
            Edge::Left | Edge::Right => [ox, r.origin.y + oy, w, r.size.height],
        }
    }
}

/// Поверхность получила размер — пересчитать её положение на выводе.
pub(crate) fn resized(key: u64, out: &OutputInfo, w: u32, h: u32) {
    PANELS.with(|p| {
        if let Some(rt) = p.borrow_mut().get_mut(&key) {
            rt.origin = origin_of(&rt.spec, out, w, h);
        }
    });
}

/// Положение поверхности панели на выводе.
pub(crate) fn origin(key: u64) -> (f32, f32) {
    PANELS.with(|p| p.borrow().get(&key).map(|p| p.origin).unwrap_or((0.0, 0.0)))
}

/// Панель (док) ещё существует — её не разобрали при пересборке.
pub(crate) fn alive(key: u64) -> bool {
    PANELS.with(|p| p.borrow().contains_key(&key))
}

pub(crate) fn next_key() -> u64 {
    NEXT_KEY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

thread_local! {
    static PANELS: RefCell<HashMap<u64, PanelRt>> = RefCell::new(HashMap::new());
}

static NEXT_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Якорь для окна апплета `kind` на первой панели, где он есть.
pub fn applet_anchor(kind: &str) -> Option<PopupAnchor> {
    PANELS.with(|p| {
        let p = p.borrow();
        let mut keys: Vec<&u64> = p.keys().collect();
        keys.sort();
        for k in keys {
            let rt = &p[k];
            if let Some(slot) = rt.applets.get(kind) {
                let r = *slot.lock().unwrap_or_else(|e| e.into_inner());
                if r.size.width > 0.0 {
                    return Some(PopupAnchor {
                        output: Some(rt.output.clone()),
                        rect: Some(rt.anchor_rect(r)),
                        edge: rt.edge,
                        attached: rt.attached,
                    });
                }
            }
        }
        None
    })
}

/// Полоса панели (дока), на которой лежит кнопка `rect` (координаты вывода
/// `output`): границы всех её апплетов — [x, y, w, h]. Для меню «по центру
/// дока», «в начале» и «в конце панели».
pub fn span_at(output: Option<&str>, rect: [f32; 4]) -> Option<[f32; 4]> {
    let (cx, cy) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
    PANELS.with(|p| {
        for rt in p.borrow().values() {
            if output.is_some_and(|o| o != rt.output) {
                continue;
            }
            let (ox, oy) = rt.origin;
            let mut u: Option<[f32; 4]> = None;
            let bar = rt.bar.iter().cloned().collect::<Vec<_>>();
            let slots: Vec<&Arc<Mutex<Rect>>> = if bar.is_empty() { rt.items.values().collect() } else { bar.iter().collect() };
            for r in slots {
                let r = *r.lock().unwrap_or_else(|e| e.into_inner());
                if r.size.width <= 0.0 {
                    continue;
                }
                let (x0, y0, x1, y1) = (r.origin.x + ox, r.origin.y + oy, r.origin.x + ox + r.size.width, r.origin.y + oy + r.size.height);
                u = Some(match u {
                    None => [x0, y0, x1, y1],
                    Some([a, b, c, d]) => [a.min(x0), b.min(y0), c.max(x1), d.max(y1)],
                });
            }
            let Some([x0, y0, x1, y1]) = u else { continue };
            let inside = if rt.edge.is_vertical() { cy >= y0 - 1.0 && cy <= y1 + 1.0 && (cx - (x0 + x1) / 2.0).abs() < 200.0 } else { cx >= x0 - 1.0 && cx <= x1 + 1.0 && (cy - (y0 + y1) / 2.0).abs() < 200.0 };
            if inside {
                return Some([x0, y0, x1 - x0, y1 - y0]);
            }
        }
        None
    })
}

pub fn forget(key: u64) {
    PANELS.with(|p| p.borrow_mut().remove(&key));
}

fn spec_for(panel: &Panel, out: &OutputInfo, hidden: bool, defloated: bool) -> SurfaceSpec {
    let vertical = panel.edge.is_vertical();
    let gap = if panel.floating && !hidden && !defloated { FLOAT_GAP } else { 0 };
    let thick = if defloated && panel.defloated_size > 0 { panel.defloated_size } else { panel.size }.max(16);
    let thickness = if hidden { HIDDEN_STRIP } else { thick };
    // Отлипшая панель — во всю длину края.
    let full = panel.length >= 0.999 || defloated;
    let edge_anchor = match panel.edge {
        Edge::Top => Anchor::TOP,
        Edge::Bottom => Anchor::BOTTOM,
        Edge::Left => Anchor::LEFT,
        Edge::Right => Anchor::RIGHT,
    };
    let (along_start, along_end) = if vertical { (Anchor::TOP, Anchor::BOTTOM) } else { (Anchor::LEFT, Anchor::RIGHT) };
    let mut anchor = edge_anchor;
    let out_len = if vertical { out.size.1 } else { out.size.0 }.max(1) as f32;
    let length = if full {
        anchor |= along_start | along_end;
        0
    } else {
        match panel.align.as_str() {
            "start" | "left" | "top" => anchor |= along_start,
            "end" | "right" | "bottom" => anchor |= along_end,
            _ => {}
        }
        ((out_len - 2.0 * gap as f32) * panel.length.clamp(0.05, 1.0)).round() as u32
    };
    let size = if vertical { (thickness, length) } else { (length, thickness) };
    // Отступы: сверху, справа, снизу, слева.
    let margin = [gap; 4];
    let exclusive = if panel.exclusive && !panel.autohide { thick as i32 } else { 0 };
    SurfaceSpec {
        namespace: "syndesktop-panel".into(),
        layer: Layer::Top,
        anchor,
        size,
        margin,
        exclusive_zone: exclusive,
        keyboard: KeyboardInteractivity::None,
        output: Some(out.name.clone()),
        auto_size: false,
        clear_color: [0.0; 4],
    }
}

/// Положение поверхности на выводе по якорям, отступам и размеру.
fn origin_of(spec: &SurfaceSpec, out: &OutputInfo, w: u32, h: u32) -> (f32, f32) {
    let (ow, oh) = (out.size.0 as f32, out.size.1 as f32);
    let [mt, mr, mb, ml] = spec.margin.map(|m| m as f32);
    let a = spec.anchor;
    let (w, h) = (w as f32, h as f32);
    let x = if a.contains(Anchor::LEFT) {
        ml
    } else if a.contains(Anchor::RIGHT) {
        ow - mr - w
    } else {
        (ow - w) / 2.0
    };
    let y = if a.contains(Anchor::TOP) {
        mt
    } else if a.contains(Anchor::BOTTOM) {
        oh - mb - h
    } else {
        (oh - h) / 2.0
    };
    (x, y)
}

/// Прижать ли плавающую панель к краю (`defloat`) при текущих окнах.
fn want_defloat(panel: &Panel, out: &OutputInfo) -> bool {
    if !panel.floating {
        return false;
    }
    let touch = match panel.defloat.as_str() {
        "maximized" => false,
        "touch" => true,
        _ => return false,
    };
    let ctx = ShellCtx::get();
    let ws = ctx.workspaces.get_untracked().iter().find(|w| w.active).map(|w| w.index);
    let offset = ctx
        .comp_outputs
        .get_untracked()
        .iter()
        .find(|o| o.name == out.name)
        .map(|o| (o.geometry[0], o.geometry[1]))
        .unwrap_or((out.position.0, out.position.1));
    // Место плавающей панели вместе с отступом до края и до окон.
    let zone = if touch {
        let spec = spec_for(panel, out, false, false);
        let (ow, oh) = (out.size.0.max(0) as u32, out.size.1.max(0) as u32);
        let (w, h) = (
            if spec.size.0 == 0 { ow.saturating_sub(2 * FLOAT_GAP as u32) } else { spec.size.0 },
            if spec.size.1 == 0 { oh.saturating_sub(2 * FLOAT_GAP as u32) } else { spec.size.1 },
        );
        let (x, y) = origin_of(&spec, out, w, h);
        let g = FLOAT_GAP as f32;
        Some([x - g, y - g, x + w as f32 + g, y + h as f32 + g])
    } else {
        None
    };
    let title_h = ctx.cfg().decorations.title_height as f32;
    ctx.windows.get_untracked().iter().any(|w| {
        if w.minimized || !(Some(w.workspace) == ws || w.sticky) || w.output.as_deref().is_some_and(|o| o != out.name) {
            return false;
        }
        if w.maximized || w.fullscreen {
            return true;
        }
        let Some([zx0, zy0, zx1, zy1]) = zone else { return false };
        let [x, y, ww, hh] = w.geometry;
        // Окна — в координатах композитора, панель — в единицах интерфейса.
        let z = syngui_layer::ui_zoom();
        let (x0, y0) = ((x - offset.0) as f32 / z, (y - offset.1) as f32 / z);
        // Геометрия — без заголовка; он сверху, высотой из настроек рамок.
        let y0 = y0 - title_h / z;
        let (x1, y1) = (x0 + ww as f32 / z, y0 + (hh as f32 + title_h) / z);
        x0 < zx1 && x1 > zx0 && y0 < zy1 && y1 > zy0
    })
}

/// Создать панель (или док) номер `index` на выводе.
pub fn create(ctx: ShellCtx, index: usize, panel: &Panel, out: &OutputInfo) -> (SurfaceId, u64) {
    if panel.is_dock() {
        return crate::dock::create(ctx, index, panel, out);
    }
    let key = next_key();
    let defloated = use_signal(want_defloat(panel, out));
    let spec = spec_for(panel, out, panel.autohide, defloated.get_untracked());
    register(key, &out.name, panel.edge, &spec, !panel.floating || defloated.get_untracked());
    let hidden = use_signal(panel.autohide);
    let id_cell: Arc<std::sync::Mutex<Option<SurfaceId>>> = Arc::new(std::sync::Mutex::new(None));
    let autohide = panel.autohide.then(|| AutoHide {
        id: id_cell.clone(),
        key,
        hidden,
        defloated,
        panel: panel.clone(),
        out: out.clone(),
        timer: Default::default(),
    });

    let pc = PanelCtx {
        key,
        index,
        output: out.name.clone(),
        edge: panel.edge,
        vertical: panel.edge.is_vertical(),
        size: panel.size,
    };

    let hooks = {
        let out_resize = out.clone();
        let panel_ptr = panel.clone();
        SurfaceHooks {
            on_resize: Some(Box::new(move |w, h| {
                resized(key, &out_resize, w, h);
                let _ = &panel_ptr;
            })),
            on_pointer: autohide.clone().map(|auto| {
                Box::new(move |inside: bool| {
                    if inside {
                        auto.show();
                    } else {
                        auto.arm_hide();
                    }
                }) as Box<dyn FnMut(bool)>
            }),
            ..Default::default()
        }
    };

    let panel_c = panel.clone();
    let id = syngui_layer::create_surface_with(spec, hooks, move || {
        let opacity = panel_c.opacity;
        let pcc = pc.clone();
        let panel_v = panel_c.clone();
        Box::new(DecoratedBox::new().class("panel-root").child(crate::ui::rx(move || {
            if hidden.get() {
                return Box::new(DecoratedBox::new().class("panel-hidden")) as Box<dyn Widget>;
            }
            // Палитра — из живого конфига: смена оформления не пересоздаёт
            // панель, а перекрашивает её.
            let bg = opacity.map(|o| {
                let p = ShellCtx::get().config.get().appearance.palette();
                let c = p.bg.with_alpha(o);
                syngui::core::Color::from_srgb(c.r, c.g, c.b, c.a as f32 / 255.0)
            });
            let editing = ShellCtx::get().editing.get() == Some(pcc.index);
            Box::new(view(&panel_v, &pcc, bg, editing, defloated.get()))
        })))
    });
    *id_cell.lock().unwrap() = Some(id);
    if let Some(auto) = autohide {
        set_reveal(key, Rc::new(move || auto.reveal()));
    }
    if panel.floating && matches!(panel.defloat.as_str(), "maximized" | "touch") {
        start_defloat_timer(id, key, panel.clone(), out.clone(), hidden, defloated);
    }
    (id, key)
}

/// Следить за окнами и прижимать/отпускать плавающую панель.
fn start_defloat_timer(id: SurfaceId, key: u64, panel: Panel, out: OutputInfo, hidden: RwSignal<bool>, defloated: RwSignal<bool>) {
    syngui_layer::add_timer(Duration::from_millis(150), move || {
        // Панель разобрана (перечитан конфиг, сменились мониторы).
        if !alive(key) {
            return None;
        }
        let want = want_defloat(&panel, &out);
        if want != defloated.get_untracked() {
            defloated.set(want);
            let s = spec_for(&panel, &out, hidden.get_untracked(), want);
            PANELS.with(|p| {
                if let Some(rt) = p.borrow_mut().get_mut(&key) {
                    rt.spec = s.clone();
                }
            });
            syngui_layer::reconfigure_surface(id, s);
        }
        Some(Duration::from_millis(150))
    });
}

fn view(panel: &Panel, pc: &PanelCtx, bg: Option<syngui::core::Color>, editing: bool, defloated: bool) -> impl Widget {
    let mut flex = Flex::new()
        .direction(if pc.vertical { FlexDirection::Column } else { FlexDirection::Row })
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center);
    for (ai, a) in panel.applets.iter().enumerate() {
        let w = crate::applets::build(a, pc, ai);
        // Каждый апплет сообщает свои границы — окна по сочетаниям клавиш
        // открываются у его кнопки.
        let slot = pc.bounds_slot(&a.kind);
        let item_slot = pc.item_slot(ai);
        // Правый клик по апплету, который сам его не обрабатывает, — меню
        // этого апплета (убрать, настроить, изменить панель).
        let w: Box<dyn Widget> = if editing || matches!(a.kind.as_str(), "taskbar" | "tray" | "app" | "group" | "folder" | "window-title") {
            w
        } else {
            let pcm = pc.clone();
            let slot = pc.item_slot(ai);
            Box::new(crate::ui::InputArea::new(w).buttons(&[syngui::input::MouseButton::Right]).on_click(move |_, _, _| {
                let r = *slot.lock().unwrap_or_else(|e| e.into_inner());
                ShellCtx::get().open_popup(crate::ctx::PopupKind::ItemMenu { panel: pcm.index, index: ai }, pcm.anchor(r));
            }))
        };
        let w = crate::edit::item_frame(pc, ai, w, editing);
        flex = flex.child(
            EventHook::new()
                .report_bounds(slot)
                .child(EventHook::new().report_bounds(item_slot).child(w))
                .class(format!("applet-slot applet-slot-{}", a.kind)),
        );
    }
    if editing {
        flex = flex.child(crate::edit::edit_controls(pc));
    }
    let edge = match panel.edge {
        Edge::Top => "top",
        Edge::Bottom => "bottom",
        Edge::Left => "left",
        Edge::Right => "right",
    };
    let mut classes = format!("panel panel-{edge}");
    if panel.floating && !defloated {
        classes.push_str(" panel-floating");
    }
    if defloated {
        classes.push_str(" panel-defloated");
    }
    if pc.vertical {
        classes.push_str(" panel-vertical");
    }
    if editing {
        classes.push_str(" panel-editing");
    }
    let b = DecoratedBox::new().child(flex.class("panel-content")).class(classes);
    let b = match bg {
        Some(c) => b.style("background-color", c),
        None => b,
    };
    // Правый клик по пустому месту — меню панели (добавить, изменить).
    let pc = pc.clone();
    // Якорь — точка клика вдоль панели и вся её толщина поперёк: меню
    // встаёт рядом с панелью, а не поверх неё.
    crate::ui::InputArea::new(b).buttons(&[syngui::input::MouseButton::Right]).on_click(move |_, p, bounds| {
        let r = if pc.vertical {
            Rect::new(syngui::core::Point::new(bounds.origin.x, p.y), syngui::core::Size::new(bounds.size.width, 1.0))
        } else {
            Rect::new(syngui::core::Point::new(p.x, bounds.origin.y), syngui::core::Size::new(1.0, bounds.size.height))
        };
        ShellCtx::get().open_popup(crate::ctx::PopupKind::PanelMenu(pc.index), pc.anchor(r));
    })
}
