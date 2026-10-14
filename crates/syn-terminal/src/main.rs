//! syn-terminal — терминал synshell: вкладки в заголовке своего окна, схемы
//! цветов, шрифт и история; на телефоне — вкладки под рамкой композитора и
//! ряд клавиш, которых нет на экранной клавиатуре.
//!
//!   syn-terminal [--working-directory ПАПКА] [-e КОМАНДА [АРГУМЕНТЫ…]]
//!
//! `-e` (и `-x`, `--`) — как у других терминалов: всё после него — команда;
//! одна строка с пробелами запускается через `sh -c`. Проверка без окна:
//! `syn-terminal --screenshot out.png [--size 1100x700] [--scale 1]`.

mod schemes;
mod settings;
mod settings_ui;
mod state;
mod ui;

use std::path::PathBuf;

use syngui::prelude::*;
use synshell_common::Config;

struct Args {
    cmd: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    screenshot: Option<String>,
    size: (u32, u32),
    scale: f64,
    settings: bool,
}

fn parse_args() -> Args {
    let mut a = Args { cmd: None, cwd: None, screenshot: None, size: (1100, 700), scale: 1.0, settings: false };
    let mut it = std::env::args().skip(1);
    while let Some(s) = it.next() {
        match s.as_str() {
            "-e" | "-x" | "--" | "--command" => {
                let rest: Vec<String> = it.by_ref().collect();
                a.cmd = Some(if rest.len() == 1 && rest[0].contains(char::is_whitespace) {
                    vec!["/bin/sh".into(), "-c".into(), rest[0].clone()]
                } else {
                    rest
                });
                break;
            }
            "--working-directory" | "--workdir" | "-d" | "-w" => a.cwd = it.next().map(PathBuf::from),
            "--screenshot" => a.screenshot = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    a.size = (w.parse().unwrap_or(a.size.0), h.parse().unwrap_or(a.size.1));
                }
            }
            "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--shot-settings" => a.settings = true,
            "-h" | "--help" => {
                println!("{}", t!("syn-terminal [--working-directory ПАПКА] [-e КОМАНДА [АРГУМЕНТЫ…]]"));
                std::process::exit(0)
            }
            // совместимость: --title, --hold, --new-tab и т.п. — без значения или с ним
            "--title" | "-T" | "--class" | "--app-id" => {
                it.next();
            }
            s if s.starts_with("--working-directory=") => a.cwd = Some(PathBuf::from(&s["--working-directory=".len()..])),
            s if s.starts_with('-') => {}
            // позиционный аргумент — команда (как у foot)
            _ => {
                let mut v = vec![s];
                v.extend(it.by_ref());
                a.cmd = Some(v);
                break;
            }
        }
    }
    a
}

/// Перечитывать config.toml и файлы темы при изменении.
fn watch_config(st: state::St) {
    std::thread::spawn(move || {
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let files = || {
            let mut v = vec![synshell_common::paths::config_file(), synshell_common::paths::user_theme_file()];
            v.extend(Config::load().0.appearance.theme_files());
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
                let (cfg, err) = Config::load();
                if err.is_none() {
                    state::set_config(cfg);
                    run_on_main_thread(move || st.retheme());
                }
            }
        }
    });
}

/// Папки и занятость вкладок — для подписей.
fn poll_tabs(st: state::St) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        run_on_main_thread(move || st.poll());
    });
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_terminal=info,syngui=warn".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let args = parse_args();
    let (cfg, err) = Config::load();
    if let Some(e) = err {
        tracing::warn!("config.toml: {e}");
    }
    state::set_config(cfg);
    let cwd = args.cwd.clone().or_else(|| std::env::current_dir().ok());

    if let Some(out) = args.screenshot.clone() {
        let mss = state::theme(&Config::load().0, &settings::Settings::load());
        let (cmd, open_settings) = (args.cmd.clone(), args.settings);
        let r = syngui::shot::render_png(
            move || {
                let st = state::St::new(settings::Settings::load());
                st.new_tab(cmd.clone(), cwd.clone());
                st.new_tab(None, Some(synshell_common::paths::home()));
                st.settings_open.set(open_settings);
                ui::root(st)
            },
            &mss,
            args.size,
            args.scale,
            &out,
        );
        match r {
            Ok((w, h)) => println!("{out}: {w}x{h}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let st = state::St::new(settings::Settings::load());
    let mss = st.theme.get_untracked();
    App::new()
        .title(t!("Терминал"))
        .app_id("syn-terminal")
        .size(args.size.0, args.size.1)
        .min_size(320, 240)
        .frameless()
        // Углы за скруглением рамки (`.window-frame`) — прозрачные; фон
        // терминала может быть полупрозрачным.
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .with_dynamic_theme(st.theme)
        .with_window_state(st.window)
        .on_close_request(move || st.request_quit())
        .run(move |_| {
            watch_config(st);
            poll_tabs(st);
            st.new_tab(args.cmd.clone(), cwd.clone());
            ui::root(st)
        });
}
