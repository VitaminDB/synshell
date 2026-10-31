//! Библиотека — главный экран заметок в духе ONYX BOOX: карточки блокнотов,
//! рисунков, книг и документов проекта, папки, поиск, создание.
//!
//! ```text
//! [проект ▾]  [Все|Блокноты|Рисунки|Книги|Документы]  [🔍] [▦/☰] [+ Создать]
//! Библиотека / Папка / …
//! ┌────┐ ┌────┐ ┌────┐
//! │    │ │    │ │    │   карточка: обложка/миниатюра, название, тип · подробности
//! └────┘ └────┘ └────┘
//! ```
//!
//! Открытая страница показывается на весь экран ([`Screen::Open`]): блокнот
//! и рисунок — рукописный холст, книга — читалка, документ — прежний
//! редактор (на широком окне — с деревом и свойствами). «Назад» возвращает
//! в библиотеку, в ту же папку.
//!
//! E Ink: никаких анимаций и наведений — карточки с чёткой рамкой, крупные
//! цели касания; смена экрана — одна перерисовка.

use std::sync::Arc;

use syngui::core::Point;
use syngui::input::CursorIcon;
use syngui::prelude::*;
use syngui::widgets::overlay::menu::MenuItem;
use syngui::widgets::{GestureDetector, PopupMenu, TextField, ToolButton};

use crate::icons::*;

use super::project::{PageKind, PageNode, ProjectTree};
use super::state::NotesCtx;

/// Что показывает окно заметок.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Library,
    /// Открытая страница (id).
    Open(String),
}

/// Раздел библиотеки.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Section {
    /// Всё по папкам (дерево).
    #[default]
    All,
    Notebooks,
    Drawings,
    Books,
    Docs,
}

impl Section {
    pub const ALL: [Section; 5] = [Section::All, Section::Notebooks, Section::Drawings, Section::Books, Section::Docs];

    pub fn title(self) -> String {
        match self {
            Section::All => tr!("notes.lib.all"),
            Section::Notebooks => tr!("notes.lib.notebooks"),
            Section::Drawings => tr!("notes.lib.drawings"),
            Section::Books => tr!("notes.lib.books"),
            Section::Docs => tr!("notes.lib.docs"),
        }
    }

    fn kind(self) -> Option<PageKind> {
        match self {
            Section::All => None,
            Section::Notebooks => Some(PageKind::Notebook),
            Section::Drawings => Some(PageKind::Drawing),
            Section::Books => Some(PageKind::Book),
            Section::Docs => Some(PageKind::Doc),
        }
    }
}

/// Порядок карточек.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Sort {
    /// Как в дереве (ручной порядок).
    #[default]
    Manual,
    Name,
    Kind,
}

/// Состояние библиотеки (контекст окна, переживает пересоздание дерева).
#[derive(Clone, Copy)]
pub struct Library {
    pub screen: RwSignal<Screen>,
    pub section: RwSignal<Section>,
    /// Текущая папка (`None` — корень проекта).
    pub folder: RwSignal<Option<String>>,
    pub search: RwSignal<String>,
    pub search_open: RwSignal<bool>,
    pub sort: RwSignal<Sort>,
    /// Карточки (true) или список.
    pub grid: RwSignal<bool>,
    pub create_open: RwSignal<bool>,
    pub card_menu: RwSignal<Option<String>>,
}

impl Library {
    pub fn new() -> Self {
        Self {
            screen: use_signal(Screen::Library),
            section: use_signal(Section::All),
            folder: use_signal(None),
            search: use_signal(String::new()),
            search_open: use_signal(false),
            sort: use_signal(Sort::Manual),
            grid: use_signal(true),
            create_open: use_signal(false),
            card_menu: use_signal(None),
        }
    }

    /// Открыть страницу на весь экран (папка — войти в неё).
    pub fn open(&self, ctx: NotesCtx, id: &str) {
        let kind = ctx.tree.get_untracked().kind_of(id);
        if kind == PageKind::Folder {
            self.folder.set(Some(id.to_string()));
            self.section.set(Section::All);
            return;
        }
        ctx.show_graph.set(false);
        ctx.activate(id);
        self.screen.set(Screen::Open(id.to_string()));
    }

