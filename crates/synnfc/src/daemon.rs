//! Демон `synnfcd`: владеет контроллером NFC (только root — `/dev/nq-nci` 0660), режим
//! задают подписчики (программа «NFC», `synnfc read`), и он живёт, пока живо их соединение.
//!
//! Поток NFC: запуск чипа (CORE_RESET/INIT), карта протоколов (RF_DISCOVER_MAP), опрос
//! NFC-A/B/F/V или прослушивание A/B для эмуляции; на активации — чтение или запись метки,
//! затем снова опрос (RF_DEACTIVATE → discovery). Режим «выключено» — чип обесточен.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::api::{self, Event, Mode, Request, Response, Status};
use crate::emu::T4Emulator;
use crate::nci::{self, Nci, Packet};
use crate::ndef::{self, Record};
use crate::tag::{self, Activation};
use synshell_tr::t;

/// Узлы драйверов NFC Android (NXP, Qualcomm, ST); `SYNNFC_DEVICE` — свой.
const NODES: &[&str] = &["/dev/nq-nci", "/dev/nxp-nci", "/dev/pn553", "/dev/pn54x", "/dev/sn100u", "/dev/st21nfc"];

pub fn find_device() -> Option<String> {
    if let Ok(d) = std::env::var("SYNNFC_DEVICE") {
        return Some(d);
    }
    NODES.iter().find(|p| std::path::Path::new(p).exists()).map(|p| p.to_string())
}

enum Cmd {
    Mode(Mode, Vec<Record>),
}

struct Daemon {
    status: Mutex<Status>,
    subs: Mutex<HashMap<u64, Sender<String>>>,
    /// Чей режим (соединение) — при его закрытии чип выключается.
    owner: Mutex<Option<u64>>,
    tx: Mutex<Sender<Cmd>>,
}

impl Daemon {
    fn emit(&self, e: &Event) {
        let line = serde_json::to_string(e).unwrap_or_default();
        self.subs.lock().unwrap().retain(|_, s| s.send(line.clone()).is_ok());
    }

    fn set_status(&self, f: impl FnOnce(&mut Status)) {
        let st = {
            let mut s = self.status.lock().unwrap();
            f(&mut s);
            s.clone()
        };
        self.emit(&Event::Status(st));
    }
}

/// Запустить демон (не возвращается). Нет контроллера — выход без ошибки.
pub fn run() -> Result<()> {
    let Some(device) = find_device() else {
        tracing::info!("контроллер NFC не найден ({})", NODES.join(", "));
        return Ok(());
    };
    let (tx, rx) = mpsc::channel();
    let d = Arc::new(Daemon {
        status: Mutex::new(Status { present: true, device: device.clone(), ..Default::default() }),
        subs: Mutex::new(HashMap::new()),
        owner: Mutex::new(None),
        tx: Mutex::new(tx),
    });
    {
        let d = d.clone();
        let open: Opener = Box::new(move || {
            let (link, rx) = nci::open_device(&device)?;
            Ok(Nci::new(Box::new(link), rx))
        });
        std::thread::Builder::new().name("nfc".into()).spawn(move || worker(d, rx, open))?;
    }
    let dir = std::path::Path::new(api::SOCKET).parent().unwrap();
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::remove_file(api::SOCKET);
    let l = UnixListener::bind(api::SOCKET).context(api::SOCKET)?;
    std::fs::set_permissions(api::SOCKET, std::fs::Permissions::from_mode(0o666))?;
    tracing::info!("synnfcd: {}", api::SOCKET);
    static NEXT: AtomicU64 = AtomicU64::new(1);
    for s in l.incoming().flatten() {
        let d = d.clone();
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || serve(d, s, id));
    }
    Ok(())
}

