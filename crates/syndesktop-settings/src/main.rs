//! syndesktop-settings — «Параметры системы» рабочего стола syndesktop.
//!
//! Правит `~/.config/syndesktop/config.toml` точечно (toml_edit), композитор
//! и оболочка подхватывают файл сами.
//!
//! Скрытый режим для проверки без окна:
//! `syndesktop-settings --screenshot out.png [--page id] [--size 1200x800] [--scale 1]`.

mod app;
mod pages;
mod shot;
mod state;
mod store;
mod sys;
mod ui;
#[cfg(test)]
mod ui_tests;

use syngui::prelude::*;

struct Args {
    page: String,
    screenshot: Option<String>,
    size: (u32, u32),
    scale: f64,
    config: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args { page: "appearance".into(), screenshot: None, size: (1180, 800), scale: 1.0, config: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--page" => a.page = it.next().unwrap_or_default(),
            "--screenshot" => a.screenshot = it.next(),
            "--config" => a.config = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    a.size = (w.parse().unwrap_or(1180), h.parse().unwrap_or(800));
                }
            }
            "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--list-pages" => {
                for p in pages::PAGES {
                    println!("{}\t{}", p.id, p.title);
                }
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!("syndesktop-settings [--page <id>] [--config <path>] [--list-pages]");
                std::process::exit(0);
            }
            // Страница без ключа: `syndesktop-settings keyboard`.
            other if !other.starts_with('-') => a.page = other.to_string(),
            _ => {}
        }
    }
    a
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let args = parse_args();
    let path = args
        .config
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(syndesktop_common::paths::config_file);
    store::init(&path);

    if let Some(out) = &args.screenshot {
        if let Err(e) = shot::screenshot(out, &args.page, args.size, args.scale) {
            eprintln!("снимок не удался: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let ctx = state::init(&args.page);
    let theme = ctx.theme;
    let initial = theme.get_untracked();

    // Внешняя правка файла (редактор, `syndesktop msg`) — перечитать.
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if store::external_change() {
            run_on_main_thread(|| {
                store::reload();
                state::notify_changed();
                state::bump();
                state::toast("config.toml изменён снаружи — перечитан");
            });
        }
    });

    App::new()
        .title("Параметры системы — syndesktop")
        .app_id("syndesktop-settings")
        .size(args.size.0, args.size.1)
        .min_size(820, 560)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&initial)
        .with_dynamic_theme(theme)
        .run(move |_| {
            provide_context(ctx);
            app::root(ctx)
        });

    // Не потерять правку, сделанную за миг до закрытия окна.
    let _ = store::save_now();
}
