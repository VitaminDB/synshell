//! Гостевой агент QEMU (qemu-ga, канал virtio-serial `org.qemu.guest_agent.0`): команды
//! внутри гостя без SSH и сети — запуск программ с кодом возврата и выводом, файлы туда и
//! обратно, выключение, сведения о системе и адресах. В Linux — пакет `qemu-guest-agent`,
//! в Windows — «Гостевые инструменты» с диска virtio-win (драйвер vioserial + служба QEMU-GA).
//!
//! Протокол — JSON построчно, как QMP, но без приветствия. Агент мог остаться с
//! недочитанным ответом прошлого клиента, поэтому каждое соединение начинается с
//! `guest-sync-delimited`: байт 0xFF сбрасывает разбор у агента, ответ приходит после 0xFF.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use serde_json::{json, Value};

use crate::model::Vm;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub struct Agent {
    w: UnixStream,
    r: BufReader<UnixStream>,
}

impl Agent {
    /// Соединиться и синхронизироваться; без ответа агента за `timeout` — ошибка.
    pub fn connect(vm: &Vm, timeout: Duration) -> Result<Agent> {
        let sock = vm.qga_path();
        if !sock.exists() {
            bail!("{}", not_configured(vm));
        }
        let s = UnixStream::connect(&sock).with_context(|| sock.display().to_string())?;
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        let w = s.try_clone()?;
        let mut a = Agent { w, r: BufReader::new(s) };
        a.sync().map_err(|e| anyhow!("{} ({e})", not_running(vm)))?;
        Ok(a)
    }

    fn sync(&mut self) -> Result<()> {
        let id = (std::process::id() as u64) << 20 ^ (crate::model::now() & 0xfffff);
        self.w.write_all(&[0xff])?;
        writeln!(self.w, "{}", json!({"execute": "guest-sync-delimited", "arguments": {"id": id}}))?;
        loop {
            // Пропустить всё до 0xFF, затем строка ответа.
            let mut b = [0u8; 1];
            loop {
                self.r.read_exact(&mut b)?;
                if b[0] == 0xff {
                    break;
                }
            }
            let mut line = String::new();
            self.r.read_line(&mut line)?;
            let v: Value = serde_json::from_str(line.trim_start_matches('\u{ff}')).unwrap_or(Value::Null);
            if v.get("return").and_then(Value::as_u64) == Some(id) {
                return Ok(());
            }
        }
    }

    pub fn call(&mut self, cmd: &str, args: Value) -> Result<Value> {
        let mut req = json!({ "execute": cmd });
        if !args.is_null() {
            req["arguments"] = args;
        }
        writeln!(self.w, "{req}")?;
        loop {
            let mut line = String::new();
            if self.r.read_line(&mut line)? == 0 {
                bail!("агент закрыл соединение");
            }
            let line = line.trim_matches(|c: char| c == '\u{ff}' || c.is_whitespace());
            if line.is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line)?;
            if let Some(r) = v.get("return") {
                return Ok(r.clone());
            }
            if let Some(e) = v.get("error") {
                bail!("{}", e.get("desc").and_then(Value::as_str).unwrap_or("guest agent error"));
            }
        }
    }

    /// Команда без ответа (агент не отвечает на `guest-shutdown`).
    pub fn send(&mut self, cmd: &str, args: Value) -> Result<()> {
        writeln!(self.w, "{}", json!({"execute": cmd, "arguments": args}))?;
        Ok(())
    }
}

fn not_configured(vm: &Vm) -> String {
    if vm.guest_agent {
        format!("«{}»: машина запущена без канала агента — перезапустите её", vm.name)
    } else {
        format!("«{}»: канал гостевого агента выключен в настройках (guest_agent = true)", vm.name)
    }
}

fn not_running(vm: &Vm) -> String {
    if vm.os.is_windows() {
        format!("«{}»: гостевой агент не отвечает — система ещё грузится или не установлены «Гостевые инструменты» (диск virtio-win → virtio-win-guest-tools.exe)", vm.name)
    } else {
        format!("«{}»: гостевой агент не отвечает — система ещё грузится или в госте нет qemu-guest-agent (pacman -S qemu-guest-agent / apt install qemu-guest-agent; systemctl enable --now qemu-guest-agent)", vm.name)
    }
}

/// Один вызов.
pub fn call(vm: &Vm, cmd: &str, args: Value) -> Result<Value> {
    Agent::connect(vm, Duration::from_secs(3))?.call(cmd, args)
}

