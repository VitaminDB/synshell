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
use synshell_tr::{n_, t};
use base64::Engine;
use serde_json::{json, Value};
use synshell_common::link::{Request, Response};

use crate::gestures;

const INSTRUCTIONS: &str = n_!("synlink управляет устройствами synshell (телефон и компьютер, связанные по USB или Wi-Fi). \
Устройство: имя или id из `devices`, `phone`, `desktop` или `local` (эта машина). \
Координаты для tap/long_press/swipe/scroll — пиксели последнего снимка `screenshot` этого устройства \
(снимок может быть уменьшен — пересчёт делается сам). На телефоне ввод идёт касаниями, на компьютере — мышью. \
Системный «назад» телефона — action `back`; домой — action `shell home`. \
После действия делайте новый снимок, чтобы увидеть результат. \
Видео экрана — `record`: start, действия, stop (или start с duration_s). \
Окна: `windows` (filter), снимок одного окна — `screenshot` с window (координаты жестов — в пикселях снимка окна), \
ожидание — `wait` вместо пауз. Проверки, не трогая живой сеанс, — `sandbox` start: устройство `sandbox`, \
программы внутри — exec (GUI через setsid -f … >/dev/null 2>&1). В живом сеансе пользователя щёлкать только по его просьбе.");

/// Последний снимок устройства: пиксель снимка (x, y) → пиксель кадра вывода
/// `output` (ox + x·k, oy + y·k). Снимок окна — со своим сдвигом.
#[derive(Clone)]
struct Shot {
    k: f64,
    ox: f64,
    oy: f64,
    output: Option<String>,
}

