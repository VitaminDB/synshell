//! Библиотека — главный экран заметок: блокноты, рисунки, книги, документы и
//! папки проекта.
//!
//! ```text
//! ┌────────────┬───────────────────────────────────────────────────────┐
//! │ MyLife   ▾ │ Главная                        [🔍 Поиск   ] [▦] [+ Создать] │
//! │            │                                                       │
//! │ ⌂ Главная  │ Продолжить                                            │
//! │ ✎ Блокноты │ [обложка][обложка][обложка] →                          │
//! │ 🖌 Рисунки  │ Блокноты                                   Все →      │
//! │ 📖 Книги    │ [блокнот][блокнот][блокнот] →                          │
//! │ 📄 Документы│ Папки · Документы …                                   │
//! │ ПАПКИ      │                                                       │
//! │ 📁 Учёба    │                                                       │
//! └────────────┴───────────────────────────────────────────────────────┘
//! ```
//!
//! Узкое окно (телефон, планшет в портрете): боковая панель уходит, внизу —
//! панель разделов. Открытая страница — на весь экран со своей шапкой
//! ([`Screen::Open`]): блокнот и рисунок — рукописный холст, книга — читалка,
//! документ — редактор. «←» в шапке, Esc и системное «назад» возвращают туда,
//! откуда открыли; из папки — к родителю.
//!
//! E Ink (`eink.mss`): без теней и наведений, рамки вместо заливок.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
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
    /// Главная: «Продолжить», полки по типам, папки; с выбранной папкой —
    /// её содержимое.
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
            Section::All => tr!("notes.lib.home"),
            Section::Notebooks => tr!("notes.lib.notebooks"),
            Section::Drawings => tr!("notes.lib.drawings"),
            Section::Books => tr!("notes.lib.books"),
            Section::Docs => tr!("notes.lib.docs"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Section::All => MI_HOME,
            Section::Notebooks => MI_EDIT_NOTE,
            Section::Drawings => MI_BRUSH,
            Section::Books => MI_AUTO_STORIES,
            Section::Docs => MI_DESCRIPTION,
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

    fn of_kind(kind: PageKind) -> Section {
        match kind {
            PageKind::Notebook => Section::Notebooks,
            PageKind::Drawing => Section::Drawings,
            PageKind::Book => Section::Books,
            PageKind::Doc => Section::Docs,
            PageKind::Folder => Section::All,
        }
    }
}

