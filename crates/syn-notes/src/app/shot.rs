//! `syn-notes --screenshot out.png [--size 1240x930] [--scale 2] [--eink]
//! [--open notebook|drawing|book|doc] [--narrow]` — снимок окна без
//! графического сеанса на демо-проекте во временном HOME (настоящие проекты
//! и состояние не трогаются). Для проверки вёрстки библиотеки и видов страниц.

use std::path::PathBuf;

use syngui::prelude::*;
use synshell_common::Config;

use crate::notes::library::{Library, Screen};
use crate::notes::project::PageKind;
use crate::notes::NotesCtx;

pub struct ShotArgs {
    pub out: String,
    pub size: (u32, u32),
    pub scale: f64,
    pub open: Option<String>,
    pub list: bool,
    /// Книги для импорта (последняя открывается, если `--open book`).
    pub books: Vec<PathBuf>,
}

pub fn parse(out: String, rest: &[String]) -> ShotArgs {
    let mut a = ShotArgs { out, size: (1240, 930), scale: 1.0, open: None, list: false, books: Vec::new() };
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x')) {
                    a.size = (w.parse().unwrap_or(1240), h.parse().unwrap_or(930));
                }
            }
            "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--open" => a.open = it.next().cloned(),
            "--eink" => std::env::set_var("SYNSHELL_EINK", "1"),
            "--list" => a.list = true,
            "--book" => a.books.push(PathBuf::from(it.next().cloned().unwrap_or_default())),
            _ => {}
        }
    }
    a
}

/// Временный HOME: состояние, основное хранилище и конфиг — в нём.
fn sandbox() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("syn-notes-shot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("Documents"));
    std::env::set_var("HOME", &dir);
    std::env::set_var("XDG_CONFIG_HOME", dir.join(".config"));
    std::env::set_var("XDG_STATE_HOME", dir.join(".local/state"));
    std::env::set_var("XDG_DATA_HOME", dir.join(".local/share"));
    std::env::set_var("XDG_CACHE_HOME", dir.join(".cache"));
    dir
}

/// Демо-проект: папка с блокнотом и книгой, рисунок, документы.
fn populate(ctx: NotesCtx) {
    let tree = ctx.tree.get_untracked();
    let first = tree.roots.first().map(|n| n.id.clone());
    drop(tree);
    if let Some(id) = first {
        ctx.rename_page(&id, "План недели");
        ctx.set_page_markdown(
            &id,
            "# План недели\n\n- Дочитать «Мастер и Маргарита»\n- Конспект лекции по матанализу\n- Эскизы для обложки\n\n> Встреча в четверг в 15:00\n",
        );
    }
    let study = ctx.insert_page_of_kind(None, None, "Учёба", PageKind::Folder, false);
    ctx.insert_page_of_kind(Some(&study), None, "Матанализ — лекции", PageKind::Notebook, false);
    ctx.insert_page_of_kind(Some(&study), None, "Физика", PageKind::Notebook, false);
    ctx.insert_page_of_kind(None, None, "Ежедневник", PageKind::Notebook, false);
    ctx.insert_page_of_kind(None, None, "Эскиз обложки", PageKind::Drawing, false);
    let doc = ctx.insert_page(None, None, "Идеи", false);
    ctx.set_page_markdown(&doc, "# Идеи\n\nРукописный ввод с распознаванием, шаблоны страниц, синхронизация.\n");
}

pub fn run(args: ShotArgs) {
    let home = sandbox();
    synshell_common::i18n::init(&crate::i18n::CATALOGS);
    let (cfg, _) = Config::load();
    super::install_state();
    let ctx = use_context::<NotesCtx>();
    let path = home.join("Documents/Демо.syn");
    if let Err(e) = ctx.create_project(&path) {
        eprintln!("демо-проект не создан: {}", e.message());
        std::process::exit(1);
    }
    populate(ctx);
    let lib = use_context::<Library>();
    let open_book = args.open.as_deref() == Some("book") && !args.books.is_empty();
    for b in &args.books {
        crate::notes::reader::import_book(ctx, lib, None, b.clone());
    }
    let books_before = crate::notes::library::count_by_kind(&ctx.tree.get_untracked()).get(&PageKind::Book).copied().unwrap_or(0);
    let want_books = books_before + args.books.len();
    if !args.books.is_empty() {
        // Импорт идёт в фоне и сам открывает книгу — см. `ready` ниже.
    } else if let Some(kind) = args.open.as_deref().and_then(PageKind::from_name) {
        let tree = ctx.tree.get_untracked();
        let mut found = None;
        fn walk(nodes: &[crate::notes::project::PageNode], kind: PageKind, found: &mut Option<String>) {
            for n in nodes {
                if found.is_none() && n.kind == kind {
                    *found = Some(n.id.clone());
                }
                walk(&n.children, kind, found);
            }
        }
        walk(&tree.roots, kind, &mut found);
        drop(tree);
        if let Some(id) = found {
            use_context::<Library>().open(ctx, &id);
        }
    } else if args.open.as_deref() == Some("folder") {
        let id = ctx.tree.get_untracked().roots.iter().find(|n| n.kind == PageKind::Folder).map(|n| n.id.clone());
        use_context::<Library>().folder.set(id);
    }
    let _ = Screen::Library;
    if args.list {
        use_context::<Library>().grid.set(false);
    }
    let mss = super::theme::mss(&cfg);
    let opts = syngui::shot::ShotOptions {
        wait: std::time::Duration::from_millis(if args.books.is_empty() { 0 } else { 2500 }),
        ready: (!args.books.is_empty()).then(|| {
            Box::new(move || {
                let n = crate::notes::library::count_by_kind(&ctx.tree.get_untracked()).get(&PageKind::Book).copied().unwrap_or(0);
                if n < want_books {
                    return false;
                }
                if !open_book && lib.screen.get_untracked() != Screen::Library {
                    lib.screen.set(Screen::Library);
                }
                true
            }) as Box<dyn Fn() -> bool>
        }),
        ..Default::default()
    };
    let r = syngui::shot::render_png_with(|| Box::new(super::root()), &mss, args.size, args.scale, &args.out, opts);
    let _ = std::fs::remove_dir_all(&home);
    match r {
        Ok((w, h)) => println!("{}: {w}x{h}", args.out),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
