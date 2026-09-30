//! MCP-сервер (stdio, JSON-RPC 2.0) для Claude Code и других агентов:
//! видеть экран устройства, нажимать, набирать, запускать команды,
//! читать журнал и уведомления. Работает через локальный демон, так что
//! годится и для телефона (по USB/Wi-Fi), и для этой машины (`local`).
//!
//! Подключение: `claude mcp add synlink -- synlink mcp`.

use std::collections::HashMap;
use std::io::{BufRead, Write};

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use synshell_common::link::{Request, Response};

use crate::gestures;

const INSTRUCTIONS: &str = "synlink управляет устройствами synshell (телефон и компьютер, связанные по USB или Wi-Fi). \
Устройство: имя или id из `devices`, `phone`, `desktop` или `local` (эта машина). \
Координаты для tap/long_press/swipe/scroll — пиксели последнего снимка `screenshot` этого устройства \
(снимок может быть уменьшен — пересчёт делается сам). На телефоне ввод идёт касаниями, на компьютере — мышью. \
Системный «назад» телефона — action `back`; домой — action `shell home`. \
После действия делайте новый снимок, чтобы увидеть результат.";

struct Server {
    /// Уменьшение последнего снимка: устройство → (ширина снимка, полная ширина).
    shots: HashMap<String, (f64, f64)>,
}

pub fn run() -> Result<()> {
    let mut srv = Server { shots: HashMap::new() };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else { continue }; // уведомления без ответа
        let reply = match srv.dispatch(method, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32603, "message": format!("{e:#}")}}),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}

fn schema(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required})
}

fn tools() -> Value {
    let dev = json!({"type": "string", "description": "устройство: имя, id, phone, desktop или local"});
    let num = |d: &str| json!({"type": "number", "description": d});
    json!([
        {"name": "devices", "description": "Эта машина, состояние кабеля USB и устройства: соединены ли, чем (USB/Wi-Fi), батарея, где файлы, ssh-имя.",
         "inputSchema": schema(json!({}), &[])},
        {"name": "screenshot", "description": "Снимок экрана устройства (PNG). Координаты для жестов — пиксели этого снимка.",
         "inputSchema": schema(json!({"device": dev, "max_size": num("длинная сторона снимка, px (по умолчанию 1280; 0 — без уменьшения)"),
             "output": {"type": "string", "description": "вывод (монитор), по умолчанию первый"},
             "wake": {"type": "boolean", "description": "разбудить погашенный экран перед снимком (по умолчанию да)"}}), &["device"])},
        {"name": "tap", "description": "Нажать (палец на телефоне, левая кнопка мыши на компьютере) в точке снимка.",
         "inputSchema": schema(json!({"device": dev, "x": num("X, px снимка"), "y": num("Y, px снимка")}), &["device", "x", "y"])},
        {"name": "long_press", "description": "Удержать палец/кнопку в точке (контекстные меню, выбор).",
         "inputSchema": schema(json!({"device": dev, "x": num("X"), "y": num("Y"), "ms": num("длительность, мс (700)")}), &["device", "x", "y"])},
        {"name": "swipe", "description": "Провести из точки в точку (прокрутка списков, жесты от краёв, перетаскивание).",
         "inputSchema": schema(json!({"device": dev, "x1": num("X начала"), "y1": num("Y начала"), "x2": num("X конца"), "y2": num("Y конца"),
             "ms": num("длительность, мс (300)")}), &["device", "x1", "y1", "x2", "y2"])},
        {"name": "scroll", "description": "Прокрутить колесом мыши в точке (компьютер). steps > 0 — вниз.",
         "inputSchema": schema(json!({"device": dev, "x": num("X"), "y": num("Y"), "steps": num("шаги колеса")}), &["device", "x", "y", "steps"])},
        {"name": "type_text", "description": "Набрать текст в поле с фокусом (раскладка переключается сама; \\n — Enter).",
         "inputSchema": schema(json!({"device": dev, "text": {"type": "string"}}), &["device", "text"])},
        {"name": "key", "description": "Нажать сочетание клавиш: Return, Escape, BackSpace, ctrl+a, Alt+Tab, Super, XF86Back…",
         "inputSchema": schema(json!({"device": dev, "combo": {"type": "string"}}), &["device", "combo"])},
        {"name": "action", "description": "Действие композитора synwm: back, shell home, shell launcher, shell shade, shell recents, workspace 2, close, lock, screenshot…",
         "inputSchema": schema(json!({"device": dev, "action": {"type": "string"}}), &["device", "action"])},
        {"name": "windows", "description": "Окна на устройстве (заголовок, app_id, фокус, геометрия) — JSON.",
         "inputSchema": schema(json!({"device": dev}), &["device"])},
        {"name": "exec", "description": "Выполнить команду оболочки на устройстве (sh -c) от пользователя сеанса; возвращает код, stdout, stderr.",
         "inputSchema": schema(json!({"device": dev, "command": {"type": "string"}, "cwd": {"type": "string"},
             "timeout_s": num("тайм-аут, с (120)")}), &["device", "command"])},
        {"name": "logs", "description": "Журнал systemd устройства (journalctl): последние строки, фильтры по юниту и шаблону.",
         "inputSchema": schema(json!({"device": dev, "unit": {"type": "string", "description": "юнит, например synshell"},
             "lines": num("строк (200)"), "grep": {"type": "string"}, "since": {"type": "string", "description": "например '-10min'"}}), &["device"])},
        {"name": "notifications", "description": "Последние уведомления, пришедшие с устройств.",
         "inputSchema": schema(json!({"device": {"type": "string"}}), &[])},
        {"name": "read_file", "description": "Прочитать текстовый файл на устройстве.",
         "inputSchema": schema(json!({"device": dev, "path": {"type": "string"}}), &["device", "path"])},
        {"name": "write_file", "description": "Записать текстовый файл на устройстве (создаётся или перезаписывается).",
         "inputSchema": schema(json!({"device": dev, "path": {"type": "string"}, "content": {"type": "string"}}), &["device", "path", "content"])},
    ])
}

