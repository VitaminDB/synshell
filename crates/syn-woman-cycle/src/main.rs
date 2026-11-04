//! syn-woman-cycle — «Цикл» synshell: календарь месячных, овуляции и фертильного окна.
//! Отметки по дням, прогноз по истории циклов, самочувствие. Данные — только на устройстве.
//!
//!   syn-woman-cycle [--screenshot out.png [--size 420x900] [--scale 2] [--page calendar|menu|history|settings|intro|cycle|day|phase[-period|-folli|-fert|-lute]] [--dark]]

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
    let c = Color::from_hex;
    if dark {
        Pal {
            rose: c("#ff4767"),
            plum: c("#a07dff"),
            teal: c("#14d1ba"),
            track: c("#2a282e"),
            fg: c("#f3f1f4"),
            surface: c("#1a191d"),
            ink: c("#ff5a96"),
            folli: c("#4f8dff"),
            fert: c("#ffa31a"),
            fert_strong: c("#ffd27a"),
            lute: c("#9a72ff"),
        }
    } else {
        Pal {
            rose: c("#e8294f"),
            plum: c("#7442f0"),
            teal: c("#00a896"),
            track: c("#ebe7e2"),
            fg: c("#18171c"),
            surface: c("#ffffff"),
            ink: c("#f0357a"),
            folli: c("#2f6ff5"),
            fert: c("#ff9f0a"),
            fert_strong: c("#b36f00"),
            lute: c("#7442f0"),
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
    p
}

/// Палитра холстов для текущего состояния (тема и оформление).
pub fn pal_st(st: state::St) -> Pal {
    pal_for(st.dark.get_untracked(), st.data.get_untracked().settings.decor)
}

/// Цвет фона окна (для рисунков, которые «вырезают» фоном).
pub fn bg_color(dark: bool, d: decor::Decor) -> Color {
    let base = if dark { "#0f0e11" } else { "#f6f4f1" };
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
        ("bg", "#f6f4f1", "#0f0e11"),
        ("bg-2", "#f6f4f1", "#0f0e11"),
        ("surface", "#ffffff", "#1a191d"),
        ("surface-alt", "#f1eeea", "#232127"),
        ("fg", "#18171c", "#f3f1f4"),
        ("muted", "#6f6a73", "#9d98a3"),
        ("faint", "#b5b0b6", "#5c5862"),
        ("border", "#e9e5e0", "#2a282e"),
        ("hover", "#18171c0a", "#ffffff0d"),
        ("pressed", "#18171c14", "#ffffff17"),
        // фазы: менструация — гранатовый, фолликулярная — синий, овуляция — янтарь, лютеиновая — фиолетовый
        ("rose", "#e8294f", "#ff4767"),
        ("rose-2", "#cc1f42", "#ff6680"),
        ("rose-soft", "#ffcad5", "#5c1626"),
        ("rose-line", "#e8294f", "#ff4767"),
        ("plum", "#7442f0", "#a07dff"),
        ("plum-soft", "#7442f022", "#a07dff2e"),
        ("teal", "#00a896", "#14d1ba"),
        ("teal-soft", "#00a89626", "#14d1ba30"),
        ("gold", "#ff9f0a", "#ffb020"),
        ("fert", "#ffd84d", "#d9a514"),
        ("fert-soft", "#fef6e4", "#2a210f"),
        ("ovul", "#ff9f0a", "#ffa31a"),
        ("ovul-line", "#b35f00", "#ffd27a"),
        ("fert-text", "#5c3a00", "#2a1c00"),
        ("ink", "#f0357a", "#ff5a96"),
        ("love", "#f0357a", "#ff5a96"),
        ("folli", "#2f6ff5", "#4f8dff"),
        ("folli-soft", "#cfe0ff", "#16316b"),
        ("lute-soft", "#ddd0ff", "#2e1f6b"),
        ("lute", "#7442f0", "#9a72ff"),
        ("late", "#b0174a", "#ff6b8e"),
        ("late-soft", "#ffd0dc", "#4a1626"),
        ("on-accent", "#ffffff", "#ffffff"),
        ("primary", "#18171c", "#f3f1f4"),
        ("on-primary", "#ffffff", "#18171c"),
        ("shadow", "#2a1e140f", "#00000059"),
        ("shadow-strong", "#2a1e1424", "#00000099"),
        ("scrim", "#18171c66", "#000000a6"),
        ("menu-bg", "#ffffff", "#1f1d23"),
        ("titlebar", "#f6f4f1", "#0f0e11"),
        ("sidebar", "#f1eeea", "#141317"),
        ("nav", "#fffffff2", "#151418f2"),
        ("danger", "#e5262f", "#ff5a5f"),
        ("today", "#00a896", "#14d1ba"),
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
    let mut data_shot: Option<model::Data> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-woman-cycle [--screenshot СНИМОК.png [--size ШxВ] [--scale N] [--page calendar|menu|history|settings|intro|cycle|day|phase[-period|-folli|-fert|-lute]] [--dark] [--decor classic|sakura|berry|lavender|daisy|night] [--data data.json]]"));
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
            // снимок с настоящими отметками (data.json с устройства) вместо примера
            "--data" => data_shot = args.next().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| serde_json::from_str(&s).ok()),
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
                let mut data = if page == "intro" || page == "cycle" { model::Data::default() } else { data_shot.clone().unwrap_or_else(demo) };
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
                    "phase" | "phase-period" | "phase-folli" | "phase-fert" | "phase-lute" => {
                        use ui::PhaseKey;
                        let k = match page.as_str() {
                            "phase-period" => PhaseKey::Period,
                            "phase-folli" => PhaseKey::Follicular,
                            "phase-lute" => PhaseKey::Luteal,
                            _ => PhaseKey::Fertile,
                        };
                        st.open_phase(k);
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
