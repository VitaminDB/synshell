//! synsettings — «Параметры системы» рабочего стола synshell.
//!
//! Правит `~/.config/synshell/config.toml` точечно (toml_edit), композитор
//! и оболочка подхватывают файл сами.
//!
//! Скрытый режим для проверки без окна:
//! `synsettings --screenshot out.png [--page id] [--size 1200x800] [--scale 1]`.

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
            "--list-themes" => {
                for t in synshell_common::theme::list() {
                    let src = match &t.source {
                        synshell_common::theme::Source::Builtin => t!("встроенная").to_string(),
                        synshell_common::theme::Source::Dir(d) => d.display().to_string(),
                    };
                    println!("{}\t{}\t{}", t.id, t.name, src);
                }
                std::process::exit(0);
            }
            "--list-pages" => {
                for p in pages::PAGES {
                    println!("{}\t{}", p.id, crate::ui::tl(p.title));
                }
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!("synsettings [--page <id>] [--config <path>] [--list-pages] [--list-themes]");
                std::process::exit(0);
            }
            // Страница без ключа: `synsettings keyboard`.
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
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let args = parse_args();
    let path = args
        .config
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(synshell_common::paths::config_file);
    store::init(&path);

    if let Some(out) = &args.screenshot {
        if let Err(e) = shot::screenshot(out, &args.page, args.size, args.scale) {
            eprintln!("{}", t!("снимок не удался: {e}", e = format!("{:#}", e)));
            std::process::exit(1);
        }
        return;
    }

    let ctx = state::init(&args.page);
    // Страница из командной строки (`synsettings wifi`, кнопки оболочки) —
    // в узком окне открыть сразу её, а не список разделов.
    if args.page != "appearance" {
        ctx.page_open.set(true);
    }
    let theme = ctx.theme;
    let initial = theme.get_untracked();

    // Внешняя правка файла (редактор, `synwm msg`) — перечитать.
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if store::external_change() {
            run_on_main_thread(|| {
                store::reload();
                synshell_common::i18n::apply(&store::config());
                state::notify_changed();
                state::bump();
                state::toast(t!("config.toml изменён снаружи — перечитан"));
            });
        }
    });

    // Виброотклик касаний — по текущим (в том числе только что изменённым) настройкам.
    syngui::input::set_haptic_handler(|h| {
        synshell_common::haptics::set_config(&store::config().haptics);
        synshell_common::haptics::play(match h {
            syngui::input::Haptic::LongPress => synshell_common::haptics::Feedback::LongPress,
            _ => synshell_common::haptics::Feedback::Tick,
        });
    });
    App::new()
        .title(t!("Параметры системы — synshell"))
        .app_id("synsettings")
        .size(args.size.0, args.size.1)
        .min_size(340, 480)
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
