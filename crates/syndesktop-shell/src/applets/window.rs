//! Панель как заголовок развёрнутого окна (как в Plasma с «глобальным
//! меню» и Latte): `window-title` — значок и заголовок активного окна,
//! `window-buttons` — свернуть/развернуть/закрыть, `appmenu` — строка меню
//! программы (dbusmenu, см. `crate::appmenu`).
//!
//! Общие настройки: `only_maximized` — показывать, только пока активное окно
//! на этом выводе развёрнуто (у заголовка и кнопок по умолчанию да, у меню —
//! нет: программа с глобальным меню убирает свою строку меню всегда).
//! Вместе с `[windows] borderless_maximized = true` панель заменяет
//! развёрнутому окну заголовок.

use std::cell::RefCell;
use std::sync::Arc;
use syndesktop_common::config::Applet;
use syndesktop_common::ipc::{WindowInfo, WindowOp};
use syngui::core::sync::Mutex;
use syngui::input::MouseButton;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::EventHook;

use crate::ctx::{PopupKind, ShellCtx};
use crate::panel::PanelCtx;
use crate::ui::{icon, mi, InputArea};

/// Активное окно на выводе панели (реактивно).
fn active(pc: &PanelCtx, only_maximized: bool) -> Option<WindowInfo> {
    let ctx = ShellCtx::get();
    let ws = ctx.active_workspace();
    ctx.windows.get().into_iter().find(|w| {
        w.focused
            && !w.minimized
            && (Some(w.workspace) == ws || w.sticky)
            && w.output.as_deref().is_none_or(|o| o == pc.output)
            && (!only_maximized || w.maximized || w.fullscreen)
    })
}

/// Имя программы окна (из .desktop) или его app_id.
fn app_name(w: &WindowInfo) -> String {
    crate::xdg::app_for_window(&w.app_id).map(|e| e.name).filter(|n| !n.is_empty()).unwrap_or_else(|| w.app_id.clone())
}

fn window_icon(w: &WindowInfo, class: &str) -> Box<dyn Widget> {
    match crate::xdg::window_icon(&w.app_id) {
        Some(p) => Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Contain).placeholder(false).class(class.to_string())),
        None => Box::new(icon(mi::WINDOW).class("window-title-glyph")),
    }
}

/// Пустое место вместо апплета, когда показывать нечего.
fn empty() -> Box<dyn Widget> {
    Box::new(DecoratedBox::new())
}

// ─── Заголовок ───────────────────────────────────────────────────────────────

/// Значок и заголовок активного окна. Двойной щелчок — развернуть/
/// восстановить, перетаскивание — вытащить окно из развёрнутого, средняя
/// кнопка — свернуть, правая — меню окна.
///
/// Настройки: `only_maximized` (да), `icon` (да), `text` — `title`
/// (заголовок), `app` (имя программы) или `both` («Программа — заголовок»),
/// `max_width` (px, 480).
pub fn title(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let only_max = a.bool_or("only_maximized", true);
    let show_icon = a.bool_or("icon", true);
    let text = a.str_or("text", "title").to_string();
    let max_width = a.int_or("max_width", 480).max(40) as f32;
    let pc = pc.clone();
    Box::new(crate::ui::rx(move || {
        let Some(w) = active(&pc, only_max) else { return empty() };
        let id = w.id;
        let label = match text.as_str() {
            "app" => app_name(&w),
            "both" if !w.title.is_empty() => format!("{} — {}", app_name(&w), w.title),
            _ if w.title.is_empty() => app_name(&w),
            _ => w.title.clone(),
        };
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if show_icon {
            row = row.child(window_icon(&w, "window-title-icon"));
        }
        row = row.child(Text::new(label).max_lines(1).class("window-title-text grow"));
        let pcm = pc.clone();
        Box::new(
            InputArea::new(
                DecoratedBox::new()
                    .child(crate::ui::vcenter(row))
                    .class("applet applet-window-title")
                    .style("max-width", StyleValue::px(max_width)),
            )
            .on_click(move |b, _, r| match b {
                MouseButton::Middle => crate::actions::window_op(id, WindowOp::Minimize),
                MouseButton::Right => ShellCtx::get().open_popup(PopupKind::WindowMenu(id), pcm.anchor(r)),
                _ => {}
            })
            .on_double_click(move |b| {
                if b == MouseButton::Left {
                    crate::actions::window_op(id, WindowOp::ToggleMaximize);
                }
            })
            .on_drag(move |b| {
                if b == MouseButton::Left {
                    crate::actions::window_op(id, WindowOp::StartMove);
                }
            }),
        )
    }))
}

// ─── Кнопки ──────────────────────────────────────────────────────────────────

/// Кнопки активного окна. Настройки: `only_maximized` (да), `buttons` —
/// через запятую из `minimize`, `maximize`, `close`.
pub fn buttons(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let only_max = a.bool_or("only_maximized", true);
    let list: Vec<String> =
        a.str_or("buttons", "minimize,maximize,close").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let pc = pc.clone();
    Box::new(crate::ui::rx(move || {
        let Some(w) = active(&pc, only_max) else { return empty() };
        let id = w.id;
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for b in &list {
            let (glyph, op) = match b.as_str() {
                "minimize" => (mi::MINIMIZE, WindowOp::Minimize),
                "maximize" => (if w.maximized { mi::RESTORE } else { mi::MAXIMIZE }, WindowOp::ToggleMaximize),
                "close" => (mi::CLOSE, WindowOp::Close),
                _ => continue,
            };
            row = row.child(
                InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(icon(glyph))).class(format!("window-button window-button-{b}")))
                    .pointer()
                    .buttons(&[MouseButton::Left])
                    .on_click(move |btn, _, _| {
                        if btn == MouseButton::Left {
                            crate::actions::window_op(id, op);
                        }
                    }),
            );
        }
        Box::new(DecoratedBox::new().child(row).class("applet-window-buttons"))
    }))
}

