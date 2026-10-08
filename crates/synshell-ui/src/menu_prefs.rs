//! Меню запуска на рабочем столе: размер и настройки прямо в меню.
//!
//! - Размер — уголок меню (у дока снизу — правый верхний) тянется мышью,
//!   отпустили — сохраняется в `[launcher] width/height`.
//! - «Настроить меню» (кнопка внизу меню) — вид (`style`), положение у
//!   панели (`align`: у кнопки, по центру дока, в начале, в конце), размер,
//!   разделы, колонки закреплённых. Всё пишется в конфиг сразу; вид и
//!   положение применяются переоткрытием меню (настройки остаются открытыми).

use std::cell::Cell;
use synshell_common::config::Edge;
use syngui::input::{CursorIcon, MouseButton};
use syngui::prelude::*;
use syngui::GestureDetector;

use crate::ctx::{PopupKind, ShellCtx};
use crate::ui::{icon, mi, rx, InputArea};

/// Меньше меню не ужимается.
pub const MIN_W: f32 = 460.0;
pub const MIN_H: f32 = 420.0;
/// Размер по умолчанию (как в `[launcher]`).
const DEFAULT_W: u32 = 640;
const DEFAULT_H: u32 = 560;

thread_local! {
    /// Меню переоткрыто из настроек — открыть его сразу на них.
    static REOPEN_SETTINGS: Cell<bool> = const { Cell::new(false) };
}

/// Открыть ли меню сразу на настройках (после переоткрытия из них).
pub fn take_reopen_settings() -> bool {
    REOPEN_SETTINGS.with(|r| r.replace(false))
}

fn clamp(w: f32, h: f32) -> (f32, f32) {
    let (ow, oh) = crate::manager::output_size(None);
    (w.clamp(MIN_W, (ow - 16.0).max(MIN_W)), h.clamp(MIN_H, (oh - 80.0).max(MIN_H)))
}

/// Размер открытого меню из `[launcher]` (сигнал живёт, пока открыто меню).
pub fn size_signal(ctx: &ShellCtx) -> RwSignal<(f32, f32)> {
    let cfg = ctx.cfg();
    use_signal(clamp(cfg.launcher.width as f32, cfg.launcher.height as f32))
}

fn save_size(w: f32, h: f32) {
    let r1 = synshell_common::config_edit::set_value(&["launcher", "width"], toml_edit::Value::from(w.round() as i64));
    let r2 = synshell_common::config_edit::set_value(&["launcher", "height"], toml_edit::Value::from(h.round() as i64));
    if let Err(e) = r1.and(r2) {
        log::error!("меню: размер не сохранён: {e:#}");
    }
    crate::reload_after_write();
}

/// Край панели, у которой открыто меню.
fn menu_edge(ctx: &ShellCtx) -> Edge {
    ctx.popup.get_untracked().map(|p| p.anchor.edge).unwrap_or(Edge::Bottom)
}

/// Уголок изменения размера: ширина растёт вправо, высота — от панели.
pub fn grip(ctx: ShellCtx, size: RwSignal<(f32, f32)>) -> impl Widget {
    let edge = menu_edge(&ctx);
    // Меню над нижней панелью растёт вверх: тянем вверх — выше.
    let dy_sign = if edge == Edge::Top { 1.0 } else { -1.0 };
    let cursor = if edge == Edge::Top { CursorIcon::SeResize } else { CursorIcon::NeResize };
    InputArea::new(
        GestureDetector::new()
            .on_pan_update(move |u| {
                let (w, h) = size.get_untracked();
                size.set(clamp(w + u.delta.x, h + dy_sign * u.delta.y));
            })
            .on_pan_end(move |_| {
                let (w, h) = size.get_untracked();
                save_size(w, h);
            })
            .child(DecoratedBox::new().child(icon("\u{E8D6}").class("menu-grip-icon")).class("menu-grip")),
    )
    .cursor(cursor)
}

/// Записать значение `[launcher] key`.
fn set(key: &str, v: impl Into<toml_edit::Value>) {
    if let Err(e) = synshell_common::config_edit::set_value(&["launcher", key], v.into()) {
        log::error!("меню: {key} не сохранён: {e:#}");
    }
    crate::reload_after_write();
}

/// Переоткрыть меню (новые вид или положение), снова на настройках.
fn reopen() {
    let ctx = ShellCtx::get();
    let anchor = ctx.popup.get_untracked().map(|p| p.anchor);
    REOPEN_SETTINGS.with(|r| r.set(true));
    ctx.close_popup();
    syngui_layer::add_timer(std::time::Duration::from_millis(60), move || {
        let ctx = ShellCtx::get();
        match anchor.clone() {
            Some(a) => ctx.open_popup(PopupKind::Launcher, a),
            None => crate::commands::handle("launcher"),
        }
        None
    });
}