    /// Назад: из страницы — в библиотеку, из папки — к родителю.
    pub fn back(&self, ctx: NotesCtx) -> bool {
        match self.screen.get_untracked() {
            Screen::Open(id) => {
                // Папка библиотеки — та, где лежит страница.
                let tree = ctx.tree.get_untracked();
                let parent = tree.parent_of(&id).filter(|p| tree.kind_of(p) == PageKind::Folder);
                if self.section.get_untracked() == Section::All {
                    self.folder.set(parent);
                }
                self.screen.set(Screen::Library);
                true
            }
            Screen::Library => {
                if self.search_open.get_untracked() {
                    self.search_open.set(false);
                    self.search.set(String::new());
                    return true;
                }
                match self.folder.get_untracked() {
                    Some(f) => {
                        let tree = ctx.tree.get_untracked();
                        self.folder.set(tree.parent_of(&f));
                        true
                    }
                    None => false,
                }
            }
        }
    }

    /// Создать элемент в текущей папке и открыть его.
    pub fn create(&self, ctx: NotesCtx, kind: PageKind) {
        let parent = if self.section.get_untracked() == Section::All { self.folder.get_untracked() } else { None };
        let title = match kind {
            PageKind::Doc => tr!("notes.untitled"),
            PageKind::Notebook => tr!("notes.lib.new.notebook"),
            PageKind::Drawing => tr!("notes.lib.new.drawing"),
            PageKind::Book => tr!("notes.lib.new.book"),
            PageKind::Folder => tr!("notes.lib.new.folder"),
        };
        let id = ctx.insert_page_of_kind(parent.as_deref(), None, &title, kind, false);
        if kind == PageKind::Folder {
            ctx.renaming.set(Some(id));
        } else {
            self.open(ctx, &id);
        }
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

/// Значок типа страницы.
pub fn kind_icon(kind: PageKind) -> &'static str {
    match kind {
        PageKind::Doc => MI_DESCRIPTION,
        PageKind::Notebook => MI_EDIT_NOTE,
        PageKind::Drawing => MI_BRUSH,
        PageKind::Book => MI_MENU_BOOK,
        PageKind::Folder => MI_FOLDER,
    }
}

pub fn kind_title(kind: PageKind) -> String {
    match kind {
        PageKind::Doc => tr!("notes.lib.kind.doc"),
        PageKind::Notebook => tr!("notes.lib.kind.notebook"),
        PageKind::Drawing => tr!("notes.lib.kind.drawing"),
        PageKind::Book => tr!("notes.lib.kind.book"),
        PageKind::Folder => tr!("notes.lib.kind.folder"),
    }
}

// ─── Выборка ────────────────────────────────────────────────────────────────

/// Элемент сетки.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub kind: PageKind,
    pub icon: Option<String>,
    pub children: usize,
}

fn item_of(n: &PageNode) -> Item {
    Item { id: n.id.clone(), title: n.title.clone(), kind: n.kind, icon: n.icon.clone(), children: n.children.len() }
}

/// Карточки раздела: «Все» — дети текущей папки; разделы по типу и поиск —
/// плоско по всему дереву.
pub fn items(tree: &ProjectTree, section: Section, folder: Option<&str>, query: &str, sort: Sort) -> Vec<Item> {
    fn walk(nodes: &[PageNode], out: &mut Vec<Item>, keep: &dyn Fn(&PageNode) -> bool) {
        for n in nodes {
            if keep(n) {
                out.push(item_of(n));
            }
            walk(&n.children, out, keep);
        }
    }
    let q = query.trim().to_lowercase();
    let mut out = Vec::new();
    if !q.is_empty() {
        let kind = section.kind();
        walk(&tree.roots, &mut out, &|n| n.title.to_lowercase().contains(&q) && kind.map_or(true, |k| n.kind == k));
    } else if let Some(kind) = section.kind() {
        walk(&tree.roots, &mut out, &|n| n.kind == kind);
    } else {
        let nodes = match folder.and_then(|f| tree.find(f)) {
            Some(f) => &f.children,
            None => &tree.roots,
        };
        out.extend(nodes.iter().map(item_of));
    }
    match sort {
        Sort::Manual => {}
        Sort::Name => out.sort_by_key(|i| (i.kind != PageKind::Folder, i.title.to_lowercase())),
        Sort::Kind => out.sort_by_key(|i| (kind_rank(i.kind), i.title.to_lowercase())),
    }
    out
}