/// Порядок карточек.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Sort {
    /// Как в дереве (ручной порядок).
    #[default]
    Manual,
    /// Недавно открытые первыми.
    Recent,
    Name,
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
    /// Недавно открытые страницы текущего проекта, свежие первыми.
    pub recent: RwSignal<Vec<String>>,
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
            recent: use_signal(Vec::new()),
        }
    }

    /// Открыть страницу на весь экран (папка — войти в неё).
    pub fn open(&self, ctx: NotesCtx, id: &str) {
        let kind = ctx.tree.get_untracked().kind_of(id);
        if kind == PageKind::Folder {
            self.go_folder(Some(id.to_string()));
            return;
        }
        ctx.show_graph.set(false);
        ctx.activate(id);
        self.touch_recent(ctx, id);
        self.screen.set(Screen::Open(id.to_string()));
    }

    /// Войти в папку (`None` — главная).
    pub fn go_folder(&self, folder: Option<String>) {
        self.section.set(Section::All);
        self.search.set(String::new());
        self.search_open.set(false);
        self.folder.set(folder);
        self.screen.set(Screen::Library);
    }

    /// Раздел из боковой панели / нижней навигации.
    pub fn go_section(&self, s: Section) {
        self.search.set(String::new());
        self.search_open.set(false);
        self.folder.set(None);
        self.section.set(s);
        self.screen.set(Screen::Library);
    }

    /// Назад: из страницы — в библиотеку, из поиска — к разделу, из папки —
    /// к родителю, из раздела — на главную.
    pub fn back(&self, ctx: NotesCtx) -> bool {
        match self.screen.get_untracked() {
            Screen::Open(_) => {
                self.screen.set(Screen::Library);
                true
            }
            Screen::Library => {
                if self.search_open.get_untracked() || !self.search.get_untracked().is_empty() {
                    self.search_open.set(false);
                    self.search.set(String::new());
                    return true;
                }
                if let Some(f) = self.folder.get_untracked() {
                    let tree = ctx.tree.get_untracked();
                    self.folder.set(tree.parent_of(&f).filter(|p| tree.kind_of(p) == PageKind::Folder));
                    return true;
                }
                if self.section.get_untracked() != Section::All {
                    self.section.set(Section::All);
                    return true;
                }
                false
            }
        }
    }

    /// Создать элемент в текущей папке и открыть его.
    pub fn create(&self, ctx: NotesCtx, kind: PageKind) {
        let parent = self.folder.get_untracked();
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

    fn touch_recent(&self, ctx: NotesCtx, id: &str) {
        self.recent.update(|v| {
            v.retain(|x| x != id);
            v.insert(0, id.to_string());
            v.truncate(RECENT_MAX);
        });
        save_recent(&ctx.project_path.get_untracked(), &self.recent.get_untracked());
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Недавние (по проектам, в файле состояния) ─────────────────────────────

const RECENT_MAX: usize = 24;

#[derive(Default, Serialize, Deserialize)]
struct RecentFile {
    #[serde(default)]
    projects: HashMap<String, Vec<String>>,
}

fn recent_path() -> PathBuf {
    crate::config::state_path().with_file_name("library.json")
}

fn load_recent_file() -> RecentFile {
    std::fs::read_to_string(recent_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn load_recent(project: &std::path::Path) -> Vec<String> {
    load_recent_file().projects.remove(&project.display().to_string()).unwrap_or_default()
}

fn save_recent(project: &std::path::Path, list: &[String]) {
    if project.as_os_str().is_empty() {
        return;
    }
    let mut f = load_recent_file();
    f.projects.insert(project.display().to_string(), list.to_vec());
    if let Ok(s) = serde_json::to_string(&f) {
        let path = recent_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = crate::fsutil::write_atomic(&path, s.as_bytes());
    }
}

// ─── Типы ───────────────────────────────────────────────────────────────────

/// Значок типа страницы.
pub fn kind_icon(kind: PageKind) -> &'static str {
    match kind {
        PageKind::Doc => MI_DESCRIPTION,
        PageKind::Notebook => MI_EDIT_NOTE,
        PageKind::Drawing => MI_BRUSH,
        PageKind::Book => MI_AUTO_STORIES,
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

/// Карточки раздела: «Главная» — дети текущей папки; разделы по типу и
/// поиск — плоско по всему дереву.
pub fn items(tree: &ProjectTree, section: Section, folder: Option<&str>, query: &str, sort: Sort) -> Vec<Item> {
    items_sorted(tree, section, folder, query, sort, &[])
}

fn items_sorted(
    tree: &ProjectTree,
    section: Section,
    folder: Option<&str>,
    query: &str,
    sort: Sort,
    recent: &[String],
) -> Vec<Item> {
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
        Sort::Recent => out.sort_by_key(|i| recent.iter().position(|r| *r == i.id).unwrap_or(usize::MAX)),
    }
    out
}

/// Все страницы типа (по дереву).
fn all_of(tree: &ProjectTree, kind: PageKind) -> Vec<Item> {
    items(tree, Section::of_kind(kind), None, "", Sort::Manual)
}

// ─── Подключаемые виды страниц ──────────────────────────────────────────────

type W = Box<dyn Widget>;

/// Миниатюра карточки и полноэкранный вид для рукописных и книжных страниц.
/// Ставятся модулями `ink` и `reader` ([`super::kinds`]); без них — заглушка.
pub struct KindViews {
    /// Миниатюра: (ctx, id страницы, тип) → виджет или None (обложка по типу).
    pub thumbnail: fn(NotesCtx, &str, PageKind) -> Option<W>,
    /// Подробности под названием: «12 стр.», «Автор · 37%», …
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

/// Подключаемый импорт книги (ставит модуль читалки).
static IMPORT_BOOK: std::sync::OnceLock<fn(NotesCtx, Library, Option<String>, PathBuf)> = std::sync::OnceLock::new();

pub fn set_book_importer(f: fn(NotesCtx, Library, Option<String>, PathBuf)) {
    let _ = IMPORT_BOOK.set(f);
}

/// Выбор файла книги и импорт в текущую папку.
pub fn import_book_dialog(ctx: NotesCtx, lib: Library) {
    let parent = lib.folder.get_untracked();
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

/// Доля прочитанного из подробностей книги («… · 37%»).
fn progress_of(details: &str) -> Option<f32> {
    let pct = details.rsplit('·').next()?.trim().strip_suffix('%')?;
    pct.trim().parse::<f32>().ok().map(|p| (p / 100.0).clamp(0.0, 1.0))
}

// ─── Общие мелочи ───────────────────────────────────────────────────────────

/// На всю доступную площадь (Reactive и Column сами берут только своё).
fn fill<M>(w: impl syngui::widgets::containers::IntoWidget<M>) -> Stack {
    Stack::new().fit(StackFit::Expand).child(w)
}

/// Кнопка с выпадающим меню под ней.
fn menu_button(
    icon: &'static str,
    tooltip: String,
    class: &'static str,
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
        .class(class);
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

fn tap<M>(child: impl syngui::widgets::containers::IntoWidget<M>, f: impl Fn() + Send + Sync + 'static) -> GestureDetector {
    GestureDetector::new().cursor(CursorIcon::Pointer).on_click(f).child(child)
}

fn create_items() -> Vec<MenuItem> {
    vec![
        MenuItem::new("notebook", tr!("notes.lib.kind.notebook")).icon(MI_EDIT_NOTE),
        MenuItem::new("drawing", tr!("notes.lib.kind.drawing")).icon(MI_BRUSH),
        MenuItem::new("doc", tr!("notes.lib.kind.doc")).icon(MI_DESCRIPTION),
        MenuItem::separator(),
        MenuItem::new("book", tr!("notes.lib.import_book")).icon(MI_UPLOAD_FILE),
        MenuItem::new("folder", tr!("notes.lib.kind.folder")).icon(MI_CREATE_NEW_FOLDER),
    ]
}

fn on_create(ctx: NotesCtx, lib: Library) -> impl FnMut(&str) + Send + 'static {
    move |k: &str| match k {
        "book" => import_book_dialog(ctx, lib),
        other => {
            if let Some(kind) = PageKind::from_name(other) {
                lib.create(ctx, kind);
            }
        }
    }
}

// ─── Корень ─────────────────────────────────────────────────────────────────

/// Экран библиотеки или открытой страницы.
pub fn view(wide: RwSignal<bool>) -> impl Widget {
    let ctx = use_context::<NotesCtx>();
    let lib = use_context::<Library>();
    // Недавние — свои у каждого проекта.
    create_effect(move || {
        let p = ctx.project_path.get();
        lib.recent.set(load_recent(&p));
    });
    // Страница пропала (удалена ассистентом, закрыт проект) — назад.
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
        let w = wide.get();
        match lib.screen.get() {
            Screen::Library => vec![Box::new(fill(library(ctx, lib, w)))],
            Screen::Open(id) => vec![Box::new(fill(open_view(ctx, lib, id)))],
        }
    }))
}

/// Тело страницы-не-документа (для центра трёхпанельного редактора synthos).
pub fn kind_body(ctx: NotesCtx, id: &str, kind: PageKind) -> W {
    let lib = syngui::context_provider::try_use_context::<Library>().unwrap_or_default();
    if kind == PageKind::Folder {
        return Box::new(placeholder(kind));
    }
    views().and_then(|v| (v.view)(ctx, lib, id, kind)).unwrap_or_else(|| Box::new(placeholder(kind)))
}

// ─── Открытая страница ──────────────────────────────────────────────────────

fn open_view(ctx: NotesCtx, lib: Library, id: String) -> W {
    let kind = ctx.tree.get_untracked().kind_of(&id);
    let body: W = match kind {
        PageKind::Doc => Box::new(super::editor_pane::body()),
        _ => views().and_then(|v| (v.view)(ctx, lib, &id, kind)).unwrap_or_else(|| Box::new(placeholder(kind))),
    };
    Box::new(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("lib-open")
            .child(open_bar(ctx, lib, id, kind))
            .child(DecoratedBox::new().class("grow lib-open-body").clip(true).child(fill(body))),
    )
}

/// Шапка открытой страницы: «←», название (тап — переименовать) и путь,
/// действия вида, подстраницы, меню.
fn open_bar(ctx: NotesCtx, lib: Library, id: String, kind: PageKind) -> impl Widget {
    let actions = views().and_then(|v| (v.actions)(ctx, lib, &id, kind));
    let title = Reactive::new({
        let id = id.clone();
        move || -> Vec<W> {
            let tree = ctx.tree.get();
            let t = tree.title_of(&id).unwrap_or_default();
            let mut path: Vec<String> = tree.path_of(&id).into_iter().map(|(_, t)| t).collect();
            path.pop();
            let sub = if path.is_empty() { kind_title(kind) } else { format!("{} · {}", kind_title(kind), path.join(" / ")) };
            vec![Box::new(
                Column::new()
                    .gap(1.0)
                    .child(Text::new(t).max_lines(1).elide(Elide::End).class("lib-open-title"))
                    .child(Text::new(sub).max_lines(1).elide(Elide::End).class("lib-open-sub")),
            )]
        }
    });
    let mut row = Row::new()
        .gap(6.0)
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
    if kind == PageKind::Doc {
        row = row.child(doc_history(ctx));
    }
    if let Some(a) = actions {
        row = row.child(a);
    }
    if let Some(sub) = subpages_menu(ctx, lib, &id) {
        row = row.child(sub);
    }
    row = row.child(page_menu(ctx, lib, id));
    DecoratedBox::new().class("lib-openbar").child(row)
}

/// Отмена/повтор документа.
fn doc_history(ctx: NotesCtx) -> impl Widget {
    use syngui::widgets::input::document_editor::DocOp;
    Reactive::new(move || -> Vec<W> {
        let (u, r) = ctx.active_page().map(|p| p.handle.history_state().get()).unwrap_or((false, false));
        vec![Box::new(
            Row::new()
                .gap(2.0)
                .child(ToolButton::new(MI_UNDO).tooltip(tr!("notes.undo")).disabled(!u).on_click(move || ctx.doc_op(DocOp::Undo)).class("lib-icon-btn"))
                .child(ToolButton::new(MI_REDO).tooltip(tr!("notes.redo")).disabled(!r).on_click(move || ctx.doc_op(DocOp::Redo)).class("lib-icon-btn")),
        )]
    })
}

/// Подстраницы открытой страницы — меню переходов (у документов бывают дети).
fn subpages_menu(ctx: NotesCtx, lib: Library, id: &str) -> Option<impl Widget> {
    let tree = ctx.tree.get_untracked();
    let node = tree.find(id)?;
    if node.children.is_empty() {
        return None;
    }
    let items: Vec<MenuItem> = node
        .children
        .iter()
        .map(|c| MenuItem::new(c.id.clone(), c.title.clone()).icon(kind_icon(c.kind)))
        .collect();
    Some(menu_button(MI_ACCOUNT_TREE, tr!("notes.lib.subpages"), "lib-icon-btn", items, move |k: &str| lib.open(ctx, k)))
}

/// «←» назад.
pub fn back_button(ctx: NotesCtx, lib: Library) -> impl Widget {
    ToolButton::new(MI_ARROW_BACK)
        .tooltip(tr!("notes.lib.back"))
        .on_click(move || {
            lib.back(ctx);
        })
        .class("lib-back")
}

fn page_menu(ctx: NotesCtx, lib: Library, id: String) -> impl Widget {
    let items = vec![
        MenuItem::new("rename", tr!("notes.tree.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE),
        MenuItem::new("duplicate", tr!("notes.tree.duplicate")).icon(MI_CONTENT_COPY),
        MenuItem::separator(),
        MenuItem::new("delete", tr!("app.delete")).icon(MI_DELETE),
    ];
    menu_button(MI_MORE_VERT, tr!("notes.lib.more"), "lib-icon-btn", items, move |key: &str| match key {
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
            .child(Icon::new(kind_icon(kind)).class("lib-empty-icon"))
            .child(Text::new(kind_title(kind)).class("lib-empty-title"))
            .child(Text::new(tr!("notes.lib.coming")).class("lib-empty-hint")),
    )
}

// ─── Библиотека ─────────────────────────────────────────────────────────────

fn library(ctx: NotesCtx, lib: Library, wide: bool) -> W {
    let content = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .class("lib-main")
        .child(header(ctx, lib, wide))
        .child(DecoratedBox::new().class("grow").child(fill(
            ScrollView::new().vertical().child(DecoratedBox::new().class("lib-content").child(content(ctx, lib, wide))),
        )));
    if wide {
        Box::new(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .class("lib")
                .child(sidebar(ctx, lib))
                .child(DecoratedBox::new().class("grow").child(fill(content))),
        )
    } else {
        Box::new(
            Column::new()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .class("lib")
                .child(DecoratedBox::new().class("grow").child(fill(content)))
                .child(bottom_nav(lib)),
        )
    }
}

// ─── Боковая панель ─────────────────────────────────────────────────────────

fn sidebar(ctx: NotesCtx, lib: Library) -> impl Widget {
    let nav = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let cur = lib.section.get();
        let folder = lib.folder.get();
        let tree = ctx.tree.get();
        let counts = count_by_kind(&tree);
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for s in Section::ALL {
            let active = s == cur && (s != Section::All || folder.is_none());
            let n = s.kind().map(|k| counts.get(&k).copied().unwrap_or(0));
            col = col.child(nav_row(s.icon(), s.title(), n, active, 0, move || lib.go_section(s)));
        }
        // Папки — деревом (вложенные — с отступом).
        let mut folders = Vec::new();
        fn walk(nodes: &[PageNode], depth: usize, out: &mut Vec<(String, String, usize)>) {
            for n in nodes {
                if n.kind == PageKind::Folder {
                    out.push((n.id.clone(), n.title.clone(), depth));
                    walk(&n.children, depth + 1, out);
                }
            }
        }
        walk(&tree.roots, 0, &mut folders);
        col = col.child(
            Row::new()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .class("lib-side-section")
                .child(DecoratedBox::new().class("grow").child(Text::new(tr!("notes.lib.folders")).class("lib-side-caption")))
                .child(
                    ToolButton::new(MI_ADD)
                        .tooltip(tr!("notes.lib.new.folder"))
                        .on_click(move || lib.create(use_context::<NotesCtx>(), PageKind::Folder))
                        .class("lib-side-add"),
                ),
        );
        if folders.is_empty() {
            col = col.child(Text::new(tr!("notes.lib.folders.none")).max_lines(2).class("lib-side-hint"));
        }
        for (id, title, depth) in folders {
            let active = folder.as_deref() == Some(id.as_str());
            let fid = id.clone();
            col = col.child(nav_row(MI_FOLDER, title, None, active, depth, move || lib.go_folder(Some(fid.clone()))));
        }
        vec![Box::new(col)]
    });
    DecoratedBox::new().class("lib-side").child(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(project_switcher(ctx))
            .child(DecoratedBox::new().class("grow").child(fill(ScrollView::new().vertical().child(DecoratedBox::new().class("lib-side-nav").child(nav))))),
    )
}

fn nav_row(icon: &'static str, title: String, count: Option<usize>, active: bool, depth: usize, f: impl Fn() + Send + Sync + 'static) -> W {
    let mut row = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(icon).class("lib-nav-icon"))
        .child(DecoratedBox::new().class("grow").child(Text::new(title).max_lines(1).elide(Elide::End).class("lib-nav-text")));
    if let Some(n) = count.filter(|n| *n > 0) {
        row = row.child(Text::new(n.to_string()).class("lib-nav-count"));
    }
    let class = match (active, depth) {
        (true, 0) => "lib-nav active",
        (false, 0) => "lib-nav",
        (true, _) => "lib-nav active nested",
        (false, _) => "lib-nav nested",
    };
    let pad = Padding::only(16.0 * depth.min(4) as f32, 0.0, 0.0, 0.0).child(row);
    Box::new(tap(DecoratedBox::new().class(class).child(pad), f))
}

/// Проект: название и меню (открытые проекты, создать, открыть, сохранить…).
fn project_switcher(ctx: NotesCtx) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let title = ctx.project_title.get();
        let current = ctx.project_path.get();
        let open = ctx.projects.get();
        let mut items: Vec<MenuItem> = open
            .iter()
            .map(|p| {
                MenuItem::new(format!("notes-switch:{}", p.path.display()), p.title())
                    .icon(if p.path == current { MI_CHECK } else { MI_FOLDER })
            })
            .collect();
        items.push(MenuItem::separator());
        items.extend(super::project_ui::project_menu_items(ctx));
        let letter = title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let face = Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("lib-proj-badge").child(Center::new().child(Text::new(letter).class("lib-proj-letter"))))
            .child(
                DecoratedBox::new().class("grow").child(
                    Column::new()
                        .gap(1.0)
                        .child(Text::new(title).max_lines(1).elide(Elide::End).class("lib-proj-title"))
                        .child(Text::new(tr!("notes.lib.project")).class("lib-proj-sub")),
                ),
            )
            .child(Icon::new(MI_EXPAND_MORE).class("lib-proj-chevron"));
        vec![Box::new(Padding::only(12.0, 10.0, 12.0, 8.0).child(menu_on(DecoratedBox::new().class("lib-proj").child(face), items, move |id: &str| {
            if let Some(path) = id.strip_prefix("notes-switch:") {
                ctx.switch_project(std::path::Path::new(path));
                use_context::<Library>().go_section(Section::All);
            } else {
                super::project_ui::handle_menu(id);
            }
        })))]
    })
}

