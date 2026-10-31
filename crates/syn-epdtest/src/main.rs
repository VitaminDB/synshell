//! syn-epdtest — «Тест экрана E Ink» и редактор профилей экрана (драйвер onyx-tcon-fpga,
//! ONYX BOOX). Сверху — профили (`synshell_common::eink_profiles`: выбор — сразу применить,
//! «Новый», «Переименовать», «Удалить»), редактор выбранного (режим волны `scheme` ui/auto,
//! автоочистка, A2, контраст, насыщенность — сразу в драйвер и в файл профилей), цвет/монохром,
//! полная очистка; вкладки с тестами: серое, цвет, текст, движение, следы, прокрутка.
//!
//!   syn-epdtest [--screenshot СНИМОК.png [--tab gray|color|text|motion|ghost|scroll]
//!               [--profile ИМЯ] [--dialog new|rename|delete] [--size ШxВ] [--scale N]]
//!
//! На компьютере — подставной sysfs: `SYNSHELL_SYSFS=/tmp/sys` (см. `synshell_common::display`),
//! файл профилей — `SYNSHELL_EINK_PROFILES_FILE`.

mod patterns;
mod state;
mod ui;

use syngui::prelude::*;
use synshell_common::Config;

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_epdtest=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut tab = state::Tab::Gray;
    let mut profile: Option<String> = None;
    let mut dialog: Option<state::Dialog> = None;
    let mut size = (1240u32, 930u32);
    let mut scale = 1.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", t!("syn-epdtest [--screenshot СНИМОК.png [--tab gray|color|text|motion|ghost|scroll] [--profile ИМЯ] [--dialog new|rename|delete] [--size ШxВ] [--scale N]]"));
                return;
            }
            "--screenshot" => shot = args.next(),
            "--tab" => tab = args.next().as_deref().and_then(state::Tab::from_name).unwrap_or(tab),
            "--profile" => profile = args.next(),
            "--dialog" => {
                dialog = match args.next().as_deref() {
                    Some("new") => Some(state::Dialog::New),
                    Some("rename") => Some(state::Dialog::Rename),
                    Some("delete") => Some(state::Dialog::Delete),
                    _ => None,
                }
            }
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            _ => {}
        }
    }
    set_dark_theme(false);
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(
            move || {
                let st = state::St::new(false);
                st.tab.set(tab);
                if let Some(p) = &profile {
                    st.select(p);
                }
                if let Some(d) = dialog {
                    st.open_dialog(d);
                }
                if tab == state::Tab::Motion {
                    st.tick.set(137);
                    st.stamp_ms.set(13_700);
                    st.rate.set(state::Rate { ticks: 10.0, drawn: 9.8 });
                }
                if tab == state::Tab::Ghost {
                    st.ghost_count.set(3);
                    st.ghost_inv.set(true);
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
    let fullscreen = state::fullscreen_saved();
    let st = state::St::new(true);
    // значения драйвера могли поменять другие (шторка, настройки) — перечитывать раз в 2 с
    // (сигнал меняется только при отличии: лишней перерисовки нет)
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        run_on_main_thread(move || st.reload());
    });
    App::new()
        .title(t!("Профили экрана E Ink"))
        .app_id("syn-epdtest")
        .size(1240, 900)
        .min_size(480, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .fullscreen(fullscreen)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss, theme))
        .with_window_state(st.window)
        .run(move |_| ui::root(st));
}

/// Светлая тема всегда: тест судит о белом и чёрном панели, палитра оболочки не нужна —
/// от неё только шрифт интерфейса и пользовательский слой.
fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "syn-epdtest").font_selector("Text, Button, TextField").base(include_str!("../styles/syn-epdtest.mss")).build()
}
