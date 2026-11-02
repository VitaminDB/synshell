//! syn-woman-cycle — «Цикл» synshell: календарь месячных, овуляции и фертильного окна.
//! Отметки по дням, прогноз по истории циклов, самочувствие. Данные — только на устройстве.
//!
//!   syn-woman-cycle [--screenshot out.png [--size 420x900] [--scale 2] [--page calendar|menu|history|settings|intro|cycle|day] [--dark]]

mod date;
mod decor;
mod icons;
mod model;
mod state;
mod ui;

use syngui::prelude::*;
use synshell_common::Config;

use model::ThemeMode;

/// Цвета для холстов (кольцо цикла, полосы истории).
#[derive(Clone, Copy)]
pub struct Pal {
    pub rose: Color,
    pub plum: Color,
    pub teal: Color,
    pub track: Color,
    pub fg: Color,
    pub surface: Color,
    /// Чернила обводки «ручкой».
    pub ink: Color,
    /// Фазы на полосе цикла.
    pub folli: Color,
    pub fert: Color,
    pub fert_strong: Color,
    pub lute: Color,
}

pub fn pal(dark: bool) -> Pal {
    if dark {
        Pal {
            rose: Color::from_hex("#f0577f"),
            plum: Color::from_hex("#a98ce0"),
            teal: Color::from_hex("#35d0be"),
            track: Color::from_hex("#2c272d"),
            fg: Color::from_hex("#f3eef1"),
            surface: Color::from_hex("#1c191d"),
            ink: Color::from_hex("#8fa8ff"),
            folli: Color::from_hex("#5f7fa8"),
            fert: Color::from_hex("#8a6d1a"),
            fert_strong: Color::from_hex("#f2b300"),
            lute: Color::from_hex("#6f5f96"),
        }
    } else {
        Pal {
            rose: Color::from_hex("#e0466f"),
            plum: Color::from_hex("#7e63c2"),
            teal: Color::from_hex("#22b8a7"),
            track: Color::from_hex("#ece7ea"),
            fg: Color::from_hex("#1e1a1d"),
            surface: Color::from_hex("#ffffff"),
            ink: Color::from_hex("#2f55b8"),
            folli: Color::from_hex("#a9c1e0"),
            fert: Color::from_hex("#f7dc8a"),
            fert_strong: Color::from_hex("#e6a800"),
            lute: Color::from_hex("#cbbde8"),
        }
    }
}

/// Палитра холстов с поправками оформления.
pub fn pal_for(dark: bool, d: decor::Decor) -> Pal {
    let mut p = pal(dark);
    let over = d.vars(dark);
    let get = |k: &str| over.iter().find(|(o, _)| *o == k).map(|(_, v)| Color::from_hex(v));
    if let Some(c) = get("rose") {
        p.rose = c;
    }
    if let Some(c) = get("plum") {
        p.plum = c;
    }
    if let Some(c) = get("surface") {
        p.surface = c;
    }
    if let Some(c) = get("fg") {
        p.fg = c;
    }
    if let Some(c) = get("border") {
        p.track = c;
    }
    if let Some(c) = get("ovul") {
        p.fert = c;
    }
    p
}

/// Палитра холстов для текущего состояния (тема и оформление).
pub fn pal_st(st: state::St) -> Pal {
    pal_for(st.dark.get_untracked(), st.data.get_untracked().settings.decor)
}

/// Цвет фона окна (для рисунков, которые «вырезают» фоном).
pub fn bg_color(dark: bool, d: decor::Decor) -> Color {
    let base = if dark { "#121013" } else { "#fbfafa" };
    Color::from_hex(d.vars(dark).iter().find(|(k, _)| *k == "bg").map(|(_, v)| *v).unwrap_or(base))
}

pub fn is_dark(cfg: &Config, s: &model::Settings) -> bool {
    if s.decor.always_dark() {
        return true;
    }
    match s.theme {
        // На iPhone «как в системе» — тёмная тема iOS, конфига оболочки там нет.
        ThemeMode::System if cfg!(target_os = "ios") => syngui::appearance::read_system_appearance().is_dark(),
        ThemeMode::System => cfg.appearance.is_dark(),
        ThemeMode::Light => false,
        ThemeMode::Dark => true,
    }
}

