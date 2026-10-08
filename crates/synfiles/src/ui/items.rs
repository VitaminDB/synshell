//! Элементы видов: строка таблицы, строка списка, плитка, значок.

use std::path::{Path, PathBuf};

use synshell_common::{mime, paths, xdg};
use syngui::prelude::*;
use syngui::data::ItemState;

use super::{boxed, W};
use crate::model::{self, Entry};
use crate::state::{Pane, ViewMode};

/// Ширины колонок таблицы (имя — остаток).
pub const COL_DATE: f32 = 168.0;
pub const COL_TYPE: f32 = 170.0;
pub const COL_SIZE: f32 = 96.0;
pub const COL_WHERE: f32 = 240.0;

/// Что нужно построителю элементов — снимок на момент построения вида.
#[derive(Clone)]
pub struct ItemCtx {
    pub pane: Pane,
    pub mode: ViewMode,
    pub icon_px: u32,
    pub cut: Vec<PathBuf>,
    pub renaming: Option<PathBuf>,
    pub thumbs: bool,
    /// Колонки «Откуда»/«Удалено» (корзина) или «Папка» (поиск).
    pub columns: Columns,
    /// Телефонная раскладка: крупные строки и сетка под палец.
    pub phone: bool,
    /// Телефон: идёт выбор (выделение не пусто) — у элементов отметки.
    pub selecting: bool,
    /// Таблица: сколько колонок кроме имени помещается (см. [`fit_columns`]).
    pub fit: ColumnsFit,
}

/// Какие колонки таблицы влезают в панель: узкой панели (половина окна,
/// маленькое окно) не хватает места — сначала уходит «Тип» (или «Откуда»),
/// потом дата. Размер остаётся всегда.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ColumnsFit {
    pub kind: bool,
    pub date: bool,
}

/// Колонки по ширине панели: имени остаётся не меньше ~200 px.
pub fn fit_columns(pane_width: f32, columns: Columns) -> ColumnsFit {
    let kind_w = if columns == Columns::Normal { COL_TYPE } else { COL_WHERE };
    let base = 200.0 + COL_SIZE + 40.0;
    ColumnsFit { date: pane_width >= base + COL_DATE, kind: pane_width >= base + COL_DATE + kind_w }
}

