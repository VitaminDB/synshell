//! syn-video-editor — ИИ-видеоредактор synshell.
//!
//!   syn-video-editor [ПРОЕКТ.syn]
//!   syn-video-editor [-p ПРОЕКТ.syn] --cmd ID 'JSON' [--cmd …]   команды без окна (проект сохраняется)
//!   syn-video-editor [-p ПРОЕКТ.syn] --ai "задача словами"       встроенный агент без окна
//!   syn-video-editor [-p ПРОЕКТ.syn] --dump                      таймлайн текстом
//!   syn-video-editor --commands [--json]                         список команд (и схемы)

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use syn_video_editor::{agent, cmd, editor::Editor, store};

fn usage() -> &'static str {
    "syn-video-editor [ПРОЕКТ.syn]\n\
     syn-video-editor [-p ПРОЕКТ.syn] --cmd ID 'JSON' [--cmd …]\n\
     syn-video-editor [-p ПРОЕКТ.syn] --ai \"задача словами\" [--yes]\n\
     syn-video-editor [-p ПРОЕКТ.syn] --dump\n\
     syn-video-editor --commands [--json]\n\
     syn-video-editor --remote ID 'JSON' | --remote-state | --remote-ai \"задача\"   (в запущенное окно, сокет)"
}

fn main() {
    // дочерний процесс рендера (экспорт, кадр): см. render::run_child
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("--preview-strips") {
        match syn_video_editor::render::preview::render_all() {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    // клиент сокета запущенного окна (E5)
    if let Some(first) = raw.first().map(String::as_str).filter(|f| f.starts_with("--remote")) {
        let req = match first {
            "--remote" => {
                let id = raw.get(1).cloned().unwrap_or_default();
                let args: serde_json::Value = raw.get(2).map(|a| serde_json::from_str(a).unwrap_or_else(|e| {
                    eprintln!("JSON: {e}");
                    std::process::exit(2);
                })).unwrap_or(serde_json::Value::Null);
                serde_json::json!({"cmd": id, "args": args})
            }
            "--remote-state" => serde_json::json!({"op": "state"}),
            "--remote-ai" => serde_json::json!({"ai": raw.get(1).cloned().unwrap_or_default()}),
            _ => {
                eprintln!("{}", usage());
                std::process::exit(2);
            }
        };
        match syn_video_editor::ui::socket::request(&req) {
            Ok(v) if v["ok"].as_bool() == Some(true) => {
                if let Some(t) = v["text"].as_str().filter(|t| !t.is_empty()) {
                    println!("{t}");
                }
                if !v["data"].is_null() && raw.iter().any(|a| a == "--json") {
                    println!("{}", serde_json::to_string_pretty(&v["data"]).unwrap_or_default());
                }
            }
            Ok(v) => {
                eprintln!("{}", v["error"].as_str().unwrap_or("ошибка"));
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if raw.first().map(String::as_str) == Some("--render-child") {
        match syn_video_editor::render::child_main(&raw[1..]) {
            Ok(s) => {
                println!("{s}");
                return;
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_video_editor=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    let mut project: Option<PathBuf> = None;
    let mut cmds: Vec<(String, String)> = Vec::new();
    let mut ai: Option<String> = None;
    let mut dump = false;
    let mut commands = false;
    let mut json = false;
    let mut yes = false;
    let mut shot: Option<(String, (u32, u32))> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return;
            }
            "-p" | "--project" => project = args.next().map(PathBuf::from),
            "--cmd" => {
                let id = args.next().unwrap_or_default();
                let js = args.next().unwrap_or_else(|| "{}".into());
                cmds.push((id, js));
            }
            "--ai" => ai = args.next(),
            "--dump" => dump = true,
            "--commands" => commands = true,
            "--json" => json = true,
            "--yes" => yes = true,
            "--screenshot" => {
                let out = args.next().unwrap_or_else(|| "shot.png".into());
                shot = Some((out, (1600, 1000)));
            }
            "--size" => {
                if let (Some(s), Some((w, h))) = (shot.as_mut(), args.next().as_deref().and_then(|v| v.split_once('x')).map(|(w, h)| (w.to_string(), h.to_string()))) {
                    s.1 = (w.parse().unwrap_or(1600), h.parse().unwrap_or(1000));
                }
            }
            s if !s.starts_with('-') && project.is_none() => project = Some(PathBuf::from(s)),
            s => {
                eprintln!("неизвестный ключ {s}\n{}", usage());
                std::process::exit(2);
            }
        }
    }

    if commands {
        if json {
            let v: Vec<Value> = cmd::all().iter().map(|c| serde_json::json!({"id": c.id, "category": c.category, "title": c.title, "shortcut": c.shortcut, "help": c.help, "danger": c.danger, "args": c.schema()})).collect();
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        } else {
            print!("{}", cmd::help_text());
        }
        return;
    }

    let headless = !cmds.is_empty() || ai.is_some() || dump;
    if !headless {
        gui(project, shot);
        return;
    }

    let mut ed = Editor::new();
    if let Some(p) = &project {
        if p.exists() {
            match store::load(p) {
                Ok(pr) => ed.replace(pr, Some(p.clone())),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        } else {
            ed.path = Some(p.clone());
            if let Some(stem) = p.file_stem() {
                ed.project.name = stem.to_string_lossy().into_owned();
            }
        }
    }

    let mut failed = false;
    for (id, js) in &cmds {
        let args: Value = match serde_json::from_str(js) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{id}: аргументы не JSON: {e}");
                std::process::exit(2);
            }
        };
        match cmd::run(&mut ed, id, &args) {
            Ok(o) => println!("{}", o.text),
            Err(e) => {
                eprintln!("{id}: {e}");
                failed = true;
                break;
            }
        }
    }

    let shared = Arc::new(Mutex::new(ed));
    if let Some(task) = ai {
        use std::io::Write;
        use syn_assist::agent::AgentEvent;
        let a = agent::agent(&shared);
        let mut chat = syn_assist::chat::Chat::new();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let r = agent::run_turn(&a, &shared, &mut chat, &task, &agent::AutoConfirm(yes), &cancel, &mut |ev| {
            match ev {
                AgentEvent::Text(t) => print!("{t}"),
                AgentEvent::ToolStart { name, summary } => eprint!("\n\x1b[33m→ {name}: {summary}\x1b[0m"),
                AgentEvent::Message(m) if m.role == syn_assist::chat::Role::Tool => {
                    let first: String = m.text.lines().take(4).collect::<Vec<_>>().join("\n  ");
                    eprint!("\n\x1b[36m  {first}\x1b[0m\n");
                }
                _ => {}
            }
            let _ = std::io::stdout().flush();
        });
        println!();
        if let Err(e) = r {
            eprintln!("ошибка агента: {e}");
            failed = true;
        }
    }

    let mut ed = shared.lock().unwrap();
    if dump {
        println!("{}", ed.project.dump());
    }
    if ed.dirty && ed.path.is_some() {
        match cmd::run(&mut ed, "project.save", &Value::Null) {
            Ok(o) => eprintln!("{}", o.text),
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

/// Окно редактора (или его снимок без экрана: `--screenshot файл.png [--size ШxВ]`).
fn gui(project: Option<PathBuf>, shot: Option<(String, (u32, u32))>) {
    use syn_video_editor::ui;
    use syngui::prelude::*;
    synshell_common::i18n::init(&[]);
    // без файла в аргументах — последний проект или автосохранение несохранённого
    if project.is_none() && shot.is_none() {
        let mut e = ui::ed();
        if let Some(p) = store::last_project() {
            if let Ok(pr) = store::load(&p) {
                e.replace(pr, Some(p));
            }
        } else if let Ok(pr) = store::load(&store::autosave_path()) {
            e.replace(pr, None);
            e.dirty = true;
        }
    }
    if let Some(p) = &project {
        let mut e = ui::ed();
        match store::load(p) {
            Ok(pr) => e.replace(pr, Some(p.clone())),
            Err(err) if p.exists() => eprintln!("{err}"),
            Err(_) => e.path = Some(p.clone()),
        }
    }
    ui::graph::init();
    let (cfg, _) = synshell_common::Config::load();
    let mss = ui::theme(&cfg);
    if let Some((out, size)) = shot {
        let r = syngui::shot::render_png(
            || {
                let view = std::env::var("SYN_VIDEO_EDITOR_VIEW").unwrap_or_else(|_| syn_video_editor::layout::DEFAULT_VIEW.to_string());
                let st = ui::St::new_ephemeral(&view);
                ui::graph::mount(st);
                if let Ok(sel) = std::env::var("SYN_VIDEO_EDITOR_SELECT") {
                    let ids: Vec<&str> = sel.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
                    st.exec("selection.set", serde_json::json!({"ids": ids}));
                }
                if let Ok(at) = std::env::var("SYN_VIDEO_EDITOR_AT") {
                    st.exec("playhead.set", serde_json::json!({"at": at}));
                }
                st.toast.set(None);
                Box::new(ui::root(st)) as Box<dyn Widget>
            },
            &mss,
            size,
            1.0,
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
    let st = ui::St::new();
    st.rev.set(ui::ed().revision);
    st.theme.set(mss.clone());
    synshell_common::live::bind(st.theme, mss, ui::theme);
    App::new()
        .title("Видеоредактор")
        .app_id("syn-video-editor")
        .size(1600, 1000)
        .min_size(900, 600)
        .maximized(std::env::var("SYN_VIDEO_EDITOR_MAXIMIZED").map(|v| v != "0").unwrap_or(true))
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(st.theme)
        .with_window_state(st.window)
        .on_close_request(move || {
            if let Some(spec) = st.dock.spec() {
                syn_video_editor::layout::save_current(&spec, &st.view.get_untracked());
            }
            true
        })
        .run(move |_| {
            ui::graph::mount(st);
            ui::socket::start(st);
            syn_video_editor::layout::autosave(st.dock, st.view);
            ui::autosave(st);
            Box::new(ui::root(st)) as Box<dyn Widget>
        });
}