fn kind_rank(k: PageKind) -> u8 {
    match k {
        PageKind::Folder => 0,
        PageKind::Notebook => 1,
        PageKind::Drawing => 2,
        PageKind::Book => 3,
        PageKind::Doc => 4,
    }
}

// ─── Подключаемые виды страниц ──────────────────────────────────────────────

type W = Box<dyn Widget>;

/// Миниатюра карточки и полноэкранный вид для рукописных и книжных страниц.
/// Ставятся модулями `ink` и `reader` при интеграции; без них — заглушка.
pub struct KindViews {
    /// Миниатюра: (ctx, id страницы, тип) → виджет или None (значок типа).
    pub thumbnail: fn(NotesCtx, &str, PageKind) -> Option<W>,
    /// Подробности под названием: «12 стр.», «37 %», …
    pub details: fn(NotesCtx, &str, PageKind) -> Option<String>,
    /// Вид страницы на весь экран.
    pub view: fn(NotesCtx, Library, &str, PageKind) -> Option<W>,
    /// Кнопки действий в верхней панели открытой страницы.
    pub actions: fn(NotesCtx, Library, &str, PageKind) -> Option<W>,
}

static VIEWS: std::sync::OnceLock<KindViews> = std::sync::OnceLock::new();

pub fn set_kind_views(v: KindViews) {
    let _ = VIEWS.set(v);
}

fn views() -> Option<&'static KindViews> {
    VIEWS.get()
}

// ─── Вид ────────────────────────────────────────────────────────────────────

/// Кнопка с выпадающим меню под ней.
fn menu_button(
    icon: &'static str,
    tooltip: String,
    items: Vec<MenuItem>,
    on_select: impl FnMut(&str) + Send + 'static,
) -> impl Widget {
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let btn = ToolButton::new(icon)
        .tooltip(tooltip)
        .on_click_with_bounds(move |_, b| {
            pos.set(Point::new(b.origin.x, b.origin.y + b.size.height + 4.0));
            open.set(true);
        })
        .class("lib-btn");
    Stack::new()
        .clip(false)
        .child(btn)
        .child(PopupMenu::new().items(items).is_open(open).position(pos).on_select(on_select))
}

/// Произвольный виджет, открывающий меню по нажатию.
fn menu_on<M>(
    child: impl syngui::widgets::containers::IntoWidget<M>,
    items: Vec<MenuItem>,
    on_select: impl FnMut(&str) + Send + 'static,
) -> impl Widget {
    let open = use_signal(false);
    let pos = use_signal(Point::zero());
    let g = GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click_with_bounds(move |_, b| {
            pos.set(Point::new(b.origin.x, b.origin.y + b.size.height + 4.0));
            open.set(true);
        })
        .child(child);
    Stack::new()
        .clip(false)
        .child(g)
        .child(PopupMenu::new().items(items).is_open(open).position(pos).on_select(on_select))
}

/// Экран библиотеки или открытой страницы.
pub fn view(wide: RwSignal<bool>) -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let lib = use_context::<Library>();
    // Страница пропала (удалена ассистентом, закрыт проект) — назад в библиотеку.
    create_effect(move || {
        let tree = ctx.tree.get();
        if let Screen::Open(id) = lib.screen.get() {
            if tree.find(&id).is_none() {
                lib.screen.set(Screen::Library);
            }
        }
        if let Some(f) = lib.folder.get() {
            if tree.find(&f).is_none() {
                lib.folder.set(None);
            }
        }
    });
    fill(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        match lib.screen.get() {
            Screen::Library => vec![Box::new(fill(library(ctx, lib, wide)))],
            Screen::Open(id) => vec![Box::new(fill(open_view(ctx, lib, wide, id)))],
        }
    }))
}

/// На всю доступную площадь (Reactive и Column сами берут только своё).
fn fill<M>(w: impl syngui::widgets::containers::IntoWidget<M>) -> Stack {
    Stack::new().fit(StackFit::Expand).child(w)
}

/// Открытая страница.
fn open_view(ctx: NotesCtx, lib: Library, wide: RwSignal<bool>, id: String) -> W {
    let kind = ctx.tree.get_untracked().kind_of(&id);
    if kind == PageKind::Doc && wide.get_untracked() {
        // Документ на широком окне — прежний редактор с деревом и свойствами
        // (кнопка «Назад» — в шапке центра).
        return Box::new(super::view());
    }
    let body: W = match kind {
        PageKind::Doc => Box::new(super::editor_pane::body()),
        _ => views()
            .and_then(|v| (v.view)(ctx, lib, &id, kind))
            .unwrap_or_else(|| Box::new(placeholder(kind))),
    };
    Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("grow lib-open")
            .child(open_bar(ctx, lib, id, kind))
            .child(DecoratedBox::new().class("grow lib-open-body").clip(true).child(fill(body))),
    )
}

