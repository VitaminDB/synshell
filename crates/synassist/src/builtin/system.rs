use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{run, which};
use crate::tools::{need_str, str_arg, Danger, Tool, ToolOutput, ToolResult};

pub struct SystemTool;

const SETTINGS_PAGES: &str = "themes, appearance, wallpaper, panels, windows, workspaces, keyboard, shortcuts, mouse, displays, wifi, audio, bluetooth, mobile-network, vpn, devices, phone, hardware, power, models, location, camera, language, datetime, lock, autostart, default-apps, about";

fn desktop_dirs() -> Vec<PathBuf> {
    let home = super::home();
    let mut dirs = vec![home.join(".local/share/applications")];
    let data = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(data.split(':').filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join("applications")));
    dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

struct Entry {
    id: String,
    name: String,
    names: Vec<String>,
    exec: String,
}

fn parse_entry(path: &Path) -> Option<Entry> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_main = false;
    let (mut name, mut exec, mut hidden) = (String::new(), String::new(), false);
    let mut names = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_main = l == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        match k.trim() {
            "Name" => name = v.trim().to_string(),
            "Exec" => exec = v.trim().to_string(),
            "NoDisplay" | "Hidden" if v.trim() == "true" => hidden = true,
            k if k.starts_with("Name[") || k == "GenericName" || k.starts_with("GenericName[") || k == "Keywords" || k.starts_with("Keywords[") => {
                names.extend(v.split(';').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()))
            }
            _ => {}
        }
    }
    if hidden || exec.is_empty() || name.is_empty() {
        return None;
    }
    let id = path.file_stem()?.to_string_lossy().into_owned();
    names.push(name.to_lowercase());
    names.push(id.to_lowercase());
    Some(Entry { id, name, names, exec })
}

fn find_app(query: &str) -> Option<Entry> {
    let q = query.trim().to_lowercase();
    let mut best: Option<(u8, Entry)> = None;
    for dir in desktop_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "desktop") {
                continue;
            }
            let Some(entry) = parse_entry(&p) else { continue };
            let score = if entry.names.iter().any(|n| *n == q) {
                3
            } else if entry.names.iter().any(|n| n.starts_with(&q)) {
                2
            } else if entry.names.iter().any(|n| n.contains(&q)) || entry.exec.to_lowercase().contains(&q) {
                1
            } else {
                0
            };
            if score > 0 && best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, entry));
            }
        }
    }
    best.map(|(_, e)| e)
}

fn clean_exec(exec: &str) -> String {
    exec.split_whitespace().filter(|w| !(w.starts_with('%') && w.len() == 2)).collect::<Vec<_>>().join(" ")
}

