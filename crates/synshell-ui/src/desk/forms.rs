//! Окна рабочего стола: «Добавить» (виджеты и значки), настройки виджета
//! и меню виджета (удержание, правый щелчок).

use std::cell::Cell;

use synshell_common::config::DeskWidget;
use syngui::input::MouseButton;
use syngui::prelude::*;

use super::{items, kind_info, label_of, KINDS};
use crate::ctx::{PopupKind, ShellCtx};
use crate::popup::menu_item;
use crate::ui::{boxed, icon, mi, rx, InputArea};

thread_local! {
    static ADD_TAB: Cell<Option<RwSignal<usize>>> = const { Cell::new(None) };
}

fn add_tab() -> RwSignal<usize> {
    ADD_TAB.with(|t| match t.get() {
        Some(s) => s,
        None => {
            let s = use_signal(0usize);
            t.set(Some(s));
            s
        }
    })
}

/// Какую вкладку «Добавить» открыть: 0 — виджеты, 1 — значки.
pub fn set_add_tab(i: usize) {
    add_tab().set(i);
}

fn title(text: impl Into<String>) -> impl Widget {
    Text::new(text.into()).max_lines(1).class("popup-title")
}

fn sep() -> impl Widget {
    DecoratedBox::new().class("menu-sep")
}

fn chip(label: impl Into<String>, on: bool, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    let label = syngui::i18n::t(&label.into());
    InputArea::new(boxed(if on { "chip chip-on" } else { "chip" }, Text::new(label).class("chip-label"))).pointer().on_click(move |b, _, _| {
        if b == MouseButton::Left {
            f();
        }
    })
}

fn labeled(label: &str, w: impl Widget + 'static) -> impl Widget {
    Column::new().gap(4.0).child(Text::new(label.to_string()).class("form-label")).child(w)
}

// ─── Добавить ───────────────────────────────────────────────────────────────

/// Окно «Добавить на рабочий стол»: виджеты из каталога, значки приложений,
/// файлы и папки. Добавленное встаёт на свободное место стола `page`
/// (0 — на все столы) и включает режим правки.
pub fn add_view(ctx: ShellCtx, page: u32) -> impl Widget {
    let tab = add_tab();
    let mut bar = Row::new().gap(4.0).class("add-tabs");
    for (i, label) in [n_!("Виджеты"), n_!("Значки")].into_iter().enumerate() {
        bar = bar.child(move || {
            let cls = if tab.get() == i { "add-tab add-tab-active" } else { "add-tab" };
            InputArea::new(boxed(cls, Text::new(syngui::i18n::t(label)).class("add-tab-label"))).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    tab.set(i);
                }
            })
        });
    }
    let where_ = if page == 0 { t!("на все столы").to_string() } else { t!("на стол {page}", page = page) };
    Column::new()
        .gap(10.0)
        .child(title(t!("Добавить на рабочий стол")))
        .child(Text::new(t!("Встанет на свободное место, {where_}; перетащите и измените размер в режиме правки.", where_ = where_)).class("add-hint"))
        .child(bar)
        .child(rx(move || -> Box<dyn Widget> {
            match tab.get() {
                0 => Box::new(widget_catalog(ctx, page)),
                _ => Box::new(icon_tab(page)),
            }
        }))
}

fn widget_catalog(ctx: ShellCtx, page: u32) -> impl Widget {
    let mut flex = Flex::new().wrap().gap(6.0).class("add-applets");
    for k in KINDS {
        let kind = k.kind;
        flex = flex.child(
            InputArea::new(
                DecoratedBox::new()
                    .child(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(k.glyph).class("add-applet-icon")).child(Text::new(syngui::i18n::t(k.label)).max_lines(2).class("add-applet-label")))
                    .class("add-applet"),
            )
            .pointer()
            .on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    let w = super::new_widget(&ctx, kind);
                    ShellCtx::get().close_popup();
                    super::add(w, page);
                    super::editing().set(true);
                }
            }),
        );
    }
    flex
}

