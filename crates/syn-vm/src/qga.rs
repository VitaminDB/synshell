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
use syngui::t;

use crate::model::Vm;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

pub struct Agent {
    w: UnixStream,
    r: BufReader<UnixStream>,
    /// Блокировка канала машины на время соединения: у QEMU на канале агента один клиент,
    /// остальные ждут своей очереди здесь, а не повисают на сокете.
    _lock: std::fs::File,
}

/// Взять блокировку канала агента машины (flock на файл в каталоге запуска), ждать до `deadline`.
fn lock_channel(vm: &Vm, deadline: Instant) -> Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let path = vm.run_dir().join("qga.lock");
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).with_context(|| path.display().to_string())?;
    loop {
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(f);
        }
        if Instant::now() >= deadline {
            bail!("{}", t!("«{name}»: гостевой агент занят другим запросом (передача файла?) — повторите позже", name = vm.name));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Agent {
    /// Соединиться и синхронизироваться, повторяя до `wait`: после отключения прошлого
    /// клиента агент (особенно в Windows) заново открывает порт virtio-serial и какое-то
    /// время не отвечает — цепочка push → exec не должна от этого падать.
    pub fn connect(vm: &Vm, wait: Duration) -> Result<Agent> {
        let sock = vm.qga_path();
        if !sock.exists() {
            bail!("{}", not_configured(vm));
        }
        let deadline = Instant::now() + wait;
        let lock = lock_channel(vm, deadline)?;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let attempt = (|| -> Result<Agent> {
                let s = UnixStream::connect(&sock).with_context(|| sock.display().to_string())?;
                s.set_read_timeout(Some(left.clamp(Duration::from_millis(300), Duration::from_millis(1500))))?;
                s.set_write_timeout(Some(Duration::from_secs(5)))?;
                let w = s.try_clone()?;
                let mut a = Agent { w, r: BufReader::new(s), _lock: lock.try_clone()? };
                // Нет ответа — повторить синхронизацию в том же соединении: переподключение
                // заставляет агент Windows заново открывать порт virtio-serial, и частые
                // переподключения сами не давали ему ответить.
                loop {
                    match a.sync() {
                        Ok(()) => break,
                        Err(e) if is_timeout(&e) && Instant::now() + Duration::from_millis(300) < deadline => continue,
                        Err(e) => return Err(e),
                    }
                }
                // Дальше — ответы на команды (чтение больших файлов, долгие команды).
                a.r.get_ref().set_read_timeout(Some(Duration::from_secs(60)))?;
                Ok(a)
            })();
            match attempt {
                Ok(a) => return Ok(a),
                Err(e) if Instant::now() + Duration::from_millis(200) >= deadline => return Err(anyhow!("{} ({e})", not_running(vm))),
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    fn sync(&mut self) -> Result<()> {
        let id = sync_id();
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
            tracing::debug!("qga sync {id}: {}", line.trim());
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
                bail!("{}", t!("агент закрыл соединение"));
            }
            let line = line.trim_matches(|c: char| c == '\u{ff}' || c.is_whitespace());
            if line.is_empty() {
                continue;
            }
            tracing::debug!("qga {cmd}: {}", &line[..line.len().min(300)]);
            let v: Value = serde_json::from_str(line)?;
            // Запоздалый ответ на синхронизацию (прошлой попытки, прошлого соединения или другой
            // программы): без пропуска он сходил за ответ на команду, а настоящий ответ терялся —
            // так пропадал итог guest-exec-status (агент отдаёт его один раз) и следующий опрос
            // получал «PID does not exist».
            if v.get("return").and_then(Value::as_u64).is_some_and(is_sync_id) {
                continue;
            }
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

/// Ответ не пришёл за время ожидания (сокет жив).
fn is_timeout(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some_and(|e| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

/// Метка номеров синхронизации (старшие биты): по ней запоздалый ответ на синхронизацию
/// отличается от ответа на команду (числа агента — время в нс — с ней не совпадают).
const SYNC_TAG: u64 = 0x5356;

/// Свой номер на каждую синхронизацию: метка, процесс, счётчик.
fn sync_id() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + (crate::model::now() << 8);
    SYNC_TAG << 40 | (std::process::id() as u64 & 0xffff) << 24 | (n & 0xff_ffff)
}

fn is_sync_id(v: u64) -> bool {
    v >> 40 == SYNC_TAG
}

fn not_configured(vm: &Vm) -> String {
    if vm.guest_agent {
        t!("«{name}»: машина запущена без канала агента — перезапустите её", name = vm.name)
    } else {
        t!("«{name}»: канал гостевого агента выключен в настройках (guest_agent = true)", name = vm.name)
    }
}

fn not_running(vm: &Vm) -> String {
    if vm.os.is_windows() {
        t!("«{name}»: гостевой агент не отвечает — система ещё грузится или не установлены «Гостевые инструменты» (диск virtio-win → virtio-win-guest-tools.exe)", name = vm.name)
    } else {
        t!("«{name}»: гостевой агент не отвечает — система ещё грузится или в госте нет qemu-guest-agent (pacman -S qemu-guest-agent / apt install qemu-guest-agent; systemctl enable --now qemu-guest-agent)", name = vm.name)
    }
}

/// Один вызов.
pub fn call(vm: &Vm, cmd: &str, args: Value) -> Result<Value> {
    Agent::connect(vm, Duration::from_secs(10))?.call(cmd, args)
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
            bail!("{}", t!("«{name}» не запущена", name = vm.name));
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
    let Some((prog, args)) = argv.split_first() else { bail!("{}", t!("пустая команда")) };
    let mut a = json!({"path": prog, "arg": args, "capture-output": true});
    if let Some(i) = input {
        a["input-data"] = json!(B64.encode(i));
    }
    if !env.is_empty() {
        a["env"] = json!(env);
    }
    let mut agent = Agent::connect(vm, Duration::from_secs(10))?;
    let r = agent.call("guest-exec", a)?;
    let pid = r.get("pid").and_then(Value::as_i64).with_context(|| format!("guest-exec: {}", t!("нет pid")))?;
    wait_exec_on(vm, Some(agent), pid, timeout)
}

/// Запустить и не ждать: pid в госте (итог — [`exec_status`]).
pub fn exec_detached(vm: &Vm, argv: &[String], input: Option<&[u8]>, env: &[String]) -> Result<i64> {
    let Some((prog, args)) = argv.split_first() else { bail!("{}", t!("пустая команда")) };
    let mut a = json!({"path": prog, "arg": args, "capture-output": true});
    if let Some(i) = input {
        a["input-data"] = json!(B64.encode(i));
    }
    if !env.is_empty() {
        a["env"] = json!(env);
    }
    call(vm, "guest-exec", a)?.get("pid").and_then(Value::as_i64).with_context(|| format!("guest-exec: {}", t!("нет pid")))
}


/// Итог запущенной команды: `None` — ещё работает. Агент отдаёт вывод один раз — после
/// успешного ответа о завершении запись о процессе у него удаляется.
pub fn exec_status(vm: &Vm, pid: i64) -> Result<Option<ExecResult>> {
    exec_result(&call(vm, "guest-exec-status", json!({"pid": pid}))?, pid)
}

/// Ответ guest-exec-status → итог (`None` — ещё работает).
fn exec_result(s: &Value, pid: i64) -> Result<Option<ExecResult>> {
    if s.get("exited").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let dec = |k: &str| s.get(k).and_then(Value::as_str).and_then(|b| B64.decode(b).ok()).unwrap_or_default();
    let code = s.get("exitcode").and_then(Value::as_i64).map(|c| c as i32).or_else(|| s.get("signal").and_then(Value::as_i64).map(|sg| 128 + sg as i32)).unwrap_or(0);
    Ok(Some(ExecResult { code, stdout: dec("out-data"), stderr: dec("err-data"), timed_out: false, pid }))
}

/// Ждать завершения по одному соединению: каждое переподключение заставляет агент Windows
/// заново открывать порт virtio-serial, и при опросе отдельными соединениями он то и дело
/// переставал отвечать на секунды. Пока ждём, другие запросы к агенту этой машины стоят в
/// очереди блокировки канала; соединение оборвалось — переподключиться.
pub fn wait_exec(vm: &Vm, pid: i64, timeout: Duration) -> Result<ExecResult> {
    wait_exec_on(vm, None, pid, timeout)
}

fn wait_exec_on(vm: &Vm, agent: Option<Agent>, pid: i64, timeout: Duration) -> Result<ExecResult> {
    let t0 = Instant::now();
    let mut delay = Duration::from_millis(50);
    let mut seen = false;
    let mut agent = agent;
    loop {
        let a = match agent.as_mut() {
            Some(a) => a,
            None => agent.insert(Agent::connect(vm, Duration::from_secs(10))?),
        };
        match a.call("guest-exec-status", json!({"pid": pid})).and_then(|s| exec_result(&s, pid)) {
            Ok(Some(r)) => return Ok(r),
            Ok(None) => seen = true,
            // Агент уже отдал итог (и забыл процесс), но ответ до нас не дошёл: процесс завершён,
            // код и вывод потеряны — сказать прямо, а не «PID lld does not exist».
            Err(e) if seen && e.to_string().contains("does not exist") => {
                bail!("{}", t!("процесс {pid} в госте завершился, но агент уже отдал его итог (код и вывод потеряны)", pid = pid))
            }
            Err(e) if e.downcast_ref::<std::io::Error>().is_some() => {
                tracing::debug!("qga: соединение при ожидании {pid}: {e} — переподключение");
                agent = None;
            }
            Err(e) => return Err(e),
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
    let mut a = Agent::connect(vm, Duration::from_secs(10))?;
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
    let mut a = Agent::connect(vm, Duration::from_secs(10))?;
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
    let mut a = Agent::connect(vm, Duration::from_secs(10))?;
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
    let mut a = Agent::connect(vm, Duration::from_secs(10))?;
    let h = a.call("guest-file-open", json!({"path": guest, "mode": "wb"}))?.as_i64().context("guest-file-open")?;
    let r = data.chunks(CHUNK).try_for_each(|c| a.call("guest-file-write", json!({"handle": h, "buf-b64": B64.encode(c)})).map(|_| ()));
    let _ = a.call("guest-file-close", json!({"handle": h}));
    r
}

/// Выключить через агента (`powerdown`, `reboot`, `halt`).
pub fn shutdown(vm: &Vm, mode: &str) -> Result<()> {
    Agent::connect(vm, Duration::from_secs(10))?.send("guest-shutdown", json!({"mode": mode}))
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
    let mut a = Agent::connect(vm, Duration::from_secs(6))?;
    let mut g = GuestInfo::default();
    if let Ok(v) = a.call("guest-info", Value::Null) {
        g.agent_version = v.get("version").and_then(Value::as_str).unwrap_or_default().to_string();
    }
    if let Ok(v) = a.call("guest-get-osinfo", Value::Null) {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        g.os = [s("pretty-name"), s("name")].into_iter().find(|x| !x.is_empty()).unwrap_or_default();
        // qemu-ga называет Windows 11 «Windows 10 Pro»: решает номер сборки (11 — с 22000).
        if s("id") == "mswindows" && s("kernel-release").parse::<u32>().is_ok_and(|b| b >= 22000) && g.os.contains("Windows 10") {
            g.os = g.os.replace("Windows 10", "Windows 11");
        }
        if s("id") == "mswindows" && !s("kernel-release").is_empty() {
            g.os = t!("{os} (сборка {build})", os = g.os, build = s("kernel-release"));
        }
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

/// Вывод программ Windows приходит в OEM-кодировке гостя (cp866 у русской системы):
/// перевести в UTF-8. Кодовая страница спрашивается у гостя один раз за запуск машины.
pub fn decode_output(vm: &Vm, bytes: Vec<u8>) -> Vec<u8> {
    if bytes.is_empty() || std::str::from_utf8(&bytes).is_ok() || !guest_is_windows(vm) {
        return bytes;
    }
    let cache = vm.run_dir().join("oemcp");
    let cp = std::fs::read_to_string(&cache).ok().and_then(|s| s.trim().parse::<u32>().ok()).or_else(|| {
        let r = exec(vm, &["reg".into(), "query".into(), r"HKLM\SYSTEM\CurrentControlSet\Control\Nls\CodePage".into(), "/v".into(), "OEMCP".into()], None, &[], Duration::from_secs(20)).ok()?;
        let cp = String::from_utf8_lossy(&r.stdout).split_whitespace().last()?.parse::<u32>().ok()?;
        let _ = std::fs::write(&cache, cp.to_string());
        Some(cp)
    });
    // Кодовые страницы OEM (консоль) и ANSI (GetACP) системы: в одном выводе бывают обе —
    // cmd.exe пишет в канал в OEM, а pnputil, certutil и другие — в ANSI.
    let (oem, ansi): (&'static encoding_rs::Encoding, &'static encoding_rs::Encoding) = match cp {
        Some(866) => (encoding_rs::IBM866, encoding_rs::WINDOWS_1251),
        Some(1251) => (encoding_rs::WINDOWS_1251, encoding_rs::WINDOWS_1251),
        Some(852) | Some(1250) => (encoding_rs::WINDOWS_1250, encoding_rs::WINDOWS_1250),
        Some(932) => (encoding_rs::SHIFT_JIS, encoding_rs::SHIFT_JIS),
        Some(936) => (encoding_rs::GBK, encoding_rs::GBK),
        Some(949) => (encoding_rs::EUC_KR, encoding_rs::EUC_KR),
        Some(950) => (encoding_rs::BIG5, encoding_rs::BIG5),
        _ => (encoding_rs::WINDOWS_1252, encoding_rs::WINDOWS_1252),
    };
    decode_mixed(&bytes, oem, ansi).into_bytes()
}

/// Построчно: UTF-8 — как есть, иначе из OEM и ANSI — вариант, похожий на текст (больше
/// букв, нет псевдографики: «╤ыєцхсэр» — признак ANSI, прочитанного как cp866).
fn decode_mixed(bytes: &[u8], oem: &'static encoding_rs::Encoding, ansi: &'static encoding_rs::Encoding) -> String {
    fn score(s: &str) -> i64 {
        s.chars()
            .map(|c| match c {
                '\u{2500}'..='\u{259F}' => -4, // рамки и заливки
                '\u{FFFD}' => -8,
                c if c.is_alphabetic() => 2,
                c if c.is_control() && !matches!(c, '\t' | '\r' | '\n') => -2,
                _ => 0,
            })
            .sum()
    }
    let mut out = String::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if let Ok(s) = std::str::from_utf8(line) {
            out.push_str(s);
            continue;
        }
        let a = oem.decode(line).0;
        if std::ptr::eq(oem, ansi) {
            out.push_str(&a);
            continue;
        }
        let b = ansi.decode(line).0;
        out.push_str(if score(&b) > score(&a) { &b } else { &a });
    }
    out
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    #[test]
    fn oem_and_ansi_lines_in_one_output() {
        // cmd.exe — OEM (cp866), pnputil — ANSI (cp1251), и строка в UTF-8.
        let cmd = encoding_rs::IBM866.encode("Системе не удается найти указанный путь.\r\n").0;
        let pnp = encoding_rs::WINDOWS_1251.encode("Служебная программа PnP (Майкрософт)\r\n").0;
        let mut b = Vec::new();
        b.extend_from_slice(&cmd);
        b.extend_from_slice(&pnp);
        b.extend_from_slice("Готово ✓\n".as_bytes());
        let s = decode_mixed(&b, encoding_rs::IBM866, encoding_rs::WINDOWS_1251);
        assert_eq!(s, "Системе не удается найти указанный путь.\r\nСлужебная программа PnP (Майкрософт)\r\nГотово ✓\n");
    }

    #[test]
    fn plain_ascii_stays() {
        let s = decode_mixed(b"Device ID: PCI\\VEN_1234\r\n", encoding_rs::IBM866, encoding_rs::WINDOWS_1251);
        assert_eq!(s, "Device ID: PCI\\VEN_1234\r\n");
    }

    #[test]
    fn sync_ids_unique_and_tagged() {
        let (a, b) = (sync_id(), sync_id());
        assert_ne!(a, b);
        assert!(is_sync_id(a) && is_sync_id(b));
        // guest-get-time (нс с 1970) и мелкие числа — не номера синхронизации.
        assert!(!is_sync_id(1_793_954_132_060_000_000));
        assert!(!is_sync_id(0) && !is_sync_id(9500));
    }
}