/// Ширина панели файлов: окно без боковой панели (`sidebar` — её ширина,
/// 0 — скрыта), пополам при разделении.
pub fn pane_width(vw: f32, sidebar: f32, split: bool) -> f32 {
    let w = vw - sidebar;
    if split {
        w / 2.0
    } else {
        w
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Columns {
    Normal,
    Trash,
    Search,
}

/// Значок папок по XDG-назначению: `folder-download`, `user-desktop`…
fn folder_icon_name(p: &Path) -> &'static str {
    if p == paths::home() {
        return "user-home";
    }
    const MAP: [(&str, &str); 8] = [
        ("DESKTOP", "user-desktop"),
        ("DOWNLOAD", "folder-download"),
        ("DOCUMENTS", "folder-documents"),
        ("PICTURES", "folder-pictures"),
        ("MUSIC", "folder-music"),
        ("VIDEOS", "folder-videos"),
        ("TEMPLATES", "folder-templates"),
        ("PUBLICSHARE", "folder-publicshare"),
    ];
    for (k, name) in MAP {
        if paths::user_dir(k).as_deref() == Some(p) {
            return name;
        }
    }
    "folder"
}

/// Путь к значку записи (тема значков), без миниатюры.
pub fn icon_path(e: &Entry) -> Option<PathBuf> {
    if e.is_dir {
        let name = folder_icon_name(&e.path);
        return xdg::lookup_icon(name).or_else(|| mime::icon_path(mime::DIRECTORY));
    }
    if e.mime == "application/x-desktop" {
        // Значок самой программы.
        let id = e.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if let Some(app) = xdg::parse_desktop_file(&e.path, id, &[]) {
            if let Some(p) = xdg::lookup_icon(&app.icon) {
                return Some(p);
            }
        }
    }
    mime::icon_path(&e.mime)
}

/// Картинка записи: миниатюра, если есть, иначе значок типа. Размер
/// задан явно: `Contain` без него берёт натуральный размер файла, и
/// 16-пиксельный значок темы садился в левый верхний угол ячейки.
pub fn entry_image(e: &Entry, px: u32, thumbs: bool) -> W {
    let img = |p: &Path, class: &str| -> W {
        let s = px as f32;
        boxed(Image::new(p.to_string_lossy().to_string()).fit(ImageFit::Contain).style("width", s).style("height", s).class(class.to_string()))
    };
    if thumbs && !e.is_dir && px >= 32 {
        if e.mime == "image/svg+xml" && e.size < 2 << 20 {
            return img(&e.path, "thumb");
        }
        if let Some(t) = crate::thumbs::get(&e.path, &e.mime, e.mtime, px.max(64)) {
            return img(&t, "thumb");
        }
    }
    match icon_path(e) {
        Some(p) => img(&p, "file-icon"),
        None => boxed(Icon::new(if e.is_dir { super::icons::FOLDER } else { super::icons::FILE }).class("icon glyph-icon")),
    }
}

fn sized(px: f32, child: W, class: &str) -> W {
    boxed(DecoratedBox::new().class(class.to_string()).style("width", px).style("height", px).child(child))
}

fn state_class(base: &str, st: ItemState, e: &Entry, cx: &ItemCtx) -> String {
    let mut c = base.to_string();
    if st.selected {
        c.push_str(" selected");
    }
    if st.cursor {
        c.push_str(" cursor");
    }
    if st.drop_target {
        c.push_str(" drop");
    }
    if cx.cut.iter().any(|p| p == &e.path) {
        c.push_str(" cut");
    }
    if e.hidden {
        c.push_str(" dim");
    }
    c
}

fn name_text(e: &Entry, class: &str, lines: usize) -> W {
    let t = Text::new(e.name.clone());
    let t = if lines <= 1 { t.elide(Elide::Middle) } else { t.max_lines(lines) };
    boxed(t.class(class.to_string()))
}

/// Поле переименования на месте имени.
fn rename_field(cx: &ItemCtx, e: &Entry) -> W {
    let p = cx.pane;
    let from = e.path.clone();
    let from2 = e.path.clone();
    boxed(
        TextField::with_text(e.name.clone())
            .autofocus(true)
            .select_range(0, e.stem_len())
            .submit_on_focus_lost(true)
            .on_submit(move |s| crate::actions::finish_rename(p, &from, s))
            .on_escape(move || {
                if p.renaming.get_untracked().as_deref() == Some(from2.as_path()) {
                    p.renaming.set(None);
                }
            })
            .class("rename-field"),
    )
}

fn secondary(e: &Entry) -> String {
    if e.is_dir {
        e.children.map(model::format_count).unwrap_or_default()
    } else {
        model::format_size(e.size)
    }
}

pub fn build(cx: &ItemCtx, e: &Entry, st: ItemState) -> W {
    let renaming = cx.renaming.as_deref() == Some(e.path.as_path());
    if cx.phone {
        return if phone_grid(cx.mode) { phone_cell(cx, e, st, renaming) } else { phone_row(cx, e, st, renaming) };
    }
    match cx.mode {
        ViewMode::Details => details_row(cx, e, st, renaming),
        ViewMode::List => list_row(cx, e, st, renaming),
        ViewMode::Tiles => tile(cx, e, st, renaming),
        ViewMode::Icons => icon_cell(cx, e, st, renaming),
    }
}

fn cell(text: String, class: &str, width: f32) -> W {
    boxed(DecoratedBox::new().class(format!("cell {class}")).style("width", width).child(Text::new(text).max_lines(1).class("cell-text")))
}

fn details_row(cx: &ItemCtx, e: &Entry, st: ItemState, renaming: bool) -> W {
    let name: W = if renaming { rename_field(cx, e) } else { name_text(e, "name", 1) };
    let mut row = Row::new()
        .gap(0.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(sized(20.0, entry_image(e, 20, false), "row-icon"))
        .child(DecoratedBox::new().class("cell name-cell grow").child(name));
    let fit = cx.fit;
    match cx.columns {
        Columns::Normal => {
            if fit.date {
                row = row.child(cell(model::format_time(e.mtime), "col-date", COL_DATE));
            }
            if fit.kind {
                row = row.child(cell(e.description(), "col-type", COL_TYPE));
            }
            row = row.child(cell(if e.is_dir { String::new() } else { model::format_size(e.size) }, "col-size", COL_SIZE));
        }
        Columns::Trash => {
            let (from, when) = e
                .trash
                .as_ref()
                .map(|t| (t.original.parent().map(|p| p.display().to_string()).unwrap_or_default(), t.deleted.clone()))
                .unwrap_or_default();
            if fit.kind {
                row = row.child(cell(from, "col-where", COL_WHERE));
            }
            if fit.date {
                row = row.child(cell(when, "col-date", COL_DATE));
            }
            row = row.child(cell(secondary(e), "col-size", COL_SIZE));
        }
        Columns::Search => {
            let folder = e.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
            if fit.kind {
                row = row.child(cell(folder, "col-where", COL_WHERE));
            }
            if fit.date {
                row = row.child(cell(model::format_time(e.mtime), "col-date", COL_DATE));
            }
            row = row.child(cell(if e.is_dir { String::new() } else { model::format_size(e.size) }, "col-size", COL_SIZE));
        }
    }
    boxed(DecoratedBox::new().class(state_class("item row", st, e, cx)).child(row))
}

fn list_row(cx: &ItemCtx, e: &Entry, st: ItemState, renaming: bool) -> W {
    let name: W = if renaming { rename_field(cx, e) } else { name_text(e, "name", 1) };
    boxed(
        DecoratedBox::new().class(state_class("item list-item", st, e, cx)).child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(sized(20.0, entry_image(e, 20, false), "row-icon"))
                .child(DecoratedBox::new().class("grow").child(name)),
        ),
    )
}

fn tile(cx: &ItemCtx, e: &Entry, st: ItemState, renaming: bool) -> W {
    let name: W = if renaming { rename_field(cx, e) } else { name_text(e, "name", 1) };
    boxed(
        DecoratedBox::new().class(state_class("item tile", st, e, cx)).child(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(sized(48.0, entry_image(e, 48, cx.thumbs), "tile-icon"))
                .child(
                    Column::new()
                        .gap(1.0)
                        .class("grow")
                        .child(name)
                        .child(Text::new(e.description()).max_lines(1).class("meta"))
                        .child(Text::new(secondary(e)).max_lines(1).class("meta")),
                ),
        ),
    )
}

fn icon_cell(cx: &ItemCtx, e: &Entry, st: ItemState, renaming: bool) -> W {
    let px = cx.icon_px as f32;
    let name: W = if renaming { rename_field(cx, e) } else { name_text(e, "name centered", 2) };
    boxed(
        DecoratedBox::new().class(state_class("item icon-item", st, e, cx)).child(
            Column::new()
                .gap(4.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(sized(px, entry_image(e, cx.icon_px, cx.thumbs), "big-icon"))
                .child(name),
        ),
    )
}

/// Телефон: вид «Значки» — сетка, остальные — список.
pub fn phone_grid(mode: ViewMode) -> bool {
    mode == ViewMode::Icons
}

/// Путь с `~` вместо домашней папки.
pub fn short_path(p: &Path) -> String {
    match p.strip_prefix(paths::home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// Вторая строка телефонного списка.
fn phone_meta(cx: &ItemCtx, e: &Entry) -> String {
    let join = |a: String, b: String| match (a.is_empty(), b.is_empty()) {
        (true, _) => b,
        (_, true) => a,
        _ => format!("{a} · {b}"),
    };
    match cx.columns {
        Columns::Trash => {
            let (from, when) = e
                .trash
                .as_ref()
                .map(|t| (t.original.parent().map(short_path).unwrap_or_default(), t.deleted.clone()))
                .unwrap_or_default();
            join(from, when)
        }
        Columns::Search => join(e.path.parent().map(short_path).unwrap_or_default(), secondary(e)),
        Columns::Normal => join(secondary(e), model::format_time(e.mtime)),
    }
}

/// Отметка выбора (телефон, режим выбора).
fn check_mark(selected: bool) -> W {
    boxed(Icon::new(if selected { super::icons::CHECK_CIRCLE } else { super::icons::UNCHECKED }).class(if selected { "icon pick on" } else { "icon pick" }))
}

fn phone_class(base: &str, st: ItemState, e: &Entry, cx: &ItemCtx) -> String {
    // Курсор клавиатуры на телефоне не рисуем.
    state_class(base, ItemState { cursor: false, ..st }, e, cx)
}

fn phone_row(cx: &ItemCtx, e: &Entry, st: ItemState, renaming: bool) -> W {
    let name: W = if renaming { rename_field(cx, e) } else { name_text(e, "name pname", 1) };
    let mut row = Row::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(sized(40.0, entry_image(e, 40, cx.thumbs), "prow-icon"))
        .child(
            Column::new()
                .gap(2.0)
                .class("grow")
                .child(name)
                .child(Text::new(phone_meta(cx, e)).elide(Elide::Middle).class("meta")),
        );
    if cx.selecting {
        row = row.child(check_mark(st.selected));
    }
    boxed(DecoratedBox::new().class(phone_class("item prow", st, e, cx)).child(row))
}

fn phone_cell(cx: &ItemCtx, e: &Entry, st: ItemState, renaming: bool) -> W {
    let px = cx.icon_px as f32;
    let name: W = if renaming { rename_field(cx, e) } else { name_text(e, "name centered", 2) };
    let body = Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(sized(px, entry_image(e, cx.icon_px, cx.thumbs), "big-icon"))
        .child(name);
    let mut stack = Stack::new().child(body);
    if cx.selecting {
        stack = stack.child(
            Row::new().main_axis_alignment(MainAxisAlignment::End).child(check_mark(st.selected)),
        );
    }
    boxed(DecoratedBox::new().class(phone_class("item pcell", st, e, cx)).child(stack))
}

/// Телефон: (наименьшая ширина, высота) ячейки; ширина 0 — список.
pub fn phone_cell_size(mode: ViewMode, icon_px: u32) -> (f32, f32) {
    if phone_grid(mode) {
        let px = icon_px as f32;
        ((px + 30.0).max(96.0), px + 54.0)
    } else {
        (0.0, 64.0)
    }
}

/// Раскладка вида: (ширина, высота) ячейки.
pub fn cell_size(mode: ViewMode, icon_px: u32) -> (f32, f32) {
    match mode {
        ViewMode::Details => (0.0, 30.0),
        ViewMode::List => (280.0, 30.0),
        ViewMode::Tiles => (300.0, 70.0),
        ViewMode::Icons => {
            let px = icon_px as f32;
            ((px + 36.0).max(96.0), px + 58.0)
        }
    }
}

/// Подпись для поиска по первым буквам.
pub fn label(e: &Entry) -> String {
    e.name.clone()
}

/// Индексы → записи для построителя (ItemView вызывает его из дерева).
pub fn builder(cx: ItemCtx, entries: std::sync::Arc<Vec<Entry>>) -> impl Fn(usize, ItemState) -> W + Send + Sync + 'static {
    move |i, st| match entries.get(i) {
        Some(e) => build(&cx, e, st),
        None => boxed(Text::new("")),
    }
}