/// Агент отвечает.
pub fn ping(vm: &Vm) -> bool {
    vm.qga_path().exists() && Agent::connect(vm, Duration::from_millis(700)).and_then(|mut a| a.call("guest-ping", Value::Null)).is_ok()
}

/// Ждать агента (загрузку системы).
pub fn wait(vm: &Vm, timeout: Duration) -> Result<()> {
    let t0 = Instant::now();
    loop {
        if crate::qemu::pid(vm).is_none() {
            bail!("«{}» не запущена", vm.name);
        }
        if ping(vm) {
            return Ok(());
        }
        if t0.elapsed() > timeout {
            bail!("{}", not_running(vm));
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

/// Итог `guest-exec`.
#[derive(Debug, Default, Clone)]
pub struct ExecResult {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Не дождались: процесс в госте продолжает работать.
    pub timed_out: bool,
    pub pid: i64,
}

/// Запустить программу в госте и дождаться: `argv[0]` ищется по PATH гостя.
pub fn exec(vm: &Vm, argv: &[String], input: Option<&[u8]>, env: &[String], timeout: Duration) -> Result<ExecResult> {
    let Some((prog, args)) = argv.split_first() else { bail!("пустая команда") };
    let mut a = json!({"path": prog, "arg": args, "capture-output": true});
    if let Some(i) = input {
        a["input-data"] = json!(B64.encode(i));
    }
    if !env.is_empty() {
        a["env"] = json!(env);
    }
    let r = call(vm, "guest-exec", a)?;
    let pid = r.get("pid").and_then(Value::as_i64).context("guest-exec: нет pid")?;
    let t0 = Instant::now();
    let mut delay = Duration::from_millis(50);
    loop {
        let s = call(vm, "guest-exec-status", json!({"pid": pid}))?;
        if s.get("exited").and_then(Value::as_bool) == Some(true) {
            let dec = |k: &str| s.get(k).and_then(Value::as_str).and_then(|b| B64.decode(b).ok()).unwrap_or_default();
            let code = s.get("exitcode").and_then(Value::as_i64).map(|c| c as i32).or_else(|| s.get("signal").and_then(Value::as_i64).map(|sg| 128 + sg as i32)).unwrap_or(0);
            return Ok(ExecResult { code, stdout: dec("out-data"), stderr: dec("err-data"), timed_out: false, pid });
        }
        if t0.elapsed() > timeout {
            return Ok(ExecResult { code: 124, timed_out: true, pid, ..Default::default() });
        }
        std::thread::sleep(delay);
        delay = (delay * 2).min(Duration::from_millis(500));
    }
}

/// Строка оболочки гостя: `sh -c` в Linux, `cmd /c` в Windows.
pub fn shell_argv(vm: &Vm, line: &str) -> Vec<String> {
    if guest_is_windows(vm) {
        vec!["cmd.exe".into(), "/c".into(), line.into()]
    } else {
        vec!["/bin/sh".into(), "-c".into(), line.into()]
    }
}

/// Windows в госте: по настройке машины, а если она «другая» — по ответу агента.
pub fn guest_is_windows(vm: &Vm) -> bool {
    vm.os.is_windows()
        || matches!(vm.os, crate::model::OsKind::Other)
            && call(vm, "guest-get-osinfo", Value::Null).ok().and_then(|v| v.get("id").and_then(Value::as_str).map(|s| s == "mswindows")).unwrap_or(false)
}

const CHUNK: usize = 1 << 20;

/// Файл хоста → гость. Возвращает число байт.
pub fn push(vm: &Vm, local: &Path, guest: &str) -> Result<u64> {
    let mut f = std::fs::File::open(local).with_context(|| local.display().to_string())?;
    let mut a = Agent::connect(vm, Duration::from_secs(30))?;
    let h = a.call("guest-file-open", json!({"path": guest, "mode": "wb"}))?.as_i64().context("guest-file-open")?;
    let r = (|| -> Result<u64> {
        let mut buf = vec![0u8; CHUNK];
        let mut total = 0u64;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            a.call("guest-file-write", json!({"handle": h, "buf-b64": B64.encode(&buf[..n])}))?;
            total += n as u64;
        }
        Ok(total)
    })();
    let _ = a.call("guest-file-close", json!({"handle": h}));
    r
}

/// Файл гостя → хост.
pub fn pull(vm: &Vm, guest: &str, local: &Path) -> Result<u64> {
    let mut a = Agent::connect(vm, Duration::from_secs(30))?;
    let h = a.call("guest-file-open", json!({"path": guest, "mode": "rb"}))?.as_i64().context("guest-file-open")?;
    let tmp = local.with_extension("syn-vm-part");
    let r = (|| -> Result<u64> {
        let mut out = std::fs::File::create(&tmp).with_context(|| tmp.display().to_string())?;
        let mut total = 0u64;
        loop {
            let v = a.call("guest-file-read", json!({"handle": h, "count": CHUNK}))?;
            let data = v.get("buf-b64").and_then(Value::as_str).map(|b| B64.decode(b)).transpose()?.unwrap_or_default();
            out.write_all(&data)?;
            total += data.len() as u64;
            if v.get("eof").and_then(Value::as_bool).unwrap_or(data.is_empty()) {
                break;
            }
        }
        Ok(total)
    })();
    let _ = a.call("guest-file-close", json!({"handle": h}));
    match r {
        Ok(n) => {
            std::fs::rename(&tmp, local)?;
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Прочитать небольшой файл гостя целиком.
pub fn read_file(vm: &Vm, guest: &str, max: usize) -> Result<Vec<u8>> {
    let mut a = Agent::connect(vm, Duration::from_secs(30))?;
    let h = a.call("guest-file-open", json!({"path": guest, "mode": "rb"}))?.as_i64().context("guest-file-open")?;
    let mut out = Vec::new();
    let r = (|| -> Result<()> {
        while out.len() < max {
            let v = a.call("guest-file-read", json!({"handle": h, "count": CHUNK.min(max - out.len())}))?;
            let data = v.get("buf-b64").and_then(Value::as_str).map(|b| B64.decode(b)).transpose()?.unwrap_or_default();
            let eof = v.get("eof").and_then(Value::as_bool).unwrap_or(data.is_empty());
            out.extend_from_slice(&data);
            if eof {
                break;
            }
        }
        Ok(())
    })();
    let _ = a.call("guest-file-close", json!({"handle": h}));
    r.map(|_| out)
}

pub fn write_file(vm: &Vm, guest: &str, data: &[u8]) -> Result<()> {
    let mut a = Agent::connect(vm, Duration::from_secs(30))?;
    let h = a.call("guest-file-open", json!({"path": guest, "mode": "wb"}))?.as_i64().context("guest-file-open")?;
    let r = data.chunks(CHUNK).try_for_each(|c| a.call("guest-file-write", json!({"handle": h, "buf-b64": B64.encode(c)})).map(|_| ()));
    let _ = a.call("guest-file-close", json!({"handle": h}));
    r
}

/// Выключить через агента (`powerdown`, `reboot`, `halt`).
pub fn shutdown(vm: &Vm, mode: &str) -> Result<()> {
    Agent::connect(vm, Duration::from_secs(3))?.send("guest-shutdown", json!({"mode": mode}))
}

/// Сведения о госте для обзора: система, имя, адреса.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuestInfo {
    pub os: String,
    pub hostname: String,
    pub ips: Vec<String>,
    pub agent_version: String,
}

pub fn info(vm: &Vm) -> Result<GuestInfo> {
    let mut a = Agent::connect(vm, Duration::from_secs(2))?;
    let mut g = GuestInfo::default();
    if let Ok(v) = a.call("guest-info", Value::Null) {
        g.agent_version = v.get("version").and_then(Value::as_str).unwrap_or_default().to_string();
    }
    if let Ok(v) = a.call("guest-get-osinfo", Value::Null) {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        g.os = [s("pretty-name"), s("name")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
    }
    if let Ok(v) = a.call("guest-get-host-name", Value::Null) {
        g.hostname = v.get("host-name").and_then(Value::as_str).unwrap_or_default().to_string();
    }
    if let Ok(v) = a.call("guest-network-get-interfaces", Value::Null) {
        for i in v.as_array().into_iter().flatten() {
            if i.get("name").and_then(Value::as_str).is_some_and(|n| n == "lo" || n.starts_with("Loopback")) {
                continue;
            }
            for ip in i.get("ip-addresses").and_then(Value::as_array).into_iter().flatten() {
                let addr = ip.get("ip-address").and_then(Value::as_str).unwrap_or_default();
                if addr.is_empty() || addr.starts_with("127.") || addr == "::1" || addr.starts_with("fe80") {
                    continue;
                }
                g.ips.push(addr.to_string());
            }
        }
    }
    Ok(g)
}

/// Поставить часы гостя по хосту (после восстановления снимка или паузы).
pub fn sync_time(vm: &Vm) -> Result<()> {
    let ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos() as i64;
    call(vm, "guest-set-time", json!({"time": ns})).map(|_| ())
}