/// Верхняя панель открытой страницы: назад, название (тап — переименовать),
/// действия вида, меню.
fn open_bar(ctx: NotesCtx, lib: Library, id: String, kind: PageKind) -> impl Widget {
    let actions = views().and_then(|v| (v.actions)(ctx, lib, &id, kind));
    let title = Reactive::new({
        let id = id.clone();
        move || -> Vec<W> {
            let tree = ctx.tree.get();
            let t = tree.title_of(&id).unwrap_or_default();
            vec![Box::new(Text::new(t).max_lines(1).elide(Elide::End).class("lib-open-title"))]
        }
    });
    let mut row = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(back_button(ctx, lib))
        .child(
            DecoratedBox::new().class("grow").child(
                GestureDetector::new()
                    .cursor(CursorIcon::Text)
                    .on_click({
                        let id = id.clone();
                        move || ctx.renaming.set(Some(id.clone()))
                    })
                    .child(title),
            ),
        );
    if let Some(a) = actions {
        row = row.child(a);
    }
    row = row.child(page_menu(ctx, lib, id));
    DecoratedBox::new().class("lib-bar").child(row)
}

/// «← Назад» в библиотеку.
pub fn back_button(ctx: NotesCtx, lib: Library) -> impl Widget {
    ToolButton::new(MI_ARROW_BACK)
        .tooltip(tr!("notes.lib.back"))
        .on_click(move || {
            lib.back(ctx);
        })
        .class("lib-btn")
}

fn page_menu(ctx: NotesCtx, lib: Library, id: String) -> impl Widget {
    let items = vec![
        MenuItem::new("rename", tr!("notes.tree.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE),
        MenuItem::new("duplicate", tr!("notes.tree.duplicate")).icon(MI_CONTENT_COPY),
        MenuItem::new("delete", tr!("app.delete")).icon(MI_DELETE),
    ];
    menu_button(MI_MORE_VERT, tr!("notes.lib.more"), items, move |key: &str| match key {
        "rename" => ctx.renaming.set(Some(id.clone())),
        "duplicate" => {
            if let Some(copy) = ctx.duplicate_page_with(&id, false) {
                lib.open(ctx, &copy);
            }
        }
        "delete" => {
            lib.screen.set(Screen::Library);
            ctx.delete_page(&id);
        }
        _ => {}
    })
}

fn placeholder(kind: PageKind) -> impl Widget {
    Center::new().child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(kind_icon(kind)).class("notes-empty-icon"))
            .child(Text::new(kind_title(kind)).class("notes-empty-title")),
    )
}

/// Сама библиотека.
fn library(ctx: NotesCtx, lib: Library, wide: RwSignal<bool>) -> impl Widget {
    let grid = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let tree = ctx.tree.get();
        let section = lib.section.get();
        let folder = lib.folder.get();
        let query = lib.search.get();
        let sort = lib.sort.get();
        let as_grid = lib.grid.get();
        let w = wide.get();
        let list = items(&tree, section, folder.as_deref(), &query, sort);
        if list.is_empty() {
            return vec![Box::new(empty(lib, section, !query.trim().is_empty()))];
        }
        let cards: Vec<W> = list.into_iter().map(|it| card(ctx, lib, it, as_grid)).collect();
        if as_grid {
            let cols = if w { 5 } else { 3 };
            vec![Box::new(Grid::new(cols).gap(16.0).children(cards))]
        } else {
            vec![Box::new(Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).children(cards))]
        }
    });
    Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("grow lib")
        .child(library_bar(ctx, lib, wide))
        .child(sections(lib))
        .child(crumbs(ctx, lib))
        .child(
            DecoratedBox::new().class("grow").child(fill(
                ScrollView::new()
                    .vertical()
                    .child(DecoratedBox::new().class("lib-grid").child(grid)),
            )),
        )
}