fn serve(d: Arc<Daemon>, s: UnixStream, id: u64) {
    let Ok(read) = s.try_clone() else { return };
    let out = Arc::new(Mutex::new(s));
    let reply = |r: &Response| {
        let line = serde_json::to_string(r).unwrap_or_default();
        let _ = writeln!(out.lock().unwrap(), "{line}");
    };
    for line in BufReader::new(read).lines().map_while(|l| l.ok()) {
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                reply(&Response { ok: false, error: t!("неверный запрос: {e}", e = e), status: None });
                continue;
            }
        };
        let status = || Some(d.status.lock().unwrap().clone());
        match req {
            Request::Status => reply(&Response { ok: true, error: String::new(), status: status() }),
            Request::Subscribe => {
                let (tx, rx) = mpsc::channel::<String>();
                let first = serde_json::to_string(&Event::Status(d.status.lock().unwrap().clone())).unwrap_or_default();
                let _ = tx.send(first);
                d.subs.lock().unwrap().insert(id, tx);
                let out = out.clone();
                std::thread::spawn(move || {
                    for l in rx {
                        if writeln!(out.lock().unwrap(), "{l}").is_err() {
                            return;
                        }
                    }
                });
            }
            Request::Read | Request::Write { .. } | Request::Emulate { .. } | Request::Stop => {
                let (mode, records) = match req {
                    Request::Read => (Mode::Read, vec![]),
                    Request::Write { records } => (Mode::Write, records),
                    Request::Emulate { records } => (Mode::Emulate, records),
                    _ => (Mode::Off, vec![]),
                };
                *d.owner.lock().unwrap() = (mode != Mode::Off).then_some(id);
                let _ = d.tx.lock().unwrap().send(Cmd::Mode(mode, records));
                reply(&Response { ok: true, error: String::new(), status: status() });
            }
        }
    }
    // соединение закрыто: подписка и режим — его
    d.subs.lock().unwrap().remove(&id);
    let mut owner = d.owner.lock().unwrap();
    if *owner == Some(id) {
        *owner = None;
        let _ = d.tx.lock().unwrap().send(Cmd::Mode(Mode::Off, vec![]));
    }
}

/// Настроить обнаружение: опрос меток или прослушивание (эмуляция).
fn configure(n: &mut Nci, listen: bool) -> Result<()> {
    // в покой (уже в покое — ошибка состояния, не страшно)
    let _ = n.cmd(1, 6, &[0x00]);
    // карта протоколов: T1T/T2T/T3T — Frame, ISO-DEP — свой интерфейс (опрос и прослушивание)
    let mut map: Vec<[u8; 3]> = vec![[0x01, 0x01, 0x01], [0x02, 0x01, 0x01], [0x03, 0x01, 0x01], [0x04, 0x03, 0x02]];
    if n.info.version >= 0x20 {
        map.push([0x06, 0x01, 0x01]);
    }
    let with_mifare: Vec<[u8; 3]> = map.iter().copied().chain([[0x80, 0x01, 0x80]]).collect();
    let pack = |m: &[[u8; 3]]| -> Vec<u8> {
        let mut v = vec![m.len() as u8];
        for e in m {
            v.extend_from_slice(e);
        }
        v
    };
    // MIFARE Classic (NXP) — если контроллер не знает, без него
    if n.cmd_ok(1, 0, &pack(&with_mifare)).is_err() {
        n.cmd_ok(1, 0, &pack(&map))?;
    }
    let disc: Vec<[u8; 2]> = if listen {
        // ISO-DEP — хосту (протокол и технология A), SAK со «поддержкой ISO-DEP»
        let route = [0x00, 0x02, 0x01, 0x03, 0x00, 0x01, 0x04, 0x00, 0x03, 0x00, 0x01, 0x00];
        if let Err(e) = n.cmd_ok(1, 1, &route) {
            tracing::warn!("маршрут прослушивания: {e:#}");
        }
        if let Err(e) = n.cmd_ok(0, 2, &[0x01, 0x32, 0x01, 0x20]) {
            tracing::warn!("LA_SEL_INFO: {e:#}");
        }
        vec![[0x80, 0x01], [0x81, 0x01]]
    } else {
        let mut v = vec![[0x00, 0x01], [0x01, 0x01], [0x02, 0x01]];
        if n.info.version >= 0x20 {
            v.push([0x06, 0x01]);
        }
        v
    };
    let mut payload = vec![disc.len() as u8];
    for e in &disc {
        payload.extend_from_slice(e);
    }
    if n.cmd_ok(1, 3, &payload).is_err() && !listen && disc.len() > 3 {
        // без NFC-V
        n.cmd_ok(1, 3, &[0x03, 0x00, 0x01, 0x01, 0x01, 0x02, 0x01])?;
    }
    Ok(())
}