pub fn theme(cfg: &Config, dark: bool, decor: decor::Decor) -> String {
    let vars: &[(&str, &str, &str)] = &[
        // имя, светлая, тёмная
        ("bg", "#fbfafa", "#121013"),
        ("bg-2", "#fbfafa", "#121013"),
        ("surface", "#ffffff", "#1c191d"),
        ("surface-alt", "#f4f1f3", "#252126"),
        ("fg", "#1e1a1d", "#f3eef1"),
        ("muted", "#857c83", "#9c939a"),
        ("faint", "#c4bcc1", "#5a5259"),
        ("border", "#ece7ea", "#2c272d"),
        ("hover", "#1e1a1d0d", "#ffffff10"),
        ("pressed", "#1e1a1d1a", "#ffffff1c"),
        ("rose", "#e0466f", "#f0577f"),
        ("rose-2", "#e85c81", "#f36b8f"),
        ("rose-soft", "#f7dfe5", "#43202b"),
        ("rose-line", "#e0466f80", "#f0577f99"),
        ("plum", "#7e63c2", "#a98ce0"),
        ("plum-soft", "#7e63c21f", "#a98ce02a"),
        ("teal", "#22b8a7", "#35d0be"),
        ("teal-soft", "#22b8a72a", "#35d0be30"),
        ("gold", "#e6a800", "#f2b300"),
        ("fert", "#f9e9be", "#3f310f"),
        ("ovul", "#f6d370", "#7e5e0a"),
        ("ovul-line", "#c98f00", "#ffd04d"),
        ("fert-text", "#8a6400", "#ffd36b"),
        ("ink", "#2f55b8", "#8fa8ff"),
        ("folli", "#a9c1e0", "#5f7fa8"),
        ("folli-soft", "#d6e0ee", "#354256"),
        ("lute-soft", "#e3dcf1", "#403854"),
        ("lute", "#cbbde8", "#6f5f96"),
        ("on-accent", "#ffffff", "#ffffff"),
        ("shadow", "#1e1a1d14", "#00000066"),
        ("shadow-strong", "#1e1a1d29", "#00000099"),
        ("scrim", "#1e1a1d4d", "#000000a0"),
        ("menu-bg", "#ffffff", "#211d22"),
        ("titlebar", "#f6f3f5", "#171418"),
        ("sidebar", "#f6f3f5", "#171418"),
        ("nav", "#fffffff5", "#1a171bf5"),
        ("danger", "#d93d47", "#ff6b6f"),
        ("today", "#22b8a7", "#35d0be"),
    ];

    let mut s = String::from(":root {\n");
    let over = decor.vars(dark);
    for (k, l, d) in vars {
        let v = over.iter().find(|(o, _)| o == k).map(|(_, v)| *v).unwrap_or(if dark { d } else { l });
        s.push_str(&format!("  --{k}: {v};\n"));
    }
    s.push_str(&format!("  --window-radius: {}px;\n}}\n", cfg.decorations.corner_radius.max(0.0)));
    s.push_str(include_str!("../styles/syn-woman-cycle.mss"));
    let font = cfg.appearance.font.trim();
    if !font.is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{font}\"; }}\n"));
    }
    s
}

/// Применить тему по текущим настройкам приложения и оболочки.
pub fn apply_theme(st: state::St) {
    apply_theme_with(st, &Config::load().0);
}

fn apply_theme_with(st: state::St, cfg: &Config) {
    let s = st.data.get_untracked().settings;
    let dark = is_dark(cfg, &s);
    st.dark.set(dark);
    let mss = theme(cfg, dark, s.decor);
    if st.theme.get_untracked() != mss {
        st.theme.set(mss);
    }
}