/// Нижняя навигация узкого окна.
fn bottom_nav(lib: Library) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let cur = lib.section.get();
        let in_folder = lib.folder.get().is_some();
        let mut row = Row::new().cross_axis_alignment(CrossAxisAlignment::Center);
        for s in Section::ALL {
            let active = s == cur && !(s == Section::All && in_folder && false);
            let item = Column::new()
                .gap(3.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class(if active { "lib-bnav-pill active" } else { "lib-bnav-pill" }).child(Center::new().child(Icon::new(s.icon()).class("lib-bnav-icon"))))
                .child(Text::new(s.title()).max_lines(1).class(if active { "lib-bnav-text active" } else { "lib-bnav-text" }));
            row = row.child(DecoratedBox::new().class("grow").child(tap(Center::new().child(item), move || lib.go_section(s))));
        }
        vec![Box::new(DecoratedBox::new().class("lib-bnav").child(row))]
    })
}

// ─── Шапка ──────────────────────────────────────────────────────────────────

fn header(ctx: NotesCtx, lib: Library, wide: bool) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let tree = ctx.tree.get();
        let section = lib.section.get();
        let folder = lib.folder.get();
        let searching = lib.search_open.get() || !lib.search.get_untracked().is_empty();
        let as_grid = lib.grid.get();
        let _ = lib.sort.get();

        // Заголовок и подзаголовок.
        let (title, sub, back) = match (&folder, section) {
            (Some(f), Section::All) => {
                let mut path: Vec<String> = tree.path_of(f).into_iter().map(|(_, t)| t).collect();
                let t = path.pop().unwrap_or_default();
                let n = tree.find(f).map(|n| n.children.len()).unwrap_or(0);
                let mut sub = vec![tr!("notes.lib.home")];
                sub.extend(path);
                (t, format!("{} · {}", sub.join(" / "), trn!("notes.lib.items", n as i64, n = n)), true)
            }
            (_, s) => {
                let sub = match s.kind() {
                    Some(k) => {
                        let n = all_of(&tree, k).len();
                        trn!("notes.lib.items", n as i64, n = n)
                    }
                    None => ctx.project_title.get(),
                };
                (s.title(), sub, s != Section::All)
            }
        };

        let mut top = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if back && !searching {
            top = top.child(back_button(ctx, lib));
        }
        if searching && !wide {
            top = top
                .child(ToolButton::new(MI_ARROW_BACK).tooltip(tr!("notes.lib.back")).on_click(move || {
                    lib.search_open.set(false);
                    lib.search.set(String::new());
                }).class("lib-back"))
                .child(DecoratedBox::new().class("grow").child(search_field(lib, true)));
        } else {
            top = top.child(
                DecoratedBox::new().class("grow").child(
                    Column::new()
                        .gap(2.0)
                        .child(Text::new(title).max_lines(1).elide(Elide::End).class(if wide { "lib-h1" } else { "lib-h1 narrow" }))
                        .child(Text::new(sub).max_lines(1).elide(Elide::End).class("lib-h1-sub")),
                ),
            );
            if wide {
                top = top.child(DecoratedBox::new().class("lib-search-box").child(search_field(lib, false)));
            } else {
                top = top.child(ToolButton::new(MI_SEARCH).tooltip(tr!("notes.lib.search")).on_click(move || lib.search_open.set(true)).class("lib-icon-btn"));
            }
            top = top.child(view_menu(lib, as_grid));
            top = top.child(create_button(ctx, lib, !wide));
        }
        vec![Box::new(DecoratedBox::new().class(if wide { "lib-header" } else { "lib-header narrow" }).child(top))]
    })
}

