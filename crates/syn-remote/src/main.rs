//! syn-remote — «Пульт» synshell: управление бытовой техникой через ИК-передатчик телефона.
//! База кодов — Flipper-IRDB (скачивается в программе), автоподбор пульта по марке или без неё.
//!
//!   syn-remote [--screenshot out.png [--size 420x860] [--scale 2] [--page home|add|db|remote|pick]]
//!   syn-remote --send ПУЛЬТ КНОПКА        ПУЛЬТ — имя из «Моих пультов» или файл .ir
//!   syn-remote --timings ФАЙЛ.ir КНОПКА   длительности посылки (мкс), без передачи
//!   syn-remote --download                  скачать базу кодов

mod db;
mod icons;
mod ir;
mod keys;
mod state;
mod store;
mod tx;
mod ui;

use syngui::prelude::*;
use synshell_common::config::Rgba;
use synshell_common::Config;

pub fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let mut s = a.mss_variables();
    let titlebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.22) } else { p.surface_alt.mix(p.bg, 0.5) };
    let sidebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.12) } else { p.surface_alt.mix(p.bg, 0.3) };
    let content = if dark { p.bg.mix(p.surface, 0.12) } else { p.bg.mix(p.surface, 0.4) };
    let body = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.35) } else { p.surface };
    let key = if dark { p.surface.mix(Rgba::rgb(255, 255, 255), 0.06) } else { p.surface_alt.mix(p.bg, 0.2) };
    let key_hi = if dark { p.surface.mix(Rgba::rgb(255, 255, 255), 0.14) } else { p.surface_alt.mix(Rgba::rgb(0, 0, 0), 0.06) };
    let accent2 = p.accent.mix(Rgba::rgb(0x8b, 0x5c, 0xf6), 0.55);
    s.push_str(&format!(
        ":root {{\n  --titlebar: {};\n  --sidebar: {};\n  --content: {};\n  --remote-body: {};\n  --key: {};\n  --key-hi: {};\n  --accent-2: {};\n  --accent-glow: {};\n  --accent-dim: {};\n  --shadow: {};\n  --scrim: {};\n  --window-radius: {}px;\n}}\n",
        titlebar.hex(),
        sidebar.hex(),
        content.hex(),
        body.hex(),
        key.hex(),
        key_hi.hex(),
        accent2.hex(),
        p.accent.with_alpha(0.45).hex(),
        p.accent.with_alpha(0.14).hex(),
        if dark { "#00000080" } else { "#1a203024" },
        p.bg.with_alpha(0.86).hex(),
        cfg.decorations.corner_radius.max(0.0),
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-remote.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

/// Тема меняется вместе с настройками оболочки.
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
                    let mss = theme(&cfg);
                    run_on_main_thread(move || st.theme.set(mss));
                }
            }
        }
    });
}

/// Кнопки пульта: из «Моих пультов» по имени (без учёта регистра) или из файла .ir.
fn cli_signals(remote: &str) -> Vec<ir::Signal> {
    if let Some(r) = store::load().into_iter().find(|r| r.name.eq_ignore_ascii_case(remote) || r.id == remote) {
        return r.signals;
    }
    std::fs::read_to_string(remote).map(|s| ir::parse(&s)).unwrap_or_default()
}

fn cli_find(remote: &str, button: &str) -> ir::Signal {
    let signals = cli_signals(remote);
    if signals.is_empty() {
        eprintln!("{}", t!("Пульт «{r}» не найден", r = remote));
        std::process::exit(1);
    }
    let found = signals.iter().find(|s| s.name.eq_ignore_ascii_case(button)).or_else(|| keys::key_of(button).and_then(|k| keys::find(&signals, k)));
    match found {
        Some(s) => s.clone(),
        None => {
            eprintln!("{}", t!("Кнопки «{b}» нет; есть: {list}", b = button, list = signals.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")));
            std::process::exit(1);
        }
    }
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_remote=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (420u32, 860u32);
    let mut scale = 2.0f64;
    let mut page = String::from("home");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-remote [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page home|add|db|remote|pick]]\nsyn-remote --send ПУЛЬТ КНОПКА | --timings ФАЙЛ.ir КНОПКА | --download"));
                return;
            }
            "--send" | "--timings" => {
                let (Some(remote), Some(button)) = (args.next(), args.next()) else {
                    eprintln!("{}", t!("Нужны пульт и кнопка"));
                    std::process::exit(2);
                };
                let sig = cli_find(&remote, &button);
                let Some(frame) = ir::encode(&sig.code) else {
                    eprintln!("{}", t!("Протокол «{p}» не поддерживается", p = ir::describe(&sig.code)));
                    std::process::exit(1);
                };
                if a == "--timings" {
                    println!("{} {} {}", sig.name, ir::describe(&sig.code), frame.freq);
                    println!("{}", frame.timings.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(" "));
                    return;
                }
                match tx::transmit(&frame) {
                    Ok(()) => println!("{} → {}", sig.name, ir::describe(&sig.code)),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            "--download" => {
                let r = db::download(|p| eprintln!("{p:?}"));
                match r {
                    Ok(n) => println!("{}", t!("База кодов загружена: {n} пультов", n = n)),
                    Err(e) => {
                        eprintln!("{e:#}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = args.next().unwrap_or(page),
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(
            || {
                let st = state::St::new();
                st.db.set(db::load().map(std::sync::Arc::new));
                screenshot_page(st, &page);
                ui::root(st)
            },
            &mss,
            size,
            scale,
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
    let st = state::St::new();
    st.theme.set(mss.clone());
    watch_config(st);
    App::new()
        .title(t!("Пульт"))
        .app_id("syn-remote")
        .size(440, 860)
        .min_size(340, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .with_dynamic_theme(st.theme)
        .with_window_state(st.window)
        .run(move |_| {
            st.start();
            ui::root(st)
        });
}

/// Экран для снимка (`--page`): проверка вёрстки без телефона.
fn screenshot_page(st: state::St, page: &str) {
    match page {
        "add" => st.page.set(state::Page::Categories),
        "db" => st.page.set(state::Page::Database),
        "brands" => st.page.set(state::Page::Brands("TVs".into())),
        "remote" => {
            if let Some(r) = st.remotes.get_untracked().first() {
                st.page.set(state::Page::Remote(r.id.clone()));
            } else if let Some(db) = st.db.get_untracked() {
                if let Some(m) = db.category("TVs").and_then(|c| c.brand("LG")).and_then(|b| b.models.iter().find(|m| m.name.contains("AKB75675311"))) {
                    st.choose_model("TVs", "LG", m);
                    st.save_found_preview();
                }
            }
        }
        "pick" => {
            if let Some(db) = st.db.get_untracked() {
                let cands = db::candidates(&db, "TVs", Some("LG"));
                st.pick.set(Some(state::Pick { cat: "TVs".into(), brand: Some("LG".into()), cands: std::sync::Arc::new(cands), idx: 0, test: "power", sent: true, auto: false, loading: false }));
                st.page.set(state::Page::Pick);
            }
        }
        "found" => {
            if let Some(db) = st.db.get_untracked() {
                if let Some(m) = db.category("TVs").and_then(|c| c.brand("LG")).and_then(|b| b.models.first()) {
                    st.choose_model("TVs", "LG", m);
                }
            }
        }
        _ => {}
    }
}
