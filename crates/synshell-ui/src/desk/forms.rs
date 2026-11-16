//! Окна рабочего стола: «Добавить» (виджеты и значки), настройки виджета
//! и меню виджета (удержание, правый щелчок).

use std::cell::Cell;

use synshell_common::config::DeskWidget;
use syngui::input::MouseButton;
use syngui::prelude::*;
use syngui::StyleValue;

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
/// Градиенты цифр часов: значение `text-gradient` и подпись.
pub const GRADIENTS: &[(&str, &str)] = &[
    ("linear-gradient(90deg, #4fc3f7, #ab47bc)", n_!("Океан")),
    ("linear-gradient(90deg, #ff7e5f, #feb47b)", n_!("Закат")),
    ("linear-gradient(90deg, #43e97b, #38f9d7)", n_!("Мята")),
    ("linear-gradient(90deg, #f83600, #f9d423)", n_!("Огонь")),
    ("linear-gradient(90deg, #a18cd1, #fbc2eb)", n_!("Сирень")),
    ("linear-gradient(90deg, #ff5f6d, #ffc371, #47e891, #4fc3f7, #a66cff)", n_!("Радуга")),
];
const CLOCK_EFFECTS: &[(&str, &str)] = &[
    ("none", n_!("Нет")),
    ("shadow", n_!("Тень")),
    ("soft", n_!("Мягкая тень")),
    ("glow", n_!("Свечение")),
    ("neon", n_!("Неон")),
    ("outline", n_!("Контур")),
    ("hollow", n_!("Только контур")),
    ("retro", n_!("Ретро")),
    ("gradient", n_!("Градиент")),
];
const CLOCK_ANIMS: &[(&str, &str)] =
    &[("none", n_!("Нет")), ("pulse", n_!("Пульс")), ("shimmer", n_!("Блик")), ("gradient", n_!("Перелив")), ("rainbow", n_!("Радуга"))];
const TEXT_COLORS: &[(&str, &str)] = &[("", n_!("Белый")), ("accent", n_!("Акцент")), ("#eceff1", n_!("Светлый")), ("#4fc3f7", n_!("Голубой")), ("#66bb6a", n_!("Зелёный")), ("#ffb74d", n_!("Оранжевый")), ("#ef5350", n_!("Красный")), ("#ab47bc", n_!("Фиолетовый")), ("#111111", n_!("Чёрный"))];
/// Шрифты, которые предлагаются первыми (если установлены).
const FONT_PICKS: &[&str] = &[
    "Inter", "Roboto", "Noto Sans", "Montserrat", "Oswald", "Bebas Neue", "Orbitron", "Rajdhani", "JetBrains Mono", "Fira Code", "DejaVu Sans Mono", "Playfair Display",
    "Caveat", "Lobster", "Comfortaa", "Manrope", "Ubuntu", "Cantarell", "Source Serif 4", "Noto Serif",
];

/// Семейства шрифтов системы (fontconfig читается один раз).
fn font_families() -> &'static [String] {
    static F: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    F.get_or_init(synshell_common::fonts::families)
}