fn search_field(lib: Library, autofocus: bool) -> impl Widget {
    TextField::new()
        .placeholder(tr!("notes.lib.search"))
        .prefix_icon(MI_SEARCH)
        .text(lib.search.get_untracked())
        .on_change(move |t| lib.search.set(t.to_string()))
        .on_escape(move || {
            lib.search.set(String::new());
            lib.search_open.set(false);
        })
        .autofocus(autofocus)
        .class("lib-search")
}

/// Вид и порядок: карточки/список, ручной/недавние/по названию.
fn view_menu(lib: Library, as_grid: bool) -> impl Widget {
    let sort = lib.sort.get_untracked();
    let mark = |s: Sort| if sort == s { MI_CHECK } else { "" };
    let items = vec![
        MenuItem::new("grid", tr!("notes.lib.as_grid")).icon(if as_grid { MI_CHECK } else { "" }),
        MenuItem::new("list", tr!("notes.lib.as_list")).icon(if as_grid { "" } else { MI_CHECK }),
        MenuItem::separator(),
        MenuItem::new("manual", tr!("notes.lib.sort.manual")).icon(mark(Sort::Manual)),
        MenuItem::new("recent", tr!("notes.lib.sort.recent")).icon(mark(Sort::Recent)),
        MenuItem::new("name", tr!("notes.lib.sort.name")).icon(mark(Sort::Name)),
    ];
    menu_button(if as_grid { MI_GRID_VIEW } else { MI_VIEW_LIST }, tr!("notes.lib.view"), "lib-icon-btn", items, move |k: &str| match k {
        "grid" => lib.grid.set(true),
        "list" => lib.grid.set(false),
        "manual" => lib.sort.set(Sort::Manual),
        "recent" => lib.sort.set(Sort::Recent),
        "name" => lib.sort.set(Sort::Name),
        _ => {}
    })
}