/// Несколько меток или протоколов: RF_DISCOVER_NTF до последнего — выбрать одну.
fn select(n: &mut Nci, first: Vec<u8>) -> Result<()> {
    let mut list = vec![first];
    let end = Instant::now() + Duration::from_millis(500);
    while list.last().and_then(|p| p.last()).copied() == Some(2) && Instant::now() < end {
        match n.event(Duration::from_millis(200))? {
            Some(Packet::Ntf { gid: 1, oid: 3, payload }) => list.push(payload),
            _ => break,
        }
    }
    // ISO-DEP лучше всего (NDEF Type 4), затем Type 2
    let pick = list.iter().find(|p| p.get(1) == Some(&tag::PROTO_ISO_DEP)).or_else(|| list.iter().find(|p| p.get(1) == Some(&tag::PROTO_T2T))).or(list.first());
    if let Some(p) = pick {
        let proto = p[1];
        let intf = match proto {
            tag::PROTO_ISO_DEP => 0x02,
            tag::PROTO_MIFARE => 0x80,
            _ => 0x01,
        };
        n.cmd_ok(1, 4, &[p[0], proto, intf])?;
    }
    Ok(())
}

/// Открыть контроллер (в тестах — имитация).
type Opener = Box<dyn Fn() -> Result<Nci> + Send>;

fn worker(d: Arc<Daemon>, rx: Receiver<Cmd>, open: Opener) {
    let mut nci: Option<Nci> = None;
    let mut mode = Mode::Off;
    let mut records: Vec<Record> = vec![];
    let mut configured: Option<bool> = None;
    let mut last: Option<(String, Instant)> = None;
    loop {
        let wait = if mode == Mode::Off { Duration::from_secs(3600) } else { Duration::ZERO };
        let mut cmd = match rx.recv_timeout(wait) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // берём последний из накопившихся
        while let Ok(c) = rx.try_recv() {
            cmd = Some(c);
        }
        if let Some(Cmd::Mode(m, r)) = cmd {
            if m != mode || m == Mode::Write || m == Mode::Emulate {
                configured = None;
            }
            mode = m;
            records = r;
            last = None;
            d.set_status(|s| {
                s.mode = m;
                s.error.clear();
            });
            tracing::info!("режим NFC: {m:?}");
        }
        if mode == Mode::Off {
            if let Some(mut n) = nci.take() {
                let _ = n.cmd(1, 6, &[0x00]);
                n.stop();
            }
            configured = None;
            continue;
        }
        if nci.is_none() {
            match open().and_then(|mut n| n.start().map(|_| n)) {
                Ok(n) => {
                    let i = &n.info;
                    let ctl = format!(
                        "NCI {}.{}{}{}",
                        i.version >> 4,
                        i.version & 15,
                        if i.manufacturer == 0x04 { " · NXP" } else { "" },
                        if i.firmware.is_empty() { String::new() } else { format!(" · {}", i.firmware) }
                    );
                    d.set_status(|s| s.controller = ctl);
                    nci = Some(n);
                }
                Err(e) => {
                    tracing::warn!("NFC не запустился: {e:#}");
                    d.set_status(|s| {
                        s.error = format!("{e:#}");
                        s.mode = Mode::Off;
                    });
                    mode = Mode::Off;
                    continue;
                }
            }
        }
        let n = nci.as_mut().unwrap();
        let listen = mode == Mode::Emulate;
        if configured != Some(listen) {
            if let Err(e) = configure(n, listen) {
                tracing::warn!("обнаружение NFC: {e:#}");
                d.set_status(|s| s.error = format!("{e:#}"));
                nci = None;
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
            configured = Some(listen);
        }
        let ev = match n.event(Duration::from_millis(200)) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("NFC: {e:#}");
                nci = None;
                continue;
            }
        };
        match ev {
            Some(Packet::Ntf { gid: 1, oid: 3, payload }) => {
                if let Err(e) = select(n, payload) {
                    tracing::debug!("выбор метки: {e:#}");
                }
            }
            Some(Packet::Ntf { gid: 1, oid: 5, payload }) => {
                let Some(a) = Activation::parse(&payload) else { continue };
                n.credits = a.credits.max(1);
                n.max_data = if a.max_payload == 0 { 255 } else { a.max_payload as usize };
                if a.listen() {
                    emulate(&d, n, &records);
                    continue;
                }
                let uid = ndef::hex(&a.uid().0);
                if mode == Mode::Write {
                    let r = tag::write(n, &a, &records);
                    let (ok, error) = match &r {
                        Ok(()) => (true, String::new()),
                        Err(e) => (false, format!("{e:#}")),
                    };
                    d.emit(&Event::Written { uid: uid.clone(), ok, error });
                    if ok {
                        mode = Mode::Read;
                        d.set_status(|s| s.mode = Mode::Read);
                    }
                    last = None;
                }
                let repeat = last.as_ref().is_some_and(|(u, t)| *u == uid && t.elapsed() < Duration::from_secs(3));
                if repeat {
                    // та же метка всё ещё у телефона — не читать заново, реже переобнаруживать
                    std::thread::sleep(Duration::from_millis(600));
                } else {
                    let t = tag::read(n, &a);
                    tracing::info!("метка {} {} {}", t.kind, t.uid, if t.error.is_empty() { "" } else { &t.error });
                    d.emit(&Event::Tag(t));
                }
                last = Some((uid, Instant::now()));
                // снова опрос
                let _ = n.cmd(1, 6, &[0x03]);
            }
            // деактивация, кредиты, ошибки — опрос продолжается сам
            Some(_) | None => {}
        }
    }
}

