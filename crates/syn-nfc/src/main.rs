//! syn-nfc — «NFC» synshell: чтение меток (ссылки, текст, визитки), сохранение, запись на
//! метки и эмуляция (телефон отвечает считывателю как метка Type 4). Чип ведёт служба
//! `synnfcd` (крейт synnfc); режим живёт, пока программа открыта.
//!
//!   syn-nfc [--screenshot out.png [--size 420x860] [--scale 2] [--tab N]]
//!
//! Без службы — `SYN_NFC_DEMO=1`: прочитанная метка для проверки.

mod state;
mod ui;

use syngui::prelude::*;
use synshell_common::Config;

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_nfc=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang"), include_str!("../../synnfc/i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (420u32, 860u32);
    let mut scale = 2.0f64;
    let mut tab = 0usize;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-nfc [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--tab N]]"));
                return;
            }
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--tab" => tab = args.next().and_then(|s| s.parse().ok()).unwrap_or(0),
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
                state::start(st);
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
    App::new()
        .title("NFC")
        .app_id("syn-nfc")
        .size(480, 820)
        .min_size(320, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = state::St::new();
            state::start(st);
            Box::new(ui::root(st))
        });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-nfc.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

/// Кодовые точки Material Icons.
pub mod icons {
    pub const NFC: &str = "\u{E1BB}";
    pub const NFC_OFF: &str = "\u{E1BB}";
    pub const SAVED: &str = "\u{E866}";
    pub const ADD: &str = "\u{E145}";
    pub const LINK: &str = "\u{E157}";
    pub const TEXT: &str = "\u{E262}";
    pub const PHONE: &str = "\u{E0B0}";
    pub const MAIL: &str = "\u{E158}";
    pub const CONTACT: &str = "\u{E7FD}";
    pub const DATA: &str = "\u{E86F}";
    pub const COPY: &str = "\u{E14D}";
    pub const SAVE: &str = "\u{E161}";
    pub const EDIT: &str = "\u{E3C9}";
    pub const CAST: &str = "\u{E307}";
    pub const DELETE: &str = "\u{E872}";
}
