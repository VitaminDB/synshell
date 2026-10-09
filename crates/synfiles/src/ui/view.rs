//! Вид панели: заголовок колонок, элементы, пустая папка, ошибки, перенос.

use std::path::{Path, PathBuf};

use syngui::prelude::*;
use syngui::data::{ItemDrop, ItemLayout, ItemView};
use syngui::widgets::*;
use syngui::DragData;

use super::items::{self, Columns, ItemCtx};
use super::{boxed, bx, icons, W};
use crate::actions;
use crate::loc::Location;
use crate::model::{Entry, SortKey};
use crate::ops::{self, Op};
use crate::state::{self, Pane, Tab, ViewMode};

/// Тип переноса файлов внутри программы (payload — `text/uri-list`).
pub const DRAG_FILES: &str = "text/uri-list";

pub fn drag_paths(d: &DragData) -> Vec<PathBuf> {
    if d.drag_type == DragData::TYPE_FILE {
        return vec![PathBuf::from(&d.payload)];
    }
    d.payload
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| if l.starts_with("file://") { synshell_common::xdg::path_from_uri(l) } else { Some(PathBuf::from(l)) })
        .collect()
}

pub fn accepts(d: &DragData) -> bool {
    d.drag_type == DRAG_FILES || d.drag_type == DragData::TYPE_FILE
}

/// Перенос в папку: Ctrl — копия, Shift — перемещение, Alt — ссылка;
/// иначе в пределах раздела — перемещение, между разделами — копия.
pub fn drop_into(dest: &Path, d: &DragData, m: Modifiers) {
    let srcs: Vec<PathBuf> = drag_paths(d).into_iter().filter(|s| s.parent() != Some(dest) || m.ctrl).collect();
    if srcs.is_empty() {
        return;
    }
    if srcs.iter().any(|s| dest.starts_with(s)) {
        state::toast_error(t!("Нельзя перенести папку в саму себя"));
        return;
    }
    let op = if m.alt {
        Op::Link { srcs, dest: dest.to_path_buf() }
    } else if m.ctrl {
        Op::Copy { srcs, dest: dest.to_path_buf() }
    } else if m.shift || same_device(&srcs[0], dest) {
        Op::Move { srcs, dest: dest.to_path_buf() }
    } else {
        Op::Copy { srcs, dest: dest.to_path_buf() }
    };
    ops::start(op);
}

fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

fn header_cell(p: Pane, key: SortKey, title: &str, class: &str, width: Option<f32>) -> W {
    let s = p.sort.get();
    let active = s.key == key;
    let arrow = if !active {
        ""
    } else if s.descending {
        "\u{e5cf}"
    } else {
        "\u{e5ce}"
    };
    let mut b = DecoratedBox::new().class(format!("hcell {class}{}", if active { " active" } else { "" }));
    if let Some(w) = width {
        b = b.style("width", w);
    }
    boxed(
        GestureDetector::new().on_click(move || actions::set_sort(p, key)).child(
            b.child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(title).max_lines(1).class("htext"))
                    .child(Icon::new(arrow).class("sort-arrow")),
            ),
        ),
    )
}

fn plain_header(title: &str, class: &str, width: f32) -> W {
    boxed(DecoratedBox::new().class(format!("hcell {class}")).style("width", width).child(Text::new(title).max_lines(1).class("htext")))
}

fn header(p: Pane, columns: Columns, fit: items::ColumnsFit) -> W {
    let mut row = Row::new()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("hcell-icon").style("width", 20.0))
        .child(DecoratedBox::new().class("grow").child(header_cell(p, SortKey::Name, &t!("Имя"), "name-cell", None)));
    match columns {
        Columns::Normal => {
            if fit.date {
                row = row.child(header_cell(p, SortKey::Modified, &t!("Дата изменения"), "col-date", Some(items::COL_DATE)));
            }
            if fit.kind {
                row = row.child(header_cell(p, SortKey::Type, &t!("Тип"), "col-type", Some(items::COL_TYPE)));
            }
        }
        Columns::Trash => {
            if fit.kind {
                row = row.child(plain_header(&t!("Откуда удалено"), "col-where", items::COL_WHERE));
            }
            if fit.date {
                row = row.child(plain_header(&t!("Дата удаления"), "col-date", items::COL_DATE));
            }
        }
        Columns::Search => {
            if fit.kind {
                row = row.child(plain_header(&t!("Папка"), "col-where", items::COL_WHERE));
            }
            if fit.date {
                row = row.child(header_cell(p, SortKey::Modified, &t!("Дата изменения"), "col-date", Some(items::COL_DATE)));
            }
        }
    }
    row = row.child(header_cell(p, SortKey::Size, &t!("Размер"), "col-size", Some(items::COL_SIZE)));
    bx("list-header", row)
}