fn create_button(ctx: NotesCtx, lib: Library, compact: bool) -> impl Widget {
    let mut face = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(MI_ADD).class("lib-create-icon"));
    if !compact {
        face = face.child(Text::new(tr!("notes.lib.create")).class("lib-create-text"));
    }
    menu_on(DecoratedBox::new().class(if compact { "lib-create compact" } else { "lib-create" }).child(face), create_items(), on_create(ctx, lib))
}

// ─── Содержимое ─────────────────────────────────────────────────────────────

fn content(ctx: NotesCtx, lib: Library, wide: bool) -> impl Widget {
    // Колонки по ширине: пересборка только при смене числа колонок.
    let vp = syngui::viewport::viewport_size();
    let side = if wide { 260.0 } else { 0.0 };
    let cols = use_signal(columns_for(vp.get_untracked().width - side));
    create_effect(move || {
        let c = columns_for(vp.get().width - side);
        if cols.get_untracked() != c {
            cols.set(c);
        }
    });
    Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let tree = ctx.tree.get();
        let section = lib.section.get();
        let folder = lib.folder.get();
        let query = lib.search.get();
        let sort = lib.sort.get();
        let as_grid = lib.grid.get();
        let recent = lib.recent.get();
        let cols = cols.get();

        let searching = !query.trim().is_empty();
        if section == Section::All && folder.is_none() && !searching {
            return vec![Box::new(home(ctx, lib, &tree, &recent, cols, as_grid))];
        }
        let list = items_sorted(&tree, section, folder.as_deref(), &query, sort, &recent);
        if list.is_empty() {
            return vec![Box::new(empty(ctx, lib, section, searching))];
        }
        vec![collection(ctx, lib, list, cols, as_grid)]
    })
}