pub fn spawn(cmdline: &str) -> Result<(), String> {
    if which("synwm") && run("synwm", &["msg", "action", &format!("spawn {cmdline}")]).is_ok() {
        return Ok(());
    }
    std::process::Command::new("sh").args(["-c", &format!("setsid -f {cmdline} >/dev/null 2>&1")]).spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn shell_action(action: &str) -> Result<String, String> {
    run("synwm", &["msg", "action", action])
}

fn fmt_volume(raw: &str) -> String {
    let muted = raw.contains("MUTED");
    let v: f64 = raw.split_whitespace().nth(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    format!("{:.0}%{}", v * 100.0, if muted { " (звук выключен)" } else { "" })
}

fn volume(args: &Value) -> ToolResult {
    let sink = "@DEFAULT_AUDIO_SINK@";
    if let Some(m) = args.get("mute").and_then(Value::as_bool) {
        run("wpctl", &["set-mute", sink, if m { "1" } else { "0" }])?;
    }
    if let Some(v) = args.get("level").and_then(Value::as_f64) {
        run("wpctl", &["set-volume", "-l", "1.0", sink, &format!("{}%", v.clamp(0.0, 100.0).round())])?;
    }
    if let Some(d) = args.get("change").and_then(Value::as_f64) {
        let arg = if d >= 0.0 { format!("{}%+", d.round()) } else { format!("{}%-", (-d).round()) };
        run("wpctl", &["set-volume", "-l", "1.0", sink, &arg])?;
    }
    let now = run("wpctl", &["get-volume", sink])?;
    Ok(ToolOutput::text(format!("Громкость: {}", fmt_volume(&now))))
}

fn brightness(args: &Value) -> ToolResult {
    if let Some(v) = args.get("level").and_then(Value::as_f64) {
        run("brightnessctl", &["-q", "set", &format!("{}%", v.clamp(1.0, 100.0).round())])?;
    } else if let Some(d) = args.get("change").and_then(Value::as_f64) {
        let arg = if d >= 0.0 { format!("+{}%", d.round()) } else { format!("{}%-", (-d).round()) };
        run("brightnessctl", &["-q", "set", &arg])?;
    }
    let cur: f64 = run("brightnessctl", &["get"])?.parse().unwrap_or(0.0);
    let max: f64 = run("brightnessctl", &["max"])?.parse().unwrap_or(1.0);
    Ok(ToolOutput::text(format!("Яркость: {:.0}%", cur / max.max(1.0) * 100.0)))
}

fn radio(kind: &str, state: Option<&str>) -> ToolResult {
    match kind {
        "wifi" => {
            if let Some(s) = state {
                run("nmcli", &["radio", "wifi", if s == "on" { "on" } else { "off" }])?;
            }
            Ok(ToolOutput::text(format!("Wi‑Fi: {}", run("nmcli", &["radio", "wifi"])?)))
        }
        _ => {
            if let Some(s) = state {
                run("bluetoothctl", &["power", if s == "on" { "on" } else { "off" }])?;
            }
            let show = run("bluetoothctl", &["show"]).unwrap_or_default();
            let powered = show.lines().find(|l| l.trim_start().starts_with("Powered:")).map(|l| l.trim().to_string()).unwrap_or_default();
            Ok(ToolOutput::text(format!("Bluetooth: {powered}")))
        }
    }
}

fn info() -> ToolResult {
    let mut out = Vec::new();
    if let Ok(up) = std::fs::read_to_string("/proc/uptime") {
        let secs: f64 = up.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        out.push(format!("Работает: {} ч {} мин", (secs / 3600.0) as u64, ((secs % 3600.0) / 60.0) as u64));
    }
    if let Ok(mem) = std::fs::read_to_string("/proc/meminfo") {
        let field = |k: &str| mem.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        out.push(format!("Память: свободно {:.1} из {:.1} ГБ", field("MemAvailable:") as f64 / 1048576.0, field("MemTotal:") as f64 / 1048576.0));
    }
    for bat in ["BAT0", "BAT1", "battery"] {
        let base = format!("/sys/class/power_supply/{bat}");
        if let Ok(cap) = std::fs::read_to_string(format!("{base}/capacity")) {
            let status = std::fs::read_to_string(format!("{base}/status")).unwrap_or_default();
            out.push(format!("Батарея: {}% ({})", cap.trim(), status.trim()));
            break;
        }
    }
    if let Ok(v) = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]) {
        out.push(format!("Громкость: {}", fmt_volume(&v)));
    }
    if let Ok(host) = std::fs::read_to_string("/etc/hostname") {
        out.push(format!("Имя компьютера: {}", host.trim()));
    }
    Ok(ToolOutput::text(out.join("\n")))
}

fn reminder(args: &Value) -> ToolResult {
    let minutes = args.get("minutes").and_then(Value::as_f64).ok_or("нужен параметр «minutes»")?;
    let text = str_arg(args, "text").unwrap_or("Напоминание");
    let secs = (minutes * 60.0).max(1.0).round() as u64;
    let q = text.replace('\'', "'\\''");
    std::process::Command::new("sh")
        .args(["-c", &format!("setsid -f sh -c 'sleep {secs}; notify-send -a Ассистент -u critical \"Напоминание\" \"{q}\"' >/dev/null 2>&1")])
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(ToolOutput::text(format!("Напомню через {minutes} мин: {text}")))
}

