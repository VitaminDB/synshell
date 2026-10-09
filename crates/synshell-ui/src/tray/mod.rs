//! Системный лоток: значки StatusNotifierItem на панели и их меню.
//!
//! Настройки апплета: `hide_passive` (прятать значки со статусом Passive,
//! по умолчанию да), `icon_size` (px, 20), `max_visible` (сколько значков
//! показывать на панели, 0 — все). Спрятанные — под стрелкой «скрытые значки».
//!
//! ЛКМ — Activate (у `ItemIsMenu` — меню), ПКМ — меню dbusmenu (или
//! ContextMenu, если меню нет), СКМ — SecondaryActivate, колесо — Scroll.

pub mod sni;

use std::cell::OnceCell;
use synshell_common::config::Applet;
use syngui::input::MouseButton;
use syngui::mss::StyleValue;
use syngui::prelude::*;

use crate::ctx::{PopupAnchor, PopupKind, ShellCtx};
use crate::panel::PanelCtx;
use crate::ui::{icon, mi, InputArea};
use sni::{Cmd, MenuEntry, TrayIcon, TrayItem, TraySignals};

thread_local! {
    static SIGNALS: OnceCell<TraySignals> = const { OnceCell::new() };
    /// Путь по подменю открытого меню (id пунктов).
    static SUBMENU: OnceCell<RwSignal<Vec<i32>>> = const { OnceCell::new() };
}

fn signals() -> TraySignals {
    SIGNALS.with(|s| *s.get_or_init(|| TraySignals { items: use_signal(Vec::new()), menu: use_signal(None) }))
}

fn submenu() -> RwSignal<Vec<i32>> {
    SUBMENU.with(|s| *s.get_or_init(|| use_signal(Vec::new())))
}

