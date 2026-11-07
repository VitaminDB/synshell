//! syn-audio-player — музыкальный плеер synshell: ffmpeg (любой аудиоформат, теги, обложки) потоком в стерео
//! (`syngui::video::AudioFilePlayer`) и интерфейс syngui.
//!
//!   syn-audio-player [ФАЙЛ|URI …]
//!
//! Без файлов — библиотека из «Музыки» (XDG_MUSIC_DIR) и «Загрузок»; с файлами — они очередь, первый играет.

mod library;
mod ui;

use std::path::PathBuf;

use syngui::prelude::*;
use synshell_common::Config;

/// `file:///…` из ярлыка (`%U`) — в путь.
fn input_of(arg: &str) -> PathBuf {
    match arg.strip_prefix("file://") {
        Some(p) => PathBuf::from(percent_decode(p)),
        None => PathBuf::from(arg),
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_audio_player=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));
    let mut files: Vec<PathBuf> = Vec::new();
    let mut shot: Option<String> = None;
    let mut size = (420u32, 720u32);
    let mut scale = 1.0f64;
    let mut page = String::from("library");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-audio-player [ФАЙЛ|URI …]"));
                return;
            }
            // снимок без окна: syn-audio-player --screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page library|player]
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = args.next().unwrap_or(page),
            _ => files.push(input_of(&a)),
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        std::process::exit(screenshot(&out, &mss, size, scale, page == "player"));
    }
    App::new()
        .title(t!("Музыка"))
        .app_id("syn-audio-player")
        .size(420, 720)
        .min_size(320, 420)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss.clone(), theme))
        .run(move |_| {
            let st = new_state(!files.is_empty());
            ui::start_ticker(st);
            if files.is_empty() {
                ui::rescan(st);
            } else {
                st.tracks.set(files.iter().cloned().map(library::Track::from_path).collect());
                ui::play_path(st, files[0].clone());
                ui::probe_all(st, files.clone());
            }
            ui::root(st)
        });
}

fn new_state(direct: bool) -> ui::St {
    ui::St {
        tracks: use_signal(Vec::new()),
        scanning: use_signal(!direct),
        current: use_signal(None),
        player: use_signal(None),
        cover: use_signal(None),
        pos: use_signal(0.0),
        dur: use_signal(0.0),
        paused: use_signal(true),
        opening: use_signal(false),
        full: use_signal(direct),
        repeat: use_signal(ui::Repeat::Off),
        error: use_signal(None),
        direct,
    }
}

/// Снимок окна без звука: библиотека читается как обычно, «текущий» трек — второй, на паузе
/// (плеер не открывается — снимок не играет музыку); `full` — экран «Сейчас играет».
fn screenshot(out: &str, mss: &str, size: (u32, u32), scale: f64, full: bool) -> i32 {
    use std::cell::Cell;
    let st_cell: std::rc::Rc<Cell<Option<ui::St>>> = Default::default();
    let st_out = st_cell.clone();
    let done = Cell::new(false);
    let opts = syngui::shot::ShotOptions {
        ready: Some(Box::new(move || {
            let Some(st) = st_cell.get() else { return false };
            if done.get() {
                return true;
            }
            let tracks = st.tracks.get_untracked();
            if st.scanning.get_untracked() || tracks.is_empty() || tracks.iter().any(|t| !t.probed) {
                return false;
            }
            let t = &tracks[1.min(tracks.len() - 1)];
            let cover = syngui::video::probe_audio(&t.path.to_string_lossy()).ok().and_then(|m| m.cover);
            st.current.set(Some(t.path.clone()));
            st.cover.set(cover.map(std::sync::Arc::new));
            st.dur.set(t.duration);
            st.pos.set(t.duration * 0.38);
            st.full.set(full);
            done.set(true);
            true
        })),
        settle: std::time::Duration::from_millis(600),
        timeout: std::time::Duration::from_secs(30),
        ..Default::default()
    };
    let r = syngui::shot::render_png_with(
        move || {
            let st = new_state(false);
            st_out.set(Some(st));
            ui::rescan(st);
            ui::root(st)
        },
        mss,
        size,
        scale,
        out,
        opts,
    );
    match r {
        Ok((w, h)) => {
            println!("{out}: {w}x{h}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "syn-audio-player").font_selector("Text, Button").base(include_str!("../styles/syn-audio-player.mss")).build()
}
