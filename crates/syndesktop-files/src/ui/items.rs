//! Элементы видов: строка таблицы, строка списка, плитка, значок.

use std::path::{Path, PathBuf};

use syndesktop_common::{mime, paths, xdg};
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

/// Картинка записи: миниатюра, если есть, иначе значок типа.
pub fn entry_image(e: &Entry, px: u32, thumbs: bool) -> W {
    if thumbs && !e.is_dir && px >= 32 {
        if e.mime == "image/svg+xml" && e.size < 2 << 20 {
            return boxed(Image::new(e.path.to_string_lossy().to_string()).fit(ImageFit::Contain).class("thumb"));
        }
        if let Some(t) = crate::thumbs::get(&e.path, &e.mime, e.mtime, px.max(64)) {
            return boxed(Image::new(t.to_string_lossy().to_string()).fit(ImageFit::Contain).class("thumb"));
        }
    }
    match icon_path(e) {
        Some(p) => boxed(Image::new(p.to_string_lossy().to_string()).fit(ImageFit::Contain).class("file-icon")),
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
    match cx.columns {
        Columns::Normal => {
            row = row
                .child(cell(model::format_time(e.mtime), "col-date", COL_DATE))
                .child(cell(e.description(), "col-type", COL_TYPE))
                .child(cell(if e.is_dir { String::new() } else { model::format_size(e.size) }, "col-size", COL_SIZE));
        }
        Columns::Trash => {
            let (from, when) = e
                .trash
                .as_ref()
                .map(|t| (t.original.parent().map(|p| p.display().to_string()).unwrap_or_default(), t.deleted.clone()))
                .unwrap_or_default();
            row = row
                .child(cell(from, "col-where", COL_WHERE))
                .child(cell(when, "col-date", COL_DATE))
                .child(cell(secondary(e), "col-size", COL_SIZE));
        }
        Columns::Search => {
            let folder = e.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
            row = row
                .child(cell(folder, "col-where", COL_WHERE))
                .child(cell(model::format_time(e.mtime), "col-date", COL_DATE))
                .child(cell(if e.is_dir { String::new() } else { model::format_size(e.size) }, "col-size", COL_SIZE));
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
