//! QMP — управление работающей машиной: состояние, пауза, выключение, привод,
//! снимки, кадр экрана. Соединение на одну команду: QEMU держит одного клиента
//! на сокете, а команды редки.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use syngui::t;

use crate::model::Vm;

/// Один вызов: команда и аргументы → поле `return`.
pub fn call(vm: &Vm, cmd: &str, args: Value) -> Result<Value> {
    call_at(&vm.qmp_path(), cmd, args, Duration::from_secs(5))
}

pub fn call_at(sock: &Path, cmd: &str, args: Value, timeout: Duration) -> Result<Value> {
    Conn::open(sock, timeout)?.call(cmd, args)
}

/// Соединение на несколько команд подряд (ввод текста, проверки): QEMU держит одного
/// клиента, так что соединение не стоит держать дольше, чем нужно.
pub struct Conn {
    w: UnixStream,
    r: BufReader<UnixStream>,
}

impl Conn {
    pub fn open(sock: &Path, timeout: Duration) -> Result<Conn> {
        let s = UnixStream::connect(sock)?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        let w = s.try_clone()?;
        let mut r = BufReader::new(s);
        let mut line = String::new();
        // Приветствие.
        r.read_line(&mut line)?;
        if !line.contains("QMP") {
            bail!("QMP: {}", line.trim());
        }
        let mut c = Conn { w, r };
        c.call("qmp_capabilities", Value::Null)?;
        Ok(c)
    }

    pub fn of(vm: &Vm) -> Result<Conn> {
        Conn::open(&vm.qmp_path(), Duration::from_secs(5))
    }

    pub fn call(&mut self, cmd: &str, args: Value) -> Result<Value> {
        let mut req = json!({ "execute": cmd });
        if !args.is_null() {
            req["arguments"] = args;
        }
        writeln!(self.w, "{req}")?;
        read_reply(&mut self.r)
    }
}

fn read_reply(r: &mut BufReader<UnixStream>) -> Result<Value> {
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            bail!("QMP: {}", t!("соединение закрыто"));
        }
        let v: Value = serde_json::from_str(&line)?;
        if let Some(ret) = v.get("return") {
            return Ok(ret.clone());
        }
        if let Some(e) = v.get("error") {
            return Err(anyhow!("{}", e.get("desc").and_then(|d| d.as_str()).unwrap_or("QMP error")));
        }
        // События (`event`) пропускаются.
    }
}

/// Отдать гостю USB-порт на ходу.
pub fn usb_attach(vm: &Vm, port: &str) -> Result<()> {
    let Some((bus, path)) = crate::usb::split_port(port) else { bail!("USB: {port}") };
    call(vm, "device_add", json!({"driver": "usb-host", "bus": "xhci.0", "hostbus": bus, "hostport": path, "id": crate::usb::qemu_id(port)}))?;
    Ok(())
}

/// Вернуть USB-порт компьютеру на ходу.
pub fn usb_detach(vm: &Vm, port: &str) -> Result<()> {
    call(vm, "device_del", json!({"id": crate::usb::qemu_id(port)}))?;
    Ok(())
}

/// Состояние QEMU: `running`, `paused`, `suspended`, `shutdown`, …
pub fn status(vm: &Vm) -> Option<String> {
    let v = call_at(&vm.qmp_path(), "query-status", Value::Null, Duration::from_millis(800)).ok()?;
    v.get("status").and_then(|s| s.as_str()).map(String::from)
}

/// Команда монитора (HMP): `savevm`, `info snapshots`…
pub fn hmp(vm: &Vm, line: &str) -> Result<String> {
    let v = call_at(&vm.qmp_path(), "human-monitor-command", json!({ "command-line": line }), Duration::from_secs(600))?;
    let out = v.as_str().unwrap_or_default().trim().to_string();
    // HMP сообщает об ошибках текстом, а не ошибкой QMP.
    if out.starts_with("Error") || out.contains("Error:") || out.starts_with("Could not") {
        bail!("{out}");
    }
    Ok(out)
}

pub fn powerdown(vm: &Vm) -> Result<()> {
    call(vm, "system_powerdown", Value::Null).map(|_| ())
}

pub fn reset(vm: &Vm) -> Result<()> {
    call(vm, "system_reset", Value::Null).map(|_| ())
}

pub fn quit(vm: &Vm) -> Result<()> {
    call(vm, "quit", Value::Null).map(|_| ())
}

pub fn pause(vm: &Vm) -> Result<()> {
    call(vm, "stop", Value::Null).map(|_| ())
}

pub fn resume(vm: &Vm) -> Result<()> {
    call(vm, "cont", Value::Null).map(|_| ())
}

