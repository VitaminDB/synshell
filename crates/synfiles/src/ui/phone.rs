//! Телефонная раскладка (узкое окно): верхняя панель (заголовок, поиск или
//! режим выбора), путь, список под палец, кнопка «Создать», нижние панели
//! действий и вставки, выдвижная панель мест и вкладок, лист «Вид и
//! сортировка». «Назад» (жест, Escape) закрывает верхний слой, снимает
//! выбор, выходит из поиска и лишь потом листает историю.

use syngui::prelude::*;
use syngui::data::ItemSelection;
use syngui::widgets::*;
use syngui::CursorIcon;

use super::{boxed, bx, icons, W};
use crate::actions;
use crate::loc::Location;
use crate::model::{self, SortKey};
use crate::state::{self, Pane, ViewMode};

/// Круглая кнопка-значок под палец.
fn pbtn(glyph: &str, class: &str, on: impl FnMut() + Send + 'static) -> W {
    boxed(
        GestureDetector::new()
            .on_click(on)
            .child(DecoratedBox::new().class(format!("pbtn {class}")).child(Icon::new(glyph).class("icon"))),
    )
}

/// Кнопка-значок, открывающая меню под собой.
fn pmenu_btn(glyph: &str, items: impl Fn() -> Vec<MenuItem> + Send + Sync + 'static) -> W {
    boxed(
        GestureDetector::new()
            .on_click_with_bounds(move |_, r| {
                actions::popup(state::pane(), items(), Point::new(r.origin.x + r.size.width, r.origin.y + r.size.height));
            })
            .child(DecoratedBox::new().class("pbtn").child(Icon::new(glyph).class("icon"))),
    )
}

fn clear_selection(p: Pane) {
    let cursor = p.sel.with_untracked(|s| s.cursor);
    p.sel.set(ItemSelection { cursor, ..Default::default() });
}

fn exit_search(p: Pane) {
    state::ctx().phone_search.set(false);
    if !p.filter.get_untracked().is_empty() {
        p.filter.set(String::new());
        state::refilter(p);
    }
    if matches!(p.loc.get_untracked(), Location::Search { .. }) {
        actions::search(p, "");
    }
}

/// «Назад»: `true` — что-то закрыли или перешли.
pub fn back() -> bool {
    let ctx = state::ctx();
    if ctx.menu_open.get_untracked() {
        ctx.menu_open.set(false);
        return true;
    }
    if ctx.dialog.get_untracked().is_some() {
        ctx.dialog.set(None);
        return true;
    }
    if ctx.sheet.get_untracked() {
        ctx.sheet.set(false);
        return true;
    }
    if ctx.drawer.get_untracked() {
        ctx.drawer.set(false);
        return true;
    }
    let p = state::pane();
    if p.renaming.get_untracked().is_some() {
        p.renaming.set(None);
        return true;
    }
    if p.sel.with_untracked(|s| !s.selected.is_empty()) {
        clear_selection(p);
        return true;
    }
    if ctx.phone_search.get_untracked() || matches!(p.loc.get_untracked(), Location::Search { .. }) {
        exit_search(p);
        return true;
    }
    if p.back.with_untracked(|b| !b.is_empty()) {
        state::go_back(p);
        return true;
    }
    // Без истории — вверх, но не выше домашней папки.
    let home = synshell_common::paths::home();
    if matches!(p.loc.get_untracked(), Location::Dir(d) if d.starts_with(&home) && d != home) {
        state::go_up(p);
        return true;
    }
    false
}

// ---------------------------------------------------------------- верх

fn more_menu() -> Vec<MenuItem> {
    let p = state::pane();
    let loc = p.loc.get_untracked();
    let mut v = vec![
        MenuItem::new("new-tab", "Новая вкладка").icon(icons::TAB),
        MenuItem::new("select-all", "Выбрать всё").icon(icons::SELECT_ALL),
    ];
    if matches!(loc, Location::Trash) {
        v.push(MenuItem::separator());
        v.push(MenuItem::new("trash:empty", "Очистить корзину").icon(icons::DELETE_FOREVER).disabled(crate::trash::is_empty()));
    }
    if let Some(d) = loc.dir() {
        v.push(MenuItem::separator());
        v.push(MenuItem::new("paste", "Вставить").icon(icons::PASTE).disabled(!actions::can_paste()));
        if let Some(t) = crate::ops::undo_title() {
            v.push(MenuItem::new("undo", t).icon(icons::UNDO));
        }
        v.push(MenuItem::separator());
        v.push(MenuItem::new("terminal", "Открыть в терминале").icon(icons::TERMINAL));
        v.push(MenuItem::new("pin-here", if actions::is_pinned(d) { "Открепить от панели" } else { "Закрепить на панели" }).icon(icons::PIN));
        v.push(MenuItem::new("copy-path-here", "Копировать путь").icon(icons::COPY_PATH));
        v.push(MenuItem::new("props-here", "Свойства папки").icon(icons::INFO));
    }
    v
}

