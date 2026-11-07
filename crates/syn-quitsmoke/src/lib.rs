//! QuitSmoke — приложение для отказа от курения на фреймворке syngui.
//!
//! Двойная точка входа: `run_desktop()` для desktop и `android_main()` для Android.

pub mod app_state;
pub mod i18n;
pub mod icons;
pub mod model;
pub mod nav;
pub mod notify;
pub mod persist;
pub mod screens;
pub mod util;
pub mod widgets;

use syngui::prelude::*;

use crate::app_state::load_state;
use crate::persist::{platform_data_dir, Persistence};

const BASE: &str = include_str!("../styles/base.mss");
const LIGHT_TOKENS: &str = include_str!("../styles/theme_light.mss");
const DARK_TOKENS: &str = include_str!("../styles/theme_dark.mss");

fn theme_sheets() -> (String, String) {
    (
        format!("{LIGHT_TOKENS}\n{BASE}"),
        format!("{DARK_TOKENS}\n{BASE}"),
    )
}

// ── Desktop ──────────────────────────────────────────────────────────────

pub fn run_desktop() {
    let persist = Persistence::new(platform_data_dir(None));
    provide_context(persist);
    let state = load_state();
    provide_context(state);
    crate::i18n::init(state.settings.get_untracked().locale);
    state.sync_dialog_labels();

    let (light, dark) = theme_sheets();
    App::new()
        .title("QuitSmoke")
        // app_id = имя .desktop: значок и группировка окна на панели (без него app_id был пустым).
        .app_id("syn-quitsmoke")
        .size(400, 860)
        .min_size(360, 640)
        // Своя рамка окна (заголовок и кнопки окна рисует nav::build_app)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_window_state(state.window)
        .vsync(true)
        .gpu_backend(GpuBackend::Auto)
        .gpu_power(GpuPowerPreference::LowPower)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_theme_styles(&light, &dark, state.theme_dark)
        .run(move |_| nav::build_app(state));
}

// ── Снимок для документации ──────────────────────────────────────────────

/// Снимок экрана без окна на придуманных данных (файл данных пользователя не читается и не пишется):
/// `syn-quitsmoke --screenshot out.png [--size 430x900] [--scale 2] [--page home|stats|health|achievements|settings|craving|onboarding] [--dark]`;
/// язык — `SYNSHELL_LANG`, иначе язык системы.
#[cfg(feature = "desktop")]
pub fn screenshot(args: &[String]) -> std::result::Result<(), String> {
    use crate::app_state::{AppState, Screen};
    use crate::model::log::CigaretteLog;
    use crate::model::profile::{QuitMode, UserProfile};
    use crate::model::settings::{Locale, Settings};
    use crate::persist::AppStateData;

    let mut it = args.iter();
    let out = it.next().cloned().ok_or("--screenshot ФАЙЛ.png")?;
    let mut size = (430u32, 900u32);
    let mut scale = 2.0f64;
    let mut page = String::from("home");
    let mut dark = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = it.next().cloned().unwrap_or(page),
            "--dark" => dark = true,
            _ => {}
        }
    }
    let locale = match std::env::var("SYNSHELL_LANG") {
        Ok(l) if !l.trim().is_empty() => Locale::new(l.trim()),
        _ => Locale::AUTO,
    };
    let ru = locale.resolve().tag().starts_with("ru");
    crate::i18n::init(locale);
    // Пример: бросил сразу 12 дней назад, 15 сигарет в день; одна сигарета на третий день.
    let now = crate::util::now_ms();
    let day = 86_400_000u64;
    let start = now - 12 * day - 5 * 3_600_000;
    let mut log = CigaretteLog::default();
    log.push(start + 2 * day + 14 * 3_600_000, false);
    let data = AppStateData {
        profile: Some(UserProfile {
            cigs_per_day: 15,
            cigs_per_pack: 20,
            price_per_pack: if ru { 1000.0 } else { 9.0 },
            years_smoking: 8.0,
            quit_started_at: start,
            mode: QuitMode::Immediate,
            ..Default::default()
        }),
        log,
        settings: Settings { dark_mode: dark, currency_symbol: if ru { "₸".into() } else { "$".into() }, locale, ..Default::default() },
        onboarded: page != "onboarding",
        ..Default::default()
    };
    let (light, dark_sheet) = theme_sheets();
    let mss = if dark { dark_sheet } else { light };
    let r = syngui::shot::render_png(
        move || {
            let state = AppState::from_snapshot(&data);
            provide_context(state);
            state.sync_dialog_labels();
            state.refresh_achievements(now);
            state.celebrate.set(0);
            state.screen.set(match page.as_str() {
                "stats" => Screen::Stats,
                "health" => Screen::Health,
                "achievements" => Screen::Achievements,
                "settings" => Screen::Settings,
                "craving" => Screen::Craving,
                "onboarding" => Screen::Onboarding,
                _ => Screen::Home,
            });
            nav::build_app(state)
        },
        &mss,
        size,
        scale,
        &out,
    )?;
    println!("{out}: {}x{}", r.0, r.1);
    Ok(())
}

// ── Android ──────────────────────────────────────────────────────────────

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: syngui::app::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );
    log::info!("QuitSmoke Android starting");

    let data_dir = app.internal_data_path();
    let persist = Persistence::new(platform_data_dir(data_dir));
    provide_context(persist);
    let state = load_state();
    provide_context(state);
    crate::i18n::init(state.settings.get_untracked().locale);
    state.sync_dialog_labels();
    crate::notify::init(&state);

    let (light, dark) = theme_sheets();
    App::new()
        .title("QuitSmoke")
        .vsync(true)
        .gpu_backend(GpuBackend::Gl)
        .gpu_power(GpuPowerPreference::LowPower)
        .with_android_app(app)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_theme_styles(&light, &dark, state.theme_dark)
        .run(move |_| nav::build_app(state));
}
