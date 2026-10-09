//! syn-compass — «Компас» synshell: стороны света по магнитометру и акселерометру
//! (Qualcomm SLPI, источник платформы `sensors-source`), калибровка «восьмёркой»,
//! уровень, метка направления, координаты GeoClue.
//!
//!   syn-compass [--screenshot out.png [--size 420x860] [--scale 2]]
//!
//! Без датчиков (компьютер) — `SYN_COMPASS_DEMO=градусы`: курс ходит вокруг заданного.

mod sensor;
mod ui;

use syngui::prelude::*;
use synshell_common::Config;

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_compass=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (420u32, 860u32);
    let mut scale = 2.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-compass [--screenshot СНИМОК.png [--size ШxВ] [--scale N]]"));
                return;
            }
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(
            || {
                let s = sensor::Sensors::new();
                let st = ui::St::new(s);
                sensor::start(s);
                ui::start_location(st);
                // SYN_COMPASS_SHOT_CALIB=1 — снимок экрана калибровки
                if std::env::var_os("SYN_COMPASS_SHOT_CALIB").is_some() {
                    s.start_calibration();
                }
                // показания приходят из фонового потока — дождаться первых
                std::thread::sleep(std::time::Duration::from_millis(400));
                syngui::async_runtime::drain_main_thread_callbacks();
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
        .title(t!("Компас"))
        .app_id("syn-compass")
        .size(420, 800)
        .min_size(320, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let s = sensor::Sensors::new();
            let st = ui::St::new(s);
            sensor::start(s);
            ui::start_location(st);
            Box::new(ui::root(st))
        });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-compass.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

/// Кодовые точки Material Icons.
pub mod icons {
    pub const EXPLORE_OFF: &str = "\u{E9A8}";
    pub const PLACE: &str = "\u{E55F}";
    pub const FLAG: &str = "\u{E153}";
    pub const CALIBRATE: &str = "\u{E863}";
}