fn title_bar(p: Pane) -> W {
    let loc = p.loc.get();
    let title = match &loc {
        Location::Search { query, .. } => format!("«{query}»"),
        l => l.title(),
    };
    boxed(
        Row::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("pbar")
            .child(pbtn(icons::MENU, "", || state::ctx().drawer.set(true)))
            .child(Text::new(title).max_lines(1).class("pbar-title grow"))
            .child(pbtn(icons::SEARCH, "", || state::ctx().phone_search.set(true)))
            .child(pbtn(icons::TUNE, "", || state::ctx().sheet.set(true)))
            .child(pmenu_btn(icons::MORE_VERT, more_menu)),
    )
}

fn search_bar(p: Pane) -> W {
    boxed(
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("pbar")
            .child(pbtn(icons::BACK, "", move || exit_search(p)))
            .child(DecoratedBox::new().class("grow").child(super::toolbar::search_field(true))),
    )
}

fn selection_bar(p: Pane, n: usize) -> W {
    let total = p.entries.with(|e| e.len());
    let all = n == total;
    // Меню выбранного — без того, что уже есть на нижней панели.
    let items = move || {
        let mut v: Vec<MenuItem> = actions::context_menu(state::pane(), true)
            .into_iter()
            .filter(|m| !matches!(m.id.as_str(), "cut" | "copy" | "rename" | "trash"))
            .collect();
        v.dedup_by(|a, b| a.separator && b.separator);
        while v.first().is_some_and(|m| m.separator) {
            v.remove(0);
        }
        while v.last().is_some_and(|m| m.separator) {
            v.pop();
        }
        v
    };
    boxed(
        Row::new()
            .gap(2.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("pbar selecting")
            .child(pbtn(icons::CLOSE, "", move || clear_selection(p)))
            .child(Text::new(format!("Выбрано: {n}")).max_lines(1).class("pbar-title grow"))
            .child(pbtn(if all { icons::DESELECT } else { icons::SELECT_ALL }, "", move || {
                if all {
                    clear_selection(p)
                } else {
                    actions::select_all(p)
                }
            }))
            .child(pmenu_btn(icons::MORE_VERT, items)),
    )
}

fn top_bar() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let p = state::tab_tracked().pane_tracked();
        let n = p.sel.with(|s| s.selected.len());
        let searching = ctx.phone_search.get();
        vec![if n > 0 {
            selection_bar(p, n)
        } else if searching {
            search_bar(p)
        } else {
            title_bar(p)
        }]
    }))
}

/// Путь хлебными крошками (хвост, сколько влезает) и число элементов.
fn path_bar() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let p = state::tab_tracked().pane_tracked();
        let loc = p.loc.get();
        let count = p.entries.with(|e| e.len());
        let loading = p.loading.get();
        let vw = syngui::viewport::viewport_size().get().width;
        let crumbs = loc.crumbs();
        let n = crumbs.len();
        let budget = if vw > 0.0 { ((vw - 150.0) / 7.4).max(10.0) as usize } else { 30 };
        let mut used = 0usize;
        let mut skip = n;
        for (title, _) in crumbs.iter().rev() {
            let w = title.chars().count() + 3;
            if used + w > budget && skip < n {
                break;
            }
            used += w;
            skip -= 1;
        }
        let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(Icon::new(loc.icon()).class("icon pcrumb-icon"));
        if skip > 0 {
            row = row.child(Text::new("…").class("crumb-more"));
        }
        for (i, (title, target)) in crumbs.into_iter().enumerate().skip(skip) {
            if i > skip || skip > 0 {
                row = row.child(Icon::new(icons::CHEVRON).class("icon crumb-sep"));
            }
            let last = i + 1 == n;
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        if !last {
                            state::navigate(p, target.clone(), true);
                        }
                    })
                    .child(DecoratedBox::new().class(if last { "pcrumb last" } else { "pcrumb" }).child(Text::new(title).max_lines(1).class("crumb-text"))),
            );
        }
        let right = if loading { "…".to_string() } else { model::format_count(count as u32) };
        vec![bx(
            "ppath",
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").clip(true).child(row))
                .child(Text::new(right).max_lines(1).class("meta")),
        )]
    }))
}