/// Сетка или список карточек.
fn collection(ctx: NotesCtx, lib: Library, list: Vec<Item>, cols: usize, as_grid: bool) -> W {
    if as_grid {
        let cards: Vec<W> = list.into_iter().map(|it| card(ctx, lib, it)).collect();
        Box::new(Grid::new(cols).row_gap(28.0).col_gap(20.0).children(cards))
    } else {
        let rows: Vec<W> = list.into_iter().map(|it| list_row(ctx, lib, it)).collect();
        Box::new(DecoratedBox::new().class("lib-list").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).children(rows)))
    }
}

/// Главная: «Продолжить», полки по типам, папки и документы корня.
fn home(ctx: NotesCtx, lib: Library, tree: &ProjectTree, recent: &[String], cols: usize, as_grid: bool) -> impl Widget {
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    // Продолжить: недавно открытые, что ещё есть в дереве.
    let cont: Vec<Item> = recent
        .iter()
        .filter_map(|id| tree.find(id))
        .filter(|n| n.kind != PageKind::Folder)
        .take(cols.max(4))
        .map(item_of)
        .collect();
    if !cont.is_empty() {
        col = col.child(section_title(tr!("notes.lib.continue"), None));
        col = col.child(shelf(ctx, lib, cont, cols));
    }
    for (kind, s) in [
        (PageKind::Notebook, Section::Notebooks),
        (PageKind::Drawing, Section::Drawings),
        (PageKind::Book, Section::Books),
    ] {
        let all = all_of(tree, kind);
        if all.is_empty() {
            continue;
        }
        let more = (all.len() > cols).then_some(s);
        col = col.child(section_title(s.title(), more.map(|s| (lib, s))));
        col = col.child(shelf(ctx, lib, all.into_iter().take(cols).collect(), cols));
    }
    // Корень проекта: папки и документы (как разложено).
    let root: Vec<Item> = tree.roots.iter().filter(|n| matches!(n.kind, PageKind::Folder | PageKind::Doc)).map(item_of).collect();
    if !root.is_empty() {
        col = col.child(section_title(tr!("notes.lib.root"), None));
        col = col.child(collection(ctx, lib, root, cols, as_grid));
    }
    let empty_all = tree.roots.is_empty();
    if empty_all {
        col = col.child(empty(ctx, lib, Section::All, false));
    } else {
        col = col.child(quick_create(ctx, lib));
    }
    col
}

fn section_title(title: String, more: Option<(Library, Section)>) -> impl Widget {
    let mut row = Row::new()
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("grow").child(Text::new(title).class("lib-h2")));
    if let Some((lib, s)) = more {
        row = row.child(tap(
            Row::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(tr!("notes.lib.see_all")).class("lib-more"))
                .child(Icon::new(MI_CHEVRON_RIGHT).class("lib-more-icon")),
            move || lib.go_section(s),
        ));
    }
    DecoratedBox::new().class("lib-h2-row").child(row)
}

/// Полка: один ряд карточек той же ширины, что в сетке.
fn shelf(ctx: NotesCtx, lib: Library, list: Vec<Item>, cols: usize) -> impl Widget {
    let mut cards: Vec<W> = list.into_iter().map(|it| card(ctx, lib, it)).collect();
    while cards.len() < cols {
        cards.push(Box::new(DecoratedBox::new()));
    }
    Grid::new(cols).col_gap(20.0).children(cards)
}

/// Быстрое создание в конце главной: крупные плитки.
fn quick_create(ctx: NotesCtx, lib: Library) -> impl Widget {
    let tile = move |icon: &'static str, title: String, kind: Option<PageKind>| -> W {
        let face = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("lib-quick-badge").child(Center::new().child(Icon::new(icon).class("lib-quick-icon"))))
            .child(Text::new(title).max_lines(1).class("lib-quick-text"));
        Box::new(tap(DecoratedBox::new().class("lib-quick").child(face), move || match kind {
            Some(k) => lib.create(ctx, k),
            None => import_book_dialog(ctx, lib),
        }))
    };
    Column::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(section_title(tr!("notes.lib.quick"), None))
        .child(
            Row::new()
                .gap(12.0)
                .child(DecoratedBox::new().class("grow").child(tile(MI_EDIT_NOTE, tr!("notes.lib.quick.notebook"), Some(PageKind::Notebook))))
                .child(DecoratedBox::new().class("grow").child(tile(MI_BRUSH, tr!("notes.lib.quick.drawing"), Some(PageKind::Drawing))))
                .child(DecoratedBox::new().class("grow").child(tile(MI_UPLOAD_FILE, tr!("notes.lib.import_book"), None))),
        )
}

