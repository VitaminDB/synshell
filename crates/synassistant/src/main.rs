mod state;
mod ui;

use syngui::prelude::*;
use synshell_common::Config;

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/synassistant.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "synassistant=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
    synshell_common::i18n::init(&[include_str!("../i18n/en.lang")]);
    let mut shot: Option<String> = None;
    let mut size = (1100u32, 760u32);
    let mut scale = 1.0f64;
    let mut ask: Option<String> = None;
    let mut open_chat: Option<String> = None;
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
            "--cli" => {
                cli(&args.collect::<Vec<_>>().join(" "));
                return;
            }
            "-h" | "--help" => {
                println!("{}", t!("synassistant [--ask ВОПРОС] [--chat ID] [--cli ВОПРОС] [--screenshot СНИМОК.png [--size ШxВ] [--scale N]]"));
                return;
            }
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        let r = syngui::shot::render_png(|| ui::root(state::St::new()), &mss, size, scale, &out);
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
        .title(t!("Ассистент"))
        .app_id("synassistant")
        .size(1100, 760)
        .min_size(340, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = state::St::new();
            if let Some(id) = open_chat.clone() {
                st.open(&id);
            }
            if let Some(q) = ask.clone() {
                st.new_chat();
                st.send(q);
            }
            ui::root(st)
        });
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
