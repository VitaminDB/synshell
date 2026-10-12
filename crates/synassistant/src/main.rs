mod icons;
mod state;
mod ui;
mod voice;

use syngui::prelude::*;
use synshell_common::config::Rgba;
use synshell_common::Config;

pub fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    let mut s = a.mss_variables();
    let titlebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.22) } else { p.surface_alt.mix(p.bg, 0.5) };
    let sidebar = if dark { p.bg.mix(Rgba::rgb(0, 0, 0), 0.12) } else { p.surface_alt.mix(p.bg, 0.3) };
    let content = if dark { p.bg.mix(p.surface, 0.12) } else { p.bg.mix(p.surface, 0.4) };
    let field = if dark { p.bg.mix(p.surface, 0.7) } else { p.surface };
    let accent2 = p.accent.mix(Rgba::rgb(0x8b, 0x5c, 0xf6), 0.55);
    let accent3 = p.accent.mix(Rgba::rgb(0x22, 0xd3, 0xee), 0.45);
    s.push_str(&format!(
        ":root {{\n  --titlebar: {};\n  --sidebar: {};\n  --content: {};\n  --field: {};\n  --accent-2: {};\n  --accent-3: {};\n  --accent-glow: {};\n  --accent-dim: {};\n  --card: {};\n  --card-hover: {};\n  --divider: {};\n  --shadow: {};\n  --danger-soft: {};\n  --scrim: {};\n  --window-radius: {}px;\n}}\n",
        titlebar.hex(),
        sidebar.hex(),
        content.hex(),
        field.hex(),
        accent2.hex(),
        accent3.hex(),
        p.accent.with_alpha(0.35).hex(),
        p.accent.with_alpha(0.12).hex(),
        if dark { p.surface.with_alpha(0.55).hex() } else { p.surface.hex() },
        if dark { p.surface.with_alpha(0.85).hex() } else { p.surface.mix(p.bg, 0.3).hex() },
        p.border.fade(0.6).hex(),
        if dark { "#00000080" } else { "#1a203024" },
        p.danger.with_alpha(0.16).hex(),
        p.bg.with_alpha(0.86).hex(),
        cfg.decorations.corner_radius.max(0.0),
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/synassistant.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

fn watch_config(st: state::St) {
    std::thread::spawn(move || {
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let files = || {
            let mut v = vec![synshell_common::paths::config_file(), synshell_common::paths::user_theme_file()];
            v.extend(Config::load().0.appearance.theme_files());
            v
        };
        let mut watched = files();
        let mut stamps: Vec<_> = watched.iter().map(|p| mtime(p)).collect();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let now: Vec<_> = watched.iter().map(|p| mtime(p)).collect();
            if now != stamps {
                watched = files();
                stamps = watched.iter().map(|p| mtime(p)).collect();
                let (cfg, err) = Config::load();
                if err.is_none() {
                    let mss = theme(&cfg);
                    run_on_main_thread(move || {
                        st.theme.set(mss);
                        st.config_changed(cfg);
                    });
                }
            }
        }
    });
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "synassistant=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (1180u32, 780u32);
    let mut scale = 1.0f64;
    let mut ask: Option<String> = None;
    let mut open_chat: Option<String> = None;
    let mut voice_mode = false;
    let mut fresh = false;
    let mut shot_voice = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--ask" => ask = args.next(),
            "--chat" => open_chat = args.next(),
            "--voice" => voice_mode = true,
            "--new" => fresh = true,
            "--shot-voice" => shot_voice = true,
            "--cli" => {
                cli(&args.collect::<Vec<_>>().join(" "));
                return;
            }
            "-h" | "--help" => {
                println!("{}", t!("synassistant [--ask ВОПРОС] [--chat ID] [--voice] [--cli ВОПРОС] [--screenshot СНИМОК.png [--size ШxВ] [--scale N]]"));
                return;
            }
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(
            || {
                let st = state::St::new(&cfg);
                if fresh {
                    st.new_chat();
                }
                if shot_voice {
                    st.voice.open.set(true);
                    st.voice.phase.set(state::Phase::Listening);
                    st.voice.heard.set(t!("Какая погода сегодня?").to_string());
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
    let st = state::St::new(&cfg);
    st.theme.set(mss.clone());
    watch_config(st);
    App::new()
        .title(t!("Ассистент"))
        .app_id("synassistant")
        .size(size.0, size.1)
        .min_size(360, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .with_dynamic_theme(st.theme)
        .with_window_state(st.window)
        .on_close_request(move || {
            voice::shutdown();
            true
        })
        .run(move |_| {
            st.start_background();
            if let Some(id) = open_chat.clone() {
                st.open(&id);
            }
            if fresh {
                st.new_chat();
            }
            if let Some(q) = ask.clone() {
                st.new_chat();
                st.send(q);
            }
            if voice_mode {
                voice::start_conversation(st);
            }
            ui::root(st)
        });
    voice::shutdown();
}

fn cli(question: &str) {
    use std::io::Write;
    use std::sync::Arc;
    use synassist::agent::{Agent, AgentEvent, DaemonBackend};
    let cfg = synshell_common::Config::load().0.assistant;
    let agent = Agent::configured(Arc::new(DaemonBackend), &cfg.disabled_tools, false, cfg.autocompact);
    let mut chat = synassist::chat::Chat::new();
    let confirm = |tool: &str, summary: &str| -> bool {
        print!("\n[{tool}] {summary} — разрешить? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        line.trim().eq_ignore_ascii_case("y")
    };
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let r = agent.run_turn(&mut chat, question, &confirm, &cancel, &mut |ev| {
        match ev {
            AgentEvent::Text(t) => print!("{t}"),
            AgentEvent::Thinking(t) => print!("\x1b[2m{t}\x1b[0m"),
            AgentEvent::ToolStart { name, summary } => print!("\n\x1b[33m→ {name}: {summary}\x1b[0m"),
            AgentEvent::Compacted(_) => print!("\n\x1b[2m[начало разговора сжато в сводку]\x1b[0m"),
            AgentEvent::Message(m) if m.role == synassist::chat::Role::Tool => {
                let first: String = m.text.lines().take(6).collect::<Vec<_>>().join("\n  ");
                print!("\n\x1b[36m  {first}\x1b[0m\n");
            }
            _ => {}
        }
        let _ = std::io::stdout().flush();
    });
    println!();
    if let Err(e) = r {
        eprintln!("ошибка: {e}");
    }
}