/// Шапка библиотеки: проект, поиск, вид, «Создать».
fn library_bar(ctx: NotesCtx, lib: Library, wide: RwSignal<bool>) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let searching = lib.search_open.get();
        let _ = lib.sort.get();
        let as_grid = lib.grid.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if searching {
            row = row
                .child(ToolButton::new(MI_ARROW_BACK).tooltip(tr!("notes.lib.back")).on_click(move || {
                    lib.search_open.set(false);
                    lib.search.set(String::new());
                }).class("lib-btn"))
                .child(
                    DecoratedBox::new().class("grow").child(
                        TextField::new()
                            .placeholder(tr!("notes.lib.search"))
                            .text(lib.search.get_untracked())
                            .on_change(move |t| lib.search.set(t.to_string()))
                            .autofocus(true)
                            .class("lib-search"),
                    ),
                );
        } else {
            let title = ctx.project_title.get();
            row = row
                .child(Icon::new(MI_LIBRARY_BOOKS).class("lib-bar-icon"))
                .child(
                    DecoratedBox::new()
                        .class("grow")
                        .child(Text::new(if wide.get() { title } else { tr!("notes.lib.title") }).max_lines(1).elide(Elide::End).class("lib-title")),
                )
                .child(super::project_ui::menu_button())
                .child(ToolButton::new(MI_SEARCH).tooltip(tr!("notes.lib.search")).on_click(move || lib.search_open.set(true)).class("lib-btn"));
        }
        row = row
            .child(sort_menu(lib))
            .child(
                ToolButton::new(if as_grid { MI_VIEW_LIST } else { MI_GRID_VIEW })
                    .tooltip(if as_grid { tr!("notes.lib.as_list") } else { tr!("notes.lib.as_grid") })
                    .on_click(move || lib.grid.update(|g| *g = !*g))
                    .class("lib-btn"),
            )
            .child(create_menu(ctx, lib));
        vec![Box::new(DecoratedBox::new().class("lib-bar").child(row))]
    })
}

fn sort_menu(lib: Library) -> impl Widget {
    let mark = |s: Sort| if lib.sort.get_untracked() == s { MI_CHECK } else { "" };
    let items = vec![
        MenuItem::new("manual", tr!("notes.lib.sort.manual")).icon(mark(Sort::Manual)),
        MenuItem::new("name", tr!("notes.lib.sort.name")).icon(mark(Sort::Name)),
        MenuItem::new("kind", tr!("notes.lib.sort.kind")).icon(mark(Sort::Kind)),
    ];
    menu_button(MI_SORT, tr!("notes.lib.sort"), items, move |k: &str| {
        lib.sort.set(match k {
            "name" => Sort::Name,
            "kind" => Sort::Kind,
            _ => Sort::Manual,
        })
    })
}

/// «+ Создать»: блокнот, рисунок, документ, книга (импорт), папка.
fn create_menu(ctx: NotesCtx, lib: Library) -> impl Widget {
    let items = vec![
        MenuItem::new("notebook", tr!("notes.lib.kind.notebook")).icon(MI_EDIT_NOTE),
        MenuItem::new("drawing", tr!("notes.lib.kind.drawing")).icon(MI_BRUSH),
        MenuItem::new("doc", tr!("notes.lib.kind.doc")).icon(MI_DESCRIPTION),
        MenuItem::new("book", tr!("notes.lib.import_book")).icon(MI_UPLOAD_FILE),
        MenuItem::new("folder", tr!("notes.lib.kind.folder")).icon(MI_CREATE_NEW_FOLDER),
    ];
    menu_on(
        DecoratedBox::new().class("lib-create").child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(MI_ADD).class("lib-create-icon"))
                .child(Text::new(tr!("notes.lib.create")).class("lib-create-text")),
        ),
        items,
        move |k: &str| match k {
            "book" => import_book_dialog(ctx, lib),
            other => {
                if let Some(kind) = PageKind::from_name(other) {
                    lib.create(ctx, kind);
                }
            }
        },
    )
}

/// Подключаемый импорт книги (ставит модуль читалки).
static IMPORT_BOOK: std::sync::OnceLock<fn(NotesCtx, Library, Option<String>, std::path::PathBuf)> =
    std::sync::OnceLock::new();

pub fn set_book_importer(f: fn(NotesCtx, Library, Option<String>, std::path::PathBuf)) {
    let _ = IMPORT_BOOK.set(f);
}

