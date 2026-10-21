//! Syn Blocks — головоломка 8×8 в духе Block Blast на syngui.
//!
//! * `game` — чистая модель (поле, фигуры, правила, честный генератор);
//! * `ui` — единственный элемент `GameView`: ввод, эффекты, отрисовка;
//! * `anim` — easing и частицы; `persist` — рекорд, статистика, партии по
//!   видам игры (`Variant`); `lang` — игровой словарь en/ru и реплики.

pub mod anim;
pub mod audio;
pub mod game;
pub mod lang;
pub mod persist;
pub mod ui;

pub use persist::Variant;
pub use ui::{GameCtx, GameView, Screen};

use game::Rng;
use syngui::prelude::*;

pub const WINDOW_W: u32 = 460;
pub const WINDOW_H: u32 = 820;

fn build_ctx() -> GameCtx {
    let stats = persist::load();
    audio::init_with_voice(stats.music, stats.sound, stats.voice, stats.track as usize, stats.loop_all);
    let ctx = GameCtx::new(Rng::from_time().state(), stats);
    // приложение открывается стартовой страницей; партия выбранного вида
    // (незаконченная или только что проигранная) загружается при входе в игру
    ctx.screen.set(Screen::Start);
    ctx.variant.set(Variant::from_index(stats.variant as usize));
    ctx
}

pub fn run_desktop() {
    let ctx = build_ctx();
    App::new()
        .title("Syn Blocks")
        .size(WINDOW_W, WINDOW_H)
        .min_size(340, 600)
        .vsync(true)
        .gpu_backend(GpuBackend::Auto)
        .gpu_power(GpuPowerPreference::HighPerformance)
        .background(Color::from_hex(ui::theme::BG_BOTTOM))
        .with_font_family(ui::theme::FONT)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .run(move |_| {
            provide_context(ctx);
            Box::new(GameView)
        });
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: syngui::app::AndroidApp) {
    use android_logger::Config;
    use log::LevelFilter;

    android_logger::init_once(Config::default().with_max_level(LevelFilter::Info).with_tag("syn_blocks"));
    // Рекорд и статистика — во внутренней папке приложения (HOME на Android нет).
    if let Some(dir) = app.internal_data_path() {
        std::env::set_var("SYN_BLOCKS_DATA", dir);
    }
    let ctx = build_ctx();
    App::new()
        .title("Syn Blocks")
        .vsync(true)
        .gpu_backend(GpuBackend::Gl)
        .gpu_power(GpuPowerPreference::LowPower)
        .with_android_app(app)
        .edge_to_edge(true)
        .background(Color::from_hex(ui::theme::BG_BOTTOM))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .run(move |_| {
            provide_context(ctx);
            Box::new(GameView)
        });
}
