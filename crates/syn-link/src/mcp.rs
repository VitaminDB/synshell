//! MCP-сервер (stdio, JSON-RPC 2.0) для Claude Code и других агентов:
//! видеть экран устройства, нажимать, набирать, запускать команды,
//! читать журнал и уведомления. Работает через локальный демон, так что
//! годится и для телефона (по USB/Wi-Fi), и для этой машины (`local`).
//!
//! Подключение: `claude mcp add synlink -- synlink mcp`.
//!
//! Запись экрана (`record`) — `synlink-view УСТРОЙСТВО --record` подпроцессом
//! без окна: он собирает кадры трансляции и пишет файл, отсюда — старт, стоп
//! (SIGTERM, файл доводится до конца) и кадр из середины для проверки. Умер
//! MCP-сервер — запись тоже штатно заканчивается (PDEATHSIG).

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

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
После действия делайте новый снимок, чтобы увидеть результат. \
Видео экрана — `record`: start, действия, stop (или start с duration_s).";

struct Server {
    /// Уменьшение последнего снимка: устройство → (ширина снимка, полная ширина).
    shots: HashMap<String, (f64, f64)>,
    /// Идущие записи экрана по устройству.
    recs: HashMap<String, Rec>,
}

/// Запись экрана: `synlink-view --record` и его строки JSON.
struct Rec {
    child: Child,
    lines: mpsc::Receiver<Value>,
    /// Строка `started`: параметры файла.
    info: Value,
}

pub fn run() -> Result<()> {
    let mut srv = Server { shots: HashMap::new(), recs: HashMap::new() };
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
        {"name": "pinch", "description": "Щипок двумя пальцами вокруг точки: расстояние между пальцами from → to (px снимка; больше — увеличить). На компьютере — Ctrl+колесо.",
         "inputSchema": schema(json!({"device": dev, "x": num("X центра"), "y": num("Y центра"), "from": num("начальное расстояние, px"), "to": num("конечное расстояние, px"),
             "ms": num("длительность, мс (400)")}), &["device", "x", "y", "from", "to"])},
        {"name": "scroll", "description": "Прокрутить колесом мыши в точке (компьютер). steps > 0 — вниз.",
         "inputSchema": schema(json!({"device": dev, "x": num("X"), "y": num("Y"), "steps": num("шаги колеса")}), &["device", "x", "y", "steps"])},
        {"name": "type_text", "description": "Набрать текст в поле с фокусом (раскладка переключается сама; \\n — Enter).",
         "inputSchema": schema(json!({"device": dev, "text": {"type": "string"}}), &["device", "text"])},
        {"name": "key", "description": "Нажать сочетание клавиш: Return, Escape, BackSpace, ctrl+a, Alt+Tab, Super, XF86Back…",
         "inputSchema": schema(json!({"device": dev, "combo": {"type": "string"}}), &["device", "combo"])},
        {"name": "action", "description": "Действие композитора synwm: back, shell home, shell launcher, shell notifications, shell quick-settings, shell recents, workspace 2, close, lock, screenshot…",
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
        {"name": "record", "description": "Запись экрана устройства в видеофайл на этой машине (по умолчанию «Видео/Записи экрана»). \
start — начать (с duration_s — записать столько секунд и вернуть результат сразу), потом жесты и прочее, stop — закончить: путь, параметры и кадр из середины. \
Без height/fps/codec/quality — исходные параметры трансляции (родное разрешение, частота экрана, её кодек, высокое качество). status — идущие записи.",
         "inputSchema": schema(json!({"device": dev,
             "action": {"type": "string", "enum": ["start", "stop", "status"], "description": "start (по умолчанию), stop, status"},
             "height": num("короткая сторона кадра, px (0 — исходная)"),
             "fps": num("кадров в секунду (1–120)"),
             "codec": {"type": "string", "enum": ["h264", "hevc", "av1"]},
             "quality": {"type": "string", "enum": ["high", "medium", "low"]},
             "sound": {"type": "boolean", "description": "со звуком устройства (он будет играть и на этой машине); по умолчанию нет"},
             "format": {"type": "string", "enum": ["mp4", "mkv"]},
             "path": {"type": "string", "description": "куда сохранить (путь на этой машине)"},
             "duration_s": num("записать столько секунд и вернуть результат (до 600)")}), &["device"])},
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
            "pinch" => {
                let s = gestures::screen(&device, None)?;
                let c = self.scale(&device, &s, arg_num(a, "x")?, arg_num(a, "y")?);
                // Расстояния — в px снимка, как точки: тот же пересчёт.
                let k = self.scale(&device, &s, 1000.0, 0.0).0 / 1000.0;
                let (d0, d1) = (arg_num(a, "from")? * k, arg_num(a, "to")? * k);
                gestures::pinch(&device, &s, c, d0, d1, a.get("ms").and_then(Value::as_u64).unwrap_or(400))?;
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
            "record" => match a.get("action").and_then(Value::as_str).unwrap_or("start") {
                "start" => self.record_start(&device, a),
                "stop" => {
                    let r = self.recs.remove(&device).with_context(|| format!("запись {device} не идёт"))?;
                    record_finish(r)
                }
                "status" => {
                    self.reap();
                    let list: Vec<Value> = self.recs.iter().map(|(d, r)| json!({"device": d, "file": r.info})).collect();
                    Ok(text(if list.is_empty() { "записей нет".to_string() } else { serde_json::to_string_pretty(&list)? }))
                }
                other => bail!("action {other}: нужно start, stop или status"),
            },
            other => bail!("нет инструмента {other}"),
        }
    }

    /// Убрать записи, которые кончились сами (ошибка, устройство пропало).
    fn reap(&mut self) {
        self.recs.retain(|_, r| matches!(r.child.try_wait(), Ok(None)));
    }

    fn record_start(&mut self, device: &str, a: &Value) -> Result<Value> {
        self.reap();
        if self.recs.contains_key(device) {
            bail!("запись {device} уже идёт — сначала action stop");
        }
        let mut cmd = Command::new("synlink-view");
        cmd.args([device, "--record"]);
        if let Some(h) = a.get("height").and_then(Value::as_u64) {
            cmd.args(["--height", &h.to_string()]);
        }
        if let Some(f) = a.get("fps").and_then(Value::as_u64) {
            cmd.args(["--fps", &f.to_string()]);
        }
        for k in ["codec", "quality"] {
            if let Some(v) = a.get(k).and_then(Value::as_str) {
                cmd.args([format!("--{k}"), v.to_string()]);
            }
        }
        if a.get("sound").and_then(Value::as_bool).unwrap_or(false) {
            cmd.arg("--sound");
        }
        if a.get("format").and_then(Value::as_str) == Some("mkv") {
            cmd.arg("--mkv");
        }
        if let Some(p) = a.get("path").and_then(Value::as_str) {
            cmd.args(["--out", p]);
        }
        let duration = a.get("duration_s").and_then(Value::as_f64).map(|d| d.clamp(0.5, 600.0));
        if let Some(d) = duration {
            cmd.args(["--duration", &d.to_string()]);
        }
        // Разбудить экран, как перед снимком: погашенный — запишется чёрный.
        let ev = vec![synshell_common::ipc::InputEvent::MotionRelative { dx: 0.0, dy: 0.0 }];
        let _ = gestures::req(&Request::Input { device: device.into(), output: None, events: ev });
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        // SAFETY: в потомке до exec — только prctl. Умер сервер — запись заканчивается штатно.
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let mut child = cmd.spawn().context("synlink-view не запускается")?;
        let out = child.stdout.take().context("stdout synlink-view")?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        // Первый кадр трансляции и кодер — до нескольких секунд.
        let info = match lines.recv_timeout(Duration::from_secs(25)) {
            Ok(v) if v["event"] == "started" => v,
            Ok(v) if v["event"] == "error" => {
                let _ = child.wait();
                bail!("{}", v["message"].as_str().unwrap_or("запись не началась"));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("запись не началась (нет трансляции {device}?)");
            }
        };
        let mut r = Rec { child, lines, info };
        if duration.is_some() {
            return record_finish_wait(&mut r);
        }
        let i = &r.info;
        let msg = format!(
            "запись {device} идёт: {}×{} · {} к/с · {} ({}){} → {}\nзакончить: record action=stop",
            i["width"], i["height"], i["fps"], i["codec"].as_str().unwrap_or(""), i["encoder"].as_str().unwrap_or(""),
            if i["sound"] == true { " · со звуком" } else { "" }, i["path"].as_str().unwrap_or(""),
        );
        self.recs.insert(device.to_string(), r);
        Ok(text(msg))
    }
}