/// Выбор файла книги и импорт в текущую папку.
pub fn import_book_dialog(ctx: NotesCtx, lib: Library) {
    let parent = if lib.section.get_untracked() == Section::All { lib.folder.get_untracked() } else { None };
    let dialog = rfd::AsyncFileDialog::new()
        .set_title(tr!("notes.lib.import_book"))
        .add_filter(tr!("notes.lib.books"), &["pdf", "epub", "fb2", "zip", "txt", "md", "markdown"]);
    syngui::async_runtime::spawn(async move {
        let Some(file) = dialog.pick_file().await else { return };
        let path = file.path().to_path_buf();
        syngui::async_runtime::run_on_main_thread(move || {
            if let Some(f) = IMPORT_BOOK.get() {
                f(ctx, lib, parent, path);
            }
        });
    });
}

/// Вкладки разделов.
fn sections(lib: Library) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let cur = lib.section.get();
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        for s in Section::ALL {
            let class = if s == cur { "lib-tab active" } else { "lib-tab" };
            row = row.child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || {
                        lib.section.set(s);
                        if s != Section::All {
                            lib.folder.set(None);
                        }
                    })
                    .child(DecoratedBox::new().class(class).child(Text::new(s.title()).class("lib-tab-text"))),
            );
        }
        vec![Box::new(DecoratedBox::new().class("lib-tabs").child(row))]
    })
}

/// Путь папок: «Библиотека / Папка / Вложенная».
fn crumbs(ctx: NotesCtx, lib: Library) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let tree = ctx.tree.get();
        let Some(folder) = lib.folder.get() else { return vec![] };
        if lib.section.get() != Section::All || !lib.search.get().is_empty() {
            return vec![];
        }
        let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
        row = row.child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || lib.folder.set(None))
                .child(Text::new(tr!("notes.lib.title")).class("lib-crumb")),
        );
        for (id, title) in tree.path_of(&folder) {
            row = row.child(Icon::new(MI_CHEVRON_RIGHT).class("lib-crumb-sep"));
            row = row.child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || lib.folder.set(Some(id.clone())))
                    .child(Text::new(title).class("lib-crumb")),
            );
        }
        vec![Box::new(DecoratedBox::new().class("lib-crumbs").child(row))]
    })
}

fn empty(lib: Library, section: Section, searching: bool) -> impl Widget {
    let (title, hint) = if searching {
        (tr!("notes.lib.empty.search"), String::new())
    } else {
        (tr!("notes.lib.empty.title"), tr!("notes.lib.empty.hint"))
    };
    let mut col = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(MI_LIBRARY_BOOKS).class("notes-empty-icon"))
        .child(Text::new(title).class("notes-empty-title"));
    if !hint.is_empty() {
        col = col.child(Text::new(hint).max_lines(3).class("notes-empty-hint"));
    }
    if !searching {
        let ctx = use_context::<NotesCtx>();
        let kind = match section {
            Section::Drawings => PageKind::Drawing,
            Section::Docs => PageKind::Doc,
            Section::Books => PageKind::Book,
            _ => PageKind::Notebook,
        };
        let label = if kind == PageKind::Book { tr!("notes.lib.import_book") } else { tr!("notes.lib.create") };
        col = col.child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    if kind == PageKind::Book {
                        import_book_dialog(ctx, lib);
                    } else {
                        lib.create(ctx, kind);
                    }
                })
                .child(
                    DecoratedBox::new().class("lib-create").child(
                        Row::new()
                            .gap(6.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(Icon::new(kind_icon(kind)).class("lib-create-icon"))
                            .child(Text::new(label).class("lib-create-text")),
                    ),
                ),
        );
    }
    DecoratedBox::new().class("lib-empty").child(Center::new().child(col))
}

/// Подробности под названием карточки.
fn details(ctx: NotesCtx, it: &Item) -> String {
    let base = kind_title(it.kind);
    let extra = match it.kind {
        PageKind::Folder => Some(trn!("notes.lib.items", it.children as i64, n = it.children)),
        PageKind::Doc => None,
        k => views().and_then(|v| (v.details)(ctx, &it.id, k)),
    };
    match extra {
        Some(e) if !e.is_empty() => format!("{base} · {e}"),
        _ => base,
    }
}