// ---------------------------------------------------------------- низ

fn action(glyph: &str, label: &str, enabled: bool, on: impl FnMut() + Send + 'static) -> W {
    let inner = DecoratedBox::new().class(if enabled { "paction" } else { "paction disabled" }).child(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(glyph).class("icon"))
            .child(Text::new(label).max_lines(1).class("paction-label")),
    );
    let g = if enabled { GestureDetector::new().on_click(on).child(inner) } else { GestureDetector::new().child(inner) };
    boxed(DecoratedBox::new().class("grow").child(g))
}

fn selection_actions(p: Pane, n: usize, in_trash: bool) -> W {
    let mut row = Row::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if in_trash {
        row = row
            .child(action(icons::RESTORE, "Восстановить", true, move || {
                actions::run(p, "trash:restore");
                clear_selection(p);
            }))
            .child(action(icons::DELETE_FOREVER, "Удалить", true, move || actions::run(p, "delete")));
    } else {
        row = row
            .child(action(icons::COPY, "Копировать", true, move || {
                actions::run(p, "copy");
                clear_selection(p);
            }))
            .child(action(icons::CUT, "Вырезать", true, move || {
                actions::run(p, "cut");
                clear_selection(p);
            }))
            .child(action(icons::RENAME, "Переименовать", n == 1, move || {
                actions::start_rename(p);
                clear_selection(p);
            }))
            .child(action(icons::DELETE, "Удалить", true, move || {
                actions::run(p, "trash");
                if state::ctx().dialog.get_untracked().is_none() {
                    clear_selection(p);
                }
            }));
    }
    bx("pbottom", row)
}

fn paste_bar(p: Pane, clip: state::Clip) -> W {
    let what = model::format_count(clip.paths.len() as u32);
    let hint = if clip.cut { format!("Перемещение: {what}") } else { format!("Копирование: {what}") };
    boxed(
        DecoratedBox::new().class("pbottom ppaste").child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icons::PASTE).class("icon ppaste-icon"))
                .child(
                    Column::new()
                        .gap(1.0)
                        .class("grow")
                        .child(Text::new("Вставить сюда?").max_lines(1).class("ppaste-title"))
                        .child(Text::new(hint).max_lines(1).class("meta")),
                )
                .child(Button::new("Отмена").class("flat").on_click(|| state::ctx().clip.set(None)))
                .child(Button::new("Вставить").class("primary").on_click(move || {
                    actions::run(p, "paste");
                    // Скопированное вставляется один раз, как на Android.
                    state::ctx().clip.set(None);
                })),
        ),
    )
}

fn bottom_bar() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let p = state::tab_tracked().pane_tracked();
        let n = p.sel.with(|s| s.selected.len());
        let loc = p.loc.get();
        let clip = ctx.clip.get();
        if n > 0 {
            return vec![selection_actions(p, n, matches!(loc, Location::Trash))];
        }
        match clip {
            Some(c) if loc.dir().is_some() && !c.paths.is_empty() => vec![paste_bar(p, c)],
            _ => vec![],
        }
    }))
}

/// Плавающая кнопка «Создать».
fn fab_layer() -> W {
    let fab = Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let p = state::tab_tracked().pane_tracked();
        let show = p.loc.with(|l| l.dir().is_some())
            && p.sel.with(|s| s.selected.is_empty())
            && !ctx.phone_search.get()
            && ctx.clip.with(|c| c.is_none());
        if !show {
            return vec![];
        }
        let fab = GestureDetector::new()
            .on_click_with_bounds(move |_, r| {
                actions::popup(state::pane(), actions::new_menu(), Point::new(r.origin.x + r.size.width, r.origin.y - 8.0));
            })
            .child(DecoratedBox::new().class("fab").child(Icon::new(icons::ADD).class("icon")));
        vec![boxed(fab)]
    });
    // Колонка снаружи: `Reactive` отдаёт детям свободные ограничения, и
    // прижать кнопку к углу изнутри него не выйдет.
    boxed(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::End)
            .class("fab-wrap")
            .child(fab),
    )
}