fn icon_tab(page: u32) -> impl Widget {
    let path = use_signal(String::new());
    let added = use_signal(0usize);
    let add_path = move || {
        let p = path.get_untracked();
        if p.trim().is_empty() {
            return;
        }
        super::add(DeskWidget::new("file", 0, 0, 1, 1).with("path", p.trim().to_string()), page);
        added.set(added.get_untracked() + 1);
    };
    let add_path2 = add_path;
    Column::new()
        .gap(8.0)
        .child(move || {
            let n = added.get();
            Text::new(if n == 0 { t!("Коснитесь приложения — его значок появится на столе.").to_string() } else { t!("Добавлено: {n}", n = n) }).class("add-hint")
        })
        .child(crate::edit::app_picker(
            move |id| {
                super::add(DeskWidget::new("app", 0, 0, 1, 1).with("app", id), page);
                added.set(added.get_untracked() + 1);
            },
            None,
        ))
        .child(
            labeled(
                &t!("Файл или папка"),
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(TextField::new().placeholder(t!("~/Документы")).on_change(move |t| path.set(t.to_string())).on_submit(move |_| add_path()).class("form-field grow"))
                    .child(InputArea::new(boxed("btn", Text::new(t!("Добавить")).class("btn-label"))).pointer().on_click(move |b, _, _| {
                        if b == MouseButton::Left {
                            add_path2();
                        }
                    })),
            ),
        )
}

// ─── Настройки виджета ──────────────────────────────────────────────────────

const COLORS: &[(&str, &str)] = &[("", n_!("Акцент")), ("#4fc3f7", n_!("Голубой")), ("#66bb6a", n_!("Зелёный")), ("#ffb74d", n_!("Оранжевый")), ("#ef5350", n_!("Красный")), ("#ab47bc", n_!("Фиолетовый")), ("#eceff1", n_!("Белый"))];
const HISTORY: &[(i64, &str)] = &[(30, n_!("30 с")), (60, n_!("1 мин")), (120, n_!("2 мин")), (300, n_!("5 мин"))];

