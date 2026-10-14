//! QMP — управление работающей машиной: состояние, пауза, выключение, привод,
//! снимки, кадр экрана. Соединение на одну команду: QEMU держит одного клиента
//! на сокете, а команды редки.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::model::Vm;

/// Один вызов: команда и аргументы → поле `return`.
pub fn call(vm: &Vm, cmd: &str, args: Value) -> Result<Value> {
    call_at(&vm.qmp_path(), cmd, args, Duration::from_secs(5))
}

pub fn call_at(sock: &Path, cmd: &str, args: Value, timeout: Duration) -> Result<Value> {
    let s = UnixStream::connect(sock)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    let mut w = s.try_clone()?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    // Приветствие.
    r.read_line(&mut line)?;
    if !line.contains("QMP") {
        bail!("QMP: {}", line.trim());
    }
    writeln!(w, "{}", json!({"execute": "qmp_capabilities"}))?;
    read_reply(&mut r)?;
    let mut req = json!({ "execute": cmd });
    if !args.is_null() {
        req["arguments"] = args;
    }
    writeln!(w, "{req}")?;
    read_reply(&mut r)
}

fn read_reply(r: &mut BufReader<UnixStream>) -> Result<Value> {
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            bail!("QMP: соединение закрыто");
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
        call(vm, "eject", json!({ "id": "cd0", "force": true })).map(|_| ())
    } else {
        call(vm, "blockdev-change-medium", json!({ "id": "cd0", "filename": iso, "format": "raw", "read-only-mode": "read-only" })).map(|_| ())
    }
}

/// Кадр экрана в PNG.
pub fn screendump(vm: &Vm, out: &Path) -> Result<()> {
    call_at(&vm.qmp_path(), "screendump", json!({ "filename": out.to_string_lossy(), "format": "png" }), Duration::from_secs(3)).map(|_| ())
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
