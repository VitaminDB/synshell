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

/// Переводы: исходные строки русские (`t!`/`tn!`/`n_!`), каталоги —
/// `i18n/<язык>.lang`; язык — `SYNSHELL_LANG`, иначе язык системы.
pub fn init_i18n() {
    syngui::i18n::set_source_language("ru");
    syngui::i18n::register_catalogs(&synshell_tr::catalogs!("../i18n"));
    match std::env::var("SYNSHELL_LANG") {
        Ok(l) if !l.trim().is_empty() => syngui::i18n::set_language(l.trim()),
        _ => syngui::i18n::set_language(syngui::i18n::system_language()),
    }
}

fn build_ctx() -> GameCtx {
    init_i18n();
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
    // своё окно (заголовок, кнопки, рамка — ui::chrome) и полноэкранный режим
    App::new()
        .title("Syn Blocks")
        .app_id("syn-blocks")
        .size(WINDOW_W, WINDOW_H + 42)
        .min_size(340, 640)
        .frameless()
        .transparent(true)
        .fullscreen(ui::chrome::start_fullscreen(ctx))
        .vsync(true)
        .gpu_backend(GpuBackend::Auto)
        .gpu_power(GpuPowerPreference::HighPerformance)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_font_family(ui::theme::FONT)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&ui::chrome::mss())
        .with_window_state(ctx.window)
        .run(move |_| {
            provide_context(ctx);
            ui::chrome::root(ctx)
        });
}

/// Снимок окна без окна и звука на демо-состоянии (`ui::demo`) — для документации:
/// `syn-blocks --screenshot out.png [--size 460x862] [--scale 2] [--page start|board|drag|over|menu|clear] [--deluxe]`.
/// Рекорд и сохранённые партии пользователя не читаются и не пишутся.
#[cfg(not(target_os = "android"))]
pub fn screenshot(args: &[String]) -> std::result::Result<(), String> {
    let mut it = args.iter();
    let out = it.next().cloned().ok_or("--screenshot ФАЙЛ.png")?;
    let mut size = (WINDOW_W, WINDOW_H + 42);
    let mut scale = 2.0f64;
    let mut page = String::from("start");
    let mut deluxe = false;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = it.next().cloned().unwrap_or(page),
            "--deluxe" => deluxe = true,
            _ => {}
        }
    }
    // Демо-состояние читает переменные окружения; данные — во временном каталоге.
    std::env::set_var("SYN_BLOCKS_DEMO", &page);
    if deluxe {
        std::env::set_var("SYN_BLOCKS_VARIANT", "deluxe");
    }
    std::env::set_var("SYN_BLOCKS_DATA", std::env::temp_dir().join(format!("syn-blocks-shot-{}", std::process::id())));
    init_i18n();
    let r = syngui::shot::render_png(
        || {
            // придуманные рекорды для стартовой страницы
            let stats = persist::Stats { best: 2654, games: 12, best_by: [[0, 2654, 0], [0, 1830, 0]], ..Default::default() };
            let ctx = GameCtx::new(2027, stats);
            ctx.screen.set(if page == "start" { Screen::Start } else { Screen::Game });
            ctx.variant.set(if deluxe { Variant::Deluxe } else { Variant::Standard });
            provide_context(ctx);
            ui::chrome::root(ctx)
        },
        &ui::chrome::mss(),
        size,
        scale,
        &out,
    )?;
    println!("{out}: {}x{}", r.0, r.1);
    Ok(())
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