/// Пустая папка, ошибка, «ищем…» — под элементами (не мешает щелчкам).
fn backdrop(p: Pane) -> W {
    let content = Reactive::new(move || -> Vec<W> {
        let loading = p.loading.get();
        let err = p.error.get();
        let empty = p.entries.with(|e| e.is_empty());
        let loc = p.loc.get();
        let (glyph, text): (&str, String) = if let Some(e) = err {
            (icons::WARNING, e)
        } else if !empty {
            return vec![];
        } else if loading {
            (icons::SEARCH, if matches!(loc, Location::Search { .. }) { t!("Поиск…").into() } else { t!("Загрузка…").into() })
        } else {
            match loc {
                Location::Trash => (icons::TRASH, t!("Корзина пуста").into()),
                Location::Search { query, .. } => (icons::SEARCH, t!("По запросу «{query}» ничего не найдено", query = query)),
                Location::Dir(_) if !p.filter.get().is_empty() => (icons::SEARCH, t!("Нет подходящих файлов").into()),
                Location::Dir(_) => (icons::FOLDER_OPEN, t!("Эта папка пуста").into()),
            }
        };
        vec![boxed(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(glyph).class("empty-icon"))
                .child(Text::new(text).class("empty-text")),
        )]
    });
    // Колонка снаружи `Reactive` (он отдаёт свободные ограничения): иначе
    // надпись прижималась к левому краю.
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("empty-state")
            .child(content),
    )
}

fn set_active(tab: Tab, idx: usize) {
    if tab.split.get_untracked() && tab.active.get_untracked() != idx {
        tab.active.set(idx);
    }
}

/// Элементы панели (виртуализированный вид).
fn items_view(tab: Tab, idx: usize, fit: RwSignal<items::ColumnsFit>) -> W {
    let p = tab.panes[idx];
    boxed(Reactive::new(move || -> Vec<W> {
        let ctx = state::ctx();
        let entries = p.entries.get();
        let sel = p.sel.get();
        let mode = p.view.get();
        let icon_px = p.icon_size.get();
        let renaming = p.renaming.get();
        let cut = ctx.clip.get().filter(|c| c.cut).map(|c| c.paths).unwrap_or_default();
        let _ = ctx.thumbs_rev.get();
        let cfg = ctx.cfg.get();
        let generation = p.generation.get();
        let scroll = p.scroll_to.get();
        let active = !tab.split.get() || tab.active.get() == idx;
        let loc = p.loc.get();
        let columns = match loc {
            Location::Trash => Columns::Trash,
            Location::Search { .. } => Columns::Search,
            Location::Dir(_) => Columns::Normal,
        };
        let phone = state::is_phone();
        let icon_px = if phone { icon_px.clamp(48, 96) } else { icon_px };
        let selecting = phone && !sel.selected.is_empty();
        let fit = fit.get();
        let cx = ItemCtx { pane: p, mode, icon_px, cut, renaming, thumbs: cfg.files.thumbnails, columns, phone, selecting, fit };
        let (w, h) = if phone { items::phone_cell_size(mode, icon_px) } else { items::cell_size(mode, icon_px) };
        let layout = if w == 0.0 || (!phone && mode == ViewMode::Details) {
            ItemLayout::Rows { row_height: h }
        } else {
            ItemLayout::Grid { item_width: w, item_height: h, gap: if mode == ViewMode::Icons { 6.0 } else { 4.0 } }
        };
        let labels = entries.clone();
        let for_drag = entries.clone();
        let for_filter = entries.clone();
        let for_drop = entries.clone();
        let for_open = entries.clone();
        let for_mid = entries.clone();
        let here = loc.dir().map(Path::to_path_buf);
        let here2 = here.clone();
        let mut view = ItemView::new(entries.len(), items::builder(cx, entries.clone()))
            .layout(layout)
            .selection(sel)
            .keyboard_active(active)
            .touch_mode(phone)
            .stretch(phone)
            .reset_key(generation)
            .class(format!("items {}{}", mode.id(), if phone { " phone" } else { "" }))
            .item_label(move |i| labels.get(i).map(items::label).unwrap_or_default())
            .on_selection_change(move |s| {
                set_active(tab, idx);
                p.sel.set(s);
            })
            .on_activate(move |i| {
                if let Some(e) = for_open.get(i) {
                    let e: Entry = e.clone();
                    actions::open_entry(p, &e, false);
                }
            })
            .on_middle_click(move |i| {
                if let Some(e) = i.and_then(|i| for_mid.get(i)) {
                    if e.is_dir {
                        state::new_tab(Location::Dir(e.path.clone()), false);
                    }
                }
            })
            .on_nav_button(move |b| match b {
                MouseButton::Back => state::go_back(p),
                MouseButton::Forward => state::go_forward(p),
                _ => {}
            })
            .on_zoom(move |d| actions::zoom(p, d))
            .on_context_menu(move |(item, at)| {
                set_active(tab, idx);
                let items = actions::context_menu(p, item.is_some());
                actions::popup(p, items, at);
            })
            .on_drag_start(move |s| {
                let paths: Vec<PathBuf> = s.selected.iter().filter_map(|&i| for_drag.get(i).map(|e| e.path.clone())).collect();
                if paths.is_empty() {
                    return None;
                }
                let label = if paths.len() == 1 {
                    ops::name_of(&paths[0])
                } else {
                    crate::model::format_count(paths.len() as u32)
                };
                let mut d = DragData::new(DRAG_FILES, syngui::clipboard::uri_list(&paths), 0);
                d.label = Some(label);
                Some(d)
            })
            .drop_filter(move |target, d| {
                if !accepts(d) {
                    return false;
                }
                match target {
                    Some(i) => for_filter.get(i).map(|e| e.is_dir && e.trash.is_none()).unwrap_or(false),
                    None => here.is_some(),
                }
            })
            .on_drop(move |drop: ItemDrop| {
                let dest = match drop.target {
                    Some(i) => for_drop.get(i).map(|e| e.path.clone()),
                    None => here2.clone(),
                };
                if let Some(d) = dest {
                    drop_into(&d, &drop.data, drop.modifiers);
                }
            });
        if let Some((i, n)) = scroll {
            view = view.scroll_to(i, n);
        }
        vec![boxed(view)]
    }))
}

