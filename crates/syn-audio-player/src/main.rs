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
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut files: Vec<PathBuf> = Vec::new();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-audio-player [ФАЙЛ|URI …]"));
                return;
            }
            _ => files.push(input_of(&a)),
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    App::new()
        .title(t!("Музыка"))
        .app_id("syn-audio-player")
        .size(420, 720)
        .min_size(320, 420)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = ui::St {
                tracks: use_signal(Vec::new()),
                scanning: use_signal(files.is_empty()),
                current: use_signal(None),
                player: use_signal(None),
                cover: use_signal(None),
                pos: use_signal(0.0),
                dur: use_signal(0.0),
                paused: use_signal(true),
                opening: use_signal(false),
                full: use_signal(!files.is_empty()),
                repeat: use_signal(ui::Repeat::Off),
                error: use_signal(None),
                direct: !files.is_empty(),
            };
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

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-audio-player.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}