/// Миниатюра: из вида страницы или значок/первые строки документа.
fn thumb(ctx: NotesCtx, it: &Item) -> W {
    if it.kind != PageKind::Doc && it.kind != PageKind::Folder {
        if let Some(w) = views().and_then(|v| (v.thumbnail)(ctx, &it.id, it.kind)) {
            return w;
        }
    }
    if it.kind == PageKind::Doc {
        let text = doc_preview(&ctx.page_markdown(&it.id));
        if !text.is_empty() {
            return Box::new(
                DecoratedBox::new()
                    .class("lib-thumb-doc")
                    .child(Text::new(text).max_lines(9).elide(Elide::End).class("lib-thumb-doc-text")),
            );
        }
    }
    let icon = it.icon.clone().filter(|_| it.kind != PageKind::Folder).unwrap_or_else(|| kind_icon(it.kind).to_string());
    Box::new(Center::new().child(super::icon_picker::render_icon(&icon, "lib-thumb-icon")))
}

/// Первые строки markdown без разметки — для карточки документа.
pub fn doc_preview(md: &str) -> String {
    let mut out = Vec::new();
    for line in md.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with("![[") || l.starts_with("```") || l.starts_with("<!--") {
            continue;
        }
        let l = l.trim_start_matches(|c: char| c == '#' || c == '>' || c == '-' || c == '*' || c == ' ');
        let l = l.replace("**", "").replace("__", "");
        if !l.is_empty() {
            out.push(l);
        }
        if out.len() >= 9 {
            break;
        }
    }
    out.join("\n")
}

