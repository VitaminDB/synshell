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
        _ => None,
    }
}

fn actions(ctx: NotesCtx, lib: Library, id: &str, kind: PageKind) -> Option<W> {
    match kind {
        PageKind::Book => super::reader::actions(ctx, lib, id, kind),
        _ => None,
    }
}

fn thumbnail(ctx: NotesCtx, id: &str, kind: PageKind) -> Option<W> {
    match kind {
        PageKind::Book => super::reader::thumbnail(ctx, id, kind),
        _ => None,
    }
}

fn details(ctx: NotesCtx, id: &str, kind: PageKind) -> Option<String> {
    match kind {
        PageKind::Book => super::reader::details(ctx, id, kind),
        _ => None,
    }
}

/// Подключить виды к библиотеке и импорт книг.
pub fn install() {
    library::set_kind_views(KindViews { thumbnail, details, view, actions });
    library::set_book_importer(super::reader::import_book);
}
