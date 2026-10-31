//! Читалка отдельно от проекта — ручная проверка и снимки:
//!
//! ```text
//! cargo run -p syn-notes --example reader_demo --profile fast-release -- КНИГА.epub
//! cargo run -p syn-notes --example reader_demo -- КНИГА.fb2 --screenshot out.png [--size 620x900] [--panel menu|toc|text|search|bookmarks] [--next N] [--select]
//! ```
//! Состояние (позиция, закладки, настройки) — рядом с книгой в
//! `<книга>.reader.json`.

use std::sync::Arc;

use syn_notes::notes::reader::view::{reader_view, Env, Panel, Reader, Sel};
use syn_notes::notes::reader::{Book, BookState, READER_MSS};
use syngui::prelude::*;

fn main() {
    let mut file = None;
    let mut shot = None;
    let mut size = (620u32, 900u32);
    let mut panel = None;
    let mut next = 0usize;
    let mut select = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screenshot" => shot = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x').map(|(w, h)| (w.parse().unwrap_or(620), h.parse().unwrap_or(900)))) {
                    size = (w, h);
                }
            }
            "--panel" => panel = it.next(),
            "--next" => next = it.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            "--select" => select = true,
            _ => file = Some(a),
        }
    }
    let Some(file) = file else {
        eprintln!("reader_demo КНИГА [--screenshot out.png] [--size WxH] [--panel menu|toc|text|search|bookmarks] [--next N] [--select]");
        std::process::exit(2);
    };
    synshell_common::i18n::init(&syn_notes::i18n::CATALOGS);
    let (cfg, _) = synshell_common::Config::load();
    let mss = format!("{}\n{}", syn_notes::app::theme::mss(&cfg), READER_MSS);
    let state_path = format!("{file}.reader.json");
    let book = match Book::open_path(std::path::Path::new(&file)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let state = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|s| BookState::parse(&s).ok())
        .filter(|_| shot.is_none())
        .unwrap_or_else(|| book.new_state(&file));
    let sp = state_path.clone();
    let save = shot.is_none();
    let env = Env {
        key: "demo".into(),
        on_state: Arc::new(move |s| {
            if save {
                let _ = std::fs::write(&sp, s.serialize());
            }
        }),
        on_close: Some(Arc::new(|| std::process::exit(0))),
    };
    if let Some(out) = shot {
        syngui::signal::init_main_thread();
        syngui::signal::allow_signal_reads_on_this_thread();
        let r = Reader::new(env, state, Some(Arc::new(book)), None);
        let r2 = r.clone();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d2 = done.clone();
        let opts = syngui::shot::ShotOptions {
            ready: Some(Box::new(move || {
                // Разметка готова — пролистать, открыть панель, выделить.
                if r2.ctl.size.get_untracked().0 > 0.0 && !d2.load(std::sync::atomic::Ordering::Relaxed) && !r2.frames().is_empty() {
                    d2.store(true, std::sync::atomic::Ordering::Relaxed);
                    for _ in 0..next {
                        r2.next();
                    }
                    if select {
                        if let Some(syn_notes::notes::reader::view::FrameKind::Flow(p)) = r2.frames().first().map(|f| f.kind.clone()) {
                            if let Some(run) = p.runs().filter(|r| r.ch1 > r.ch0).nth(3) {
                                let a = syn_notes::notes::reader::model::Pos { block: run.block, ch: run.ch0 };
                                let b = syn_notes::notes::reader::model::Pos { block: run.block, ch: run.ch0 + 30 };
                                r2.ctl.selection.set(Some(Sel::Flow { a, b }));
                                r2.highlight_selection("демо");
                                r2.ctl.selection.set(Some(Sel::Flow { a: b, b: syn_notes::notes::reader::model::Pos { block: b.block, ch: b.ch + 12 } }));
                            }
                        }
                    }
                    let p = match panel.as_deref() {
                        Some("menu") => Panel::Menu,
                        Some("toc") => Panel::Toc,
                        Some("text") => Panel::Text,
                        Some("search") => {
                            r2.search("и");
                            Panel::Search
                        }
                        Some("bookmarks") => {
                            r2.toggle_bookmark();
                            Panel::Bookmarks
                        }
                        _ => Panel::None,
                    };
                    r2.ctl.panel.set(p);
                    return false;
                }
                done.load(std::sync::atomic::Ordering::Relaxed)
            })),
            settle: std::time::Duration::from_millis(1500),
            timeout: std::time::Duration::from_secs(20),
            ..Default::default()
        };
        match syngui::shot::render_png_with(move || Box::new(reader_view(r)), &mss, size, 1.0, &out, opts) {
            Ok((w, h)) => println!("{out}: {w}×{h}"),
            Err(e) => {
                eprintln!("снимок: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let theme = use_signal(mss);
    let r = Reader::new(env, state, Some(Arc::new(book)), None);
    App::new()
        .title("Читалка")
        .app_id("syn-notes-reader-demo")
        .size(size.0, size.1)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(theme)
        .run(move |_| Box::new(reader_view(r.clone())));
}