/// Считыватель активировал телефон как метку Type 4: отвечать на его APDU до деактивации.
fn emulate(d: &Daemon, n: &mut Nci, records: &[Record]) {
    let mut e = T4Emulator::new(records);
    let mut told = false;
    loop {
        let Ok(c) = n.wait_data(Duration::from_secs(5)) else { break };
        let r = e.apdu(&c);
        if let Err(err) = n.send_data(&r) {
            tracing::debug!("эмуляция: {err:#}");
            break;
        }
        if e.read_done && !told {
            told = true;
            d.emit(&Event::EmulationRead);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nci::fake;

    #[test]
    fn read_mode_reports_tag_and_powers_off() {
        // контроллер: запуск, карта, опрос; на «опрос» — сразу активация NTAG без NDEF-разметки
        let respond: fake::Responder = Box::new(|p: &[u8]| {
            if let Some(r) = fake::startup(p) {
                return r;
            }
            match (p[0], p[1]) {
                (0x21, 0x06) => vec![vec![0x41, 0x06, 0x01, 0x00]],
                (0x21, 0x00) => vec![vec![0x41, 0x00, 0x01, 0x00]],
                (0x21, 0x03) => vec![
                    vec![0x41, 0x03, 0x01, 0x00],
                    vec![0x61, 0x05, 0x17, 0x01, 0x01, 0x02, 0x00, 0xFF, 0x01, 0x0C, 0x44, 0x00, 0x07, 0x04, 0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00],
                ],
                // данные: READ — память без CC, остальное — статус ошибки
                (0x00, _) if p.get(3) == Some(&0x30) => {
                    let mut v = vec![0x00, 0x00, 17];
                    v.extend_from_slice(&[0x04, 0xA1, 0xB2, 0x00, 0xC3, 0xD4, 0xE5, 0xF6, 0x00, 0x48, 0, 0, 0, 0, 0, 0, 0x00]);
                    vec![v, vec![0x60, 0x06, 0x03, 0x01, 0x00, 0x01]]
                }
                (0x00, _) => vec![vec![0x00, 0x00, 0x01, 0x03], vec![0x60, 0x06, 0x03, 0x01, 0x00, 0x01]],
                _ => vec![],
            }
        });
        let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(respond)));
        let open: Opener = Box::new(move || {
            let r = slot.lock().unwrap().take().ok_or_else(|| anyhow::anyhow!("уже открыт"))?;
            Ok(fake::nci(r).0)
        });
        let (tx, rx) = mpsc::channel();
        let d = Arc::new(Daemon { status: Mutex::new(Status { present: true, ..Default::default() }), subs: Mutex::new(HashMap::new()), owner: Mutex::new(None), tx: Mutex::new(tx.clone()) });
        let (etx, erx) = mpsc::channel::<String>();
        d.subs.lock().unwrap().insert(1, etx);
        {
            let d = d.clone();
            std::thread::spawn(move || worker(d, rx, open));
        }
        tx.send(Cmd::Mode(Mode::Read, vec![])).unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        let mut tag = None;
        let mut controller = String::new();
        while Instant::now() < end && tag.is_none() {
            if let Ok(l) = erx.recv_timeout(Duration::from_millis(200)) {
                match serde_json::from_str::<Event>(&l).unwrap() {
                    Event::Tag(t) => tag = Some(t),
                    Event::Status(s) if !s.controller.is_empty() => controller = s.controller,
                    _ => {}
                }
            }
        }
        let tag = tag.expect("нет события метки");
        assert_eq!(tag.uid, "04A1B2C3D4E5F6");
        assert_eq!(tag.protocol, "T2T");
        assert_eq!(tag.ndef, None);
        assert_eq!(controller, "NCI 2.0 · NXP · 01.C0.EF");
        tx.send(Cmd::Mode(Mode::Off, vec![])).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(d.status.lock().unwrap().mode, Mode::Off);
    }
}
