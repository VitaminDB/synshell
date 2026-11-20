//! syn-systeminfo — монитор ресурсов synshell: обзор, процессор и ядра, графика и
//! видеопамять, нейропроцессор, память, сеть, накопители, датчики, питание, процессы.
//!
//!   syn-systeminfo [СТРАНИЦА]       окно; страница — `cpu`, `gpu`, `vram`, `npu`, `memory`
//!                                   (`ram`), `network`, `disk`, `sensors` (`temps`), `battery`,
//!                                   `processes` (`apps`), `overview`
//!   syn-systeminfo --connections ПРОГРАММА   окно соединений программы за сегодня (`-` — всех);
//!                                   программа — имя исполняемого файла, как в synnetd
//!   syn-systeminfo --list-pages
//!   syn-systeminfo --screenshot out.png [--page id] [--size 420x860] [--scale 2] [--history СЕКУНД]
//!                                   (`--history` — сначала копить замеры столько секунд: графики не пустые)
//!
//! Окно одно: повторный запуск (тап по виджету рабочего стола) передаёт страницу уже
//! открытому окну через сокет `$XDG_RUNTIME_DIR/syn-systeminfo.sock` и выходит.

mod netapps;
mod sampler;
mod state;
mod ui;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use syngui::prelude::*;
use synshell_common::Config;

use state::{Page, St};

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_systeminfo=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));
    let mut page = Page::Overview;
    let mut shot: Option<String> = None;
    let mut size = (420u32, 860u32);
    let mut scale = 2.0f64;
    let mut history = 0u32;
    // Окно соединений: программа (`-` — все).
    let mut conns: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-systeminfo [СТРАНИЦА] [--connections ПРОГРАММА] [--list-pages] [--screenshot СНИМОК.png [--page СТРАНИЦА] [--size ШxВ] [--scale N] [--history СЕКУНД]]"));
                return;
            }
            "--list-pages" => {
                for p in Page::ALL {
                    println!("{}\t{}", p.id(), syngui::i18n::t(p.title()));
                }
                return;
            }
            "--page" => page = args.next().as_deref().and_then(Page::parse).unwrap_or(page),
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--history" => history = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
            "--connections" => {
                conns = args.next();
                page = Page::Network;
            }
            other if !other.starts_with('-') => match Page::parse(other) {
                Some(p) => page = p,
                None => eprintln!("{}", t!("неизвестная страница «{p}» (--list-pages)", p = other)),
            },
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        sampler::set_page(page);
        let snap = if history > 0 { sampler::collect(history) } else { sampler::once() };
        let r = syngui::shot::render_png(
            move || {
                if let Some(app) = &conns {
                    let app = (app != "-").then(|| app.clone());
                    return ui::conns::snapshot(app, ui::conns::Range::Today, true);
                }
                let st = St::new(page);
                st.snap.set_always(snap.clone());
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
    // Окно уже открыто — передать ему страницу.
    let sock = socket_path();
    if let Ok(mut s) = UnixStream::connect(&sock) {
        let line = match &conns {
            Some(app) => format!("conns:{app}"),
            None => page.id().to_string(),
        };
        if writeln!(s, "{line}").is_ok() {
            return;
        }
    }
    let _ = std::fs::remove_file(&sock);
    let listener = std::sync::Mutex::new(UnixListener::bind(&sock).ok());
    let st = St::new(page);
    App::new()
        .title(t!("Монитор ресурсов"))
        .app_id("syn-systeminfo")
        .size(1120, 760)
        .min_size(340, 480)
        .frameless()
        .transparent(true)
        .background(syngui::core::Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss.clone(), theme))
        .with_window_state(st.window)
        .run(move |_| {
            sampler::set_page(page);
            sampler::start(st.snap);
            if let Some(l) = listener.lock().ok().and_then(|mut l| l.take()) {
                listen(l, st);
            }
            if let Some(app) = conns.clone() {
                run_on_main_thread(move || open_conns(&app));
            }
            ui::root(st)
        });
    let _ = std::fs::remove_file(&sock);
}

/// Окно соединений программы (`-` — всех) за сегодня.
fn open_conns(app: &str) {
    let app = (app != "-" && !app.is_empty()).then(|| app.to_string());
    let title = app.as_deref().map(|a| netapps::info(a).name).unwrap_or_else(|| t!("Все программы"));
    ui::conns::open(app, title, ui::conns::Range::Today);
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join("syn-systeminfo.sock")
}

/// Страницы от повторных запусков.
fn listen(l: UnixListener, st: St) {
    std::thread::spawn(move || {
        for conn in l.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(conn).read_line(&mut line).is_ok() {
                if let Some(app) = line.trim().strip_prefix("conns:") {
                    let app = app.to_string();
                    run_on_main_thread(move || {
                        ui::open(st, Page::Network);
                        open_conns(&app);
                    });
                } else if let Some(p) = Page::parse(&line) {
                    run_on_main_thread(move || ui::open(st, p));
                }
            }
        }
    });
}

fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "syn-systeminfo").font_selector("Text, Button").csd().base(include_str!("../styles/syn-systeminfo.mss")).build()
}
