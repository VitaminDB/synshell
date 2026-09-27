//! Адресная строка (навигация, хлебные крошки, ввод пути, поиск) и командная
//! панель в духе Проводника Windows 11.

use syngui::prelude::*;
use syngui::widgets::*;

use super::{boxed, bx, icon_button, icons, W};
use crate::actions;
use crate::loc::Location;
use crate::state::{self, ViewMode};

/// Кнопка, открывающая меню под собой.
fn menu_button(glyph: &str, label: &str, class: &str, items: impl Fn() -> Vec<MenuItem> + Send + Sync + 'static) -> W {
    let label = label.to_string();
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class("icon"));
    if !label.is_empty() {
        row = row.child(Text::new(label).class("cmd-label"));
    }
    row = row.child(Icon::new(icons::EXPAND).class("icon chevron"));
    boxed(
        GestureDetector::new()
            .on_click_with_bounds(move |_, r| {
                let p = state::pane();
                actions::popup(p, items(), Point::new(r.origin.x, r.origin.y + r.size.height + 4.0));
            })
            .child(DecoratedBox::new().class(format!("cmd-btn {class}")).child(row)),
    )
}

fn sep() -> W {
    bx("cmd-sep", DecoratedBox::new())
}

/// Командная панель.
pub fn command_bar() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let tab = state::tab_tracked();
        let p = tab.pane_tracked();
        let sel_n = p.sel.with(|s| s.selected.len());
        let loc = p.loc.get();
        let in_dir = loc.dir().is_some();
        let in_trash = matches!(loc, Location::Trash);
        let _ = state::ctx().clip.get();
        let has = sel_n > 0;
        let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if in_trash {
            row = row
                .child(super::text_button(icons::RESTORE, "Восстановить", if has { "" } else { "disabled" }, move || actions::run(p, "trash:restore")))
                .child(super::text_button(icons::DELETE_FOREVER, "Очистить корзину", "", move || actions::run(p, "trash:empty")))
                .child(sep());
        } else {
            row = row.child(menu_button(icons::ADD, "Создать", "accent-btn", actions::new_menu)).child(sep());
        }
        row = row
            .child(icon_button(icons::CUT, "Вырезать (Ctrl+X)", "", has && !in_trash, move || actions::run(p, "cut")))
            .child(icon_button(icons::COPY, "Копировать (Ctrl+C)", "", has && !in_trash, move || actions::run(p, "copy")))
            .child(icon_button(icons::PASTE, "Вставить (Ctrl+V)", "", in_dir, move || actions::run(p, "paste")))
            .child(icon_button(icons::RENAME, "Переименовать (F2)", "", sel_n == 1 && !in_trash, move || actions::run(p, "rename")))
            .child(icon_button(icons::DELETE, "Удалить (Delete)", "", has, move || actions::run(p, "trash")))
            .child(sep())
            .child(menu_button(icons::SORT, "Сортировка", "", move || actions::sort_menu(state::pane())))
            .child(menu_button(view_glyph(p.view.get()), "Вид", "", move || actions::view_menu(state::pane())))
            .child(menu_button(icons::MORE, "", "", more_menu))
            .child(DecoratedBox::new().class("grow"))
            .child(icon_button(
                icons::SPLIT,
                "Две панели (F3)",
                if tab.split.get() { "toggled" } else { "" },
                true,
                state::toggle_split,
            ));
        vec![bx("command-bar", row)]
    }))
}

fn view_glyph(v: ViewMode) -> &'static str {
    match v {
        ViewMode::Details => icons::VIEW_DETAILS,
        ViewMode::List => icons::VIEW_LIST,
        ViewMode::Tiles => icons::VIEW_TILES,
        ViewMode::Icons => icons::VIEW_ICONS,
    }
}

fn more_menu() -> Vec<MenuItem> {
    let ctx = state::ctx();
    vec![
        MenuItem::new("select-all", "Выделить всё").icon(icons::SELECT_ALL).shortcut("Ctrl+A"),
        MenuItem::new("select-none", "Снять выделение").icon(icons::DESELECT),
        MenuItem::new("invert-selection", "Обратить выделение").shortcut("Ctrl+Shift+I"),
        MenuItem::separator(),
        MenuItem::new("terminal", "Открыть в терминале").icon(icons::TERMINAL).shortcut("Shift+F4"),
        MenuItem::new("copy-path-here", "Копировать путь папки").icon(icons::COPY_PATH),
        MenuItem::new("undo", crate::ops::undo_title().unwrap_or_else(|| "Отменить".into()))
            .icon(icons::UNDO)
            .shortcut("Ctrl+Z")
            .disabled(crate::ops::undo_title().is_none()),
        MenuItem::separator(),
        {
            let m = MenuItem::new("hidden", "Скрытые файлы").shortcut("Ctrl+H");
            if ctx.show_hidden.get_untracked() {
                m.icon(icons::CHECK)
            } else {
                m
            }
        },
        MenuItem::new("props-here", "Свойства папки").icon(icons::INFO),
    ]
}

