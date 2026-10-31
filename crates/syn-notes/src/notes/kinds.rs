//! Виды страниц-не-документов в библиотеке и редакторе: книга — читалка
//! ([`super::reader`]), блокнот и рисунок — рукописный холст ([`super::ink`]).
//! Регистрация — один раз при старте окна ([`install`]).

use syngui::prelude::*;

use super::library::{self, KindViews, Library};
use super::project::PageKind;
use super::state::NotesCtx;

type W = Box<dyn Widget>;

fn view(ctx: NotesCtx, lib: Library, id: &str, kind: PageKind) -> Option<W> {
    match kind {
        PageKind::Book => super::reader::page_view(ctx, lib, id, kind),
        PageKind::Notebook | PageKind::Drawing => super::ink::page_view(ctx, lib, id, kind),
        _ => None,
    }
}

fn actions(ctx: NotesCtx, lib: Library, id: &str, kind: PageKind) -> Option<W> {
    match kind {
        PageKind::Book => super::reader::actions(ctx, lib, id, kind),
        // У рукописи отмена, повтор и экспорт — на её панели инструментов.
        PageKind::Notebook | PageKind::Drawing => None,
        _ => None,
    }
}

fn thumbnail(ctx: NotesCtx, id: &str, kind: PageKind) -> Option<W> {
    match kind {
        PageKind::Book => super::reader::thumbnail(ctx, id, kind),
        PageKind::Notebook | PageKind::Drawing => super::ink::thumbnail(ctx, id, kind),
        _ => None,
    }
}

fn details(ctx: NotesCtx, id: &str, kind: PageKind) -> Option<String> {
    match kind {
        PageKind::Book => super::reader::details(ctx, id, kind),
        PageKind::Notebook | PageKind::Drawing => super::ink::details(ctx, id, kind),
        _ => None,
    }
}

/// Копия страницы: несохранённые рукописи — сначала в бандл.
fn live_bytes(_ctx: NotesCtx, kind: PageKind, _id: &str) -> Option<Vec<u8>> {
    if matches!(kind, PageKind::Notebook | PageKind::Drawing) {
        let _ = super::autosave::flush_all();
    }
    None
}

/// Удаление страницы: миниатюра рукописи — вместе с ней.
fn forget(id: &str) {
    super::autosave::queue_remove(&super::ink::views::thumb_path(id));
}

/// Подключить виды к библиотеке и импорт книг.
pub fn install() {
    library::set_kind_views(KindViews { thumbnail, details, view, actions });
    library::set_book_importer(super::reader::import_book);
    super::state::set_page_data(super::state::PageData { bytes: live_bytes, forget });
}