// ---------------------------------------------------------------- слои

/// Затемнение под панелью; касание закрывает её. `Presence` меряет
/// ребёнка по содержимому — размер задаём явно, по окну.
fn scrim(open: RwSignal<bool>) -> W {
    boxed(
        Presence::signal(open, move || {
            boxed(Reactive::new(move || -> Vec<W> {
                let vp = syngui::viewport::viewport_size().get();
                vec![boxed(
                    GestureDetector::new()
                        .cursor(CursorIcon::Default)
                        .on_click(move || open.set(false))
                        .child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)),
                )]
            }))
        })
        .enter(Motion::fade())
        .exit(Motion::fade())
        .duration_ms(200),
    )
}

fn drawer_tab(i: usize, t: state::Tab, current: bool, many: bool) -> W {
    let loc = t.pane_tracked().loc.get();
    let mut row = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(loc.icon()).class("icon place-icon"))
        .child(Text::new(loc.title()).max_lines(1).class("place-title grow"));
    if many {
        row = row.child(
            GestureDetector::new()
                .on_click(move || state::close_tab(i))
                .child(DecoratedBox::new().class("tab-close").child(Icon::new(icons::CLOSE).class("icon"))),
        );
    }
    boxed(
        GestureDetector::new()
            .on_click(move || {
                let ctx = state::ctx();
                ctx.cur.set(i);
                ctx.drawer.set(false);
            })
            .child(DecoratedBox::new().class(if current { "place active" } else { "place" }).child(row)),
    )
}

fn drawer_panel() -> W {
    let tabs = Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let tabs = ctx.tabs.get();
        let cur = ctx.cur.get();
        let many = tabs.len() > 1;
        let mut col = Column::new().gap(1.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new("Вкладки").class("side-title grow"))
                .child(pbtn(icons::ADD, "small", || {
                    let loc = state::pane().loc.get_untracked();
                    state::new_tab(loc, true);
                    state::ctx().drawer.set(false);
                })),
        );
        for (i, t) in tabs.iter().enumerate() {
            col = col.child(drawer_tab(i, *t, i == cur, many));
        }
        vec![boxed(col)]
    });
    let panel = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("drawer")
        .child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("drawer-head")
                .child(DecoratedBox::new().class("drawer-logo").child(Icon::new(icons::FOLDER).class("icon")))
                .child(Text::new("Проводник").class("drawer-title")),
        )
        .child(
            ScrollView::new().vertical().class("grow").child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(tabs)
                    .child(DecoratedBox::new().class("side-sep"))
                    .child(super::sidebar::places_list()),
            ),
        );
    let vw = syngui::viewport::viewport_size().get_untracked().width;
    let w = (vw - 56.0).clamp(240.0, 340.0);
    boxed(Row::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(GestureDetector::new().cursor(CursorIcon::Default).child(DecoratedBox::new().style("width", w).class("drawer-wrap").child(panel))))
}

fn drawer_layer() -> W {
    let open = state::ctx().drawer;
    boxed(
        Stack::new().fit(StackFit::Expand).child(scrim(open)).child(
            Presence::signal(open, drawer_panel)
                .enter(Motion::none().slide(-340.0, 0.0))
                .exit(Motion::none().slide(-340.0, 0.0))
                .duration_ms(240)
                .exit_duration_ms(180),
        ),
    )
}

fn sheet_option(glyph: &str, label: &str, on: bool, f: impl FnMut() + Send + 'static) -> W {
    boxed(
        GestureDetector::new().on_click(f).child(
            DecoratedBox::new().class(if on { "sheet-row on" } else { "sheet-row" }).child(
                Row::new()
                    .gap(14.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(glyph).class("icon"))
                    .child(Text::new(label).class("sheet-label grow"))
                    .child(Icon::new(if on { icons::CHECK } else { "" }).class("icon sheet-check")),
            ),
        ),
    )
}

fn sheet_switch(label: &str, on: bool, f: impl FnMut(bool) + Send + 'static) -> W {
    boxed(
        DecoratedBox::new().class("sheet-row").child(
            Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(label).class("sheet-label grow"))
                .child(Toggle::with_state(on).on_change(f)),
        ),
    )
}