/// Ряд выбора: подписи-«таблетки», выбранная подсвечена.
fn choices(items: &[(&'static str, &'static str, &'static str)], cur: &str, on: impl Fn(&'static str) + Clone + Send + Sync + 'static) -> impl Widget {
    let mut row = Flex::row().gap(6.0).wrap();
    for (key, glyph, label) in items.iter().copied() {
        let on = on.clone();
        row = row.child(
            InputArea::new(
                DecoratedBox::new()
                    .child(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(glyph).class("mp-choice-icon")).child(Text::new(label).class("mp-choice-text")))
                    .class(if cur == key { "mp-choice mp-choice-on" } else { "mp-choice" }),
            )
            .pointer()
            .on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    on(key);
                }
            }),
        );
    }
    row
}

fn section(title: &str) -> impl Widget {
    Text::new(title.to_string()).class("mp-section")
}

fn switch(title: &str, on: bool, f: impl FnMut(bool) + Send + 'static) -> impl Widget {
    Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(title.to_string()).class("mp-label grow"))
        .child(Toggle::with_state(on).on_change(f))
        .class("mp-row")
}

/// Настройки меню (внутри самого меню). `close` — вернуться к приложениям.
pub fn view(ctx: ShellCtx, size: RwSignal<(f32, f32)>, close: impl Fn() + Clone + Send + Sync + 'static) -> impl Widget {
    let back = close.clone();
    let head = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(InputArea::new(DecoratedBox::new().child(icon(mi::BACK).class("mp-back-icon")).class("mp-back")).pointer().on_click(move |_, _, _| back()))
        .child(Text::new("Настройки меню").class("mp-title grow"));
    let body = rx(move || {
        let cfg = ctx.config.get();
        let l = &cfg.launcher;
        let (w, h) = size.get();
        let mut col = Column::new()
            .gap(10.0)
            .child(section("Вид"))
            .child(choices(
                &[("win11", mi::GRID, "Пуск"), ("menu", mi::LIST, "С разделами"), ("fullscreen", mi::FULLSCREEN, "На весь экран")],
                &l.style,
                |k| {
                    set("style", k);
                    reopen();
                },
            ))
            .child(section("Положение у панели"))
            .child(choices(
                &[("icon", "\u{E55F}", "У кнопки"), ("center", "\u{E234}", "По центру дока"), ("start", "\u{E236}", "В начале"), ("end", "\u{E237}", "В конце")],
                &l.align,
                |k| {
                    set("align", k);
                    reopen();
                },
            ))
            .child(section("Размер"))
            .child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(format!("{} × {}", w.round(), h.round())).class("mp-label grow"))
                    .child(Button::new("Сбросить").class("mp-btn").on_click(move || {
                        size.set(clamp(DEFAULT_W as f32, DEFAULT_H as f32));
                        save_size(DEFAULT_W as f32, DEFAULT_H as f32);
                    })),
            )
            .child(Text::new("Потяните за уголок меню, чтобы изменить размер — он запомнится.").class("mp-hint"));
        if l.style == "win11" {
            let cols = l.columns.clamp(3, 10);
            col = col.child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new("Колонок закреплённых").class("mp-label grow"))
                    .child(Button::new("−").class("mp-btn").disabled(cols <= 3).on_click(move || set("columns", (cols as i64) - 1)))
                    .child(Text::new(cols.to_string()).class("mp-label"))
                    .child(Button::new("+").class("mp-btn").disabled(cols >= 10).on_click(move || set("columns", (cols as i64) + 1)))
                    .class("mp-row"),
            );
        }
        if l.style == "menu" {
            col = col
                .child(switch("Раздел «Недавние»", l.show_recent, |v| set("show_recent", v)))
                .child(switch("Разделы по категориям", l.show_categories, |v| set("show_categories", v)));
        }
        col = col
            .child(switch("Калькулятор в поиске", l.calculator, |v| set("calculator", v)))
            .child(switch("Запуск команд из поиска", l.run_commands, |v| set("run_commands", v)));
        Box::new(col)
    });
    Column::new()
        .gap(12.0)
        .child(head)
        .child(ScrollView::new().vertical().child(body).class("grow"))
        .child(Row::new().child(Button::new("Все параметры…").class("mp-btn").on_click(|| {
            ShellCtx::get().close_popup();
            crate::actions::spawn("synsettings");
        })))
        .class("grow mp")
}

/// Кнопка «Настроить меню» для низа меню.
pub fn button(open: RwSignal<bool>, class: &'static str, icon_class: &'static str) -> impl Widget {
    GestureDetector::new().on_click(move || open.set(!open.get_untracked())).child(DecoratedBox::new().child(icon("\u{E429}").class(icon_class)).class(class))
}