fn empty(ctx: NotesCtx, lib: Library, section: Section, searching: bool) -> impl Widget {
    let (icon, title, hint) = if searching {
        (MI_SEARCH_OFF, tr!("notes.lib.empty.search"), String::new())
    } else {
        (section.icon(), tr!("notes.lib.empty.title"), tr!("notes.lib.empty.hint"))
    };
    let mut col = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class("lib-empty-badge").child(Center::new().child(Icon::new(icon).class("lib-empty-icon"))))
        .child(Text::new(title).class("lib-empty-title"));
    if !hint.is_empty() {
        col = col.child(Text::new(hint).max_lines(3).class("lib-empty-hint"));
    }
    if !searching {
        let kind = match section {
            Section::Drawings => PageKind::Drawing,
            Section::Docs => PageKind::Doc,
            Section::Books => PageKind::Book,
            _ => PageKind::Notebook,
        };
        let label = match kind {
            PageKind::Book => tr!("notes.lib.import_book"),
            PageKind::Drawing => tr!("notes.lib.quick.drawing"),
            PageKind::Doc => tr!("notes.lib.quick.doc"),
            _ => tr!("notes.lib.quick.notebook"),
        };
        col = col.child(tap(
            DecoratedBox::new().class("lib-create").child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new(MI_ADD).class("lib-create-icon"))
                    .child(Text::new(label).class("lib-create-text")),
            ),
            move || {
                if kind == PageKind::Book {
                    import_book_dialog(ctx, lib);
                } else {
                    lib.create(ctx, kind);
                }
            },
        ));
    }
    DecoratedBox::new().class("lib-empty").child(Center::new().child(col))
}

// ─── Карточки ───────────────────────────────────────────────────────────────

/// Подробности под названием карточки.
fn details(ctx: NotesCtx, it: &Item) -> String {
    let extra = match it.kind {
        PageKind::Folder => Some(trn!("notes.lib.items", it.children as i64, n = it.children)),
        PageKind::Doc => (it.children > 0).then(|| trn!("notes.lib.subpages.n", it.children as i64, n = it.children)),
        k => views().and_then(|v| (v.details)(ctx, &it.id, k)),
    };
    extra.filter(|e| !e.is_empty()).unwrap_or_else(|| kind_title(it.kind))
}

/// Цвет обложки блокнота — стабильный по id.
fn cover_tone(id: &str) -> usize {
    id.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize)) % 8
}

/// Обложка карточки 3:4.
fn cover(ctx: NotesCtx, it: &Item, details: &str) -> W {
    let thumb = (it.kind != PageKind::Doc && it.kind != PageKind::Folder)
        .then(|| views().and_then(|v| (v.thumbnail)(ctx, &it.id, it.kind)))
        .flatten();
    match it.kind {
        PageKind::Book => {
            let mut st = Stack::new().fit(StackFit::Expand);
            st = st.child(match thumb {
                Some(t) => t,
                None => Box::new(DecoratedBox::new().class("lib-cover-book-blank").child(Center::new().child(Icon::new(MI_AUTO_STORIES).class("lib-cover-glyph")))),
            });
            Box::new(DecoratedBox::new().class("lib-cover book").clip(true).child(st))
        }
        PageKind::Notebook => {
            let tone = cover_tone(&it.id);
            let body: W = match thumb {
                // Есть рукопись — первая страница на «бумаге» с корешком.
                Some(t) => Box::new(
                    Row::new()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(DecoratedBox::new().class(format!("lib-spine tone{tone}")))
                        .child(DecoratedBox::new().class("grow lib-paper").child(fill(t))),
                ),
                None => Box::new(
                    Row::new()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(DecoratedBox::new().class("lib-spine dark"))
                        .child(
                            DecoratedBox::new().class(format!("grow lib-nb-cover tone{tone}")).child(
                                Column::new()
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .main_axis_alignment(MainAxisAlignment::Center)
                                    .child(
                                        DecoratedBox::new()
                                            .class("lib-nb-label")
                                            .child(Text::new(it.title.clone()).max_lines(3).elide(Elide::End).class("lib-nb-label-text")),
                                    ),
                            ),
                        ),
                ),
            };
            Box::new(DecoratedBox::new().class("lib-cover notebook").clip(true).child(fill(body)))
        }
        PageKind::Drawing => {
            let body: W = thumb.unwrap_or_else(|| {
                Box::new(DecoratedBox::new().class("lib-canvas").child(Center::new().child(Icon::new(MI_BRUSH).class("lib-cover-glyph"))))
            });
            Box::new(DecoratedBox::new().class("lib-cover drawing").clip(true).child(fill(body)))
        }
        PageKind::Doc => {
            let text = doc_preview(&ctx.page_markdown(&it.id));
            let mut col = Column::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
            col = col.child(Text::new(it.title.clone()).max_lines(2).elide(Elide::End).class("lib-doc-h"));
            let body = text.splitn(2, '\n').nth(if text.starts_with(&it.title) { 1 } else { 0 }).unwrap_or("").to_string();
            let body = if text.starts_with(&it.title) { body } else { text.clone() };
            if !body.trim().is_empty() {
                col = col.child(Text::new(body).max_lines(12).elide(Elide::End).class("lib-doc-text"));
            }
            Box::new(DecoratedBox::new().class("lib-cover doc").clip(true).child(col))
        }
        PageKind::Folder => {
            let _ = details;
            Box::new(
                DecoratedBox::new().class("lib-cover folder").child(
                    Column::new()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .gap(8.0)
                        .child(Icon::new(MI_FOLDER).class("lib-folder-glyph"))
                        .child(Text::new(trn!("notes.lib.items", it.children as i64, n = it.children)).class("lib-folder-count")),
                ),
            )
        }
    }
}

fn card_menu_items(kind: PageKind) -> Vec<MenuItem> {
    let mut v = vec![
        MenuItem::new("open", if kind == PageKind::Folder { tr!("notes.lib.open_folder") } else { tr!("notes.lib.open") }).icon(MI_OPEN_IN_FULL),
        MenuItem::new("rename", tr!("notes.tree.rename")).icon(MI_DRIVE_FILE_RENAME_OUTLINE),
        MenuItem::new("duplicate", tr!("notes.tree.duplicate")).icon(MI_CONTENT_COPY),
        MenuItem::new("move_up", tr!("notes.lib.move_out")).icon(MI_ARROW_BACK),
    ];
    v.push(MenuItem::separator());
    v.push(MenuItem::new("delete", tr!("app.delete")).icon(MI_DELETE));
    v
}

