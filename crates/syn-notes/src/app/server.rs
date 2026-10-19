//! Сокет хозяина заметок `$XDG_RUNTIME_DIR/syn-notes.sock`.
//!
//! Одна строка JSON — запрос, одна строка JSON — ответ:
//!
//! ```text
//! {"cmd":"ping"}                          → {"ok":true}
//! {"cmd":"show"}                          — показать окно
//! {"cmd":"open","target":"Страница"}      — страница (id, название, путь)
//!                                           или файл проекта .syn
//! {"cmd":"tool","args":{…}}               — инструмент `notes` ассистента:
//!                                           {"ok":true,"text":"…"} / {"ok":false,"error":"…"}
//! ```
//!
//! Исполнение — на главном потоке, в живом состоянии приложения: правки
//! ассистента сразу видны в открытом редакторе и уходят на диск
//! автосохранением, как правки пользователя.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use syngui::async_runtime::run_on_main_thread;
use syngui::context_provider::use_context;

use crate::notes::NotesCtx;

#[derive(Clone, Debug)]
pub enum Command {
    Ping,
    Show,
    Open(String),
}

/// `SYN_NOTES_SOCKET` — другой путь (проверка рядом с работающим хозяином).
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_NOTES_SOCKET") {
        return PathBuf::from(p);
    }
    synshell_common::paths::runtime_dir().join("syn-notes.sock")
}

/// Передать команду работающему хозяину. Ошибка — хозяина нет.
pub fn send(cmd: &Command) -> std::io::Result<()> {
    let req = match cmd {
        Command::Ping => json!({"cmd": "ping"}),
        Command::Show => json!({"cmd": "show"}),
        Command::Open(t) => json!({"cmd": "open", "target": t}),
    };
    request(&req, Duration::from_secs(5)).map(|_| ())
}

/// Запрос к хозяину: строка JSON туда, строка JSON обратно.
pub fn request(req: &Value, timeout: Duration) -> std::io::Result<Value> {
    let mut s = UnixStream::connect(socket_path())?;
    s.set_read_timeout(Some(timeout))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    serde_json::from_str(&reply).map_err(std::io::Error::other)
}

/// Команда запуска (на главном потоке, до цикла событий или из сокета).
pub fn handle(cmd: Command) {
    match cmd {
        Command::Ping => {}
        Command::Show => {
            syngui::app::show_main_window();
        }
        Command::Open(target) => {
            let _ = open(&target);
            syngui::app::show_main_window();
        }
    }
}

/// Открыть проект (`.syn`) или страницу (id, название или путь в дереве).
fn open(target: &str) -> Result<(), String> {
    let ctx = use_context::<NotesCtx>();
    let target = target.trim();
    if target.is_empty() {
        return Ok(());
    }
    let path = synshell_common::paths::expand_tilde(target);
    if target.ends_with(".syn") {
        return match ctx.open_project(&path) {
            Ok(_) => Ok(()),
            Err(e) => {
                crate::notes::project_ui::report_error(&e);
                Err(e.message())
            }
        };
    }
    if ctx.open_page_anywhere(target) {
        return Ok(());
    }
    // Название или путь — через инструмент, он разбирает адресацию.
    crate::tool::dispatch_now(&json!({"action": "open", "page": target})).map(|_| ())
}

/// Слушать сокет в фоне. Занятый чужим живым процессом сокет не трогаем.
pub fn start() {
    let path = socket_path();
    if UnixStream::connect(&path).is_ok() {
        tracing::warn!("{}: хозяин заметок уже работает", path.display());
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("сокет заметок {}: {e}", path.display());
            return;
        }
    };
    std::thread::Builder::new()
        .name("notes-socket".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                std::thread::spawn(move || serve(conn));
            }
        })
        .ok();
}

fn serve(conn: UnixStream) {
    let Ok(read) = conn.try_clone() else { return };
    let mut reader = BufReader::new(read);
    let mut out = conn;
    let mut line = String::new();
    while reader.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => answer(req),
            Err(e) => json!({"ok": false, "error": format!("bad request: {e}")}),
        };
        let mut text = reply.to_string();
        text.push('\n');
        if out.write_all(text.as_bytes()).is_err() {
            break;
        }
        line.clear();
    }
}

/// Выполнить запрос на главном потоке и дождаться ответа.
fn on_main<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    run_on_main_thread(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(120)).ok()
}

fn answer(req: Value) -> Value {
    let cmd = req.get("cmd").and_then(Value::as_str).unwrap_or_default().to_string();
    match cmd.as_str() {
        "ping" => json!({"ok": true}),
        "show" => {
            on_main(|| handle(Command::Show));
            json!({"ok": true})
        }
        "open" => {
            let target = req.get("target").and_then(Value::as_str).unwrap_or_default().to_string();
            match on_main(move || {
                let r = open(&target);
                syngui::app::show_main_window();
                r
            }) {
                Some(Ok(())) => json!({"ok": true}),
                Some(Err(e)) => json!({"ok": false, "error": e}),
                None => json!({"ok": false, "error": "timeout"}),
            }
        }
        "tool" => {
            let args = req.get("args").cloned().unwrap_or(Value::Null);
            match on_main(move || crate::tool::dispatch_now(&args)) {
                Some(Ok(text)) => json!({"ok": true, "text": text}),
                Some(Err(e)) => json!({"ok": false, "error": e}),
                None => json!({"ok": false, "error": "timeout"}),
            }
        }
        other => json!({"ok": false, "error": format!("unknown cmd: {other}")}),
    }
}