/// Тема «как в системе» меняется вместе с настройками оболочки. Своя
/// дизайн-система: таблица собирается без `AppStyle` (палитра и переменные
/// темы оболочки перекрыли бы свои цвета программы).
fn watch_config(st: state::St) {
    synshell_common::live::spawn(Vec::new(), move |cfg| run_on_main_thread(move || apply_theme_with(st, &cfg)));
}

/// Пример истории для снимков экрана: три цикла по 28–30 дней.
fn demo() -> model::Data {
    let t = date::today();
    let mut d = model::Data { onboarded: true, ..Default::default() };
    for (start, len) in [(t - 72, 5), (t - 43, 4), (t - 14, 5)] {
        for i in 0..len {
            d.days.insert(start + i, model::DayLog { flow: if i == 0 || i == len - 1 { 2 } else { 3 }, sym: if i < 2 { 1 } else { 0 }, mood: 0 });
        }
    }
    d.days.entry(t).or_default().mood = 2;
    d.intimacy.push(model::Intimacy { id: 1, day: t - 3, time: Some(22 * 60 + 30), protected: Some(true) });
    d.intimacy.push(model::Intimacy { id: 2, day: t, time: Some(8 * 60 + 15), protected: None });
    d
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_woman_cycle=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (420u32, 900u32);
    let mut scale = 2.0f64;
    let mut page = String::from("calendar");
    let mut dark_shot = false;
    let mut decor_shot = decor::Decor::Classic;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-woman-cycle [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page calendar|menu|history|settings|intro|cycle|day] [--dark] [--decor classic|sakura|berry|lavender|daisy|night]]"));
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
            "--dark" => dark_shot = true,
            "--decor" => {
                let name = args.next().unwrap_or_default();
                decor_shot = decor::Decor::ALL.into_iter().find(|d| format!("{d:?}").eq_ignore_ascii_case(&name)).unwrap_or(decor_shot);
            }
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    if let Some(out) = shot {
        let dark_shot = dark_shot || decor_shot.always_dark();
        let mss = theme(&cfg, dark_shot, decor_shot);
        let r = syngui::shot::render_png(
            || {
                let mut data = if page == "intro" || page == "cycle" { model::Data::default() } else { demo() };
                data.settings.decor = decor_shot;
                let st = state::St::new(data);
                st.dark.set(dark_shot);
                match page.as_str() {
                    "menu" => st.open_menu(date::today(), syngui::core::Rect::new(syngui::core::Point::new(330.0, 300.0), syngui::core::Size::new(40.0, 40.0))),
                    "history" => st.tab.set(state::Tab::History),
                    "settings" => st.tab.set(state::Tab::Settings),
                    "cycle" => {
                        st.intro_step.set(1);
                        st.intro_day.set(Some(date::today() - 10));
                    }
                    "day" => {
                        st.tab.set(state::Tab::Calendar);
                        st.open_day(date::today());
                    }
                    _ => {}
                }
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
    let data = model::load();
    let set = data.settings.clone();
    let st = state::St::new(data);
    let dark = is_dark(&cfg, &set);
    let mss = theme(&cfg, dark, set.decor);
    st.dark.set(dark);
    st.theme.set(mss);
    watch_config(st);
    // iOS: смена тёмной темы системы приходит через сигнал syngui.
    let system_look = use_signal(syngui::appearance::SystemAppearance::default());
    if cfg!(target_os = "ios") {
        create_effect(move || {
            system_look.get();
            apply_theme(st);
        });
    }
    let app = App::new()
        .title(t!("Цикл"))
        .app_id("syn-woman-cycle")
        .size(460, 900)
        .min_size(340, 560)
        .frameless()
        // На iPhone окно — весь экран, прозрачность не нужна.
        .transparent(!cfg!(target_os = "ios"))
        // iPhone: всегда во весь экран, без строки состояния.
        .fullscreen(set.fullscreen || cfg!(target_os = "ios"))
        .ui_scale(set.scale)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(st.theme)
        .with_theme_transition(320)
        .with_window_state(st.window);
    let app = if cfg!(target_os = "ios") { app.with_system_appearance(system_look) } else { app };
    app.run(move |_| {
        st.start();
        ui::root(st)
    });
}