fn text(s: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}]})
}

fn arg_str<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    a.get(k).and_then(Value::as_str).with_context(|| format!("нужен параметр {k}"))
}

fn arg_num(a: &Value, k: &str) -> Result<f64> {
    a.get(k).and_then(Value::as_f64).with_context(|| format!("нужен числовой параметр {k}"))
}

fn exec(device: &str, command: &str, cwd: Option<String>, timeout_s: u64) -> Result<(i32, String, String)> {
    let r = gestures::req(&Request::Exec {
        device: device.into(),
        argv: vec!["/bin/sh".into(), "-c".into(), command.into()],
        cwd,
        stream: false,
        pty: None,
        timeout_ms: Some(timeout_s * 1000),
    })?;
    let Response::Exec { code, stdout, stderr } = r else { bail!("нет ответа") };
    Ok((code, stdout, stderr))
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!("[… обрезано {} байт …]\n{}", cut, &s[cut..])
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl Server {
    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value> {
        match method {
            "initialize" => {
                let ver = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18");
                Ok(json!({
                    "protocolVersion": ver,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "synlink", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(match self.call(name, &args) {
                    Ok(v) => v,
                    Err(e) => json!({"content": [{"type": "text", "text": format!("ошибка: {e:#}")}], "isError": true}),
                })
            }
            "resources/list" => Ok(json!({"resources": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            other => bail!("метод {other} не поддерживается"),
        }
    }

    /// Пиксели снимка → пиксели экрана устройства.
    fn scale(&self, device: &str, s: &gestures::Screen, x: f64, y: f64) -> (f64, f64) {
        match self.shots.get(device) {
            Some((img_w, full_w)) if *img_w > 0.0 => {
                let k = full_w / img_w;
                let _ = s;
                (x * k, y * k)
            }
            _ => (x, y),
        }
    }

    fn call(&mut self, name: &str, a: &Value) -> Result<Value> {
        let device = a.get("device").and_then(Value::as_str).unwrap_or("local").to_string();
        match name {
            "devices" => {
                let Response::Status { status } = gestures::req(&Request::Status)? else { bail!("нет состояния") };
                Ok(text(serde_json::to_string_pretty(&status)?))
            }
            "screenshot" => {
                // Разбудить экран (погашенный монитор — клиенты стоят на полпути
                // анимаций): нулевое движение указателя = активность.
                if a.get("wake").and_then(Value::as_bool).unwrap_or(true) {
                    let ev = vec![synshell_common::ipc::InputEvent::MotionRelative { dx: 0.0, dy: 0.0 }];
                    if gestures::req(&Request::Input { device: device.clone(), output: None, events: ev }).is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                    }
                }
                let max = a.get("max_size").and_then(Value::as_u64).unwrap_or(1280) as u32;
                let output = a.get("output").and_then(Value::as_str).map(String::from);
                let r = gestures::req(&Request::Screenshot { device: device.clone(), output: output.clone(), path: None, max_size: Some(max) })?;
                let Response::Screenshot { path, width, height, output } = r else { bail!("нет снимка") };
                let png = std::fs::read(&path)?;
                let _ = std::fs::remove_file(&path);
                let full = gestures::screen(&device, Some(&output)).map(|s| (s.w, s.h)).unwrap_or((width as f64, height as f64));
                self.shots.insert(device.clone(), (width as f64, full.0));
                Ok(json!({"content": [
                    {"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"},
                    {"type": "text", "text": format!("{device}: вывод {output}, снимок {width}×{height} (экран {}×{}); координаты жестов — в пикселях этого снимка", full.0, full.1)},
                ]}))
            }
            "tap" | "long_press" => {
                let s = gestures::screen(&device, None)?;
                let (x, y) = self.scale(&device, &s, arg_num(a, "x")?, arg_num(a, "y")?);
                if name == "tap" {
                    gestures::tap(&device, &s, x, y)?;
                } else {
                    gestures::hold(&device, &s, x, y, a.get("ms").and_then(Value::as_u64).unwrap_or(700))?;
                }
                Ok(text("готово"))
            }
            "swipe" => {
                let s = gestures::screen(&device, None)?;
                let p1 = self.scale(&device, &s, arg_num(a, "x1")?, arg_num(a, "y1")?);
                let p2 = self.scale(&device, &s, arg_num(a, "x2")?, arg_num(a, "y2")?);
                gestures::swipe(&device, &s, p1, p2, a.get("ms").and_then(Value::as_u64).unwrap_or(300))?;
                Ok(text("готово"))
            }
            "scroll" => {
                let s = gestures::screen(&device, None)?;
                let (x, y) = self.scale(&device, &s, arg_num(a, "x")?, arg_num(a, "y")?);
                gestures::scroll(&device, &s, x, y, arg_num(a, "steps")?, 0.0)?;
                Ok(text("готово"))
            }
            "type_text" => {
                gestures::text(&device, arg_str(a, "text")?)?;
                Ok(text("готово"))
            }
            "key" => {
                gestures::combo(&device, arg_str(a, "combo")?)?;
                Ok(text("готово"))
            }
            "action" => {
                gestures::action(&device, arg_str(a, "action")?)?;
                Ok(text("готово"))
            }
            "windows" => {
                let r = gestures::req(&Request::Wm { device, wm: synshell_common::ipc::Request::Windows })?;
                let Response::Wm { wm } = r else { bail!("нет ответа") };
                Ok(text(serde_json::to_string_pretty(&wm)?))
            }
            "exec" => {
                let t = a.get("timeout_s").and_then(Value::as_u64).unwrap_or(120);
                let cwd = a.get("cwd").and_then(Value::as_str).map(String::from);
                let (code, out, err) = exec(&device, arg_str(a, "command")?, cwd, t)?;
                Ok(text(format!("код выхода: {code}\n--- stdout ---\n{}\n--- stderr ---\n{}", clip(&out, 60_000), clip(&err, 20_000))))
            }
            "logs" => {
                let mut cmd = String::from("journalctl --no-pager -o short-iso");
                if let Some(u) = a.get("unit").and_then(Value::as_str) {
                    cmd.push_str(&format!(" -u {}", sh_quote(u)));
                }
                if let Some(s) = a.get("since").and_then(Value::as_str) {
                    cmd.push_str(&format!(" --since {}", sh_quote(s)));
                }
                if let Some(g) = a.get("grep").and_then(Value::as_str) {
                    cmd.push_str(&format!(" -g {}", sh_quote(g)));
                }
                cmd.push_str(&format!(" -n {}", a.get("lines").and_then(Value::as_u64).unwrap_or(200)));
                let (code, out, err) = exec(&device, &cmd, None, 60)?;
                Ok(text(format!("{}{}", clip(&out, 80_000), if code != 0 { format!("\n[код {code}] {err}") } else { String::new() })))
            }
            "notifications" => {
                let dev = a.get("device").and_then(Value::as_str).map(String::from);
                let Response::Notifications { notifications } = gestures::req(&Request::Notifications { device: dev })? else {
                    bail!("нет ответа")
                };
                Ok(text(serde_json::to_string_pretty(&notifications)?))
            }
            "read_file" => {
                let (code, out, err) = exec(&device, &format!("cat -- {}", sh_quote(arg_str(a, "path")?)), None, 30)?;
                if code != 0 {
                    bail!("{err}");
                }
                Ok(text(clip(&out, 200_000)))
            }
            "write_file" => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(arg_str(a, "content")?.as_bytes());
                let path = sh_quote(arg_str(a, "path")?);
                let (code, _, err) = exec(&device, &format!("printf %s {b64} | base64 -d > {path}"), None, 30)?;
                if code != 0 {
                    bail!("{err}");
                }
                Ok(text("записано"))
            }
            other => bail!("нет инструмента {other}"),
        }
    }
}
