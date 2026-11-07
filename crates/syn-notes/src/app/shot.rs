//! `syn-notes --screenshot out.png [--size 1240x930] [--scale 2] [--eink]
//! [--open notebook|drawing|book|doc|settings|folder|canvas|kanban|gantt|mindmap|calendar]
//! [--narrow]` — снимок окна без
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
    /// Файлы для редактора (`--file`), последний — на экране.
    pub files: Vec<PathBuf>,
    /// Markdown — исходником, а не оформленным видом.
    pub source: bool,
}

pub fn parse(out: String, rest: &[String]) -> ShotArgs {
    let mut a = ShotArgs { out, size: (1240, 930), scale: 1.0, open: None, list: false, books: Vec::new(), files: Vec::new(), source: false };
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
            "--file" => a.files.push(PathBuf::from(it.next().cloned().unwrap_or_default())),
            "--source" => a.source = true,
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

/// Язык снимка — русский или английский (демо-текст на нём же).
fn ru() -> bool {
    syngui::i18n::language().tag().starts_with("ru")
}

/// Демо-проект: папка с блокнотом и книгой, рисунок, документы.
fn populate(ctx: NotesCtx) {
    let ru = ru();
    let l = |r: &'static str, e: &'static str| if ru { r } else { e };
    let tree = ctx.tree.get_untracked();
    let first = tree.roots.first().map(|n| n.id.clone());
    drop(tree);
    if let Some(id) = first {
        ctx.rename_page(&id, l("План недели", "Week plan"));
        ctx.set_page_markdown(
            &id,
            l(
                "# План недели\n\n- Дочитать «Мастер и Маргарита»\n- Конспект лекции по матанализу\n- Эскизы для обложки\n\n> Встреча в четверг в 15:00\n",
                "# Week plan\n\n- Finish reading “The Master and Margarita”\n- Calculus lecture notes\n- Cover sketches\n\n> Meeting on Thursday at 3 pm\n",
            ),
        );
    }
    let study = ctx.insert_page_of_kind(None, None, l("Учёба", "Study"), PageKind::Folder, false);
    ctx.insert_page_of_kind(Some(&study), None, l("Матанализ — лекции", "Calculus lectures"), PageKind::Notebook, false);
    ctx.insert_page_of_kind(Some(&study), None, l("Физика", "Physics"), PageKind::Notebook, false);
    ctx.insert_page_of_kind(None, None, l("Ежедневник", "Diary"), PageKind::Notebook, false);
    ctx.insert_page_of_kind(None, None, l("Эскиз обложки", "Cover sketch"), PageKind::Drawing, false);
    let doc = ctx.insert_page(None, None, l("Идеи", "Ideas"), false);
    ctx.set_page_markdown(
        &doc,
        l(
            "# Идеи\n\nРукописный ввод с распознаванием, шаблоны страниц, синхронизация.\n",
            "# Ideas\n\nHandwriting recognition, page templates, sync.\n",
        ),
    );
}

/// Вызов инструмента `notes` (тот же, что у ассистента); ошибка — в stderr.
fn call(action: &str, mut v: serde_json::Value) -> String {
    v["action"] = serde_json::Value::String(action.to_string());
    match crate::tool::dispatch_now(&v) {
        Ok(out) => out,
        Err(e) => {
            eprintln!("демо {action}: {e}");
            String::new()
        }
    }
}

/// Id страницы из ответа `create` (`page: <12 hex> …`).
fn page_of(out: &str) -> Option<String> {
    out.lines().find_map(|l| l.strip_prefix("page: ")).map(|r| r.chars().take(12).collect())
}

/// Демо-страница с объектом (`canvas`, `kanban`, `gantt`, `mindmap`,
/// `calendar`): придуманное содержимое, создаётся инструментом `notes`.
/// Возвращает id страницы.
fn demo_page(what: &str) -> Option<String> {
    use serde_json::json;
    let ru = ru();
    let l = |r: &'static str, e: &'static str| if ru { r } else { e };
    let day = |off: i64| crate::notes::gantt::calendar::days_to_iso(crate::notes::gantt::calendar::today_days() + off);
    match what {
        "kanban" => {
            let page = page_of(&call("create", json!({"title": l("Запуск сайта", "Website launch"), "icon": "🚀", "props_panel": "hidden",
                "content": l("# Запуск сайта\n\nДоска задач к релизу. Карточки со сроками попадают в календарь.\n",
                    "# Website launch\n\nRelease task board. Cards with due dates show up in the calendar.\n")})))?;
            call("kanban", json!({"op": "create", "page": page,
                "columns": [l("Идеи", "Ideas"), l("К выполнению", "To do"), l("В работе", "In progress"), l("Готово", "Done")],
                "done_column": l("Готово", "Done")}));
            call("kanban", json!({"op": "set_style", "page": page, "column_width": 236}));
            let cards: [(&str, &str, &str, &str, &[&str], Option<i64>); 9] = [
                (l("Идеи", "Ideas"), l("Тёмная тема для блога", "Dark theme for the blog"), "low", "", &[], None),
                (l("Идеи", "Ideas"), l("Рассылка для подписчиков", "Newsletter for subscribers"), "", "", &[], None),
                (l("К выполнению", "To do"), l("Тексты для главной", "Home page copy"), "high", "", &[l("тексты", "content")], Some(3)),
                (l("К выполнению", "To do"), l("Картинки для соцсетей", "Social media images"), "medium", "", &[l("дизайн", "design")], Some(5)),
                (l("К выполнению", "To do"), l("Проверить формы на телефоне", "Test forms on a phone"), "medium", "", &[l("проверка", "qa")], None),
                (l("В работе", "In progress"), l("Вёрстка страницы цен", "Pricing page layout"), "high", "", &[l("сайт", "web")], Some(1)),
                (l("В работе", "In progress"), l("Перевод на немецкий", "German translation"), "", "", &[l("тексты", "content")], Some(2)),
                (l("Готово", "Done"), l("Домен и почта", "Domain and mail"), "", "", &[], None),
                (l("Готово", "Done"), l("Логотип", "Logo"), "", "", &[l("дизайн", "design")], None),
            ];
            for (col, title, prio, _md, tags, due) in cards {
                let mut v = json!({"op": "add_card", "page": page, "column": col, "title": title});
                if !prio.is_empty() {
                    v["priority"] = json!(prio);
                }
                if !tags.is_empty() {
                    v["tags"] = json!(tags);
                }
                if let Some(d) = due {
                    v["due"] = json!(day(d));
                }
                call("kanban", v);
            }
            Some(page)
        }
        "gantt" => {
            let page = page_of(&call("create", json!({"title": l("Ремонт кухни", "Kitchen renovation"), "icon": "🛠️", "props_panel": "hidden",
                "content": l("# Ремонт кухни\n\nПлан работ по дням. Стрелки — зависимости.\n",
                    "# Kitchen renovation\n\nDay-by-day plan. Arrows are dependencies.\n")})))?;
            call("gantt", json!({"op": "create", "page": page}));
            let tasks: [(&str, i64, i64, Option<&str>, &str); 7] = [
                (l("Замеры", "Measuring"), -6, -4, None, "blue"),
                (l("Заказ мебели", "Order cabinets"), -3, 1, Some(l("Замеры", "Measuring")), "purple"),
                (l("Демонтаж", "Demolition"), -3, -1, Some(l("Замеры", "Measuring")), "red"),
                (l("Электрика", "Wiring"), 0, 3, Some(l("Демонтаж", "Demolition")), "orange"),
                (l("Плитка", "Tiling"), 4, 8, Some(l("Электрика", "Wiring")), "teal"),
                (l("Сборка мебели", "Assembly"), 9, 11, Some(l("Плитка", "Tiling")), "green"),
                (l("Техника", "Appliances"), 12, 13, Some(l("Сборка мебели", "Assembly")), "yellow"),
            ];
            for (name, s, e, after, color) in tasks {
                let mut v = json!({"op": "add_task", "page": page, "name": name, "start": day(s), "end": day(e), "color": color});
                if let Some(a) = after {
                    v["after"] = json!(a);
                }
                call("gantt", v);
            }
            call("gantt", json!({"op": "set_zoom", "page": page, "zoom": 30}));
            call("gantt", json!({"op": "show_today", "page": page}));
            Some(page)
        }
        "mindmap" => {
            let page = page_of(&call("create", json!({"title": l("Отпуск", "Vacation"), "icon": "🏖️", "props_panel": "hidden"})))?;
            let outline = l(
                "# Отпуск\n\n- Куда\n  - Горы\n  - Море\n  - Старый город\n- Когда\n  - Июль\n  - Две недели\n- Бюджет\n  - Билеты\n  - Жильё\n  - Еда\n- Взять с собой\n  - Документы\n  - Аптечка\n  - Фотоаппарат\n",
                "# Vacation\n\n- Where\n  - Mountains\n  - Seaside\n  - Old town\n- When\n  - July\n  - Two weeks\n- Budget\n  - Tickets\n  - Lodging\n  - Food\n- Packing\n  - Documents\n  - First aid kit\n  - Camera\n",
            );
            call("mindmap", json!({"op": "create", "page": page, "title": l("Отпуск", "Vacation"), "outline": outline, "direction": "both", "h": 560}));
            call("mindmap", json!({"op": "set_style", "page": page, "style": {"palette": "rainbow"}}));
            Some(page)
        }
        "calendar" => {
            let page = page_of(&call("create", json!({"title": l("Календарь", "Calendar"), "icon": "📅", "props_panel": "hidden"})))?;
            call("calendar", json!({"op": "create", "page": page, "view": "week", "h": 660}));
            // По дням текущей недели (0 — понедельник).
            let monday = -crate::notes::gantt::calendar::weekday_of(crate::notes::gantt::calendar::today_days());
            let events: [(&str, i64, &str, &str); 10] = [
                (l("Планёрка", "Team stand-up"), 0, "10:00", "11:00"),
                (l("Созвон с заказчиком", "Client call"), 0, "15:00", "16:00"),
                (l("Стоматолог", "Dentist"), 1, "09:00", "10:00"),
                (l("Планёрка", "Team stand-up"), 2, "10:00", "11:00"),
                (l("Спортзал", "Gym"), 2, "17:30", "19:00"),
                (l("Обед с Аней", "Lunch with Anna"), 3, "13:00", "14:00"),
                (l("Курс по рисованию", "Drawing class"), 3, "16:00", "18:00"),
                (l("Сдать отчёт", "Send the report"), 4, "11:00", "12:00"),
                (l("Прогулка в парке", "Walk in the park"), 5, "10:00", "13:00"),
                (l("Кино", "Cinema"), 6, "17:00", "19:00"),
            ];
            for (title, wd, s, e) in events {
                call("calendar", json!({"op": "add_event", "title": title, "date": day(monday + wd), "start_time": s, "end_time": e}));
            }
            Some(page)
        }
        "canvas" => {
            let page = page_of(&call("create", json!({"title": l("Схема приложения", "App flow"), "icon": "🧭", "props_panel": "hidden", "layout": "free", "grid": "dots",
                "content": l("# Схема приложения\n\nКак пользователь проходит регистрацию.\n",
                    "# App flow\n\nHow a new user signs up.\n")})))?;
            call("blocks", json!({"op": "arrange", "page": page, "only": "all", "x": 60, "y": 40, "w": 520}));
            let (w, h) = (170.0, 80.0);
            let boxes: [(&str, f64, f64, &str); 5] = [
                (l("Главная", "Home"), 40.0, 200.0, "#3b82f6"),
                (l("Регистрация", "Sign up"), 270.0, 200.0, "#8b5cf6"),
                (l("Письмо с кодом", "Email code"), 500.0, 110.0, "#f59e0b"),
                (l("Вход по телефону", "Phone login"), 500.0, 290.0, "#f59e0b"),
                (l("Личный кабинет", "Dashboard"), 730.0, 200.0, "#10b981"),
            ];
            for (_, x, y, fill) in boxes {
                call("shape", json!({"op": "create", "page": page, "kind": "rect", "x": x, "y": y, "w": w, "h": h, "fill": fill}));
            }
            // Стрелки: от правой кромки к левой следующего.
            let mid = |i: usize, right: bool| (boxes[i].1 + if right { w } else { 0.0 }, boxes[i].2 + h / 2.0);
            for (from, to) in [(0, 1), (1, 2), (1, 3), (2, 4), (3, 4)] {
                let ((x1, y1), (x2, y2)) = (mid(from, true), mid(to, false));
                call("shape", json!({"op": "create", "page": page, "kind": "arrow", "x1": x1, "y1": y1, "x2": x2, "y2": y2}));
            }
            // Подписи — текстовые блоки поверх фигур.
            for (text, x, y, _) in boxes {
                call("blocks", json!({"op": "insert", "page": page, "md": format!("**{text}**"), "x": x + 12.0, "y": y + 26.0, "w": w - 24.0}));
            }
            let note = l("Код действует 10 минут; три ошибки подряд — пауза 1 минута.", "The code is valid for 10 minutes; three wrong tries pause for a minute.");
            call("blocks", json!({"op": "insert", "page": page, "md": format!("> {note}"), "x": 270, "y": 420, "w": 430}));
            Some(page)
        }
        _ => None,
    }
}

pub fn run(args: ShotArgs) {
    let home = sandbox();
    synshell_common::i18n::init(&crate::i18n::CATALOGS);
    let (cfg, _) = Config::load();
    super::install_state();
    let ctx = use_context::<NotesCtx>();
    let path = home.join(if ru() { "Documents/Демо.syn" } else { "Documents/Demo.syn" });
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
    } else if args.open.as_deref() == Some("settings") {
        use_context::<Library>().screen.set(Screen::Settings);
    } else if args.open.as_deref() == Some("folder") {
        let id = ctx.tree.get_untracked().roots.iter().find(|n| n.kind == PageKind::Folder).map(|n| n.id.clone());
        use_context::<Library>().folder.set(id);
    }
    if let Some(what) = args.open.as_deref().filter(|w| matches!(*w, "canvas" | "kanban" | "gantt" | "mindmap" | "calendar")) {
        match demo_page(what) {
            Some(id) => use_context::<Library>().open(ctx, &id),
            None => eprintln!("демо {what}: страница не создана"),
        }
    }
    let _ = Screen::Library;
    for f in &args.files {
        // Путь — до подмены HOME: относительный от каталога запуска.
        if let Err(e) = crate::notes::files::Files::get().open_path(f) {
            eprintln!("{}: {e}", f.display());
        }
    }
    if args.source {
        if let Some(b) = crate::notes::files::Files::get().current_buf() {
            b.set_visual(false);
        }
    }
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