impl Tool for SystemTool {
    fn name(&self) -> &'static str {
        "system"
    }
    fn group(&self) -> &'static str {
        "система"
    }
    fn description(&self) -> String {
        format!(
            "Управление системой. action: open_app (app — название программы), open_settings (page — одна из: {SETTINGS_PAGES}), volume (без параметров — узнать текущую громкость; level 0-100 — задать; change ±N — изменить; mute true/false), brightness (без параметров — узнать; level | change), wifi/bluetooth (state on|off, без state — узнать), notify (title, text), reminder (minutes, text), info (время работы, память, батарея, громкость), lock, suspend, reboot, poweroff."
        )
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["open_app", "open_settings", "volume", "brightness", "wifi", "bluetooth", "notify", "reminder", "info", "lock", "suspend", "reboot", "poweroff"]},
                "app": {"type": "string"},
                "page": {"type": "string"},
                "level": {"type": "number"},
                "change": {"type": "number"},
                "mute": {"type": "boolean"},
                "state": {"type": "string", "enum": ["on", "off"]},
                "title": {"type": "string"},
                "text": {"type": "string"},
                "minutes": {"type": "number"}
            },
            "required": ["action"]
        })
    }
    fn danger(&self, args: &Value) -> Danger {
        match str_arg(args, "action") {
            Some("reboot" | "poweroff" | "suspend") => Danger::Confirm,
            _ => Danger::Safe,
        }
    }
    fn summary(&self, args: &Value) -> String {
        let a = str_arg(args, "action").unwrap_or("?");
        match a {
            "open_app" => format!("Открыть «{}»", str_arg(args, "app").unwrap_or("?")),
            "open_settings" => format!("Параметры: {}", str_arg(args, "page").unwrap_or("")),
            "volume" => "Громкость".into(),
            "brightness" => "Яркость".into(),
            "wifi" => "Wi‑Fi".into(),
            "bluetooth" => "Bluetooth".into(),
            "notify" => format!("Уведомление: {}", str_arg(args, "title").unwrap_or("")),
            "reminder" => format!("Напоминание через {} мин", args.get("minutes").and_then(Value::as_f64).unwrap_or(0.0)),
            "info" => "Состояние системы".into(),
            "lock" => "Заблокировать экран".into(),
            "suspend" => "Сон".into(),
            "reboot" => "Перезагрузка".into(),
            "poweroff" => "Выключение".into(),
            other => other.to_string(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        match need_str(args, "action")? {
            "open_app" => {
                let q = need_str(args, "app")?;
                let e = find_app(q).ok_or_else(|| format!("программа «{q}» не найдена"))?;
                spawn(&clean_exec(&e.exec))?;
                Ok(ToolOutput::text(format!("Запущена «{}» ({})", e.name, e.id)))
            }
            "open_settings" => {
                let page = str_arg(args, "page").unwrap_or("");
                spawn(&format!("synsettings {page}"))?;
                Ok(ToolOutput::text(format!("Открыты Параметры{}", if page.is_empty() { String::new() } else { format!(" → {page}") })))
            }
            "volume" => volume(args),
            "brightness" => brightness(args),
            "wifi" => radio("wifi", str_arg(args, "state")),
            "bluetooth" => radio("bluetooth", str_arg(args, "state")),
            "notify" => {
                run("notify-send", &["-a", "Ассистент", str_arg(args, "title").unwrap_or("Ассистент"), str_arg(args, "text").unwrap_or("")])?;
                Ok(ToolOutput::text("Уведомление показано"))
            }
            "reminder" => reminder(args),
            "info" => info(),
            "lock" => shell_action("lock").map(|_| ToolOutput::text("Экран заблокирован")),
            "suspend" => shell_action("suspend").map(|_| ToolOutput::text("Ухожу в сон")),
            "reboot" => shell_action("reboot").map(|_| ToolOutput::text("Перезагружаюсь")),
            "poweroff" => shell_action("poweroff").map(|_| ToolOutput::text("Выключаюсь")),
            other => Err(format!("неизвестное действие {other}")),
        }
    }
}

pub struct WindowsTool;

impl Tool for WindowsTool {
    fn name(&self) -> &'static str {
        "windows"
    }
    fn group(&self) -> &'static str {
        "система"
    }
    fn description(&self) -> String {
        "Окна программ. action: list — список открытых окон (id, программа, заголовок); focus, minimize, maximize, close — с id окна.".into()
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "focus", "minimize", "maximize", "close"]},
                "id": {"type": "integer"}
            },
            "required": ["action"]
        })
    }
    fn danger(&self, args: &Value) -> Danger {
        if str_arg(args, "action") == Some("close") {
            Danger::Confirm
        } else {
            Danger::Safe
        }
    }
    fn summary(&self, args: &Value) -> String {
        let a = str_arg(args, "action").unwrap_or("list");
        match args.get("id").and_then(Value::as_u64) {
            Some(id) => format!("Окно {id}: {a}"),
            None => "Список окон".into(),
        }
    }
    fn run(&self, args: &Value) -> ToolResult {
        let action = need_str(args, "action")?;
        if action == "list" {
            let raw = run("synwm", &["msg", "windows"])?;
            let v: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            let list: Vec<String> = v["windows"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|w| {
                            format!(
                                "{} · {} · {}{}",
                                w["id"],
                                w["app_id"].as_str().unwrap_or(""),
                                w["title"].as_str().unwrap_or(""),
                                if w["focused"].as_bool() == Some(true) { " (активно)" } else if w["minimized"].as_bool() == Some(true) { " (свёрнуто)" } else { "" }
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Ok(ToolOutput::text(if list.is_empty() { "Открытых окон нет".into() } else { list.join("\n") }));
        }
        let id = args.get("id").and_then(Value::as_u64).ok_or("нужен id окна")?;
        let op = match action {
            "focus" => "activate",
            "minimize" => "minimize",
            "maximize" => "maximize",
            "close" => "close",
            other => return Err(format!("неизвестное действие {other}")),
        };
        run("synwm", &["msg", "window", &id.to_string(), op])?;
        Ok(ToolOutput::text(format!("Окно {id}: {action}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_cleanup_and_entry() {
        assert_eq!(clean_exec("firefox %u --new"), "firefox --new");
        assert_eq!(fmt_volume("Volume: 0.35 [MUTED]"), "35% (звук выключен)");
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("org.test.App.desktop");
        std::fs::write(&p, "[Desktop Entry]\nName=Test App\nName[ru]=Тестовая\nExec=testapp %F\nKeywords=demo;пример;\n").unwrap();
        let e = parse_entry(&p).unwrap();
        assert!(e.names.contains(&"тестовая".to_string()));
        assert!(e.names.contains(&"пример".to_string()));
        assert_eq!(e.id, "org.test.App");
    }
}