fn view_chip(p: Pane, v: ViewMode, glyph: &str, label: &str, current: bool) -> W {
    boxed(
        DecoratedBox::new().class("grow").child(
            GestureDetector::new().on_click(move || actions::set_view(p, v)).child(
                DecoratedBox::new().class(if current { "view-chip on" } else { "view-chip" }).child(
                    Row::new()
                        .gap(8.0)
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(glyph).class("icon"))
                        .child(Text::new(label).class("sheet-label")),
                ),
            ),
        ),
    )
}

fn sheet_body() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let p = state::tab_tracked().pane_tracked();
        let grid = super::items::phone_grid(p.view.get());
        let s = p.sort.get();
        let hidden = ctx.show_hidden.get();
        let mut col = Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new("Вид").class("side-title"))
            .child(
                Row::new()
                    .gap(8.0)
                    .class("view-chips")
                    .child(view_chip(p, ViewMode::Details, icons::VIEW_LIST, "Список", !grid))
                    .child(view_chip(p, ViewMode::Icons, icons::VIEW_ICONS, "Сетка", grid)),
            )
            .child(Text::new("Сортировка").class("side-title"));
        for (key, glyph) in [(SortKey::Name, icons::SORT), (SortKey::Modified, icons::REFRESH), (SortKey::Type, icons::FILE), (SortKey::Size, icons::DRIVE)] {
            col = col.child(sheet_option(glyph, key.title(), s.key == key, move || {
                let p = state::pane();
                actions::run(p, &format!("sort:{}", key.id()));
            }));
        }
        col = col
            .child(DecoratedBox::new().class("side-sep"))
            .child(sheet_switch("По убыванию", s.descending, move |on| {
                actions::run(state::pane(), if on { "sort:desc" } else { "sort:asc" })
            }))
            .child(sheet_switch("Папки сверху", s.folders_first, move |on| {
                if state::pane().sort.get_untracked().folders_first != on {
                    actions::run(state::pane(), "sort:folders")
                }
            }))
            .child(sheet_switch("Скрытые файлы", hidden, move |on| {
                if state::ctx().show_hidden.get_untracked() != on {
                    actions::toggle_hidden()
                }
            }));
        vec![boxed(col)]
    }))
}

fn sheet_panel() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let vw = syngui::viewport::viewport_size().get().width;
        let panel = DecoratedBox::new().class("sheet").style("width", vw).child(
            Column::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("sheet-handle")))
                .child(sheet_body()),
        );
        vec![boxed(GestureDetector::new().cursor(CursorIcon::Default).child(panel))]
    }))
}

fn sheet_layer() -> W {
    let open = state::ctx().sheet;
    boxed(
        Stack::new().fit(StackFit::Expand).child(scrim(open)).child(
            // Прижать к низу — снаружи `Presence` (он меряется по содержимому).
            Column::new().main_axis_alignment(MainAxisAlignment::End).child(
                Presence::signal(open, sheet_panel)
                    .enter(Motion::none().slide(0.0, 420.0))
                    .exit(Motion::none().slide(0.0, 420.0))
                    .duration_ms(260)
                    .exit_duration_ms(180),
            ),
        ),
    )
}

fn pane() -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let t = state::tab_tracked();
        let idx = if t.split.get() { t.active.get().min(1) } else { 0 };
        vec![super::view::pane_view(t, idx)]
    }))
}

pub fn root() -> W {
    let content = Stack::new()
        .fit(StackFit::Expand)
        .child(pane())
        .child(fab_layer())
        .child(super::app::jobs_layer())
        .child(super::app::toast_layer())
        .class("grow");
    let main = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("window phone")
        .child(top_bar())
        .child(path_bar())
        .child(content)
        .child(bottom_bar());
    let layers = Stack::new()
        .fit(StackFit::Expand)
        .child(main)
        .child(drawer_layer())
        .child(sheet_layer())
        .child(super::dialogs::dialogs())
        .child(super::app::menu_layer());
    let hook = EventHook::new()
        .on_key_down(|k, m| {
            state::set_modifiers(m);
            if k == Key::Escape {
                return if back() { KeyReply::Handled } else { KeyReply::Ignore };
            }
            if state::ctx().dialog.get_untracked().is_some() {
                return KeyReply::Ignore;
            }
            if crate::actions::key(k, m) {
                KeyReply::Handled
            } else {
                KeyReply::Ignore
            }
        })
        .on_key_up(|_, m| {
            state::set_modifiers(m);
            KeyReply::Ignore
        })
        .child(GestureDetector::new().on_back(back).child(layers));
    boxed(hook)
}