fn card(ctx: NotesCtx, lib: Library, it: Item, as_grid: bool) -> W {
    let id = it.id.clone();
    let open_id = id.clone();
    let menu_id = id.clone();
    let title = it.title.clone();
    let sub = details(ctx, &it);
    let body: W = if as_grid {
        Box::new(
            Column::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(
                    DecoratedBox::new()
                        .class(if it.kind == PageKind::Folder { "lib-thumb folder" } else { "lib-thumb" })
                        .clip(true)
                        .child(AspectRatio::new(0.75).child(thumb(ctx, &it))),
                )
                .child(
                    Row::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Icon::new(kind_icon(it.kind)).class("lib-card-kind"))
                        .child(DecoratedBox::new().class("grow").child(Text::new(title).max_lines(2).elide(Elide::End).class("lib-card-title"))),
                )
                .child(Text::new(sub).max_lines(1).elide(Elide::End).class("lib-card-sub")),
        )
    } else {
        Box::new(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(kind_icon(it.kind)).class("lib-row-icon"))
                .child(
                    DecoratedBox::new().class("grow").child(
                        Column::new()
                            .gap(2.0)
                            .child(Text::new(title).max_lines(1).elide(Elide::End).class("lib-card-title"))
                            .child(Text::new(sub).max_lines(1).elide(Elide::End).class("lib-card-sub")),
                    ),
                ),
        )
    };
    let items = vec![
        MenuItem::new("open", tr!("notes.lib.open")).icon(MI_OPEN_IN_FULL),
        MenuItem::new("rename", tr!("notes.tree.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE),
        MenuItem::new("duplicate", tr!("notes.tree.duplicate")).icon(MI_CONTENT_COPY),
        MenuItem::new("move_up", tr!("notes.lib.move_out")).icon(MI_ARROW_BACK),
        MenuItem::new("delete", tr!("app.delete")).icon(MI_DELETE),
    ];
    Box::new(
        ContextMenu::new()
            .items(items)
            .on_select(move |k: &str| match k {
                "open" => lib.open(ctx, &menu_id),
                "rename" => ctx.renaming.set(Some(menu_id.clone())),
                "duplicate" => {
                    ctx.duplicate_page_with(&menu_id, false);
                }
                "move_up" => {
                    let tree = ctx.tree.get_untracked();
                    if let Some(p) = tree.parent_of(&menu_id) {
                        let grand = tree.parent_of(&p);
                        drop(tree);
                        ctx.move_page(&menu_id, grand.as_deref(), None);
                    }
                }
                "delete" => ctx.delete_page(&menu_id),
                _ => {}
            })
            .child(
                GestureDetector::new()
                    .cursor(CursorIcon::Pointer)
                    .on_click(move || lib.open(ctx, &open_id))
                    .child(DecoratedBox::new().class(if as_grid { "lib-card" } else { "lib-row" }).child(body)),
            ),
    )
}

/// Переименование из библиотеки: диалог поверх (ctx.renaming).
pub fn rename_dialog(ctx: NotesCtx) -> impl Widget {
    let open = use_signal(false);
    create_effect(move || {
        let v = ctx.renaming.get().is_some();
        if open.get_untracked() != v {
            open.set(v);
        }
    });
    create_effect(move || {
        if !open.get() && ctx.renaming.get_untracked().is_some() {
            ctx.renaming.set(None);
        }
    });
    Portal::new().is_open(open).modal(true).backdrop(true).child(Reactive::new(move || -> Vec<W> {
        let Some(id) = ctx.renaming.get() else { return vec![] };
        let current = ctx.tree.get_untracked().title_of(&id).unwrap_or_default();
        let value = use_signal(current);
        let commit = {
            let id = id.clone();
            move || {
                let t = value.get_untracked();
                if !t.trim().is_empty() {
                    ctx.rename_page(&id, t.trim());
                }
                ctx.renaming.set(None);
            }
        };
        let commit2 = commit.clone();
        vec![Box::new(
            DecoratedBox::new().class("lib-dialog").child(
                Column::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(Text::new(tr!("notes.tree.rename")).class("lib-dialog-title"))
                    .child(
                        TextField::new()
                            .text(value.get_untracked())
                            .on_change(move |t| value.set(t.to_string()))
                            .autofocus(true)
                            .on_submit(move |_| commit())
                            .class("lib-search"),
                    )
                    .child(
                        Row::new()
                            .gap(8.0)
                            .main_axis_alignment(MainAxisAlignment::End)
                            .child(Button::new(tr!("app.cancel")).class("lib-dialog-btn").on_click(move || ctx.renaming.set(None)))
                            .child(Button::new(tr!("notes.lib.ok")).class("lib-dialog-btn primary").on_click(move || commit2())),
                    ),
            ),
        )]
    }))
}

/// Подсчёт для тестов и сводок.
pub fn count_by_kind(tree: &ProjectTree) -> Arc<std::collections::HashMap<PageKind, usize>> {
    let mut m = std::collections::HashMap::new();
    fn walk(nodes: &[PageNode], m: &mut std::collections::HashMap<PageKind, usize>) {
        for n in nodes {
            *m.entry(n.kind).or_insert(0) += 1;
            walk(&n.children, m);
        }
    }
    walk(&tree.roots, &mut m);
    Arc::new(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(title: &str, kind: PageKind, children: Vec<PageNode>) -> PageNode {
        let mut n = PageNode::new(title);
        n.kind = kind;
        n.children = children;
        n
    }

    fn tree() -> ProjectTree {
        let mut t = ProjectTree::new();
        t.roots = vec![
            node("Учёба", PageKind::Folder, vec![node("Лекция", PageKind::Notebook, vec![]), node("Учебник", PageKind::Book, vec![])]),
            node("Эскиз", PageKind::Drawing, vec![]),
            node("План", PageKind::Doc, vec![]),
        ];
        t
    }

    #[test]
    fn all_shows_folder_children() {
        let t = tree();
        let root = items(&t, Section::All, None, "", Sort::Manual);
        assert_eq!(root.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Учёба", "Эскиз", "План"]);
        let fid = t.roots[0].id.clone();
        let inside = items(&t, Section::All, Some(&fid), "", Sort::Manual);
        assert_eq!(inside.len(), 2);
    }

    #[test]
    fn sections_are_flat_by_kind() {
        let t = tree();
        let nb = items(&t, Section::Notebooks, None, "", Sort::Manual);
        assert_eq!(nb.len(), 1);
        assert_eq!(nb[0].title, "Лекция");
        assert_eq!(items(&t, Section::Books, None, "", Sort::Manual)[0].title, "Учебник");
    }

    #[test]
    fn search_is_flat_and_case_insensitive() {
        let t = tree();
        let r = items(&t, Section::All, None, "лек", Sort::Manual);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, PageKind::Notebook);
    }

    #[test]
    fn sort_by_kind_puts_folders_first() {
        let t = tree();
        let r = items(&t, Section::All, None, "", Sort::Kind);
        assert_eq!(r[0].kind, PageKind::Folder);
        assert_eq!(r.last().unwrap().kind, PageKind::Doc);
    }

    #[test]
    fn preview_strips_markup() {
        assert_eq!(doc_preview("# Заголовок\n\n![[kanban:x]]\n- **пункт**\n"), "Заголовок\nпункт");
    }
}
