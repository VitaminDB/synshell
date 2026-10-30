//! syn-dlss — окно DLSS 5 synshell: запуск помощника NGX под syn-wine, состояние игры со слоем,
//! все настройки нейрорендеринга (через общую память `/tmp/dlssnr-$UID/shm.bin`), профили, импорт DLL NGX.
//! Замена Qt-окна `dlssnr-gui` форка DLSS5VKLayer (`crates/syn-wine/dlss5`); config.ini и профили — общие с ним.
//!
//!   syn-dlss [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--tab rendering|quality|composition|inspect|helper]]

mod icons;
mod state;
mod ui;

use syn_dlss::{config, paths, shm};

use syngui::prelude::*;
use synshell_common::config::Rgba;
use synshell_common::Config;

pub fn theme(cfg: &Config) -> String {
    use synshell_common::style::{px, AppStyle};
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let titlebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.22) } else { p.surface_alt.mix(p.bg, 0.5) };
    let content = if dark { p.bg.mix(p.surface, 0.12) } else { p.bg.mix(p.surface, 0.4) };
    let card = if dark { p.surface.mix(Rgba::rgb(255, 255, 255), 0.03) } else { p.surface };
    // зелёный NVIDIA как второй цвет знака
    let accent2 = p.accent.mix(Rgba::rgb(0x76, 0xb9, 0x00), 0.6);
    let input = if dark { p.bg.mix(p.surface, 0.35) } else { p.surface };
    AppStyle::new(cfg, "syn-dlss")
        .vars(&[
            ("titlebar", titlebar.hex()),
            ("content", content.hex()),
            ("card", card.hex()),
            ("input-bg", input.hex()),
            ("accent-2", accent2.hex()),
            ("accent-dim", p.accent.with_alpha(0.14).hex()),
            ("shadow", (if dark { "#00000080" } else { "#1a203024" }).to_string()),
            ("scrim", p.bg.with_alpha(0.86).hex()),
            ("window-radius", px(cfg.decorations.corner_radius.max(0.0))),
        ])
        .base(include_str!("../styles/syn-dlss.mss"))
        .build()
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_dlss=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (760u32, 860u32);
    let mut scale = 1.0f64;
    let mut tab = state::Tab::Rendering;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-dlss [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--tab rendering|quality|composition|inspect|helper]]"));
                return;
            }
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--tab" => tab = args.next().as_deref().and_then(state::Tab::parse).unwrap_or(tab),
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(
            || {
                let st = state::St::new();
                st.tab.set(tab);
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
    // Тема меняется вместе с настройками оболочки и примеркой из редактора тем.
    synshell_common::live::bind(st.theme, mss, theme);
    App::new()
        .title("DLSS 5")
        .app_id("syn-dlss")
        .size(780, 880)
        .min_size(560, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(st.theme)
        .with_window_state(st.window)
        .run(move |_| {
            st.start();
            ui::root(st)
        });
}