fn columns_of(loc: &Location) -> Columns {
    match loc {
        Location::Trash => Columns::Trash,
        Location::Search { .. } => Columns::Search,
        Location::Dir(_) => Columns::Normal,
    }
}

/// Колонки таблицы по ширине панели. Сигнал меняется, только когда меняется
/// набор колонок, — ресайз окна не пересобирает вид на каждом шаге.
fn columns_fit(tab: Tab, p: Pane) -> RwSignal<items::ColumnsFit> {
    let compute = move || {
        let vw = syngui::viewport::viewport_size().get().width;
        let vw = if vw > 0.0 { vw } else { 1280.0 };
        let columns = p.loc.with(columns_of);
        let ctx = state::ctx();
        let sidebar = if ctx.sidebar.get() { ctx.sidebar_width.get() } else { 0.0 };
        items::fit_columns(items::pane_width(vw, sidebar, tab.split.get()), columns)
    };
    // Эффект сразу же вычислит настоящее значение (и не подпишет вызывающего).
    // Перетаскивание разделителя боковой панели пересчитывает его на каждом
    // шаге — сигнал меняем, только когда набор колонок другой.
    let fit = use_signal(items::ColumnsFit { kind: true, date: true });
    create_effect(move || {
        let v = compute();
        if fit.get_untracked() != v {
            fit.set(v);
        }
    });
    fit
}

fn header_view(p: Pane, fit: RwSignal<items::ColumnsFit>) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if p.view.get() != ViewMode::Details || state::is_phone() {
            return vec![];
        }
        let columns = p.loc.with(columns_of);
        vec![header(p, columns, fit.get())]
    }))
}

/// Панель целиком: заголовок таблицы (в виде «Таблица») и элементы.
pub fn pane_view(tab: Tab, idx: usize) -> W {
    let p = tab.panes[idx];
    let fit = columns_fit(tab, p);
    boxed(Reactive::new(move || -> Vec<W> {
        let cls = if tab.split.get() && tab.active.get() == idx { "pane active-pane" } else { "pane" };
        vec![boxed(
            DecoratedBox::new().class(cls).child(
                Column::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(header_view(p, fit))
                    .child(Stack::new().fit(StackFit::Expand).child(backdrop(p)).child(items_view(tab, idx, fit)).class("grow")),
            ),
        )]
    }))
}