/// Поднять хост (и наблюдателя) — один раз, при первом апплете.
fn ensure_started(icon_px: i32) {
    thread_local! { static STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    if STARTED.with(|s| s.replace(true)) {
        return;
    }
    sni::start(signals(), icon_px);
    // Меню закрыли — фоновому потоку незачем его обновлять.
    let ctx = ShellCtx::get();
    create_effect(move || {
        let open = match ctx.popup.get().map(|p| p.kind) {
            Some(PopupKind::TrayMenu(k)) => Some(k),
            _ => None,
        };
        sni::set_open_menu(open);
    });
}

pub fn icon_widget(ic: &TrayIcon, size: f32, class: &str) -> Box<dyn Widget> {
    let px = StyleValue::px(size);
    match ic {
        TrayIcon::Path(p) => Box::new(
            Image::new(p.clone())
                .fit(ImageFit::Contain)
                .placeholder(false)
                .class(class.to_string())
                .style("width", px.clone())
                .style("height", px),
        ),
        TrayIcon::Rgba { key, w, h, data } => Box::new(
            Image::from_rgba(key.clone(), *w, *h, data.as_ref().clone())
                .fit(ImageFit::Contain)
                .placeholder(false)
                .class(class.to_string())
                .style("width", px.clone())
                .style("height", px),
        ),
        TrayIcon::None => Box::new(icon(mi::APPS).class(class.to_string())),
    }
}

/// Глобальные координаты центра прямоугольника на панели (для Activate).
fn global_point(pc: &PanelCtx, r: Rect) -> (i32, i32) {
    let a = pc.anchor(r);
    let [x, y, w, h] = a.rect.unwrap_or([0.0; 4]);
    let pos = syngui_layer::outputs()
        .get_untracked()
        .into_iter()
        .find(|o| Some(&o.name) == a.output.as_ref())
        .map(|o| o.position)
        .unwrap_or((0, 0));
    let z = syngui_layer::ui_zoom();
    (((x + w / 2.0) * z) as i32 + pos.0, ((y + h / 2.0) * z) as i32 + pos.1)
}

fn open_menu(item: &TrayItem, anchor: PopupAnchor, x: i32, y: i32) {
    if item.menu.is_some() {
        submenu().set(Vec::new());
        signals().menu.set(None);
        sni::send(Cmd::OpenMenu { key: item.key.clone() });
        ShellCtx::get().open_popup(PopupKind::TrayMenu(item.key.clone()), anchor);
    } else {
        sni::send(Cmd::ContextMenu { key: item.key.clone(), x, y });
    }
}

/// Кнопка одного значка.
fn item_button(item: TrayItem, size: f32, pc: PanelCtx) -> impl Widget {
    let attention = item.status == "NeedsAttention";
    let ic = if attention && !item.attention.is_none() { &item.attention } else { &item.icon };
    let mut stack = Stack::new().child(icon_widget(ic, size, "tray-icon"));
    if !item.overlay.is_none() {
        stack = stack.child(icon_widget(&item.overlay, size / 2.0, "tray-overlay"));
    }
    let tip = if item.tooltip.is_empty() { item.title.clone() } else { item.tooltip.clone() };
    let key = item.key.clone();
    let it = item.clone();
    let body = DecoratedBox::new()
        .child(crate::ui::vcenter(stack))
        .class(if attention { "tray-item tray-attention" } else { "tray-item" });
    let _ = tip;
    InputArea::new(body)
        .pointer()
        .on_click(move |b, _, r| {
            let (x, y) = global_point(&pc, r);
            match b {
                MouseButton::Left if it.item_is_menu => open_menu(&it, pc.anchor(r), x, y),
                MouseButton::Left => sni::send(Cmd::Activate { key: it.key.clone(), x, y }),
                MouseButton::Middle => sni::send(Cmd::Secondary { key: it.key.clone(), x, y }),
                MouseButton::Right => open_menu(&it, pc.anchor(r), x, y),
                _ => {}
            }
        })
        .on_wheel(move |dy| sni::send(Cmd::Scroll { key: key.clone(), delta: if dy > 0.0 { -120 } else { 120 } }))
}

/// Разделить значки на видимые и спрятанные.
fn split(items: Vec<TrayItem>, hide_passive: bool, max_visible: usize) -> (Vec<TrayItem>, Vec<TrayItem>) {
    let (mut shown, mut hidden): (Vec<_>, Vec<_>) =
        items.into_iter().partition(|i| !(hide_passive && i.status == "Passive"));
    if max_visible > 0 && shown.len() > max_visible {
        let extra = shown.split_off(max_visible);
        hidden.splice(0..0, extra);
    }
    (shown, hidden)
}

pub fn applet(a: &Applet, pc: &PanelCtx) -> Box<dyn Widget> {
    let size = a.int_or("icon_size", 20).clamp(12, 64) as f32;
    let hide_passive = a.bool_or("hide_passive", true);
    let max_visible = a.int_or("max_visible", 0).max(0) as usize;
    ensure_started((size * 2.0) as i32);
    let sig = signals();
    let pc = pc.clone();
    Box::new(crate::ui::rx(move || {
        let items = sig.items.get();
        let (shown, hidden) = split(items, hide_passive, max_visible);
        let mut flex = Flex::new()
            .direction(if pc.vertical { FlexDirection::Column } else { FlexDirection::Row })
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center);
        if !hidden.is_empty() {
            let pc2 = pc.clone();
            flex = flex.child(
                InputArea::new(DecoratedBox::new().child(crate::ui::vcenter(icon(mi::ARROW_UP))).class("tray-item tray-more"))
                    .pointer()
                    .on_click(move |b, _, r| {
                        if b == MouseButton::Left {
                            ShellCtx::get().open_popup(PopupKind::TrayOverflow, pc2.anchor(r));
                        }
                    }),
            );
        }
        // Связь с устройством: значок в лотке, если своего апплета `link` на
        // панелях нет (конфиг до synlink) и что-то соединено или ждёт ответа.
        if let Some(w) = crate::link::tray_item(&pc) {
            flex = flex.child(w);
        }
        for it in shown {
            flex = flex.child(item_button(it, size, pc.clone()));
        }
        Box::new(DecoratedBox::new().child(flex).class("applet-tray"))
    }))
}

// ─── Всплывающие окна ────────────────────────────────────────────────────────

/// Спрятанные значки (стрелка).
pub fn overflow_view(ctx: ShellCtx) -> impl Widget {
    let sig = signals();
    let cfg = ctx.cfg();
    let applet = cfg.panels.iter().flat_map(|p| p.applets.iter()).find(|a| a.kind == "tray").cloned();
    let hide_passive = applet.as_ref().map(|a| a.bool_or("hide_passive", true)).unwrap_or(true);
    let max_visible = applet.as_ref().map(|a| a.int_or("max_visible", 0).max(0) as usize).unwrap_or(0);
    Column::new().gap(4.0).child(Text::new(t!("Скрытые значки")).class("popup-title")).child(move || {
        let (_, hidden) = split(sig.items.get(), hide_passive, max_visible);
        let mut col = Column::new().gap(2.0);
        for it in hidden {
            let name = if it.title.is_empty() { it.id.clone() } else { it.title.clone() };
            let it2 = it.clone();
            col = col.child(
                InputArea::new(
                    DecoratedBox::new()
                        .child(
                            Row::new()
                                .gap(10.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(icon_widget(&it.icon, 20.0, "tray-icon"))
                                .child(Text::new(name).max_lines(1).class("menu-label")),
                        )
                        .class("menu-item"),
                )
                .pointer()
                .on_click(move |b, _, _| match b {
                    MouseButton::Left => {
                        ShellCtx::get().close_popup();
                        sni::send(Cmd::Activate { key: it2.key.clone(), x: 0, y: 0 });
                    }
                    MouseButton::Right => {
                        let anchor = ShellCtx::get().popup.get_untracked().map(|p| p.anchor);
                        if let Some(a) = anchor {
                            open_menu(&it2, a, 0, 0);
                        }
                    }
                    _ => {}
                }),
            );
        }
        col
    })
}

fn find_path<'a>(entries: &'a [MenuEntry], path: &[i32]) -> Option<&'a [MenuEntry]> {
    let mut cur = entries;
    for id in path {
        cur = &cur.iter().find(|e| e.id == *id)?.children;
    }
    Some(cur)
}

