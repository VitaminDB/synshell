//! syn-audio-recorder — «Диктофон» synshell: живая волна записи, выбор микрофона, системное шумоподавление
//! телефона, запись в Opus/FLAC/WAV (ffmpeg) и список записей с плеером.
//!
//!   syn-audio-recorder [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page record|rec|paused|list|player|mics|settings]]
//!
//! Записи — в `$XDG_MUSIC_DIR/Диктофон` (меняется в настройках), сведения о них — в `.meta/` рядом.

mod engine;
mod store;
mod ui;

use syngui::prelude::*;
use synshell_common::config::Rgba;
use synshell_common::Config;

pub fn theme(cfg: &Config) -> String {
    use synshell_common::style::{px, AppStyle};
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let titlebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.22) } else { p.surface_alt.mix(p.bg, 0.5) };
    let sidebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.12) } else { p.surface_alt.mix(p.bg, 0.3) };
    let content = if dark { p.bg.mix(p.surface, 0.12) } else { p.bg.mix(p.surface, 0.4) };
    let card = if dark { p.surface.mix(Rgba::rgb(255, 255, 255), 0.02) } else { p.surface };
    // запись — всегда красная, оттенок чуть подстроен под тему
    let rec = if dark { Rgba::rgb(0xff, 0x4d, 0x5e) } else { Rgba::rgb(0xe5, 0x30, 0x45) };
    let rec2 = rec.mix(Rgba::rgb(0xff, 0x8a, 0x3d), 0.35);
    AppStyle::new(cfg, "syn-audio-recorder")
        .vars(&[
            ("titlebar", titlebar.hex()),
            ("sidebar", sidebar.hex()),
            ("content", content.hex()),
            ("card", card.hex()),
            ("rec", rec.hex()),
            ("rec-2", rec2.hex()),
            ("rec-soft", rec.with_alpha(0.14).hex()),
            ("rec-ring", rec.with_alpha(0.28).hex()),
            ("rec-glow", rec.with_alpha(if dark { 0.45 } else { 0.35 }).hex()),
            ("accent-dim", p.accent.with_alpha(0.14).hex()),
            ("shadow", (if dark { "#00000080" } else { "#1a203024" }).to_string()),
            ("scrim", (if dark { "#000000a0" } else { "#10182060" }).to_string()),
            ("window-radius", px(cfg.decorations.corner_radius.max(0.0))),
        ])
        .base(include_str!("../styles/syn-audio-recorder.mss"))
        .build()
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_audio_recorder=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (420u32, 900u32);
    let mut scale = 2.0f64;
    let mut page = String::from("record");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-audio-recorder [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page record|rec|paused|list|player|mics|settings]]"));
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
                let st = ui::St::new();
                screenshot_state(st, &page);
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
    let st = ui::St::new();
    st.theme.set(mss.clone());
    synshell_common::live::bind(st.theme, mss, theme);
    App::new()
        .title(t!("Диктофон"))
        .app_id("syn-audio-recorder")
        .size(980, 700)
        .min_size(340, 560)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(st.theme)
        .with_window_state(st.window)
        // закрытие во время записи (кнопка окна, двойной «назад» на телефоне) — дубль сохраняется, не теряется
        .on_close_request(|| {
            match engine::stop() {
                Ok(Some(p)) => tracing::info!("закрытие: запись сохранена — {}", p.display()),
                Ok(None) => {}
                Err(e) => tracing::warn!("закрытие: запись не сохранилась — {e}"),
            }
            true
        })
        .run(move |_| {
            st.start();
            ui::root(st)
        });
}

/// Состояние для снимка (`--page`): проверка вёрстки без микрофона.
fn screenshot_state(st: ui::St, page: &str) {
    use ui::{MicItem, MicState, Ns, Page, Rec, Sheet};
    let now = store::now();
    st.mics.set(vec![
        MicItem { name: "palaudio.mic".into(), label: "Микрофон".into(), default: true },
        MicItem { name: "palaudio.mic.bottom".into(), label: "Нижний микрофон".into(), default: false },
        MicItem { name: "palaudio.mic.top".into(), label: "Верхний микрофон".into(), default: false },
    ]);
    st.ns.set(Some(Ns { on: true, active: true }));
    st.mic.set(MicState::Open);
    let recs: Vec<store::Recording> = [
        ("Запись 2026-11-02 19:30", 42.0, 0, 330_000),
        ("Лекция: архитектура ПО", 3121.0, 86_400, 24_500_000),
        ("Идея для песни", 95.0, 3 * 86_400, 780_000),
        ("Совещание", 1820.0, 9 * 86_400, 14_200_000),
        ("Запись 2026-10-20 08:12", 12.0, 13 * 86_400, 98_000),
    ]
    .iter()
    .enumerate()
    .map(|(k, (t, d, ago, sz))| store::Recording {
        path: std::path::PathBuf::from(format!("/tmp/demo{k}.ogg")),
        title: t.to_string(),
        duration: *d,
        size: *sz,
        created: now - ago - 600,
        peaks: (0..store::PEAKS).map(|i| (store::loudness(ui::wave::speech(i * 3 + k * 50)) * 255.0) as u8).collect(),
        pending: false,
    })
    .collect();
    st.recordings.set(recs.clone());
    st.scanning.set(false);
    match page {
        "rec" | "paused" => {
            ui::wave::demo(83.4, page == "paused");
            st.rec.set(if page == "paused" { Rec::Paused } else { Rec::Recording });
            st.tenths.set(834);
        }
        "list" => st.page.set(Page::List),
        "player" => {
            st.page.set(Page::List);
            st.selected.set(Some(recs[1].path.clone()));
            st.dur.set(recs[1].duration);
            st.pos.set(recs[1].duration * 0.37);
            st.playing.set(true);
        }
        "mics" => {
            st.settings.update(|s| s.mic = "palaudio.mic.top".into());
            st.sheet.set(Some(Sheet::Mics));
        }
        "settings" => st.sheet.set(Some(Sheet::Settings)),
        _ => {
            for (i, v) in (0..400).map(|i| ui::wave::speech(i) * 0.5).enumerate() {
                let _ = i;
                engine::vis().live.push_back(v);
            }
            engine::vis().open = true;
        }
    }
}
