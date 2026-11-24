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
     syn-video-editor --commands [--json]"
}

fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "syn_video_editor=info".into());
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();

    let mut project: Option<PathBuf> = None;
    let mut cmds: Vec<(String, String)> = Vec::new();
    let mut ai: Option<String> = None;
    let mut dump = false;
    let mut commands = false;
    let mut json = false;
    let mut yes = false;
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
        eprintln!("Окно редактора — следующий шаг этапа 0; пока работает режим без окна:\n{}", usage());
        std::process::exit(2);
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