struct Server {
    shots: HashMap<String, Shot>,
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
    let dev = json!({"type": "string", "description": t!("устройство: имя, id, phone, desktop или local")});
    let num = |d: &str| json!({"type": "number", "description": d});
    json!([
        {"name": "devices", "description": t!("Эта машина, состояние кабеля USB и устройства: соединены ли, чем (USB/Wi-Fi), батарея, где файлы, ssh-имя."),
         "inputSchema": schema(json!({}), &[])},
        {"name": "screenshot", "description": t!("Снимок экрана устройства (PNG). Координаты для жестов — пиксели этого снимка."),
         "inputSchema": schema(json!({"device": dev, "max_size": num(&t!("длинная сторона снимка, px (по умолчанию 1280; 0 — без уменьшения)")),
             "output": {"type": "string", "description": t!("вывод (монитор), по умолчанию первый")},
             "wake": {"type": "boolean", "description": t!("разбудить погашенный экран перед снимком (по умолчанию да)")},
             "window": {"type": "string", "description": t!("снять только окно: часть app_id или заголовка, pid:N или id:N (из windows); вместе с его меню и подсказками; координаты жестов — в пикселях снимка окна")},
             "offscreen": {"type": "boolean", "description": t!("окно рисуется отдельно: видно и перекрытое другими, но без меню (только local)")}}), &["device"])},
        {"name": "tap", "description": t!("Нажать (палец на телефоне, левая кнопка мыши на компьютере) в точке снимка."),
         "inputSchema": schema(json!({"device": dev, "x": num(&t!("X, px снимка")), "y": num(&t!("Y, px снимка"))}), &["device", "x", "y"])},
        {"name": "long_press", "description": t!("Удержать палец/кнопку в точке (контекстные меню, выбор)."),
         "inputSchema": schema(json!({"device": dev, "x": num("X"), "y": num("Y"), "ms": num(&t!("длительность, мс (700)"))}), &["device", "x", "y"])},
        {"name": "swipe", "description": t!("Провести из точки в точку (прокрутка списков, жесты от краёв, перетаскивание)."),
         "inputSchema": schema(json!({"device": dev, "x1": num(&t!("X начала")), "y1": num(&t!("Y начала")), "x2": num(&t!("X конца")), "y2": num(&t!("Y конца")),
             "ms": num(&t!("длительность, мс (300)"))}), &["device", "x1", "y1", "x2", "y2"])},
        {"name": "pinch", "description": t!("Щипок двумя пальцами вокруг точки: расстояние между пальцами from → to (px снимка; больше — увеличить). На компьютере — Ctrl+колесо."),
         "inputSchema": schema(json!({"device": dev, "x": num(&t!("X центра")), "y": num(&t!("Y центра")), "from": num(&t!("начальное расстояние, px")), "to": num(&t!("конечное расстояние, px")),
             "ms": num(&t!("длительность, мс (400)"))}), &["device", "x", "y", "from", "to"])},
        {"name": "scroll", "description": t!("Прокрутить колесом мыши в точке (компьютер). steps > 0 — вниз."),
         "inputSchema": schema(json!({"device": dev, "x": num("X"), "y": num("Y"), "steps": num(&t!("шаги колеса"))}), &["device", "x", "y", "steps"])},
        {"name": "type_text", "description": t!("Набрать текст в поле с фокусом (раскладка переключается сама; \\n — Enter)."),
         "inputSchema": schema(json!({"device": dev, "text": {"type": "string"}}), &["device", "text"])},
        {"name": "key", "description": t!("Нажать сочетание клавиш: Return, Escape, BackSpace, ctrl+a, Alt+Tab, Super, XF86Back…"),
         "inputSchema": schema(json!({"device": dev, "combo": {"type": "string"}}), &["device", "combo"])},
        {"name": "action", "description": t!("Действие композитора synwm: back, shell home, shell launcher, shell notifications, shell quick-settings, shell recents, workspace 2, close, lock, screenshot…"),
         "inputSchema": schema(json!({"device": dev, "action": {"type": "string"}}), &["device", "action"])},
        {"name": "windows", "description": t!("Окна на устройстве: id, app_id, заголовок, pid, стол, геометрия; у X11 — положение глазами X-сервера (⚠ — не совпадает), popup — меню и подсказки X11."),
         "inputSchema": schema(json!({"device": dev,
             "filter": {"type": "string", "description": t!("часть app_id или заголовка, pid:N, id:N (меню программы показываются вместе с ней)")},
             "popups": {"type": "boolean", "description": t!("показывать меню и подсказки X11 (по умолчанию да)")},
             "json": {"type": "boolean", "description": t!("полный JSON вместо строк")}}), &["device"])},
        {"name": "sandbox", "description": t!("Песочница для проверок: вложенный synwm с оболочкой в окне на этом столе, свой D-Bus, копия настроек — живой сеанс не трогается. start возвращает устройство (sandbox или sandbox:ИМЯ) для screenshot/tap/windows/exec; exec запускает программы внутри (WAYLAND_DISPLAY, DISPLAY, D-Bus песочницы; GUI — через setsid -f). stop — закрыть, list — запущенные."),
         "inputSchema": schema(json!({"action": {"type": "string", "enum": ["start", "stop", "list"]},
             "name": {"type": "string", "description": t!("имя (по умолчанию default → устройство sandbox)")},
             "size": {"type": "string", "description": t!("размер окна, например 1600x900")},
             "form_factor": {"type": "string", "enum": ["desktop", "phone", "tablet", "game"]},
             "shell": {"type": "string", "description": t!("своя оболочка (путь к собранному бинарнику) вместо установленной")},
             "synwm": {"type": "string", "description": t!("свой synwm (путь к собранному бинарнику)")}}), &["action"])},
        {"name": "ui", "description": t!("Элементы окна (кнопки, поля, пункты, тексты) с ролями, подписями и рамками — из программы (syngui, AT-SPI: GTK, Qt, браузеры), без снимка. Для игр и OpenGL дерева нет — снимок."),
         "inputSchema": schema(json!({"device": dev, "window": {"type": "string", "description": t!("окно: часть app_id или заголовка, pid:N, id:N")},
             "filter": {"type": "string", "description": t!("часть текста или #id")}, "role": {"type": "string"}}), &["device", "window"])},
        {"name": "press", "description": t!("Нажать элемент окна по тексту (подпись, значение, подсказка) — настоящим вводом в его центр; value — затем набрать текст (поле). via a11y — действием через дерево syngui, без мыши."),
         "inputSchema": schema(json!({"device": dev, "window": {"type": "string"}, "text": {"type": "string", "description": t!("часть текста элемента или #id из ui")},
             "role": {"type": "string"}, "index": num(&t!("какой по счёту из подходящих (0)")),
             "action": {"type": "string", "enum": ["click", "double", "right", "long", "hover"]},
             "value": {"type": "string", "description": t!("текст для поля")},
             "via": {"type": "string", "enum": ["input", "a11y"]}}), &["device", "window", "text"])},
        {"name": "wait", "description": t!("Дождаться окна (появится или закроется) вместо пауз: возвращает строку окна."),
         "inputSchema": schema(json!({"device": dev, "window": {"type": "string", "description": t!("часть app_id или заголовка, pid:N, id:N")},
             "gone": {"type": "boolean", "description": t!("ждать, пока закроется")}, "timeout_s": num(&t!("тайм-аут, с (10)"))}), &["device", "window"])},
        {"name": "exec", "description": t!("Выполнить команду оболочки на устройстве (sh -c) от пользователя сеанса; возвращает код, stdout, stderr."),
         "inputSchema": schema(json!({"device": dev, "command": {"type": "string"}, "cwd": {"type": "string"},
             "timeout_s": num(&t!("тайм-аут, с (120)"))}), &["device", "command"])},
        {"name": "logs", "description": t!("Журнал systemd устройства (journalctl): последние строки, фильтры по юниту и шаблону."),
         "inputSchema": schema(json!({"device": dev, "unit": {"type": "string", "description": t!("юнит, например synshell")},
             "lines": num(&t!("строк (200)")), "grep": {"type": "string"}, "since": {"type": "string", "description": t!("например '-10min'")},
             "file": {"type": "string", "description": t!("вместо journalctl — файл журнала: shell (оболочка), synwm (композитор) или путь")}}), &["device"])},
        {"name": "notifications", "description": t!("Последние уведомления, пришедшие с устройств."),
         "inputSchema": schema(json!({"device": {"type": "string"}}), &[])},
        {"name": "read_file", "description": t!("Прочитать текстовый файл на устройстве."),
         "inputSchema": schema(json!({"device": dev, "path": {"type": "string"}}), &["device", "path"])},
        {"name": "write_file", "description": t!("Записать файл на устройстве (создаётся или перезаписывается): текст из content или любой файл этой машины из from (двоичный, любого размера — программа, картинка, архив); mode — права (например 755)."),
         "inputSchema": schema(json!({"device": dev, "path": {"type": "string"}, "content": {"type": "string"},
             "from": {"type": "string", "description": t!("путь к файлу на этой машине (вместо content)")},
             "mode": {"type": "string", "description": t!("права в восьмеричном виде, например 755")}}), &["device", "path"])},
        {"name": "record", "description": t!("Запись экрана устройства в видеофайл на этой машине (по умолчанию «Видео/Записи экрана»). \
start — начать (с duration_s — записать столько секунд и вернуть результат сразу), потом жесты и прочее, stop — закончить: путь, параметры и кадр из середины. \
Без height/fps/codec/quality — исходные параметры трансляции (родное разрешение, частота экрана, её кодек, высокое качество). status — идущие записи."),
         "inputSchema": schema(json!({"device": dev,
             "action": {"type": "string", "enum": ["start", "stop", "status"], "description": t!("start (по умолчанию), stop, status")},
             "height": num(&t!("короткая сторона кадра, px (0 — исходная)")),
             "fps": num(&t!("кадров в секунду (1–120)")),
             "codec": {"type": "string", "enum": ["h264", "hevc", "av1"]},
             "quality": {"type": "string", "enum": ["high", "medium", "low"]},
             "sound": {"type": "boolean", "description": t!("со звуком устройства (он будет играть и на этой машине); по умолчанию нет")},
             "format": {"type": "string", "enum": ["mp4", "mkv"]},
             "path": {"type": "string", "description": t!("куда сохранить (путь на этой машине)")},
             "duration_s": num(&t!("записать столько секунд и вернуть результат (до 600)"))}), &["device"])},
    ])
}

