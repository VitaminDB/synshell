//! syn-health — «Спорт и здоровье» synshell: шаги за день по шагомеру телефона (алгоритм SLPI
//! `pedometer` через источник платформы), расстояние, калории, шаги по часам и за неделю.
//!
//!   syn-health                программа
//!   syn-health --track        счётчик шагов в фоне (автозапуск сеанса; без шагомера — выход)
//!   syn-health --screenshot out.png [--size 420x860] [--scale 2]
//!
//! Без шагомера — `SYN_HEALTH_DEMO=1`: неделя шагов для проверки.

mod store;
mod ui;

use syngui::prelude::*;
use synshell_common::Config;

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_health=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (420u32, 860u32);
    let mut scale = 2.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-health [--track] [--screenshot СНИМОК.png [--size ШxВ] [--scale N]]"));
                return;
            }
            "--track" => {
                store::track();
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
    if std::env::var_os("SYN_HEALTH_DEMO").is_some() {
        demo();
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(|| Box::new(ui::root(ui::St::new())), &mss, size, scale, &out);
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
        .title(t!("Спорт и здоровье"))
        .app_id("syn-health")
        .size(440, 860)
        .min_size(320, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = ui::St::new();
            ui::start(st);
            Box::new(ui::root(st))
        });
}

/// Неделя шагов для проверки без шагомера (в свой XDG_DATA_HOME!).
fn demo() {
    let mut s = store::Steps::default();
    for ago in 0..7 {
        let (date, hour) = store::local_date(ago);
        let last = if ago == 0 { hour } else { 23 };
        for h in 7..=last.min(22) {
            let v = ((h * 37 + ago as usize * 91) % 900 + if h == 8 || h == 18 { 1500 } else { 100 }) as u32;
            s.add(&date, h, v);
        }
    }
    s.save();
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/syn-health.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}