fn card_action(ctx: NotesCtx, lib: Library, id: String) -> impl FnMut(&str) + Send + 'static {
    move |k: &str| match k {
        "open" => lib.open(ctx, &id),
        "rename" => ctx.renaming.set(Some(id.clone())),
        "duplicate" => {
            ctx.duplicate_page_with(&id, false);
        }
        "move_up" => {
            let tree = ctx.tree.get_untracked();
            if let Some(p) = tree.parent_of(&id) {
                let grand = tree.parent_of(&p);
                drop(tree);
                ctx.move_page(&id, grand.as_deref(), None);
            }
        }
        "delete" => ctx.delete_page(&id),
        _ => {}
    }
}

fn card(ctx: NotesCtx, lib: Library, it: Item) -> W {
    let id = it.id.clone();
    let det = details(ctx, &it);
    let progress = (it.kind == PageKind::Book).then(|| progress_of(&det)).flatten();
    let mut info = Column::new()
        .gap(3.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(it.title.clone()).max_lines(2).elide(Elide::End).class("lib-card-title"));
    let meta = Row::new()
        .gap(5.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Icon::new(kind_icon(it.kind)).class("lib-card-kind"))
        .child(DecoratedBox::new().class("grow").child(Text::new(det).max_lines(1).elide(Elide::End).class("lib-card-sub")));
    info = info.child(meta);
    if let Some(p) = progress.filter(|p| *p > 0.0) {
        info = info.child(ProgressBar::with_value(p).class("lib-progress"));
    }
    let body = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(AspectRatio::new(0.75).child(cover(ctx, &it, "")))
        .child(info);
    let open_id = id.clone();
    Box::new(
        ContextMenu::new()
            .items(card_menu_items(it.kind))
            .on_select(card_action(ctx, lib, id))
            .child(tap(DecoratedBox::new().class("lib-card").child(body), move || lib.open(ctx, &open_id))),
    )
}

fn list_row(ctx: NotesCtx, lib: Library, it: Item) -> W {
    let id = it.id.clone();
    let det = details(ctx, &it);
    let row = Row::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class(format!("lib-row-badge k-{}", it.kind.as_str())).child(Center::new().child(Icon::new(kind_icon(it.kind)).class("lib-row-icon"))))
        .child(
            DecoratedBox::new().class("grow").child(
                Column::new()
                    .gap(2.0)
                    .child(Text::new(it.title.clone()).max_lines(1).elide(Elide::End).class("lib-card-title"))
                    .child(Text::new(det).max_lines(1).elide(Elide::End).class("lib-card-sub")),
            ),
        )
        .child(menu_button(MI_MORE_VERT, tr!("notes.lib.more"), "lib-icon-btn small", card_menu_items(it.kind), card_action(ctx, lib, id.clone())));
    Box::new(tap(DecoratedBox::new().class("lib-row").child(row), move || lib.open(ctx, &id)))
}

// ─── Превью документа ───────────────────────────────────────────────────────

/// Первые строки markdown без разметки — для карточки документа.
pub fn doc_preview(md: &str) -> String {
    let mut out = Vec::new();
    for line in md.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with("![[") || l.starts_with("```") || l.starts_with("<!--") {
            continue;
        }
        // Атрибуты блоков холста (`{x=… w=…}`) и ограды `:::` — не текст.
        let (_, l) = syngui::widgets::input::document_editor::attrs::split_leading_attrs(l);
        let (l, _) = syngui::widgets::input::document_editor::attrs::split_trailing_attrs(&l);
        let l = l.trim_start_matches(|c: char| c == ':' || c == ' ');
        let l = l.trim_start_matches(|c: char| c == '#' || c == '>' || c == '-' || c == '*' || c == ' ');
        if l.chars().all(|c| c.is_ascii_digit() || c.is_whitespace()) {
            continue;
        }
        let mut l = l.replace("**", "").replace("__", "").replace("[[", "").replace("]]", "");
        // Выноска `[!note] …` — без метки.
        if l.starts_with("[!") {
            if let Some(end) = l.find(']') {
                l = l[end + 1..].trim_start().to_string();
            }
        }
        if !l.is_empty() {
            out.push(l);
        }
        if out.len() >= 12 {
            break;
        }
    }
    out.join("\n")
}

/// Колонок карточек при ширине области (логические px): карточка ~170–210.
pub fn columns_for(width: f32) -> usize {
    (((width - 48.0) / 190.0).floor() as usize).clamp(2, 8)
}

// ─── Диалог переименования ─────────────────────────────────────────────────

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
                    .gap(14.0)
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

/// Подсчёт страниц по типам.
pub fn count_by_kind(tree: &ProjectTree) -> Arc<HashMap<PageKind, usize>> {
    let mut m = HashMap::new();
    fn walk(nodes: &[PageNode], m: &mut HashMap<PageKind, usize>) {
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
    fn sort_recent_first() {
        let t = tree();
        let plan = t.roots[2].id.clone();
        let r = items_sorted(&t, Section::All, None, "", Sort::Recent, &[plan]);
        assert_eq!(r[0].title, "План");
    }

    #[test]
    fn progress_parsed_from_details() {
        assert_eq!(progress_of("Автор · 37%"), Some(0.37));
        assert_eq!(progress_of("12 стр."), None);
    }

    #[test]
    fn preview_strips_markup() {
        assert_eq!(doc_preview("# Заголовок\n\n![[kanban:x]]\n- **пункт**\n"), "Заголовок\nпункт");
        assert_eq!(doc_preview("::: {x=1060 w=1630 y=0}\nТекст {color=red}\n:::\n"), "Текст");
        assert_eq!(doc_preview("> [!note] Хаб\n[[Цели]] — миссия"), "Хаб\nЦели — миссия");
    }
}
