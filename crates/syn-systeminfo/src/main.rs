//! syn-systeminfo — монитор ресурсов synshell: обзор, процессор и ядра, графика и
//! видеопамять, нейропроцессор, память, сеть, накопители, датчики, питание, процессы.
//!
//!   syn-systeminfo [СТРАНИЦА]       окно; страница — `cpu`, `gpu`, `vram`, `npu`, `memory`
//!                                   (`ram`), `network`, `disk`, `sensors` (`temps`), `battery`,
//!                                   `processes` (`apps`), `overview`
//!   syn-systeminfo --list-pages
//!   syn-systeminfo --screenshot out.png [--page id] [--size 420x860] [--scale 2]
//!
//! Окно одно: повторный запуск (тап по виджету рабочего стола) передаёт страницу уже
//! открытому окну через сокет `$XDG_RUNTIME_DIR/syn-systeminfo.sock` и выходит.

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
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut page = Page::Overview;
    let mut shot: Option<String> = None;
    let mut size = (420u32, 860u32);
    let mut scale = 2.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-systeminfo [СТРАНИЦА] [--list-pages] [--screenshot СНИМОК.png [--page СТРАНИЦА] [--size ШxВ] [--scale N]]"));
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
        let snap = sampler::once();
        let r = syngui::shot::render_png(
            move || {
                let st = St::new(page);
                st.snap.set_always(snap.clone());
                Box::new(ui::root(st))
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
        if writeln!(s, "{}", page.id()).is_ok() {
            return;
        }
    }
    let _ = std::fs::remove_file(&sock);
    let listener = std::sync::Mutex::new(UnixListener::bind(&sock).ok());
    App::new()
        .title(t!("Монитор ресурсов"))
        .app_id("syn-systeminfo")
        .size(1120, 760)
        .min_size(340, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = St::new(page);
            sampler::set_page(page);
            sampler::start(st.snap);
            if let Some(l) = listener.lock().ok().and_then(|mut l| l.take()) {
                listen(l, st);
            }
            Box::new(ui::root(st))
        });
    let _ = std::fs::remove_file(&sock);
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
                if let Some(p) = Page::parse(&line) {
                    run_on_main_thread(move || ui::open(st, p));
                }
            }
        }
    });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-systeminfo.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}