/// Остановить запись (SIGTERM) и дождаться файла.
fn record_finish(mut r: Rec) -> Result<Value> {
    unsafe {
        libc::kill(r.child.id() as i32, libc::SIGTERM);
    }
    record_finish_wait(&mut r)
}

/// Дождаться строки `done`/`error`; результат с кадром из середины.
fn record_finish_wait(r: &mut Rec) -> Result<Value> {
    let end = loop {
        match r.lines.recv_timeout(Duration::from_secs(660)) {
            Ok(v) if v["event"] == "done" || v["event"] == "error" => break v,
            Ok(_) => continue,
            Err(_) => {
                let _ = r.child.kill();
                let _ = r.child.wait();
                bail!("запись не закончилась вовремя");
            }
        }
    };
    let _ = r.child.wait();
    if end["event"] == "error" {
        bail!("{}", end["message"].as_str().unwrap_or("запись не удалась"));
    }
    let path = end["path"].as_str().unwrap_or_default().to_string();
    let secs = end["seconds"].as_f64().unwrap_or(0.0);
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let summary = format!(
        "записано: {path}\n{}×{} · {} к/с · {} ({}){} · {secs:.1} с · {:.1} МБ",
        end["width"], end["height"], end["fps"], end["codec"].as_str().unwrap_or(""), end["encoder"].as_str().unwrap_or(""),
        if end["sound"] == true { " · со звуком" } else { "" }, size as f64 / 1_048_576.0,
    );
    let mut content = vec![json!({"type": "text", "text": summary})];
    if let Some(png) = middle_frame(&path, secs) {
        content.push(json!({"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"}));
        content.push(json!({"type": "text", "text": format!("кадр на {:.1} с", secs / 2.0)}));
    }
    Ok(json!({"content": content}))
}

/// Кадр из середины видео (PNG, длинная сторона ≤ 800) — проверить, что записалось.
fn middle_frame(path: &str, secs: f64) -> Option<Vec<u8>> {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-ss", &format!("{:.2}", secs / 2.0), "-i", path])
        .args(["-frames:v", "1", "-vf", "scale='if(gt(iw,ih),min(800,iw),-2)':'if(gt(iw,ih),-2,min(800,ih))'", "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}
