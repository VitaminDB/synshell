//! syn-themeeditor — редактор тем synshell.
//!
//!   syn-themeeditor [ТЕМА] [--new [ОБРАЗЕЦ]] [--import ФАЙЛ.syntheme]
//!                   [--screenshot out.png [--size 1400x900] [--scale 1] [--mode quick|palette|shape|code|about]
//!                    [--scene desktop|settings] [--light]]
//!   syn-themeeditor --mss-context [ФАЙЛ]   контекст для mss-lsp (переменные темы) — в файл или stdout
//!
//! Тема — каталог `~/.config/synshell/themes/<id>/` (`theme.toml` и MSS-слои).
//! Встроенная тема при правке сохраняется своей копией с тем же именем
//! (заменяет встроенную; «Удалить» возвращает встроенную).

mod color;
mod draft;
mod files;
mod icons;
mod preview_style;
mod scope;
mod state;
mod ui;

use syngui::prelude::*;
use synshell_common::config::Rgba;
use synshell_common::style::{px, AppStyle};
use synshell_common::{t, Config};

use state::{Mode, Scene, St};

/// Стили самого окна редактора (тема системы).
pub fn editor_styles(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let titlebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.22) } else { p.surface_alt.mix(p.bg, 0.5) };
    let sidebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.12) } else { p.surface_alt.mix(p.bg, 0.3) };
    let content = if dark { p.bg.mix(p.surface, 0.12) } else { p.bg.mix(p.surface, 0.4) };
    let card = if dark { p.surface.with_alpha(0.6) } else { p.surface };
    AppStyle::new(cfg, "syn-themeeditor")
        .vars(&[
            ("titlebar", titlebar.hex()),
            ("sidebar", sidebar.hex()),
            ("content", content.hex()),
            ("card", card.hex()),
            ("input-bg", if dark { p.bg.mix(p.surface, 0.5).hex() } else { p.surface.hex() }),
            ("divider", p.border.fade(0.6).hex()),
            ("checker-a", if dark { "#3a3d44".into() } else { "#e4e6ea".into() }),
            ("checker-b", if dark { "#2a2c31".into() } else { "#f6f7f9".into() }),
            ("window-radius", px(cfg.decorations.corner_radius.max(0.0))),
            ("shadow", if dark { "#00000080".into() } else { "#1a203024".into() }),
            ("scrim", p.bg.with_alpha(0.7).hex()),
        ])
        .base(include_str!("../styles/editor.mss"))
        .build()
}

struct Args {
    theme: Option<String>,
    new: Option<Option<String>>,
    import: Option<String>,
    screenshot: Option<String>,
    size: (u32, u32),
    scale: f64,
    mode: Mode,
    scene: Scene,
    light: bool,
}

fn parse_args() -> Args {
    let mut a = Args { theme: None, new: None, import: None, screenshot: None, size: (1440, 900), scale: 1.0, mode: Mode::Quick, scene: Scene::Desktop, light: false };
    let mut it = std::env::args().skip(1).peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--new" => a.new = Some(it.next_if(|n| !n.starts_with('-'))),
            "--import" => a.import = it.next(),
            "--screenshot" => a.screenshot = it.next(),
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    a.size = (w.parse().unwrap_or(1440), h.parse().unwrap_or(900));
                }
            }
            "--scale" => a.scale = it.next().and_then(|s| s.parse().ok()).unwrap_or(1.0),
            "--mode" => {
                a.mode = match it.next().as_deref() {
                    Some("palette") => Mode::Palette,
                    Some("shape") => Mode::Shape,
                    Some("code") => Mode::Code,
                    Some("about") => Mode::About,
                    _ => Mode::Quick,
                }
            }
            "--scene" => a.scene = if it.next().as_deref() == Some("settings") { Scene::Settings } else { Scene::Desktop },
            "--light" => a.light = true,
            "--mss-context" => {
                let json = synshell_common::tokens::mss_context_json(&Config::load().0.appearance);
                let text = serde_json_pretty(&json);
                match it.next() {
                    Some(f) => {
                        if let Err(e) = std::fs::write(&f, text) {
                            eprintln!("{f}: {e}");
                            std::process::exit(1);
                        }
                    }
                    None => println!("{text}"),
                }
                std::process::exit(0);
            }
            "-h" | "--help" => {
                println!("syn-themeeditor [ТЕМА] [--new [ОБРАЗЕЦ]] [--import ФАЙЛ] [--mss-context [ФАЙЛ]] [--screenshot out.png [--size WxH] [--mode m] [--scene s] [--light]]");
                std::process::exit(0);
            }
            other if !other.starts_with('-') => a.theme = Some(other.to_string()),
            _ => {}
        }
    }
    a
}

fn serde_json_pretty(v: &serde_json::Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// Подготовить состояние: тема из аргументов, иначе активная в системе.
fn init(st: St, args: &Args, cfg: &Config) {
    st.mode.set(args.mode);
    st.scene.set(args.scene);
    st.reload_list();
    if let Some(path) = &args.import {
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| draft::Draft::import_bundle(&t)) {
            Ok(d) => ui::import_draft(st, d),
            Err(e) => st.toast.set(t!("Не удалось импортировать: {e}", e = e)),
        }
        return;
    }
    if let Some(base) = &args.new {
        st.create(base.as_deref().or(Some(cfg.appearance.theme.as_str())).filter(|s| !s.is_empty()), &t!("Моя тема"));
        return;
    }
    let id = args
        .theme
        .clone()
        .or_else(|| Some(cfg.appearance.theme.clone()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| synshell_common::theme::builtin_ids().next().unwrap_or("nord").to_string());
    st.open(&id);
    if args.light {
        st.set_variant("light");
    }
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_themeeditor=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    // Само окно редактора — в теме системы, а не в примеряемом черновике.
    std::env::set_var("SYNSHELL_NO_THEME_PREVIEW", "1");
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let args = parse_args();
    let (cfg, _) = Config::load();

    if let Some(out) = args.screenshot.clone() {
        // Снимок: стили собираются синхронно, без фоновых потоков.
        let st = St::new();
        init(st, &args, &cfg);
        let mss = state::with_draft(|d| d.to_theme()).map(|t| {
            let pc = preview_style::preview_config(&cfg, &t, st.dark());
            let mut s = editor_styles(&cfg);
            s.push_str(&preview_style::build(&pc));
            s
        });
        let mss = mss.unwrap_or_else(|_| editor_styles(&cfg));
        let r = syngui::shot::render_png(move || ui::root(st), &mss, args.size, args.scale, &out);
        match r {
            Ok((w, h)) => println!("{out}: {w}x{h}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let _guard = state::TryonGuard;
    let st = St::new();
    st.styles.set(editor_styles(&cfg));
    init(st, &args, &cfg);
    // Тема системы поменялась (в том числе примеркой) — окно редактора тоже.
    synshell_common::live::spawn(Vec::new(), move |_| run_on_main_thread(move || st.rebuild_styles()));
    App::new()
        .title(t!("Редактор тем"))
        .app_id("syn-themeeditor")
        .size(args.size.0, args.size.1)
        .min_size(720, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(st.styles)
        .with_window_state(st.window)
        .run(move |_| ui::root(st));
    synshell_common::preview::clear();
}