/// Ряд вариантов-чипов строкового ключа `key` виджета `index`.
fn str_chips(ctx: ShellCtx, index: usize, key: &'static str, default: &'static str, options: &'static [(&'static str, &'static str)]) -> impl Widget {
    rx(move || {
        let _ = ctx.config.get();
        let cur = items(&ShellCtx::get()).get(index).map(|w| w.str_or(key, default).to_string()).unwrap_or_default();
        let mut flex = Flex::new().wrap().gap(4.0);
        for (v, l) in options {
            flex = flex.child(chip(*l, cur == *v, move || super::update(index, |w| w.set(key, *v))));
        }
        Box::new(flex)
    })
}

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

    let icon_like = super::is_icon(&kind);
    if kind == "clock" {
        return clock_edit(ctx, index, col, status);
    }
    if let Some(v) = sec_view(ctx, index) {
        col = col.child(v);
    }
    col = col.child(sec_size(ctx, index, status));
    if let Some(p) = sec_page(ctx, index) {
        col = col.child(p);
    }
    if !icon_like {
        col = col.child(sec_look(ctx, index));
        col = col.child(labeled(&t!("Цвет"), swatches(ctx, index, "color", "", COLORS)));
    }

    // Размер значка.
    if icon_like {
        col = col.child(labeled(
            &t!("Размер значка"),
            rx(move || {
                let _ = ctx.config.get();
                let own = cur().map(|w| w.int_or("icon_size", 0)).unwrap_or(0);
                let mut flex = Flex::new().wrap().gap(4.0).child(chip(t!("Как у всех"), own <= 0, move || super::update(index, |w| {
                    w.options.remove("icon_size");
                })));
                for v in [32i64, 48, 64, 96, 128, 160, 192, 256] {
                    flex = flex.child(chip(v.to_string(), own == v, move || set_icon_size(index, v)));
                }
                Box::new(flex)
            }),
        ));
        col = col.child(Text::new(t!("Крупный значок сам займёт больше клеток, если рядом свободно; иначе растяните его ручкой в углу в режиме «Изменить рабочий стол».")).max_lines(3).class("add-hint"));
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
        "calendar" => {
            col = col.child(rx(move || {
                let _ = ctx.config.get();
                let on = cur().is_some_and(|w| w.bool_or("weeks", false));
                Box::new(chip(t!("Номера недель"), on, move || super::update(index, |w| w.set("weeks", !on))))
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
        "notes-board" | "notes-page" => {
            let keys: &[(&str, String, String)] = if kind == "notes-board" {
                &[("board", t!("Доска"), t!("Страница с доской или id; пусто — первая")), ("column", t!("Колонка"), t!("Пусто — все открытые"))]
            } else {
                &[("page", t!("Страница"), t!("Название страницы; пусто — страница стола"))]
            };
            for (key, label, hint) in keys {
                let key: &'static str = key;
                let text = w0.str(key).unwrap_or_default().to_string();
                col = col.child(labeled(
                    label,
                    TextField::with_text(text).placeholder(hint.clone()).submit_on_focus_lost(true).on_submit(move |t| {
                        let t = t.to_string();
                        super::update(index, |w| w.set(key, t));
                        super::notes::refresh();
                    }).class("form-field"),
                ));
            }
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
        "storage" => {
            col = col.child(labeled(
                &t!("Разделы"),
                rx(move || {
                    let _ = ctx.config.get();
                    let snap = super::data::snap().get();
                    let chosen = cur().map(|w| w.strings("mounts")).unwrap_or_default();
                    let mut flex = Flex::new().wrap().gap(4.0).child(chip(t!("Все"), chosen.is_empty(), move || super::update(index, |w| {
                        w.options.remove("mounts");
                    })));
                    for m in &snap.mounts {
                        let path = m.path.clone();
                        let on = chosen.contains(&path);
                        let label = format!("{} · {}", super::views::mount_name(&path), path);
                        flex = flex.child(chip(label, on, move || {
                            let path = path.clone();
                            super::update(index, move |w| {
                                let mut list = w.strings("mounts");
                                if let Some(i) = list.iter().position(|p| *p == path) {
                                    list.remove(i);
                                } else {
                                    list.push(path);
                                }
                                if list.is_empty() {
                                    w.options.remove("mounts");
                                } else {
                                    w.set("mounts", toml::Value::Array(list.into_iter().map(toml::Value::String).collect()));
                                }
                            })
                        }));
                    }
                    Box::new(flex)
                }),
            ));
            col = col.child(labeled(
                &t!("Показывать"),
                rx(move || {
                    let _ = ctx.config.get();
                    let w = cur();
                    let removable = w.as_ref().is_none_or(|w| w.bool_or("removable", true));
                    let details = w.as_ref().is_none_or(|w| w.bool_or("details", true));
                    Box::new(
                        Flex::new()
                            .wrap()
                            .gap(4.0)
                            .child(chip(t!("Съёмные диски"), removable, move || super::update(index, |w| w.set("removable", !removable))))
                            .child(chip(t!("Подробности"), details, move || super::update(index, |w| w.set("details", !details)))),
                    )
                }),
            ));
            col = col.child(labeled(
                &t!("Порядок"),
                rx(move || {
                    let _ = ctx.config.get();
                    let sort = cur().map(|w| w.str_or("sort", "system").to_string()).unwrap_or_else(|| "system".into());
                    let mut flex = Flex::new().wrap().gap(4.0);
                    for (v, l) in [("system", t!("Система сначала")), ("fill", t!("По заполненности")), ("size", t!("По размеру"))] {
                        flex = flex.child(chip(l, sort == v, move || super::update(index, |w| w.set("sort", v))));
                    }
                    Box::new(flex)
                }),
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
    if matches!(kind.as_str(), "cpu" | "memory" | "gpu" | "battery" | "network" | "temps" | "apps" | "note" | "storage") {
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

    col = col.child(sep()).child(buttons(index));
    Box::new(ScrollView::new().vertical().child(col).class("desk-form"))
}

/// «Убрать» и «Готово».
fn buttons(index: usize) -> impl Widget {
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
        }))
}

fn cur_item(index: usize) -> Option<DeskWidget> {
    items(&ShellCtx::get()).get(index).cloned()
}

/// Вид виджета (если у типа их несколько).
fn sec_view(ctx: ShellCtx, index: usize) -> Option<Box<dyn Widget>> {
    let kind = cur_item(index)?.kind;
    let k = kind_info(&kind).filter(|k| k.views.len() > 1)?;
    Some(Box::new(labeled(
        &t!("Вид"),
        rx(move || {
            let _ = ctx.config.get();
            let view = cur_item(index).map(|w| super::view_of(&w).to_string()).unwrap_or_default();
            let mut flex = Flex::new().wrap().gap(4.0);
            for (v, l) in k.views {
                flex = flex.child(chip(*l, view == *v, move || super::update(index, |w| w.set("view", *v))));
            }
            Box::new(flex)
        }),
    )))
}

/// Размер в клетках и строка состояния («не помещается»).
fn sec_size(ctx: ShellCtx, index: usize, status: RwSignal<String>) -> Box<dyn Widget> {
    Box::new(
        Column::new()
            .gap(4.0)
            .child(labeled(
                &t!("Размер, клеток"),
                rx(move || {
                    let _ = ctx.config.get();
                    let Some(w) = cur_item(index) else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
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
            ))
            .child(move || Text::new(status.get()).class("add-hint")),
    )
}

/// Стол (если столов больше одного).
fn sec_page(ctx: ShellCtx, index: usize) -> Option<Box<dyn Widget>> {
    let count = ctx.cfg().workspaces.count.max(1);
    (count > 1).then(|| {
        Box::new(labeled(
            &t!("Стол"),
            rx(move || {
                let _ = ctx.config.get();
                let page = cur_item(index).map(|w| w.page).unwrap_or(0);
                let mut flex = Flex::new().wrap().gap(4.0).child(chip(t!("Все"), page == 0, move || move_to_page(index, 0)));
                for p in 1..=count {
                    flex = flex.child(chip(p.to_string(), page == p, move || move_to_page(index, p)));
                }
                Box::new(flex)
            }),
        )) as Box<dyn Widget>
    })
}

/// Подложка и заголовок.
fn sec_look(ctx: ShellCtx, index: usize) -> Box<dyn Widget> {
    Box::new(labeled(
        &t!("Оформление"),
        rx(move || {
            let _ = ctx.config.get();
            let Some(w) = cur_item(index) else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
            let card_default = !matches!(w.kind.as_str(), "clock" | "launcher");
            let card = w.bool_or("card", card_default);
            let title_on = w.bool_or("title", true);
            let mut row = Flex::new().wrap().gap(4.0).child(chip(t!("Подложка"), card, move || super::update(index, |w| w.set("card", !card))));
            if card {
                row = row.child(chip(t!("Заголовок"), title_on, move || super::update(index, |w| w.set("title", !title_on))));
            }
            Box::new(row)
        }),
    ))
}

fn accent_hex() -> String {
    ShellCtx::get().cfg().appearance.palette().accent.hex()
}

/// Образцы цветов кружками для ключа `key`: `""` — `default_label` (акцент или белый), `accent` —
/// акцент темы, иначе `#rrggbb`; последним — поле своего цвета.
fn swatches(ctx: ShellCtx, index: usize, key: &'static str, default: &'static str, options: &'static [(&'static str, &'static str)]) -> impl Widget {
    rx(move || {
        let _ = ctx.config.get();
        let cur = cur_item(index).map(|w| w.str_or(key, default).to_string()).unwrap_or_default();
        let mut row = Flex::new().wrap().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        let known = options.iter().any(|(v, _)| *v == cur);
        for (v, l) in options {
            let hex = match *v {
                "" if key == "text_color" => "#ffffff".to_string(),
                "" | "accent" => accent_hex(),
                h => h.to_string(),
            };
            let on = cur == *v;
            let dot = DecoratedBox::new()
                .class(if on { "desk-swatch on" } else { "desk-swatch" })
                .style("background-color", StyleValue::Color(super::views::mss_color(syngui::core::Color::from_hex(&hex))));
            row = row.child(Tooltip::new(
                InputArea::new(dot).pointer().on_click(move |b, _, _| {
                    if b == MouseButton::Left {
                        super::update(index, |w| w.set(key, *v));
                    }
                }),
                syngui::i18n::t(l),
            ));
        }
        let field = TextField::with_text(if known { String::new() } else { cur.clone() })
            .placeholder("#rrggbb")
            .submit_on_focus_lost(true)
            .on_submit(move |t| {
                let t = t.trim().to_string();
                let t = if t.starts_with('#') { t } else { format!("#{t}") };
                if t.len() == 7 && t[1..].chars().all(|c| c.is_ascii_hexdigit()) {
                    super::update(index, move |w| w.set(key, t));
                }
            })
            .class(if known { "form-field desk-hex" } else { "form-field desk-hex on" });
        Box::new(row.child(field))
    })
}

/// Отложенная запись числового параметра (ползунок): пишется, когда ползунок замер на 300 мс,
/// а не на каждый шаг перетаскивания.
fn set_later(index: usize, key: &'static str, v: i64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static GEN: AtomicU64 = AtomicU64::new(0);
    let g = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    syngui_layer::add_timer(std::time::Duration::from_millis(300), move || {
        if GEN.load(Ordering::SeqCst) == g {
            super::update(index, |w| w.set(key, v));
        }
        None
    });
}

/// Ползунок целого параметра с подписью значения (`fmt`).
fn slider(ctx: ShellCtx, index: usize, key: &'static str, (min, max, step): (f32, f32, f32), fmt: fn(i64) -> String) -> impl Widget {
    rx(move || {
        let _ = ctx.config.get();
        let v = cur_item(index).map(|w| w.int_or(key, 0)).unwrap_or(0);
        let shown = use_signal(v);
        Box::new(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Slider::new().range(min, max).step(step).value(v as f32).on_change(move |x| {
                    let x = x.round() as i64;
                    shown.set(x);
                    set_later(index, key, x);
                }).class("grow"))
                .child(move || Text::new(fmt(shown.get())).class("desk-slider-value")),
        )
    })
}

/// Образец «12:34» в оформлении часов `w` с заменой `key` = `v` (плитки эффектов, шрифтов, градиентов).
fn sample(w: &DeskWidget, key: &str, v: &str, size: f32) -> Box<dyn Widget> {
    let mut w = w.clone();
    w.set(key, v.to_string());
    if key == "gradient" {
        w.set("effect", "gradient");
    }
    super::views::clock_sample(&ShellCtx::get(), &w, "12:34", size)
}

/// Плитки вариантов строкового ключа часов: в каждой — образец, под ним подпись.
fn tiles(ctx: ShellCtx, index: usize, key: &'static str, default: &'static str, options: Vec<(String, String)>) -> impl Widget {
    rx(move || {
        let _ = ctx.config.get();
        let Some(w) = cur_item(index) else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
        let cur = w.str_or(key, default).to_string();
        let mut flex = Flex::new().wrap().gap(6.0);
        for (v, l) in options.clone() {
            let on = cur == v;
            let body = Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(sample(&w, key, &v, 22.0))
                .child(Text::new(syngui::i18n::t(&l)).max_lines(1).class("desk-tile-label"));
            flex = flex.child(InputArea::new(boxed(if on { "desk-tile on" } else { "desk-tile" }, body)).pointer().on_click(move |b, _, _| {
                if b == MouseButton::Left {
                    let v = v.clone();
                    super::update(index, move |w| w.set(key, v));
                }
            }));
        }
        Box::new(flex)
    })
}

thread_local! {
    /// Вкладка, с которой открываются настройки часов (снимки в тестах — каждая вкладка).
    pub(super) static START_TAB: Cell<usize> = const { Cell::new(0) };
}

/// Настройки часов: образец вверху и три вкладки — «Вид», «Шрифт», «Цвет и эффект».
fn clock_edit(ctx: ShellCtx, index: usize, col: Column, status: RwSignal<String>) -> Box<dyn Widget> {
    let tab = use_signal(START_TAB.with(|t| t.get()));
    // Образец — сами часы с текущими настройками (живые).
    let preview = rx(move || {
        let _ = ctx.config.get();
        let Some(w) = cur_item(index) else { return Box::new(DecoratedBox::new()) as Box<dyn Widget> };
        let mut w = w.clone();
        w.set("card", false);
        Box::new(DecoratedBox::new().class("desk-preview").child(super::views::build(ctx, &super::Slot::Item(index), &w, 392.0, 130.0, true)))
    });
    let tabs = rx(move || {
        let t = tab.get();
        let mut row = Row::new().gap(4.0);
        for (i, l) in [n_!("Вид"), n_!("Шрифт"), n_!("Цвет и эффект")].into_iter().enumerate() {
            row = row.child(chip(l, t == i, move || tab.set(i)));
        }
        Box::new(row)
    });
    let body = rx(move || {
        let mut c = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        match tab.get() {
            0 => {
                if let Some(v) = sec_view(ctx, index) {
                    c = c.child(v);
                }
                c = c.child(labeled(
                    &t!("Время и дата"),
                    rx(move || {
                        let _ = ctx.config.get();
                        let w = cur_item(index);
                        let get = |k: &str, d: bool| w.as_ref().is_some_and(|w| w.bool_or(k, d));
                        let (sec, h12, date) = (get("seconds", false), get("h12", false), get("date", true));
                        Box::new(
                            Flex::new()
                                .wrap()
                                .gap(4.0)
                                .child(chip(t!("Секунды"), sec, move || super::update(index, |w| w.set("seconds", !sec))))
                                .child(chip(t!("12 часов"), h12, move || super::update(index, |w| w.set("h12", !h12))))
                                .child(chip(t!("Дата"), date, move || super::update(index, |w| w.set("date", !date)))),
                        )
                    }),
                ));
                c = c.child(labeled(&t!("Вид даты"), str_chips(ctx, index, "date_fmt", "", &[("", n_!("Как у вида")), ("long", n_!("Понедельник, 16 ноября")), ("short", n_!("Пн, 16 нояб")), ("numeric", n_!("16.11.2026"))])));
                c = c.child(sec_size(ctx, index, status));
                if let Some(p) = sec_page(ctx, index) {
                    c = c.child(p);
                }
                c = c.child(sec_look(ctx, index));
            }
            1 => {
                c = c.child(labeled(&t!("Шрифт"), font_picker(ctx, index)));
                c = c.child(labeled(&t!("Насыщенность"), slider(ctx, index, "weight", (0.0, 900.0, 100.0), |v| if v == 0 { t!("как в теме").into() } else { v.to_string() })));
                c = c.child(labeled(&t!("Размер цифр"), slider(ctx, index, "size", (0.0, 300.0, 4.0), |v| if v == 0 { t!("по виджету").into() } else { format!("{v} px") })));
                c = c.child(labeled(&t!("Интервал между буквами"), slider(ctx, index, "spacing", (-6.0, 20.0, 1.0), |v| format!("{v} px"))));
            }
            _ => {
                c = c.child(labeled(&t!("Цвет цифр"), swatches(ctx, index, "text_color", "", TEXT_COLORS)));
                c = c.child(labeled(&t!("Эффект"), tiles(ctx, index, "effect", "shadow", CLOCK_EFFECTS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect())));
                c = c.child(rx(move || {
                    let _ = ctx.config.get();
                    let eff = cur_item(index).map(|w| w.str_or("effect", "shadow").to_string()).unwrap_or_default();
                    if eff == "gradient" {
                        return Box::new(labeled(&t!("Градиент"), tiles(ctx, index, "gradient", GRADIENTS[0].0, GRADIENTS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect()))) as Box<dyn Widget>;
                    }
                    if matches!(eff.as_str(), "glow" | "neon" | "outline" | "retro") {
                        return Box::new(labeled(&t!("Цвет эффекта"), swatches(ctx, index, "color", "", COLORS)));
                    }
                    Box::new(DecoratedBox::new())
                }));
                c = c.child(labeled(&t!("Анимация"), str_chips(ctx, index, "anim", "none", CLOCK_ANIMS)));
            }
        }
        Box::new(c)
    });
    // Прокручивается только содержимое вкладки: образец и вкладки всегда видны.
    let (_, screen_h) = crate::manager::output_size(Some(&super::output_name(&ctx)));
    let body_h = (screen_h - 400.0).clamp(220.0, 460.0);
    let col = col.child(preview).child(tabs).child(DecoratedBox::new().style("height", StyleValue::px(body_h)).child(ScrollView::new().vertical().child(body).class("desk-form-body")));
    Box::new(col.child(sep()).child(buttons(index)))
}

/// Выбор шрифта: поиск по семействам системы; плитка — «12:34» этим шрифтом и имя.
fn font_picker(ctx: ShellCtx, index: usize) -> impl Widget {
    let query = use_signal(String::new());
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(TextField::new().placeholder(t!("Найти шрифт…")).on_change(move |t| query.set(t.to_string())).class("form-field"))
        .child(rx(move || {
            let _ = ctx.config.get();
            let q = query.get().to_lowercase();
            let font = cur_item(index).map(|w| w.str_or("font", "").to_string()).unwrap_or_default();
            let all = font_families();
            if !q.is_empty() && !all.iter().any(|f| f.to_lowercase().contains(&q)) {
                return Box::new(Text::new(t!("Такого шрифта нет")).class("add-hint")) as Box<dyn Widget>;
            }
            let mut list: Vec<(String, String)> = vec![(String::new(), t!("Как в теме").into())];
            let found: Vec<String> = if q.is_empty() {
                FONT_PICKS.iter().filter(|f| all.iter().any(|a| a == *f)).map(|f| f.to_string()).collect()
            } else {
                all.iter().filter(|f| f.to_lowercase().contains(&q)).take(24).cloned().collect()
            };
            if !font.is_empty() && !found.contains(&font) {
                list.push((font.clone(), font.clone()));
            }
            list.extend(found.into_iter().map(|f| (f.clone(), f)));
            Box::new(tiles(ctx, index, "font", "", list))
        }))
}

/// Свой размер значка; не помещается в его клетки — значок занимает больше клеток, если рядом
/// свободно.
fn set_icon_size(index: usize, v: i64) {
    let ctx = ShellCtx::get();
    let g = super::grid(&ctx, &super::output_name(&ctx));
    let Some(w) = items(&ctx).get(index).cloned() else { return };
    let (x, y, ww, hh) = g.cells(&w);
    let need = v as f32 + super::ICON_ROOM;
    let nw = ww.max((need / g.cw).ceil() as u32);
    let nh = hh.max((need / g.ch).ceil() as u32);
    let grow = (nw, nh) != (ww, hh) && x + nw <= g.cols && super::fits(&ctx, &g, &items(&ctx), w.page, (x, y, nw, nh), Some(index));
    super::update(index, |w| {
        w.set("icon_size", v);
        if grow {
            w.w = nw;
            w.h = nh;
        }
    });
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
    let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(title(w.as_ref().map(label_of).unwrap_or_default()));
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
    let col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(title(super::file_name(&p)));
    file_items(col, p).child(sep()).child(menu_item("\u{E3C9}", t!("Изменить рабочий стол"), super::start_editing))
}