fn text(s: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": s.into()}]})
}

fn arg_str<'a>(a: &'a Value, k: &str) -> Result<&'a str> {
    a.get(k).and_then(Value::as_str).with_context(|| t!("нужен параметр {k}", k = k))
}

fn arg_num(a: &Value, k: &str) -> Result<f64> {
    a.get(k).and_then(Value::as_f64).with_context(|| t!("нужен числовой параметр {k}", k = k))
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
    let Response::Exec { code, stdout, stderr } = r else { bail!("{}", t!("нет ответа")) };
    Ok((code, stdout, stderr))
}

/// Команда с данными на stdin (поток `Request::Exec { stream: true }`): файл любого размера одним потоком,
/// без предела длины командной строки.
fn exec_stdin(device: &str, command: &str, data: Vec<u8>, timeout_s: u64) -> Result<(i32, String, String)> {
    if let Some(n) = crate::sandbox::device_name(device) {
        let s = crate::sandbox::load(&n).with_context(|| t!("песочница «{n}» не запущена", n = n))?;
        return crate::sandbox::run(&s, &["/bin/sh".into(), "-c".into(), command.into()], None, Some(data), Duration::from_secs(timeout_s));
    }
    use synshell_common::link::{self, ExecFrame};
    let mut c = link::Client::connect().with_context(|| t!("synlink не запущен"))?;
    match c.request(&Request::Exec {
        device: device.into(),
        argv: vec!["/bin/sh".into(), "-c".into(), command.into()],
        cwd: None,
        stream: true,
        pty: None,
        timeout_ms: Some(timeout_s * 1000),
    })? {
        Response::Ok => {}
        Response::Error { message } => bail!("{message}"),
        other => bail!("{}", t!("неожиданный ответ: {other}", other = format!("{other:?}"))),
    }
    let (mut rd, wr) = c.into_parts();
    let mut w = wr.try_clone()?;
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        for ch in data.chunks(256 << 10) {
            link::write_frame(&mut w, &serde_json::to_vec(&ExecFrame::Stdin { data: ch.to_vec() })?)?;
        }
        link::write_frame(&mut w, &serde_json::to_vec(&ExecFrame::Eof)?)
    });
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = loop {
        let Some(f) = link::read_frame(&mut rd)? else { bail!("{}", t!("соединение оборвалось")) };
        match serde_json::from_slice::<ExecFrame>(&f)? {
            ExecFrame::Stdout { data } => out.extend(data),
            ExecFrame::Stderr { data } => err.extend(data),
            ExecFrame::Exit { code } => break code,
            ExecFrame::Error { message } => bail!("{message}"),
            _ => {}
        }
    };
    drop(wr);
    let _ = writer.join();
    Ok((code, String::from_utf8_lossy(&out).into_owned(), String::from_utf8_lossy(&err).into_owned()))
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!("{}\n{}", t!("[… обрезано {cut} байт …]", cut = cut), &s[cut..])
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
                    "instructions": synshell_tr::t(INSTRUCTIONS),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(match self.call(name, &args) {
                    Ok(v) => v,
                    Err(e) => json!({"content": [{"type": "text", "text": t!("ошибка: {e}", e = format!("{e:#}"))}], "isError": true}),
                })
            }
            "resources/list" => Ok(json!({"resources": []})),
            "prompts/list" => Ok(json!({"prompts": []})),
            other => bail!("{}", t!("метод {other} не поддерживается", other = other)),
        }
    }

    /// Пиксели снимка → пиксели экрана устройства.
    fn scale(&self, device: &str, _s: &gestures::Screen, x: f64, y: f64) -> (f64, f64) {
        match self.shots.get(device) {
            Some(s) => (s.ox + x * s.k, s.oy + y * s.k),
            None => (x, y),
        }
    }

    /// Экран для жестов: вывод последнего снимка (снимок окна — его вывод).
    fn screen(&self, device: &str) -> Result<gestures::Screen> {
        gestures::screen(device, self.shots.get(device).and_then(|s| s.output.as_deref()))
    }


    /// Элементы окна: syngui или AT-SPI; песочница и другие устройства — через
    /// `synlink ui-tree` там (у песочницы своя шина доступности).
    fn ui_tree(&self, device: &str, w: &synshell_common::ipc::WindowInfo) -> Result<crate::uitree::Tree> {
        let pid = w.pid.context(t!("у окна нет pid (X11-программа без _NET_WM_PID?)"))?;
        let x11 = w.x11_id.zip(w.x11_geometry).map(|(id, geo)| crate::uitree::X11Ctx { id, geo, scale: w.x11_scale.unwrap_or(1.0) });
        let x11_args = x11.map(|x| format!(" {} {} {} {} {} {}", x.id, x.geo[0], x.geo[1], x.geo[2], x.geo[3], x.scale)).unwrap_or_default();
        if matches!(device, "local" | "self" | "") {
            return crate::uitree::local_tree(pid, x11);
        }
        let exe = if crate::sandbox::device_name(device).is_some() {
            std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "synlink".into())
        } else {
            "synlink".into()
        };
        let (code, out, err) = exec(device, &format!("{} ui-tree {pid}{x11_args}", sh_quote(&exe)), None, 30)?;
        if code != 0 {
            bail!("{}", if err.trim().is_empty() { out } else { err });
        }
        Ok(serde_json::from_str(out.trim())?)
    }

    /// Точка окна (логические пиксели окна) → пиксели кадра его вывода; экран для жестов.
    fn window_point(&self, device: &str, w: &synshell_common::ipc::WindowInfo, x: f64, y: f64) -> Result<(gestures::Screen, f64, f64)> {
        let outs = crate::winshot::outputs(device)?;
        let o = crate::winshot::output_of(&outs, w).context(t!("нет вывода"))?;
        let s = gestures::screen(device, Some(&o.name))?;
        let k = s.w / o.geometry[2].max(1) as f64;
        let gx = w.geometry[0] as f64 + x - o.geometry[0] as f64;
        let gy = w.geometry[1] as f64 + y - o.geometry[1] as f64;
        Ok((s, gx * k, gy * k))
    }

    /// Снимок одного окна (с его меню и подсказками). `offscreen` — окно
    /// рисуется отдельно (видно и перекрытое, но без меню; только эта машина).
    fn window_shot(&mut self, device: &str, q: &str, a: &Value) -> Result<Value> {
        let list = crate::winshot::windows(device)?;
        let w = crate::winshot::pick(&list, q)?.clone();
        let outs = crate::winshot::outputs(device)?;
        let o = crate::winshot::output_of(&outs, &w).context(t!("нет вывода"))?.clone();
        let max = a.get("max_size").and_then(Value::as_u64).unwrap_or(1280) as u32;
        let s = gestures::screen(device, Some(&o.name))?;
        let offscreen = a.get("offscreen").and_then(Value::as_bool).unwrap_or(false);
        let (png, crop) = if offscreen {
            if !matches!(device, "local" | "self" | "") {
                bail!("{}", t!("offscreen — только для этой машины (local)"));
            }
            // Кадр окна — в логических пикселях (композитор не увеличивает).
            let px = |v: i32| v.max(1) as u32;
            let r = gestures::req(&Request::Wm { device: device.into(), wm: synshell_common::ipc::Request::Thumbnails { ids: vec![w.id], max: [px(w.geometry[2]).max(1), px(w.geometry[3]).max(1)] } })?;
            let Response::Wm { wm: synshell_common::ipc::Response::Thumbnails { thumbs: thumbnails } } = r else { bail!("{}", t!("композитор не отдал окно")) };
            let th = thumbnails.into_iter().next().context(t!("у окна ещё нет кадра"))?;
            let rgba = std::fs::read(&th.path)?;
            let _ = std::fs::remove_file(&th.path);
            let png = crate::services::encode_png(th.width, th.height, &rgba)?;
            // Где окно на экране (если видно): туда и пойдут щелчки.
            let c = crate::winshot::to_frame(&o, s.w, s.h, w.geometry).unwrap_or([0, 0, th.width, th.height]);
            (png, [c[0], c[1], c[2]])
        } else {
            let region = crate::winshot::region(&list, &w);
            let c = crate::winshot::to_frame(&o, s.w, s.h, region).context(t!("окно за пределами экрана (свёрнуто или на другом столе?) — попробуйте offscreen"))?;
            let r = gestures::req(&Request::Screenshot { device: device.into(), output: Some(o.name.clone()), path: None, max_size: Some(0) })?;
            let Response::Screenshot { path, .. } = r else { bail!("{}", t!("нет снимка")) };
            let full = std::fs::read(&path)?;
            let _ = std::fs::remove_file(&path);
            (crate::winshot::crop_png(&full, c)?, [c[0], c[1], c[2]])
        };
        let (iw, ih, png) = if max > 0 { crate::services::downscale_png(&png, max)? } else { let (w, h, _) = crate::winshot::decode_rgba(&png)?; (w, h, png) };
        // crop = [x, y, ширина] того, что на снимке, в пикселях кадра вывода.
        let k = crop[2] as f64 / iw.max(1) as f64;
        self.shots.insert(device.to_string(), Shot { k, ox: crop[0] as f64, oy: crop[1] as f64, output: Some(o.name.clone()) });
        let note = if w.minimized { t!(" (свёрнуто — щелчки не попадут)") } else { Default::default() };
        Ok(json!({"content": [
            {"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"},
            {"type": "text", "text": format!("{}{}\n{}", crate::winshot::line(&w), note, t!("снимок окна {iw}×{ih}; координаты tap/swipe/scroll — в пикселях этого снимка", iw = iw, ih = ih))},
        ]}))
    }

    fn call(&mut self, name: &str, a: &Value) -> Result<Value> {
        let device = a.get("device").and_then(Value::as_str).unwrap_or("local").to_string();
        match name {
            "devices" => {
                let Response::Status { status } = gestures::req(&Request::Status)? else { bail!("{}", t!("нет состояния")) };
                Ok(text(serde_json::to_string_pretty(&status)?))
            }
            "screenshot" => {
                if let Some(q) = a.get("window").and_then(Value::as_str).filter(|q| !q.trim().is_empty()) {
                    return self.window_shot(&device, q, a);
                }
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
                let Response::Screenshot { path, width, height, output } = r else { bail!("{}", t!("нет снимка")) };
                let png = std::fs::read(&path)?;
                let _ = std::fs::remove_file(&path);
                let full = gestures::screen(&device, Some(&output)).map(|s| (s.w, s.h)).unwrap_or((width as f64, height as f64));
                self.shots.insert(device.clone(), Shot { k: full.0 / (width as f64).max(1.0), ox: 0.0, oy: 0.0, output: Some(output.clone()) });
                Ok(json!({"content": [
                    {"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"},
                    {"type": "text", "text": t!("{device}: вывод {output}, снимок {width}×{height} (экран {sw}×{sh}); координаты жестов — в пикселях этого снимка", device = device, output = output, width = width, height = height, sw = full.0, sh = full.1)},
                ]}))
            }
            "tap" | "long_press" => {
                let s = self.screen(&device)?;
                let (x, y) = self.scale(&device, &s, arg_num(a, "x")?, arg_num(a, "y")?);
                if name == "tap" {
                    gestures::tap(&device, &s, x, y)?;
                } else {
                    gestures::hold(&device, &s, x, y, a.get("ms").and_then(Value::as_u64).unwrap_or(700))?;
                }
                Ok(text(t!("готово")))
            }
            "swipe" => {
                let s = self.screen(&device)?;
                let p1 = self.scale(&device, &s, arg_num(a, "x1")?, arg_num(a, "y1")?);
                let p2 = self.scale(&device, &s, arg_num(a, "x2")?, arg_num(a, "y2")?);
                gestures::swipe(&device, &s, p1, p2, a.get("ms").and_then(Value::as_u64).unwrap_or(300))?;
                Ok(text(t!("готово")))
            }
            "pinch" => {
                let s = self.screen(&device)?;
                let c = self.scale(&device, &s, arg_num(a, "x")?, arg_num(a, "y")?);
                // Расстояния — в px снимка, как точки: тот же пересчёт.
                let k = self.scale(&device, &s, 1000.0, 0.0).0 / 1000.0;
                let (d0, d1) = (arg_num(a, "from")? * k, arg_num(a, "to")? * k);
                gestures::pinch(&device, &s, c, d0, d1, a.get("ms").and_then(Value::as_u64).unwrap_or(400))?;
                Ok(text(t!("готово")))
            }
            "scroll" => {
                let s = self.screen(&device)?;
                let (x, y) = self.scale(&device, &s, arg_num(a, "x")?, arg_num(a, "y")?);
                gestures::scroll(&device, &s, x, y, arg_num(a, "steps")?, 0.0)?;
                Ok(text(t!("готово")))
            }
            "type_text" => {
                gestures::text(&device, arg_str(a, "text")?)?;
                Ok(text(t!("готово")))
            }
            "key" => {
                gestures::combo(&device, arg_str(a, "combo")?)?;
                Ok(text(t!("готово")))
            }
            "action" => {
                gestures::action(&device, arg_str(a, "action")?)?;
                Ok(text(t!("готово")))
            }
            "windows" => {
                let list = crate::winshot::windows(&device)?;
                let q = a.get("filter").and_then(Value::as_str).filter(|q| !q.trim().is_empty());
                let popups = a.get("popups").and_then(Value::as_bool).unwrap_or(true);
                let sel: Vec<_> = list
                    .iter()
                    .filter(|w| popups || !w.override_redirect)
                    .filter(|w| match q {
                        None => true,
                        Some(q) => crate::winshot::pick(std::slice::from_ref(*w), q).is_ok() || (w.override_redirect && crate::winshot::pick(&list, q).is_ok_and(|m| m.pid == w.pid && w.pid.is_some())),
                    })
                    .collect();
                if a.get("json").and_then(Value::as_bool).unwrap_or(false) {
                    return Ok(text(serde_json::to_string_pretty(&sel)?));
                }
                let lines: Vec<String> = sel.iter().map(|w| crate::winshot::line(w)).collect();
                Ok(text(if lines.is_empty() { t!("окон нет").to_string() } else { lines.join("\n") }))
            }
            "sandbox" => {
                let name = a.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                match a.get("action").and_then(Value::as_str).unwrap_or("start") {
                    "start" => {
                        let o = crate::sandbox::Opts {
                            name: name.clone(),
                            size: a.get("size").and_then(Value::as_str).unwrap_or("").into(),
                            form_factor: a.get("form_factor").and_then(Value::as_str).unwrap_or("").into(),
                            shell: a.get("shell").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
                            synwm: a.get("synwm").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
                        };
                        let s = crate::sandbox::start(&o)?;
                        let dev = if s.name == "default" { "sandbox".to_string() } else { format!("sandbox:{}", s.name) };
                        self.shots.remove(&dev);
                        Ok(text(t!("запущена — устройство «{dev}»: {info}", dev = dev, info = crate::sandbox::describe(&s))))
                    }
                    "stop" => {
                        let n = if name.is_empty() { "default".to_string() } else { name };
                        crate::sandbox::stop(&n)?;
                        Ok(text(t!("песочница «{n}» закрыта", n = n)))
                    }
                    "list" => {
                        let l: Vec<String> = crate::sandbox::list().iter().map(crate::sandbox::describe).collect();
                        Ok(text(if l.is_empty() { t!("песочниц нет").to_string() } else { l.join("\n") }))
                    }
                    other => bail!("{}", t!("action {other}: нужно start, stop или list", other = other)),
                }
            }
            "ui" => {
                let list = crate::winshot::windows(&device)?;
                let w = crate::winshot::pick(&list, arg_str(a, "window")?)?.clone();
                let t = self.ui_tree(&device, &w)?;
                let q = a.get("filter").and_then(Value::as_str).unwrap_or("");
                let role = a.get("role").and_then(Value::as_str);
                let lines: Vec<String> = t.nodes.iter().filter(|n| crate::uitree::matches(n, q, role)).take(400).map(|n| n.line()).collect();
                Ok(text(format!(
                    "{}\n{}\n{}",
                    crate::winshot::line(&w),
                    t!("элементы ({src}): #id роль «текст» [x y w×h — логические пиксели окна]; нажать — press", src = t.source),
                    if lines.is_empty() { t!("ничего не найдено").to_string() } else { lines.join("\n") }
                )))
            }
            "press" => {
                let list = crate::winshot::windows(&device)?;
                let w = crate::winshot::pick(&list, arg_str(a, "window")?)?.clone();
                let t = self.ui_tree(&device, &w)?;
                let q = arg_str(a, "text")?;
                let role = a.get("role").and_then(Value::as_str);
                let idx = a.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let n = crate::uitree::best(&t, q, role, idx).cloned().with_context(|| t!("в окне нет элемента «{q}» — посмотрите ui", q = q))?;
                let how = a.get("action").and_then(Value::as_str).unwrap_or("click");
                if a.get("via").and_then(Value::as_str) == Some("a11y") {
                    if t.source != "syngui" {
                        bail!("{}", t!("via a11y — пока только у программ на syngui"));
                    }
                    let pid = w.pid.unwrap_or(0);
                    let req = match a.get("value").and_then(Value::as_str) {
                        Some(v) => format!("set {} {v}", n.id),
                        None => format!("click {}", n.id),
                    };
                    if matches!(device.as_str(), "local" | "self" | "") || crate::sandbox::device_name(&device).is_some() {
                        crate::uitree::syngui_request(pid, &req)?;
                    } else {
                        bail!("{}", t!("via a11y — только на этой машине и в песочнице"));
                    }
                    return Ok(text(t!("{how} через дерево: {n}", how = how, n = n.line().trim())));
                }
                let (cx, cy) = n.center();
                let (s, x, y) = self.window_point(&device, &w, cx, cy)?;
                match how {
                    "double" => {
                        gestures::tap(&device, &s, x, y)?;
                        std::thread::sleep(Duration::from_millis(80));
                        gestures::tap(&device, &s, x, y)?;
                    }
                    "long" => gestures::hold(&device, &s, x, y, 800)?,
                    "right" => gestures::right_click(&device, &s, x, y)?,
                    "hover" => gestures::hover(&device, &s, x, y)?,
                    _ => gestures::tap(&device, &s, x, y)?,
                }
                if let Some(v) = a.get("value").and_then(Value::as_str) {
                    std::thread::sleep(Duration::from_millis(120));
                    gestures::text(&device, v)?;
                }
                Ok(text(t!("{how}: {n}", how = how, n = n.line().trim())))
            }
            "wait" => {
                let q = arg_str(a, "window")?.to_string();
                let gone = a.get("gone").and_then(Value::as_bool).unwrap_or(false);
                let deadline = std::time::Instant::now() + Duration::from_secs_f64(a.get("timeout_s").and_then(Value::as_f64).unwrap_or(10.0).clamp(0.1, 300.0));
                loop {
                    let list = crate::winshot::windows(&device)?;
                    let found = crate::winshot::pick(&list, &q).ok().map(crate::winshot::line);
                    match (&found, gone) {
                        (Some(l), false) => return Ok(text(t!("есть: {l}", l = l))),
                        (None, true) => return Ok(text(t!("окна «{q}» больше нет", q = q))),
                        _ => {}
                    }
                    if std::time::Instant::now() > deadline {
                        bail!("{}", if gone { t!("окно «{q}» не закрылось", q = q) } else { t!("окно «{q}» не появилось", q = q) });
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
            "exec" => {
                let t = a.get("timeout_s").and_then(Value::as_u64).unwrap_or(120);
                let cwd = a.get("cwd").and_then(Value::as_str).map(String::from);
                let (code, out, err) = exec(&device, arg_str(a, "command")?, cwd, t)?;
                Ok(text(format!("{}\n--- stdout ---\n{}\n--- stderr ---\n{}", t!("код выхода: {code}", code = code), clip(&out, 60_000), clip(&err, 20_000))))
            }
            "logs" => {
                if let Some(f) = a.get("file").and_then(Value::as_str).filter(|f| !f.is_empty()) {
                    let path = match f {
                        "shell" => "\"${XDG_STATE_HOME:-$HOME/.local/state}/synwm/shell.log\"".to_string(),
                        "synwm" => "\"${XDG_STATE_HOME:-$HOME/.local/state}/synwm/synwm.log\"".to_string(),
                        p => sh_quote(p),
                    };
                    let n = a.get("lines").and_then(Value::as_u64).unwrap_or(200);
                    let cmd = match a.get("grep").and_then(Value::as_str).filter(|g| !g.is_empty()) {
                        Some(g) => format!("grep -a -E -- {} {path} | tail -n {n}", sh_quote(g)),
                        None => format!("tail -n {n} -- {path}"),
                    };
                    let (code, out, err) = exec(&device, &cmd, None, 60)?;
                    return Ok(text(format!("{}{}", clip(&out, 80_000), if code > 1 { t!("\n[код {code}] {err}", code = code, err = err) } else { String::new() })));
                }
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
                Ok(text(format!("{}{}", clip(&out, 80_000), if code != 0 { t!("\n[код {code}] {err}", code = code, err = err) } else { String::new() })))
            }
            "notifications" => {
                let dev = a.get("device").and_then(Value::as_str).map(String::from);
                let Response::Notifications { notifications } = gestures::req(&Request::Notifications { device: dev })? else {
                    bail!("{}", t!("нет ответа"))
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
                let data: Vec<u8> = match a.get("from").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    Some(from) => std::fs::read(from).with_context(|| from.to_string())?,
                    None => arg_str(a, "content")?.as_bytes().to_vec(),
                };
                let path = sh_quote(arg_str(a, "path")?);
                // одним потоком через stdin во временный файл рядом (каталог создаётся), проверка размера, затем на место
                let tmp = sh_quote(&format!("{}.synlink-part", arg_str(a, "path")?));
                let len = data.len();
                let (code, _, err) = exec_stdin(&device, &format!("mkdir -p -- \"$(dirname -- {path})\" && cat > {tmp}"), data, 600)?;
                if code != 0 {
                    let _ = exec(&device, &format!("rm -f -- {tmp}"), None, 10);
                    bail!("{err}");
                }
                let mode = a.get("mode").and_then(Value::as_str).filter(|m| !m.is_empty() && m.chars().all(|c| c.is_digit(8)));
                let chmod = mode.map(|m| format!(" && chmod {m} -- {tmp}")).unwrap_or_default();
                let (code, out, err) = exec(&device, &format!("test \"$(stat -c %s -- {tmp})\" = {}{chmod} && mv -f -- {tmp} {path} && echo ok || {{ rm -f -- {tmp}; echo size-mismatch >&2; exit 1; }}", len), None, 30)?;
                if code != 0 || !out.contains("ok") {
                    bail!("{}", if err.trim().is_empty() { t!("файл записан не целиком").to_string() } else { err });
                }
                Ok(text(t!("записано: {n} байт", n = len)))
            }
            "record" => match a.get("action").and_then(Value::as_str).unwrap_or("start") {
                "start" => self.record_start(&device, a),
                "stop" => {
                    let r = self.recs.remove(&device).with_context(|| t!("запись {device} не идёт", device = device))?;
                    record_finish(r)
                }
                "status" => {
                    self.reap();
                    let list: Vec<Value> = self.recs.iter().map(|(d, r)| json!({"device": d, "file": r.info})).collect();
                    Ok(text(if list.is_empty() { t!("записей нет") } else { serde_json::to_string_pretty(&list)? }))
                }
                other => bail!("{}", t!("action {other}: нужно start, stop или status", other = other)),
            },
            other => bail!("{}", t!("нет инструмента {other}", other = other)),
        }
    }

    /// Убрать записи, которые кончились сами (ошибка, устройство пропало).
    fn reap(&mut self) {
        self.recs.retain(|_, r| matches!(r.child.try_wait(), Ok(None)));
    }

    fn record_start(&mut self, device: &str, a: &Value) -> Result<Value> {
        self.reap();
        if self.recs.contains_key(device) {
            bail!("{}", t!("запись {device} уже идёт — сначала action stop", device = device));
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
        let mut child = cmd.spawn().with_context(|| t!("synlink-view не запускается"))?;
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
                bail!("{}", v["message"].as_str().map(String::from).unwrap_or_else(|| t!("запись не началась")));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{}", t!("запись не началась (нет трансляции {device}?)", device = device));
            }
        };
        let mut r = Rec { child, lines, info };
        if duration.is_some() {
            return record_finish_wait(&mut r);
        }
        let i = &r.info;
        let msg = t!(
            "запись {device} идёт: {w}×{h} · {fps} к/с · {codec} ({encoder}){sound} → {path}\nзакончить: record action=stop",
            device = device,
            w = i["width"],
            h = i["height"],
            fps = i["fps"],
            codec = i["codec"].as_str().unwrap_or(""),
            encoder = i["encoder"].as_str().unwrap_or(""),
            sound = if i["sound"] == true { t!(" · со звуком") } else { String::new() },
            path = i["path"].as_str().unwrap_or(""),
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
                bail!("{}", t!("запись не закончилась вовремя"));
            }
        }
    };
    let _ = r.child.wait();
    if end["event"] == "error" {
        bail!("{}", end["message"].as_str().map(String::from).unwrap_or_else(|| t!("запись не удалась")));
    }
    let path = end["path"].as_str().unwrap_or_default().to_string();
    let secs = end["seconds"].as_f64().unwrap_or(0.0);
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let summary = t!(
        "записано: {path}\n{w}×{h} · {fps} к/с · {codec} ({encoder}){sound} · {secs} с · {mb} МБ",
        path = path,
        w = end["width"],
        h = end["height"],
        fps = end["fps"],
        codec = end["codec"].as_str().unwrap_or(""),
        encoder = end["encoder"].as_str().unwrap_or(""),
        sound = if end["sound"] == true { t!(" · со звуком") } else { String::new() },
        secs = format!("{secs:.1}"),
        mb = format!("{:.1}", size as f64 / 1_048_576.0),
    );
    let mut content = vec![json!({"type": "text", "text": summary})];
    if let Some(png) = middle_frame(&path, secs) {
        content.push(json!({"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(&png), "mimeType": "image/png"}));
        content.push(json!({"type": "text", "text": t!("кадр на {secs} с", secs = format!("{:.1}", secs / 2.0))}));
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