/// Меню значка (dbusmenu): подменю открываются на месте, «Назад» — вверх.
pub fn menu_view(_ctx: ShellCtx, key: String) -> impl Widget {
    let sig = signals();
    let sub = submenu();
    Column::new().gap(2.0).child(move || {
        let menu = sig.menu.get();
        let Some((k, entries)) = menu.filter(|(k, _)| *k == key) else {
            return Column::new().gap(2.0).child(Text::new(t!("Загрузка…")).class("launcher-empty"));
        };
        menu_list(&entries, sub, move |id| sni::send(Cmd::MenuEvent { key: k.clone(), id }), |_| {})
    })
}

/// Список пунктов dbusmenu с переходом по подменю на месте (`sub` — путь
/// по id подменю). `activate` — выбран пункт, `enter` — открыто подменю
/// (глобальное меню подгружает его содержимое).
pub fn menu_list(
    entries: &[MenuEntry],
    sub: RwSignal<Vec<i32>>,
    activate: impl Fn(i32) + Clone + Send + Sync + 'static,
    enter: impl Fn(i32) + Clone + Send + Sync + 'static,
) -> Column {
    let path = sub.get();
    let mut col = Column::new().gap(2.0);
    if !path.is_empty() {
        col = col.child(
            InputArea::new(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(icon("\u{E5C4}").class("menu-icon"))
                            .child(Text::new(t!("Назад")).class("menu-label")),
                    )
                    .class("menu-item"),
            )
            .pointer()
            .on_click(move |_, _, _| sub.update(|p| {
                p.pop();
            })),
        );
    }
    let Some(list) = find_path(entries, &path) else {
        return col.child(Text::new(t!("Меню изменилось")).class("launcher-empty"));
    };
    if list.is_empty() {
        col = col.child(Text::new(t!("Меню пусто")).class("launcher-empty"));
    }
    for e in list.iter().filter(|e| e.visible) {
        if e.separator {
            col = col.child(DecoratedBox::new().class("menu-sep"));
            continue;
        }
        let lead: Box<dyn Widget> = match e.toggle {
            Some((radio, on)) => {
                let g = match (radio, on) {
                    (true, true) => "\u{E837}",
                    (true, false) => "\u{E836}",
                    (false, true) => "\u{E834}",
                    (false, false) => "\u{E835}",
                };
                Box::new(icon(g).class("menu-icon"))
            }
            None if !e.icon.is_none() => icon_widget(&e.icon, 18.0, "menu-img"),
            None => Box::new(DecoratedBox::new().class("menu-icon-space")),
        };
        let mut row = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(lead)
            .child(Text::new(e.label.clone()).max_lines(1).class("menu-label grow"));
        if !e.shortcut.is_empty() {
            row = row.child(Text::new(e.shortcut.clone()).max_lines(1).class("menu-shortcut"));
        }
        let has_sub = e.submenu || !e.children.is_empty();
        if has_sub {
            row = row.child(icon("\u{E5CC}").class("menu-icon"));
        }
        let (id, enabled) = (e.id, e.enabled);
        let (activate, enter) = (activate.clone(), enter.clone());
        col = col.child(
            InputArea::new(DecoratedBox::new().child(row).class(if enabled { "menu-item" } else { "menu-item menu-disabled" }))
                .pointer()
                .on_click(move |b, _, _| {
                    if b != MouseButton::Left || !enabled {
                        return;
                    }
                    if has_sub {
                        enter(id);
                        sub.update(|p| p.push(id));
                    } else {
                        activate(id);
                        ShellCtx::get().close_popup();
                    }
                }),
        );
    }
    col
}

/// Открыть меню `index`-го значка (команда `tray-menu N`).
pub fn open_menu_at(index: usize, anchor: PopupAnchor) {
    if let Some(it) = signals().items.get_untracked().get(index).cloned() {
        open_menu(&it, anchor, 0, 0);
    }
}

/// Activate у `index`-го значка (команда `tray-activate N`).
pub fn activate_at(index: usize) {
    if let Some(it) = signals().items.get_untracked().get(index) {
        sni::send(Cmd::Activate { key: it.key.clone(), x: 0, y: 0 });
    }
}

/// Пункт меню `id` у `index`-го значка (отладочная команда).
pub fn menu_event_at(index: usize, id: i32) {
    if let Some(it) = signals().items.get_untracked().get(index) {
        sni::send(Cmd::MenuEvent { key: it.key.clone(), id });
    }
}
