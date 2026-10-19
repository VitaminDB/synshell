//! Протокол заметок synshell. Хозяин проектов — процесс `syn-notes`
//! (см. `docs/NOTES.md`); ассистент, рабочий стол и команды запуска говорят
//! с ним через сокет `$XDG_RUNTIME_DIR/syn-notes.sock`: одна строка JSON —
//! запрос, одна строка JSON — ответ.
//!
//! ```text
//! {"cmd":"ping"}                       → {"ok":true}
//! {"cmd":"show"}                       — показать окно
//! {"cmd":"open","target":"…"}          — страница (id, название, путь) или файл .syn
//! {"cmd":"tool","args":{…}}            — инструмент `notes` → {"ok":true,"text":"…"}
//! ```
//!
//! Крейт без интерфейса и тяжёлых зависимостей — его подключают клиенты.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub mod schema;

/// `SYN_NOTES_SOCKET` — другой путь (проверка рядом с работающим хозяином).
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_NOTES_SOCKET") {
        return PathBuf::from(p);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe_uid())));
    dir.join("syn-notes.sock")
}

fn unsafe_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(1000)
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

/// Работает ли хозяин.
pub fn running() -> bool {
    request(&json!({"cmd": "ping"}), Duration::from_secs(2)).is_ok()
}

/// Хозяин работает — или запущен сейчас (`syn-notes --background`) и
/// поднял сокет за `wait`.
pub fn ensure_running(wait: Duration) -> Result<(), String> {
    if running() {
        return Ok(());
    }
    std::process::Command::new("syn-notes")
        .arg("--background")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("не удалось запустить syn-notes: {e}"))?;
    let start = Instant::now();
    while start.elapsed() < wait {
        std::thread::sleep(Duration::from_millis(200));
        if running() {
            return Ok(());
        }
    }
    Err("syn-notes не ответил".into())
}

/// Вызов инструмента `notes`: аргументы как у модели, ответ — текст.
pub fn tool(args: &Value) -> Result<String, String> {
    ensure_running(Duration::from_secs(20))?;
    let reply = request(&json!({"cmd": "tool", "args": args}), Duration::from_secs(180))
        .map_err(|e| format!("заметки не ответили: {e}"))?;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply.get("text").and_then(Value::as_str).unwrap_or_default().to_string())
    } else {
        Err(reply.get("error").and_then(Value::as_str).unwrap_or("ошибка заметок").to_string())
    }
}

/// Показать страницу (или проект `.syn`) пользователю.
pub fn open(target: &str) -> Result<(), String> {
    ensure_running(Duration::from_secs(20))?;
    let reply = request(&json!({"cmd": "open", "target": target}), Duration::from_secs(30)).map_err(|e| e.to_string())?;
    match reply.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(()),
        _ => Err(reply.get("error").and_then(Value::as_str).unwrap_or("не открылось").to_string()),
    }
}

/// Сводка для рабочего стола: повестка, доски, разделы страницы стола
/// (формат — `syn-notes/src/app/desk.rs`). Хозяина не запускает: стол
/// опрашивает его часто, а стартует хозяин с сеансом.
pub fn desk(page: Option<&str>) -> Result<Value, String> {
    let reply = request(&json!({"cmd": "desk", "page": page}), Duration::from_secs(10)).map_err(|e| e.to_string())?;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply)
    } else {
        Err(reply.get("error").and_then(Value::as_str).unwrap_or("нет данных").to_string())
    }
}