// ─── Глобальное меню ─────────────────────────────────────────────────────────

thread_local! {
    /// Пункты строки меню на панелях: (панель, границы, id) — щелчок по
    /// другому пункту при открытом меню переключает меню.
    static ITEMS: RefCell<Vec<(PanelCtx, Arc<Mutex<Rect>>, i32)>> = const { RefCell::new(Vec::new()) };
}

/// Строка меню активного окна. Настройки: `only_maximized` (нет),
/// `app_name` — первым пунктом жирное имя программы (да).
pub fn appmenu(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let only_max = a.bool_or("only_maximized", false);
    let show_name = a.bool_or("app_name", true);
    let pc = pc.clone();
    Box::new(crate::ui::rx(move || {
        let w = active(&pc, only_max);
        let addr = w.as_ref().and_then(crate::appmenu::address_of);
        crate::appmenu::watch(addr.clone());
        let menu = crate::appmenu::menu();
        ITEMS.with(|it| it.borrow_mut().retain(|(p, _, _)| p.key != pc.key));
        let Some(w) = w else { return empty() };
        let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if show_name {
            row = row.child(DecoratedBox::new().child(crate::ui::vcenter(Text::new(app_name(&w)).max_lines(1).class("appmenu-app"))).class("appmenu-item"));
        }
        let key = addr.as_ref().map(|(s, p)| format!("{s}|{p}"));
        if let Some(m) = menu.filter(|m| Some(&m.key) == key.as_ref()) {
            for e in m.entries.iter().filter(|e| e.visible && !e.separator) {
                row = row.child(menu_button(&pc, e.id, &e.label, e.enabled));
            }
        }
        Box::new(DecoratedBox::new().child(row).class("applet-appmenu"))
    }))
}

fn menu_button(pc: &PanelCtx, id: i32, label: &str, enabled: bool) -> impl Widget {
    let slot = Arc::new(Mutex::new(Rect::zero()));
    ITEMS.with(|it| it.borrow_mut().push((pc.clone(), slot.clone(), id)));
    let pc = pc.clone();
    let open = ShellCtx::get().popup.get().is_some_and(|p| p.kind == PopupKind::GlobalMenu { output: pc.output.clone(), id });
    let mut cls = String::from("appmenu-item appmenu-button");
    if open {
        cls.push_str(" appmenu-open");
    }
    if !enabled {
        cls.push_str(" menu-disabled");
    }
    EventHook::new().report_bounds(slot).child(
        InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(Text::new(label.to_string()).max_lines(1).class("appmenu-label"))).class(cls))
            .buttons(&[MouseButton::Left])
            .on_click(move |b, _, r| {
                if b == MouseButton::Left && enabled {
                    open_menu(&pc, id, r);
                }
            }),
    )
}

fn open_menu(pc: &PanelCtx, id: i32, r: Rect) {
    crate::appmenu::expand(id);
    // Якорь нулевой ширины у левого края пункта: меню встаёт от него
    // вправо, как в строке меню программы.
    let mut anchor = pc.anchor(r);
    if !pc.vertical {
        if let Some(rect) = anchor.rect.as_mut() {
            rect[2] = 0.0;
        }
    }
    ShellCtx::get().open_popup(PopupKind::GlobalMenu { output: pc.output.clone(), id }, anchor);
}

/// Щелчок мимо открытого окна глобального меню в точке `p` (координаты
/// вывода): попал в другой пункт строки меню — открыть его меню.
pub fn switch_menu_at(p: Point) -> bool {
    let ctx = ShellCtx::get();
    let Some(PopupKind::GlobalMenu { output, id: cur }) = ctx.popup.get_untracked().map(|p| p.kind) else { return false };
    let hit = ITEMS.with(|it| {
        it.borrow().iter().find_map(|(pc, slot, id)| {
            if pc.output != output || *id == cur {
                return None;
            }
            let r = *slot.lock().unwrap_or_else(|e| e.into_inner());
            let [x, y, w, h] = pc.anchor(r).rect?;
            (p.x >= x && p.x < x + w && p.y >= y && p.y < y + h).then(|| (pc.clone(), r, *id))
        })
    });
    match hit {
        Some((pc, r, id)) => {
            open_menu(&pc, id, r);
            true
        }
        None => false,
    }
}

/// Окно меню верхнего уровня `id`: подменю открываются на месте.
pub fn menu_view(id: i32) -> impl Widget {
    let sub = use_signal(Vec::<i32>::new());
    Column::new().gap(2.0).child(move || {
        let entries = crate::appmenu::menu().and_then(|m| m.entries.into_iter().find(|e| e.id == id)).map(|e| e.children);
        match entries {
            Some(list) if !list.is_empty() => crate::tray::menu_list(&list, sub, crate::appmenu::activate, crate::appmenu::expand),
            _ => Column::new().child(Text::new("Загрузка…").class("launcher-empty")),
        }
    })
}
