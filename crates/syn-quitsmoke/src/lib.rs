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
        .size(400, 860)
        .min_size(360, 640)
        .vsync(true)
        .gpu_backend(GpuBackend::Auto)
        .gpu_power(GpuPowerPreference::LowPower)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_theme_styles(&light, &dark, state.theme_dark)
        .run(move |_| Box::new(nav::build_app(state)));
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
        .run(move |_| Box::new(nav::build_app(state)));
}