/// Вставить образ в привод (пустая строка — извлечь).
pub fn change_cdrom(vm: &Vm, iso: &str) -> Result<()> {
    if iso.is_empty() {
        call(vm, "eject", json!({ "id": "cdrom", "force": true })).map(|_| ())
    } else {
        call(vm, "blockdev-change-medium", json!({ "id": "cdrom", "filename": iso, "format": "raw", "read-only-mode": "read-only" })).map(|_| ())
    }
}

/// Кадр экрана в PNG.
pub fn screendump(vm: &Vm, out: &Path) -> Result<()> {
    call_at(&vm.qmp_path(), "screendump", json!({ "filename": out.to_string_lossy(), "format": "png" }), Duration::from_secs(3)).map(|_| ())
}

// ─── Ввод без окна: текст, сочетания, мышь ──────────────────────────────

/// Символ → (qcode, нужен Shift) для раскладки US: гость получает клавиши, а не символы.
pub fn char_qcode(c: char) -> Option<(String, bool)> {
    let plain = |s: &str| Some((s.to_string(), false));
    let shift = |s: &str| Some((s.to_string(), true));
    match c {
        'a'..='z' | '0'..='9' => plain(&c.to_string()),
        'A'..='Z' => shift(&c.to_ascii_lowercase().to_string()),
        ' ' => plain("spc"),
        '\n' => plain("ret"),
        '\t' => plain("tab"),
        '-' => plain("minus"),
        '=' => plain("equal"),
        '[' => plain("bracket_left"),
        ']' => plain("bracket_right"),
        '\\' => plain("backslash"),
        ';' => plain("semicolon"),
        '\'' => plain("apostrophe"),
        '`' => plain("grave_accent"),
        ',' => plain("comma"),
        '.' => plain("dot"),
        '/' => plain("slash"),
        '!' => shift("1"),
        '@' => shift("2"),
        '#' => shift("3"),
        '$' => shift("4"),
        '%' => shift("5"),
        '^' => shift("6"),
        '&' => shift("7"),
        '*' => shift("8"),
        '(' => shift("9"),
        ')' => shift("0"),
        '_' => shift("minus"),
        '+' => shift("equal"),
        '{' => shift("bracket_left"),
        '}' => shift("bracket_right"),
        '|' => shift("backslash"),
        ':' => shift("semicolon"),
        '"' => shift("apostrophe"),
        '~' => shift("grave_accent"),
        '<' => shift("comma"),
        '>' => shift("dot"),
        '?' => shift("slash"),
        _ => None,
    }
}

/// Имя клавиши в сочетании → qcode: `ctrl`, `alt`, `del`, `enter`, `f4`, `win`, `a`, `/`…
pub fn key_qcode(name: &str) -> Option<String> {
    let n = name.trim().to_ascii_lowercase();
    let q = match n.as_str() {
        "ctrl" | "control" | "ctrl_l" => "ctrl",
        "ctrl_r" | "rctrl" => "ctrl_r",
        "alt" | "alt_l" => "alt",
        "altgr" | "alt_r" | "ralt" => "alt_r",
        "shift" | "shift_l" => "shift",
        "shift_r" | "rshift" => "shift_r",
        "super" | "win" | "meta" | "cmd" | "logo" | "meta_l" => "meta_l",
        "del" | "delete" => "delete",
        "ins" | "insert" => "insert",
        "enter" | "return" | "ret" => "ret",
        "esc" | "escape" => "esc",
        "space" | "spc" => "spc",
        "backspace" | "bksp" | "bs" => "backspace",
        "tab" => "tab",
        "up" | "down" | "left" | "right" | "home" | "end" | "pause" | "menu" | "print" | "sysrq" => n.as_str(),
        "pgup" | "pageup" | "page_up" => "pgup",
        "pgdn" | "pagedown" | "page_down" => "pgdn",
        "capslock" | "caps" | "caps_lock" => "caps_lock",
        "minus" | "equal" | "comma" | "dot" | "slash" | "semicolon" | "apostrophe" | "backslash" | "grave_accent" | "bracket_left" | "bracket_right" => n.as_str(),
        _ if n.len() >= 2 && n.starts_with('f') && n[1..].parse::<u8>().is_ok_and(|x| (1..=24).contains(&x)) => n.as_str(),
        _ => {
            let mut ch = n.chars();
            return match (ch.next(), ch.next()) {
                (Some(c), None) => char_qcode(c).map(|(q, _)| q),
                _ => None,
            };
        }
    };
    Some(q.to_string())
}

fn key_list(codes: &[String]) -> Value {
    Value::Array(codes.iter().map(|q| json!({"type": "qcode", "data": q})).collect())
}

/// Нажать сочетание: `ctrl+alt+delete`, `ctrl-alt-del`, `alt+f4`, `win+r`, `enter`.
pub fn send_combo(vm: &Vm, combo: &str) -> Result<()> {
    let parts: Vec<&str> = if combo.contains('+') { combo.split('+').collect() } else if combo.len() > 1 && combo.contains('-') && combo != "-" { combo.split('-').collect() } else { vec![combo] };
    let codes = parts.iter().map(|p| key_qcode(p).ok_or_else(|| anyhow!("{}", t!("неизвестная клавиша «{p}»", p = p)))).collect::<Result<Vec<_>>>()?;
    call(vm, "send-key", json!({ "keys": key_list(&codes), "hold-time": 120 }))?;
    Ok(())
}