/// Хлебные крошки или поле ввода пути.
fn address() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let p = state::tab_tracked().pane_tracked();
        let loc = p.loc.get();
        if ctx.address_edit.get() {
            let field = TextField::with_text(loc.address())
                .autofocus(true)
                .select_range(0, loc.address().len())
                .on_submit(move |s| {
                    match Location::parse(s) {
                        Some(Location::Dir(d)) if d.is_file() => {
                            // Путь к файлу — открыть папку и выделить его.
                            if let Some(parent) = d.parent() {
                                state::navigate(p, Location::Dir(parent.to_path_buf()), true);
                                state::select_later(p, d.clone());
                            }
                        }
                        Some(l) => state::navigate(p, l, true),
                        None => {}
                    }
                    state::ctx().address_edit.set(false);
                })
                .on_escape(move || state::ctx().address_edit.set(false))
                .class("address-field");
            return vec![bx("address edit", field)];
        }
        let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(boxed(Icon::new(loc.icon()).class("icon crumb-icon")));
        let crumbs = loc.crumbs();
        let n = crumbs.len();
        // Длинный путь — только хвост, сколько влезает (начало — по клику на
        // поле или по «…»). Ширина — от окна: слева кнопки, справа поиск.
        let vw = syngui::viewport::viewport_size().get().width;
        let budget = if vw > 0.0 { ((vw - 520.0) / 7.6).max(12.0) as usize } else { 70 };
        let mut used = 0usize;
        let mut skip = n;
        for (title, _) in crumbs.iter().rev() {
            let w = title.chars().count() + 4;
            if used + w > budget && skip < n {
                break;
            }
            used += w;
            skip -= 1;
        }
        if skip > 0 {
            row = row.child(Text::new("…").class("crumb-more"));
        }
        for (i, (title, target)) in crumbs.into_iter().enumerate().skip(skip) {
            row = row.child(Icon::new(icons::CHEVRON).class("icon crumb-sep"));
            let last = i + 1 == n;
            let target2 = target.clone();
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        if !last {
                            state::navigate(p, target.clone(), true);
                        }
                    })
                    .child(
                        DecoratedBox::new()
                            .class(if last { "crumb last" } else { "crumb" })
                            .child(Text::new(title).max_lines(1).class("crumb-text")),
                    ),
            );
            let _ = target2;
        }
        let bar = GestureDetector::new()
            .on_click(move || state::ctx().address_edit.set(true))
            .child(DecoratedBox::new().class("address").clip(true).child(Row::new().child(row).child(DecoratedBox::new().class("grow"))));
        vec![boxed(bar)]
    }))
}

fn search_box() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let focus = ctx.search_focus.get();
        let p = state::tab_tracked().pane_tracked();
        let loc = p.loc.get();
        let place = match &loc {
            Location::Search { root, .. } => Location::Dir(root.clone()).title(),
            l => l.title(),
        };
        let text = match &loc {
            Location::Search { query, .. } => query.clone(),
            _ => p.filter.get_untracked(),
        };
        let field = TextField::with_text(text)
            .placeholder(format!("Поиск: {place}"))
            .prefix_icon(icons::SEARCH)
            .autofocus(focus > 0)
            .on_change(move |s| {
                // Пока печатают — фильтр текущей папки; Enter — поиск вглубь.
                if !matches!(p.loc.get_untracked(), Location::Search { .. }) {
                    p.filter.set(s.to_string());
                    state::refilter(p);
                } else if s.trim().is_empty() {
                    actions::search(p, "");
                }
            })
            .on_submit(move |s| actions::search(p, s))
            .on_escape(move || {
                p.filter.set(String::new());
                state::refilter(p);
                if matches!(p.loc.get_untracked(), Location::Search { .. }) {
                    actions::search(p, "");
                }
            })
            .class("search-field");
        vec![bx("search", field)]
    }))
}

/// Строка навигации: назад/вперёд/вверх/обновить, адрес, поиск.
pub fn nav_bar() -> W {
    let nav = Reactive::new(move || -> Vec<W> {
        let p = state::tab_tracked().pane_tracked();
        let can_back = p.back.with(|b| !b.is_empty());
        let can_fwd = p.fwd.with(|f| !f.is_empty());
        let can_up = p.loc.with(|l| l.parent().is_some());
        let loading = p.loading.get();
        vec![boxed(
            Row::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon_button(icons::BACK, "Назад (Alt+←)", "nav-btn", can_back, move || state::go_back(p)))
                .child(icon_button(icons::FORWARD, "Вперёд (Alt+→)", "nav-btn", can_fwd, move || state::go_forward(p)))
                .child(icon_button(icons::UP, "Вверх (Alt+↑)", "nav-btn", can_up, move || state::go_up(p)))
                .child(icon_button(
                    if loading { icons::CLOSE } else { icons::REFRESH },
                    if loading { "Остановить" } else { "Обновить (F5)" },
                    "nav-btn",
                    true,
                    move || state::load(p, Some(state::selected_paths(p))),
                )),
        )]
    });
    bx(
        "nav-bar",
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(nav)
            .child(DecoratedBox::new().class("grow").child(address()))
            .child(search_box()),
    )
}
