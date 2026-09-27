//! syndesktop-files — проводник рабочего стола syndesktop.
//!
//! `syndesktop-files [ПУТЬ|URI…]` — открыть папки во вкладках (путь к файлу —
//! его папка с выделенным файлом). Без аргументов — вкладки прошлого сеанса
//! или домашняя папка.
//!
//! Проверка без окна: `syndesktop-files --screenshot out.png [--size 1280x800]
//! [--scale 1] [--script 'key:ctrl+a;click:300,200'] [ПУТЬ…]`.

mod actions;
mod loc;
mod model;
mod ops;
mod places;
mod search;
mod shot;
mod state;
mod thumbs;
mod trash;
mod ui;

use std::path::PathBuf;

use syngui::prelude::*;

use crate::loc::Location;

struct Args {
    locations: Vec<Location>,
    select: Vec<PathBuf>,
    screenshot: Option<String>,
    size: (u32, u32),
    scale: f64,
    script: Vec<String>,
    view: Option<String>,
    split: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        locations: Vec::new(),
        select: Vec::new(),
        screenshot: None,
        size: (1280, 800),
        scale: 1.0,
        script: Vec::new(),
        view: None,
        split: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--screenshot" => a.screenshot = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    a.size = (w.parse().unwrap_or(1280), h.parse().unwrap_or(800));
                }
            }
            "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--script" => a.script.extend(it.next().unwrap_or_default().split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty())),
            "--view" => a.view = it.next(),
            "--split" => a.split = true,
            "-h" | "--help" => {
                println!("syndesktop-files [--view details|list|tiles|icons] [--split] [ПУТЬ|URI…]");
                std::process::exit(0);
            }
            // Совместимость с вызовами «как у Dolphin/Nautilus».
            "--new-window" | "-w" | "--select" | "--" => {}
            other if other.starts_with("--") => {}
            other => match Location::parse(other) {
                Some(Location::Dir(p)) if p.is_file() || (!p.exists() && p.parent().map(|d| d.is_dir()).unwrap_or(false)) => {
                    if let Some(parent) = p.parent() {
                        a.locations.push(Location::Dir(parent.to_path_buf()));
                    }
                    a.select.push(p);
                }
                Some(l) => a.locations.push(l),
                None => {}
            },
        }
    }
    a
}

fn load_config() -> syndesktop_common::Config {
    let (cfg, err) = syndesktop_common::Config::load();
    if let Some(e) = err {
        eprintln!("config.toml: {e}");
    }
    cfg
}

/// Создать состояние: вкладки из аргументов или прошлого сеанса.
fn make_state(args: &Args, cfg: syndesktop_common::Config) -> state::Ctx {
    let (locs, cur) = if !args.locations.is_empty() {
        (args.locations.clone(), 0)
    } else if cfg.files.restore_tabs && args.screenshot.is_none() {
        state::load_session().unwrap_or((Vec::new(), 0))
    } else {
        (Vec::new(), 0)
    };
    let ctx = state::init(cfg, locs);
    ctx.cur.set(cur.min(ctx.tabs.get_untracked().len().saturating_sub(1)));
    if let Some(v) = &args.view {
        for t in ctx.tabs.get_untracked() {
            for p in t.panes {
                p.view.set(state::ViewMode::parse(v));
            }
        }
    }
    if args.split {
        state::toggle_split();
    }
    for s in &args.select {
        state::select_later(state::pane(), s.clone());
    }
    ctx
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    let args = parse_args();
    let cfg = load_config();
    syndesktop_common::xdg::set_icon_theme(&cfg.appearance.icon_theme);
    state::connect_background();

    if let Some(out) = args.screenshot.clone() {
        let (size, scale, script) = (args.size, args.scale, args.script.clone());
        if let Err(e) = shot::screenshot(&out, size, scale, move || make_state(&args, cfg), &script) {
            eprintln!("снимок не удался: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    // Прогреть базу типов и значков, пока строится окно.
    std::thread::spawn(|| {
        let _ = syndesktop_common::mime::guess_by_name("a.txt");
        let _ = syndesktop_common::xdg::apps();
    });

    let ctx = make_state(&args, cfg);
    let theme = ctx.theme;
    let initial = theme.get_untracked();
    watch_config();

    App::new()
        .title("Проводник")
        .app_id("syndesktop-files")
        .size(args.size.0, args.size.1)
        .min_size(640, 420)
        .frameless()
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&initial)
        .with_dynamic_theme(theme)
        .with_window_state(ctx.window)
        .on_close_request(|| {
            state::save_session();
            true
        })
        .run(move |_| {
            provide_context(ctx);
            ui::app::root()
        });
    state::save_session();
}

/// Перечитывать config.toml и файлы темы при изменении.
fn watch_config() {
    std::thread::spawn(|| {
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let files = || {
            let mut v = vec![syndesktop_common::paths::config_file(), syndesktop_common::paths::user_theme_file()];
            v.extend(syndesktop_common::Config::load().0.appearance.theme_files());
            v
        };
        let mut watched = files();
        let mut stamps: Vec<_> = watched.iter().map(|p| mtime(p)).collect();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let now: Vec<_> = watched.iter().map(|p| mtime(p)).collect();
            if now != stamps {
                watched = files();
                stamps = watched.iter().map(|p| mtime(p)).collect();
                let (cfg, err) = syndesktop_common::Config::load();
                if err.is_none() {
                    run_on_main_thread(move || state::config_changed(cfg));
                }
            }
        }
    });
}