/// Настройки виджета `index`: вид, размер, стол, подложка, заголовок, цвет,
/// история графика и свои параметры типа. Всё применяется сразу.
pub fn edit_view(ctx: ShellCtx, index: usize) -> Box<dyn Widget> {
    let Some(w0) = items(&ctx).get(index).cloned() else {
        return Box::new(Text::new(t!("Виджета больше нет")).class("add-hint"));
    };
    // Состояние виджета — живое из конфига (каждая правка его пишет).
    let cur = move || items(&ShellCtx::get()).get(index).cloned();
    let status = use_signal(String::new());
    let kind = w0.kind.clone();
    let mut col = Column::new().gap(12.0).child(title(t!("Виджет «{v}»", v = label_of(&w0))));

    // Вид.
    if let Some(k) = kind_info(&kind).filter(|k| k.views.len() > 1) {
        col = col.child(labeled(
            &t!("Вид"),
            rx(move || {
                let _ = ctx.config.get();
                let view = cur().map(|w| super::view_of(&w).to_string()).unwrap_or_default();
                let mut flex = Flex::new().wrap().gap(4.0);
                for (v, l) in k.views {
                    flex = flex.child(chip(*l, view == *v, move || super::update(index, |w| w.set("view", *v))));
                }
                Box::new(flex)
            }),
        ));
    }

    // Размер в клетках.
    col = col.child(labeled(
        &t!("Размер, клеток"),
        rx(move || {
            let _ = ctx.config.get();
            let Some(w) = cur() else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
            let out = super::output_name(&ctx);
            let g = super::grid(&ctx, &out);
            let (x, y, ww, hh) = g.cells(&w);
            let resize = move |dw: i32, dh: i32| {
                let ctx = ShellCtx::get();
                let nw = (ww as i32 + dw).max(1) as u32;
                let nh = (hh as i32 + dh).max(1) as u32;
                let list = items(&ctx);
                if super::fits(&ctx, &g, &list, w.page, (x, y, nw, nh), Some(index)) {
                    status.set(String::new());
                    super::update(index, |w| {
                        w.w = nw;
                        w.h = nh;
                    });
                } else {
                    status.set(t!("Не помещается: рядом другой виджет или край стола").into());
                }
            };
            let stepper = move |label: &'static str, v: u32, horizontal: bool| {
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(syngui::i18n::t(label)).class("form-label"))
                    .child(chip("−", false, move || if horizontal { resize(-1, 0) } else { resize(0, -1) }))
                    .child(Text::new(v.to_string()).class("desk-step-value"))
                    .child(chip("+", false, move || if horizontal { resize(1, 0) } else { resize(0, 1) }))
            };
            Box::new(Row::new().gap(16.0).child(stepper(n_!("Ширина"), ww, true)).child(stepper(n_!("Высота"), hh, false)))
        }),
    ));
    col = col.child(move || Text::new(status.get()).class("add-hint"));

    // Стол.
    let count = ctx.cfg().workspaces.count.max(1);
    if count > 1 {
        col = col.child(labeled(
            &t!("Стол"),
            rx(move || {
                let _ = ctx.config.get();
                let page = cur().map(|w| w.page).unwrap_or(0);
                let mut flex = Flex::new().wrap().gap(4.0).child(chip(t!("Все"), page == 0, move || move_to_page(index, 0)));
                for p in 1..=count {
                    flex = flex.child(chip(p.to_string(), page == p, move || move_to_page(index, p)));
                }
                Box::new(flex)
            }),
        ));
    }

    // Оформление.
    let icon_like = super::is_icon(&kind);
    if !icon_like {
        col = col.child(labeled(
            &t!("Оформление"),
            rx(move || {
                let _ = ctx.config.get();
                let Some(w) = cur() else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
                let card_default = !matches!(w.kind.as_str(), "clock" | "launcher");
                let card = w.bool_or("card", card_default);
                let title_on = w.bool_or("title", true);
                let mut row = Flex::new().wrap().gap(4.0).child(chip(t!("Подложка"), card, move || super::update(index, |w| w.set("card", !card))));
                if card {
                    row = row.child(chip(t!("Заголовок"), title_on, move || super::update(index, |w| w.set("title", !title_on))));
                }
                Box::new(row)
            }),
        ));
        col = col.child(labeled(
            &t!("Цвет"),
            rx(move || {
                let _ = ctx.config.get();
                let color = cur().and_then(|w| w.str("color").map(String::from)).unwrap_or_default();
                let mut flex = Flex::new().wrap().gap(4.0);
                for (c, l) in COLORS {
                    flex = flex.child(chip(*l, color == *c, move || super::update(index, |w| w.set("color", *c))));
                }
                Box::new(flex)
            }),
        ));
    }

    // История графиков.
    if matches!(kind.as_str(), "cpu" | "memory" | "gpu" | "battery" | "network" | "temps") {
        col = col.child(labeled(
            &t!("История на графике"),
            rx(move || {
                let _ = ctx.config.get();
                let h = cur().map(|w| w.int_or("history", 60)).unwrap_or(60);
                let mut flex = Flex::new().wrap().gap(4.0);
                for (v, l) in HISTORY {
                    flex = flex.child(chip(*l, h == *v, move || super::update(index, |w| w.set("history", *v))));
                }
                Box::new(flex)
            }),
        ));
    }

    // Своё у типа.
    match kind.as_str() {
        "clock" => {
            col = col.child(rx(move || {
                let _ = ctx.config.get();
                let on = cur().is_some_and(|w| w.bool_or("seconds", false));
                Box::new(chip(t!("Секунды"), on, move || super::update(index, |w| w.set("seconds", !on))))
            }));
        }
        "note" => {
            let text = w0.str("text").unwrap_or_default().to_string();
            col = col.child(labeled(
                &t!("Текст"),
                TextField::with_text(text).placeholder(t!("Текст заметки")).submit_on_focus_lost(true).on_submit(move |t| {
                    let t = t.to_string();
                    super::update(index, |w| w.set("text", t))
                }).class("form-field"),
            ));
        }
        "file" => {
            let p = w0.str("path").unwrap_or_default().to_string();
            col = col.child(labeled(
                &t!("Путь"),
                TextField::with_text(p).submit_on_focus_lost(true).on_submit(move |t| {
                    let t = t.to_string();
                    super::update(index, |w| w.set("path", t))
                }).class("form-field"),
            ));
        }
        "launcher" => {
            col = col.child(labeled(
                &t!("Колонок"),
                rx(move || {
                    let _ = ctx.config.get();
                    let n = cur().map(|w| w.int_or("columns", 0)).unwrap_or(0);
                    let mut flex = Flex::new().wrap().gap(4.0).child(chip(t!("Авто"), n == 0, move || super::update(index, |w| w.set("columns", 0i64))));
                    for c in 2..=8i64 {
                        flex = flex.child(chip(c.to_string(), n == c, move || super::update(index, |w| w.set("columns", c))));
                    }
                    Box::new(flex)
                }),
            ));
        }
        _ => {}
    }
    if matches!(kind.as_str(), "cpu" | "memory" | "gpu" | "battery" | "network" | "temps" | "apps" | "note") {
        let name = w0.str("name").unwrap_or_default().to_string();
        col = col.child(labeled(
            &t!("Своё название"),
            TextField::with_text(name).placeholder(label_of(&w0)).submit_on_focus_lost(true).on_submit(move |t| {
                let t = t.trim().to_string();
                super::update(index, |w| {
                    if t.is_empty() {
                        w.options.remove("name");
                    } else {
                        w.set("name", t)
                    }
                })
            }).class("form-field"),
        ));
    }

    col = col.child(sep()).child(
        Row::new()
            .gap(8.0)
            .child(InputArea::new(boxed("btn btn-danger", Text::new(t!("Убрать")).class("btn-label"))).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    ShellCtx::get().close_popup();
                    super::remove(index);
                }
            }))
            .child(DecoratedBox::new().class("grow"))
            .child(InputArea::new(boxed("btn btn-primary", Text::new(t!("Готово")).class("btn-label"))).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    ShellCtx::get().close_popup();
                }
            })),
    );
    Box::new(ScrollView::new().vertical().child(col).class("desk-form"))
}