/// Набрать текст (латиница и знаки раскладки US; `\n` — Enter).
pub fn type_text(vm: &Vm, text: &str, delay: Duration) -> Result<()> {
    let keys = text.chars().map(|c| char_qcode(c).ok_or_else(|| anyhow!("{}", t!("символ «{c}» нельзя набрать клавишами US — используйте --exec или буфер обмена", c = c)))).collect::<Result<Vec<_>>>()?;
    let mut c = Conn::of(vm)?;
    for (q, sh) in keys {
        let codes = if sh { vec!["shift".to_string(), q] } else { vec![q] };
        c.call("send-key", json!({ "keys": key_list(&codes), "hold-time": 30 }))?;
        std::thread::sleep(delay);
    }
    Ok(())
}

/// Щелчок (или перемещение без кнопки) по точке экрана гостя: координаты — пиксели кадра
/// `width`×`height` (абсолютный указатель usb-tablet).
pub fn click(vm: &Vm, x: f64, y: f64, size: (u32, u32), button: Option<&str>, double: bool) -> Result<()> {
    let (w, h) = (size.0.max(1) as f64, size.1.max(1) as f64);
    let ax = ((x / w).clamp(0.0, 1.0) * 32767.0).round() as i64;
    let ay = ((y / h).clamp(0.0, 1.0) * 32767.0).round() as i64;
    let mut c = Conn::of(vm)?;
    c.call("input-send-event", json!({"events": [
        {"type": "abs", "data": {"axis": "x", "value": ax}},
        {"type": "abs", "data": {"axis": "y", "value": ay}}]}))?;
    if let Some(b) = button {
        for _ in 0..if double { 2 } else { 1 } {
            std::thread::sleep(Duration::from_millis(40));
            c.call("input-send-event", json!({"events": [{"type": "btn", "data": {"down": true, "button": b}}]}))?;
            std::thread::sleep(Duration::from_millis(60));
            c.call("input-send-event", json!({"events": [{"type": "btn", "data": {"down": false, "button": b}}]}))?;
        }
    }
    Ok(())
}

/// Прокрутка колесом: `steps` > 0 — вниз.
pub fn wheel(vm: &Vm, steps: i32) -> Result<()> {
    let b = if steps > 0 { "wheel-down" } else { "wheel-up" };
    let mut c = Conn::of(vm)?;
    for _ in 0..steps.unsigned_abs() {
        c.call("input-send-event", json!({"events": [{"type": "btn", "data": {"down": true, "button": b}}]}))?;
        c.call("input-send-event", json!({"events": [{"type": "btn", "data": {"down": false, "button": b}}]}))?;
    }
    Ok(())
}

/// Снимок состояния машины (qcow2, внутренний).
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub name: String,
    /// Размер сохранённой памяти, человеческой строкой QEMU.
    pub vm_size: String,
    pub date: String,
}

/// Разбор `info snapshots`.
pub fn parse_snapshots(out: &str) -> Vec<Snapshot> {
    // ID        TAG               VM SIZE                DATE     VM CLOCK     ICOUNT
    // --        start             1.21 GiB 2026-10-14 12:00:00 00:01:02.345
    let mut v = Vec::new();
    for l in out.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 6 || f[0] == "ID" || f[0] == "List" || !(f[0] == "--" || f[0].chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        // Дата — первое поле вида ГГГГ-ММ-ДД; всё между тегом и размером — тег.
        let Some(di) = f.iter().position(|x| x.len() == 10 && x.as_bytes().get(4) == Some(&b'-') && x.as_bytes().get(7) == Some(&b'-')) else { continue };
        if di < 4 {
            continue;
        }
        let name = f[1..di - 2].join(" ");
        let vm_size = format!("{} {}", f[di - 2], f[di - 1]);
        let date = format!("{} {}", f[di], f.get(di + 1).copied().unwrap_or_default());
        v.push(Snapshot { name, vm_size, date });
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots() {
        let out = "List of snapshots present on all disks:\r\nID        TAG               VM SIZE                DATE     VM CLOCK     ICOUNT\r\n--        clean install    1.21 GiB 2026-10-14 12:00:00 00:01:02.345\r\n--        after-update     512 MiB 2026-10-15 09:30:01 01:11:02.000\r\n";
        let s = parse_snapshots(out);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "clean install");
        assert_eq!(s[0].vm_size, "1.21 GiB");
        assert_eq!(s[1].date, "2026-10-15 09:30:01");
        assert!(parse_snapshots("There is no snapshot available.").is_empty());
    }
}
