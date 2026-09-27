//! Панели `[[panel]]`: layer-поверхность на краю вывода, внутри — апплеты.
//! Горизонтальные и вертикальные, во всю длину или частью края, плавающие
//! (отступ + скругление, как в Plasma 6), с автоскрытием.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use syndesktop_common::config::{Edge, Panel};
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
        let (ox, oy) = PANELS.with(|p| p.borrow().get(&self.key).map(|p| p.origin).unwrap_or((0.0, 0.0)));
        PopupAnchor {
            output: Some(self.output.clone()),
            rect: Some([r.origin.x + ox, r.origin.y + oy, r.size.width, r.size.height]),
            edge: self.edge,
        }
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
}

/// Завести учёт панели (док ведёт его так же): якоря окон считаются от
/// положения поверхности.
pub(crate) fn register(key: u64, output: &str, edge: Edge, spec: &SurfaceSpec) {
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
            },
        )
    });
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
                        rect: Some([r.origin.x + rt.origin.0, r.origin.y + rt.origin.1, r.size.width, r.size.height]),
                        edge: rt.edge,
                    });
                }
            }
        }
        None
    })
}

pub fn forget(key: u64) {
    PANELS.with(|p| p.borrow_mut().remove(&key));
}

fn spec_for(panel: &Panel, out: &OutputInfo, hidden: bool) -> SurfaceSpec {
    let vertical = panel.edge.is_vertical();
    let gap = if panel.floating && !hidden { FLOAT_GAP } else { 0 };
    let thickness = if hidden { HIDDEN_STRIP } else { panel.size.max(16) };
    let full = panel.length >= 0.999;
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
    let exclusive = if panel.exclusive && !panel.autohide { panel.size as i32 } else { 0 };
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

/// Создать панель (или док) номер `index` на выводе.
pub fn create(ctx: ShellCtx, index: usize, panel: &Panel, out: &OutputInfo) -> (SurfaceId, u64) {
    if panel.is_dock() {
        return crate::dock::create(ctx, index, panel, out);
    }
    let key = next_key();
    let spec = spec_for(panel, out, panel.autohide);
    register(key, &out.name, panel.edge, &spec);
    let hidden = use_signal(panel.autohide);
    let hide_timer: Arc<std::sync::Mutex<Option<u64>>> = Arc::new(std::sync::Mutex::new(None));
    let id_cell: Arc<std::sync::Mutex<Option<SurfaceId>>> = Arc::new(std::sync::Mutex::new(None));

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
        let out_hover = out.clone();
        let id_hover = id_cell.clone();
        let panel_hover = panel.clone();
        SurfaceHooks {
            on_resize: Some(Box::new(move |w, h| {
                resized(key, &out_resize, w, h);
                let _ = &panel_ptr;
            })),
            on_pointer: if panel.autohide {
                Some(Box::new(move |inside| {
                    let Some(id) = *id_hover.lock().unwrap() else { return };
                    if let Some(t) = hide_timer.lock().unwrap().take() {
                        syngui_layer::cancel_timer(t);
                    }
                    if inside {
                        if hidden.get_untracked() {
                            hidden.set(false);
                            let s = spec_for(&panel_hover, &out_hover, false);
                            PANELS.with(|p| {
                                if let Some(rt) = p.borrow_mut().get_mut(&key) {
                                    rt.spec = s.clone();
                                }
                            });
                            syngui_layer::reconfigure_surface(id, s);
                        }
                    } else {
                        let panel = panel_hover.clone();
                        let out = out_hover.clone();
                        let t = syngui_layer::add_timer(Duration::from_millis(700), move || {
                            // Пока открыто окно апплета — не прятать.
                            if ShellCtx::get().popup.get_untracked().is_some() {
                                return Some(Duration::from_millis(500));
                            }
                            hidden.set(true);
                            let s = spec_for(&panel, &out, true);
                            PANELS.with(|p| {
                                if let Some(rt) = p.borrow_mut().get_mut(&key) {
                                    rt.spec = s.clone();
                                }
                            });
                            syngui_layer::reconfigure_surface(id, s);
                            None
                        });
                        *hide_timer.lock().unwrap() = Some(t);
                    }
                }))
            } else {
                None
            },
            ..Default::default()
        }
    };

    let panel_c = panel.clone();
    let cfg = ctx.cfg();
    let id = syngui_layer::create_surface_with(spec, hooks, move || {
        let opacity = panel_c.opacity;
        let bg = opacity.map(|o| {
            let p = cfg.appearance.palette();
            let c = p.bg.with_alpha(o);
            syngui::core::Color::from_srgb(c.r, c.g, c.b, c.a as f32 / 255.0)
        });
        let pcc = pc.clone();
        let panel_v = panel_c.clone();
        Box::new(DecoratedBox::new().class("panel-root").child(crate::ui::rx(move || {
            if hidden.get() {
                return Box::new(DecoratedBox::new().class("panel-hidden")) as Box<dyn Widget>;
            }
            let editing = ShellCtx::get().editing.get() == Some(pcc.index);
            Box::new(view(&panel_v, &pcc, bg, editing))
        })))
    });
    *id_cell.lock().unwrap() = Some(id);
    (id, key)
}

fn view(panel: &Panel, pc: &PanelCtx, bg: Option<syngui::core::Color>, editing: bool) -> impl Widget {
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
        let w: Box<dyn Widget> = if editing || matches!(a.kind.as_str(), "taskbar" | "tray" | "app" | "group" | "folder") {
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
    if panel.floating {
        classes.push_str(" panel-floating");
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