/// Перенести виджет на другой стол (на свободное место, если там занято).
fn move_to_page(index: usize, page: u32) {
    let ctx = ShellCtx::get();
    let out = super::output_name(&ctx);
    let g = super::grid(&ctx, &out);
    let mut list = items(&ctx);
    let Some(w) = list.get(index).cloned() else { return };
    let cells = g.cells(&w);
    let (x, y) = if super::fits(&ctx, &g, &list, page, cells, Some(index)) {
        (cells.0, cells.1)
    } else {
        let mut others = list.clone();
        others.remove(index);
        super::free_spot(&ctx, &g, &others, page, cells.2, cells.3)
    };
    let w = &mut list[index];
    w.page = page;
    w.x = x;
    w.y = y;
    super::save(list);
}

// ─── Меню ───────────────────────────────────────────────────────────────────

/// Меню виджета (удержание, правый щелчок).
pub fn item_menu(ctx: ShellCtx, index: usize) -> impl Widget {
    let w = items(&ctx).get(index).cloned();
    let mut col = Column::new().gap(2.0).child(title(w.as_ref().map(label_of).unwrap_or_default()));
    if let Some(w) = &w {
        match w.kind.as_str() {
            "app" => {
                if let Some(e) = w.str("app").and_then(crate::xdg::app_by_id) {
                    let l = crate::launchers::Launchable::from_entry(&e);
                    col = col.child(menu_item("\u{E89E}", t!("Открыть"), move || {
                        ShellCtx::get().close_popup();
                        crate::launchers::launch(ShellCtx::get(), &l)
                    }));
                }
            }
            "file" => {
                let p = synshell_common::paths::expand_tilde(w.str("path").unwrap_or_default());
                col = file_items(col, p);
            }
            _ => {}
        }
    }
    col.child(menu_item(mi::SETTINGS, t!("Настроить…"), move || super::open_later(PopupKind::DeskEdit(index))))
    .child(menu_item("\u{E3C9}", t!("Изменить рабочий стол"), super::start_editing))
    .child(sep())
    .child(menu_item(mi::CLOSE, t!("Убрать со стола"), move || {
        ShellCtx::get().close_popup();
        super::remove(index);
    }))
}

fn file_items(mut col: Column, p: std::path::PathBuf) -> Column {
    let cmd = crate::manager::open_command(&p);
    col = col.child(menu_item("\u{E89E}", t!("Открыть"), move || {
        ShellCtx::get().close_popup();
        crate::actions::spawn(&cmd);
    }));
    if let Some(dir) = p.parent().map(|d| d.to_path_buf()) {
        let q = dir.to_string_lossy().replace('\'', "'\\''");
        col = col.child(menu_item("\u{E2C8}", t!("Показать в Проводнике"), move || {
            ShellCtx::get().close_popup();
            crate::actions::spawn(&format!("synfiles '{q}'"));
        }));
    }
    col
}

/// Меню файла `~/Desktop`, место которому выбрано само.
pub fn file_menu(_ctx: ShellCtx, path: &str) -> impl Widget {
    let p = std::path::PathBuf::from(path);
    let col = Column::new().gap(2.0).child(title(super::file_name(&p)));
    file_items(col, p).child(sep()).child(menu_item("\u{E3C9}", t!("Изменить рабочий стол"), super::start_editing))
}

