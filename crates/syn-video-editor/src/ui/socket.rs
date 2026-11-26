//! Сокет команд (E5): запущенное окно принимает команды извне — Claude Code,
//! synlink, сценарии. `$XDG_RUNTIME_DIR/syn-video-editor.sock`, по строке
//! JSON на запрос и ответ:
//! - `{"cmd": "clip.add", "args": {…}}` → `{"ok": true, "text": …, "data": …}`;
//! - `{"op": "state"}` — проект текстом, выделение, позиция, файл;
//! - `{"op": "commands"}` — таблица команд со схемами;
//! - `{"ai": "задача словами"}` — ход встроенного агента в панели ИИ.
//!
//! Команды идут в окне той же таблицей, что меню и агент: шаг отмены,
//! обновление таймлайна и просмотра.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use super::{ed, St};
use crate::cmd;

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_VIDEO_EDITOR_SOCKET") {
        return p.into();
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("syn-video-editor.sock")
}

/// Слушать сокет (окно уже есть). Занят живым окном — второе не слушает.
pub fn start(st: St) {
    let p = path();
    if UnixStream::connect(&p).is_ok() {
        tracing::info!("сокет {} занят другим окном", p.display());
        return;
    }
    let _ = std::fs::remove_file(&p);
    let l = match UnixListener::bind(&p) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("сокет {}: {e}", p.display());
            return;
        }
    };
    tracing::info!("сокет команд: {}", p.display());
    let _ = std::thread::Builder::new().name("ve-socket".into()).spawn(move || {
        for c in l.incoming().flatten() {
            let _ = std::thread::Builder::new().name("ve-socket-conn".into()).spawn(move || serve(st, c));
        }
    });
}

fn serve(st: St, c: UnixStream) {
    let Ok(w) = c.try_clone() else { return };
    let mut w = w;
    for line in BufReader::new(c).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => handle(st, req),
            Err(e) => json!({"ok": false, "error": format!("JSON: {e}")}),
        };
        if writeln!(w, "{reply}").is_err() {
            break;
        }
    }
}

/// Выполнить на главном потоке и дождаться ответа.
fn on_main(f: impl FnOnce() -> Value + Send + 'static) -> Value {
    let (tx, rx) = std::sync::mpsc::channel();
    syngui::async_runtime::run_on_main_thread(move || {
        let _ = tx.send(f());
    });
    // долгие команды (экспорт, рецепт) — до часа
    rx.recv_timeout(Duration::from_secs(3600)).unwrap_or_else(|_| json!({"ok": false, "error": "окно не ответило"}))
}

fn handle(st: St, req: Value) -> Value {
    if let Some(id) = req.get("cmd").and_then(Value::as_str) {
        let id = id.to_string();
        let args = req.get("args").cloned().unwrap_or(Value::Null);
        return on_main(move || {
            let r = {
                let mut e = ed();
                let r = cmd::run(&mut e, &id, &args);
                st.rev.set(e.revision);
                st.ph.set(e.playhead);
                r
            };
            match r {
                Ok(o) => json!({"ok": true, "text": o.text, "data": o.data}),
                Err(e) => json!({"ok": false, "error": e}),
            }
        });
    }
    if let Some(text) = req.get("ai").and_then(Value::as_str) {
        let text = text.to_string();
        return on_main(move || {
            if st.ai_busy.get_untracked() {
                return json!({"ok": false, "error": "агент занят"});
            }
            st.open_panel("ai");
            st.ask_ai(text);
            json!({"ok": true, "text": "передано агенту (панель ИИ)"})
        });
    }
    match req.get("op").and_then(Value::as_str) {
        Some("state") => on_main(|| {
            let e = ed();
            json!({"ok": true, "text": e.project.dump(), "data": {
                "path": e.path.as_ref().map(|p| p.display().to_string()),
                "dirty": e.dirty,
                "selection": e.selection,
                "playhead": e.playhead,
                "duration": e.project.duration(),
            }})
        }),
        // ход агента: занят ли и что он написал в последнем ходе (для автоматических прогонов)
        Some("ai_state") => on_main(move || {
            let log = st.ai_log.get_untracked();
            let start = log.iter().rposition(|l| matches!(l, super::AiLine::User(_))).unwrap_or(0);
            let mut text = String::new();
            let mut tools = Vec::new();
            for l in &log[start..] {
                match l {
                    super::AiLine::Text(t) => text.push_str(t),
                    super::AiLine::Tool(t) => tools.push(Value::from(t.clone())),
                    super::AiLine::Error(e) => {
                        text.push_str("\nОшибка: ");
                        text.push_str(e);
                    }
                    _ => {}
                }
            }
            json!({"ok": true, "text": text, "data": {"busy": st.ai_busy.get_untracked(), "tools": tools}})
        }),
        Some("commands") => {
            let v: Vec<Value> = cmd::all().iter().map(|c| json!({"id": c.id, "category": c.category, "title": c.title, "help": c.help, "args": c.schema()})).collect();
            json!({"ok": true, "data": v})
        }
        _ => json!({"ok": false, "error": "запрос: {\"cmd\": id, \"args\": {…}} | {\"op\": \"state\"|\"commands\"} | {\"ai\": текст}"}),
    }
}

/// Клиент (CLI `--remote`): один запрос — ответ.
pub fn request(req: &Value) -> Result<Value, String> {
    let p = path();
    let mut c = UnixStream::connect(&p).map_err(|e| format!("{}: окно не запущено ({e})", p.display()))?;
    writeln!(c, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(c).read_line(&mut line).map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| format!("ответ: {e}"))
}
