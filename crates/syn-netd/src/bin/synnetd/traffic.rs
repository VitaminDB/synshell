//! Учёт трафика по программам и журнал соединений.
//!
//! Раз в секунду служба читает таблицы BPF ([`crate::tbpf`]): новые адреса
//! назначения (кто, куда, когда), байты каждого сокета (разность с прошлым
//! разом идёт программе-владельцу и в часовую ячейку дня), закрытые сокеты
//! (запись в журнал). Сокеты, о которых не было `connect` (принятые
//! входящие, открытые до запуска службы), узнаются через sock_diag — адрес,
//! inode и по нему процесс. Трафик канала, не доставшийся программам
//! (ответы ядра, пересылка, туннели), идёт «системе».
//!
//! На диске (`/var/lib/synnetd/traffic`):
//! * `log/<мс>.jsonl` — журнал закрытых соединений, кольцо файлов с общим
//!   пределом [`syn_netd::TrafficConfig::log_mb`]: старые файлы удаляются;
//! * `usage/ГГГГ-ММ-ДД.json` — расход за день по программам, каналам и
//!   часам; хранится [`syn_netd::TrafficConfig::days`] дней.

use std::collections::{HashMap, HashSet};
use std::io::{BufWriter, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use syn_netd::{AppTraffic, Config, Conn, ConnQuery, DayUsage, UsageRow, APP_SYSTEM};

use crate::tbpf;

/// Сколько адресов UDP-сокета без `connect` записывать отдельно.
const ALSO_MAX: usize = 16;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn boot_ns() -> u64 {
    let mut ts: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// Дата `ГГГГ-ММ-ДД` и час по местному времени.
pub fn local(ms: u64) -> (String, usize) {
    let t = (ms / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    (format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday), tm.tm_hour.clamp(0, 23) as usize)
}

fn is_local(ip: &IpAddr) -> bool {
    ip.is_loopback() || ip.is_unspecified()
}

fn proto_name(p: u8) -> String {
    match p as i32 {
        libc::IPPROTO_TCP => "tcp".into(),
        libc::IPPROTO_UDP => "udp".into(),
        libc::IPPROTO_ICMP | libc::IPPROTO_ICMPV6 => "icmp".into(),
        0 => String::new(),
        x => x.to_string(),
    }
}

fn ifname(idx: u32, cache: &mut HashMap<u32, String>) -> String {
    if idx == 0 {
        return String::new();
    }
    cache
        .entry(idx)
        .or_insert_with(|| {
            let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
            let p = unsafe { libc::if_indextoname(idx, buf.as_mut_ptr()) };
            if p.is_null() {
                format!("if{idx}")
            } else {
                unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
            }
        })
        .clone()
}

/// Программа процесса: имя исполняемого файла; у интерпретаторов — имя
/// скрипта, у Wine — имя `.exe`. Процесса уже нет — имя из BPF (`comm`).
pub fn app_of(pid: i32, comm: &str) -> String {
    let base = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or(s).to_string();
    let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
        return if comm.is_empty() { APP_SYSTEM.to_string() } else { comm.to_string() };
    };
    let exe = base(exe.to_string_lossy().trim_end_matches(" (deleted)"));
    let argv: Vec<String> = std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| b.split(|c| *c == 0).filter(|s| !s.is_empty()).take(8).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
        .unwrap_or_default();
    if exe.starts_with("wine") {
        if let Some(a) = argv.first() {
            let b = base(a);
            if b.to_ascii_lowercase().ends_with(".exe") {
                return b;
            }
        }
        if comm.to_ascii_lowercase().ends_with(".exe") {
            return comm.to_string();
        }
    }
    let interp = ["python", "perl", "ruby", "node", "java", "bash", "sh", "mono", "dotnet"];
    if interp.iter().any(|i| exe.starts_with(i)) {
        if let Some(script) = argv.iter().skip(1).find(|a| !a.starts_with('-') && !a.contains('=')) {
            let b = base(script);
            if !b.is_empty() && b != "-c" {
                return b;
            }
        }
    }
    exe
}

/// Сокет, который сейчас ведёт учёт.
struct Sock {
    owner: Option<Owner>,
    /// Сколько раз не нашли владельца через sock_diag.
    tries: u8,
    proto: u8,
    remote: Option<(IpAddr, u16)>,
    also: Vec<String>,
    more: u32,
    seen: HashSet<(IpAddr, u16)>,
    /// Ключи `conns` этого сокета — убрать при закрытии.
    keys: Vec<Vec<u8>>,
    host: Option<String>,
    start: u64,
    /// Последние показания `bytes`.
    rx: u64,
    tx: u64,
    /// Байты, пришедшие до того, как стал известен владелец.
    pend_rx: u64,
    pend_tx: u64,
    iface: String,
    incoming: bool,
    /// Когда последний раз менялись байты.
    changed: Instant,
}

#[derive(Clone)]
struct Owner {
    app: String,
    comm: String,
    pid: i32,
    uid: u32,
}

#[derive(Default, Clone)]
struct Row {
    rx: u64,
    tx: u64,
    hours: [u64; 24],
    conns: u32,
}

#[derive(Default)]
struct Day {
    date: String,
    rows: HashMap<(String, u32, String), Row>,
    dirty: bool,
}

impl Day {
    fn credit(&mut self, app: &str, uid: u32, iface: &str, rx: u64, tx: u64, hour: usize) {
        if rx == 0 && tx == 0 {
            return;
        }
        let r = self.rows.entry((app.to_string(), uid, iface.to_string())).or_default();
        r.rx += rx;
        r.tx += tx;
        r.hours[hour] += rx + tx;
        self.dirty = true;
    }

    fn to_usage(&self) -> DayUsage {
        let mut rows: Vec<UsageRow> = self
            .rows
            .iter()
            .map(|((app, uid, iface), r)| UsageRow { app: app.clone(), uid: *uid, iface: iface.clone(), rx: r.rx, tx: r.tx, hours: r.hours.to_vec(), conns: r.conns })
            .collect();
        rows.sort_by(|a, b| (b.rx + b.tx).cmp(&(a.rx + a.tx)));
        DayUsage { day: self.date.clone(), rows }
    }

    fn from_usage(u: DayUsage) -> Day {
        let mut d = Day { date: u.day, ..Default::default() };
        for r in u.rows {
            let mut hours = [0u64; 24];
            for (i, h) in r.hours.iter().take(24).enumerate() {
                hours[i] = *h;
            }
            d.rows.insert((r.app, r.uid, r.iface), Row { rx: r.rx, tx: r.tx, hours, conns: r.conns });
        }
        d
    }
}

/// Кольцо файлов журнала.
struct Log {
    dir: PathBuf,
    cur: Option<(PathBuf, BufWriter<std::fs::File>, u64)>,
}

impl Log {
    fn segments(&self) -> Vec<(u64, PathBuf, u64)> {
        let mut v: Vec<(u64, PathBuf, u64)> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_string();
                let ms = name.strip_suffix(".jsonl")?.parse::<u64>().ok()?;
                Some((ms, e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)))
            })
            .collect();
        v.sort_by_key(|s| s.0);
        v
    }

    fn write(&mut self, c: &Conn, limit: u64) {
        let seg_max = (limit / 16).max(256 * 1024);
        if self.cur.as_ref().is_some_and(|c| c.2 >= seg_max) {
            self.flush();
            self.cur = None;
            // Предел: самые старые файлы — прочь.
            let segs = self.segments();
            let mut total: u64 = segs.iter().map(|s| s.2).sum();
            for (_, p, len) in segs {
                if total <= limit.saturating_sub(seg_max) {
                    break;
                }
                let _ = std::fs::remove_file(p);
                total = total.saturating_sub(len);
            }
        }
        if self.cur.is_none() {
            let _ = std::fs::create_dir_all(&self.dir);
            // Продолжить последний недописанный файл.
            let segs = self.segments();
            let newest = segs.last().map(|s| s.0).unwrap_or(0);
            let last = segs.into_iter().last().filter(|s| s.2 < seg_max);
            let path = last.as_ref().map(|s| s.1.clone()).unwrap_or_else(|| self.dir.join(format!("{}.jsonl", now_ms().max(newest + 1))));
            match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                Ok(f) => {
                    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                    self.cur = Some((path, BufWriter::new(f), len));
                }
                Err(e) => {
                    tracing::warn!("журнал {}: {e}", path.display());
                    return;
                }
            }
        }
        if let Some((_, w, len)) = &mut self.cur {
            if let Ok(mut line) = serde_json::to_string(c) {
                line.push('\n');
                if w.write_all(line.as_bytes()).is_ok() {
                    *len += line.len() as u64;
                }
            }
        }
    }

    fn flush(&mut self) {
        if let Some((_, w, _)) = &mut self.cur {
            let _ = w.flush();
        }
    }
}

struct State {
    acc: Option<tbpf::Accounting>,
    error: Option<String>,
    socks: HashMap<u64, Sock>,
    ifnames: HashMap<u32, String>,
    day: Day,
    log: Log,
    /// Скорость по программам за последний такт: (app, uid) → (rx/с, tx/с).
    rates: HashMap<(String, u32), (f64, f64)>,
    last_tick: Option<Instant>,
    /// Счётчики каналов прошлого такта — для доли «системы».
    if_last: HashMap<String, (u64, u64)>,
    last_diag: Option<Instant>,
    last_save: Instant,
    owners: HashMap<i32, (Owner, Instant)>,
    /// Недавно закрытые сокеты: ядро ещё досылает FIN/RST осиротевшего TCP —
    /// эти байты идут прежнему владельцу, а не новым «сокетом без хозяина».
    recent: HashMap<u64, (Owner, String, Instant)>,
    last_sweep: Instant,
}

pub struct Traffic {
    cfg: Arc<RwLock<Config>>,
    dir: PathBuf,
    st: Mutex<State>,
    pub names: Arc<Mutex<crate::dns::Names>>,
}

impl Traffic {
    pub fn new(cfg: Arc<RwLock<Config>>, dir: PathBuf) -> Traffic {
        // Журнал — чужие адреса всех пользователей: каталог только для root
        // (StateDirectory службы — 0755). Пользователи читают через сокет.
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let today = local(now_ms()).0;
        let day = std::fs::read_to_string(dir.join("usage").join(format!("{today}.json")))
            .ok()
            .and_then(|s| serde_json::from_str::<DayUsage>(&s).ok())
            .map(Day::from_usage)
            .unwrap_or(Day { date: today, ..Default::default() });
        let names = Arc::new(Mutex::new(crate::dns::Names::default()));
        // Подписка дешёвая; включено ли «Имена узлов» — смотрится при записи.
        crate::dns::start(names.clone());
        Traffic {
            st: Mutex::new(State {
                acc: None,
                error: None,
                socks: HashMap::new(),
                ifnames: HashMap::new(),
                day,
                log: Log { dir: dir.join("log"), cur: None },
                rates: HashMap::new(),
                last_tick: None,
                if_last: HashMap::new(),
                last_diag: None,
                last_save: Instant::now(),
                owners: HashMap::new(),
                recent: HashMap::new(),
                last_sweep: Instant::now(),
            }),
            cfg,
            dir,
            names,
        }
    }

    fn owner(st: &mut State, pid: i32, uid: u32, comm: &str) -> Owner {
        let fresh = st.owners.get(&pid).filter(|(o, t)| t.elapsed().as_secs() < 30 && (comm.is_empty() || o.comm == comm || o.comm.starts_with(comm)));
        if let Some((o, _)) = fresh {
            return o.clone();
        }
        let comm = if comm.is_empty() { std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_string()).unwrap_or_default() } else { comm.to_string() };
        let o = Owner { app: if pid <= 0 { APP_SYSTEM.to_string() } else { app_of(pid, &comm) }, comm, pid, uid };
        if st.owners.len() > 4096 {
            st.owners.retain(|_, (_, t)| t.elapsed().as_secs() < 30);
        }
        st.owners.insert(pid, (o.clone(), Instant::now()));
        o
    }

    /// Один такт (раз в секунду). `uplinks` — каналы в интернет (для доли «системы»).
    pub fn tick(&self, uplinks: &[String]) {
        let tcfg = self.cfg.read().unwrap().traffic.clone();
        let mut guard = self.st.lock().unwrap();
        let st = &mut *guard;
        // Включить или выключить.
        if tcfg.enabled && st.acc.is_none() && st.error.is_none() {
            match tbpf::Accounting::new() {
                Ok(a) => {
                    tracing::info!("учёт трафика включён{}", if a.has_comm { "" } else { " (ядро не даёт имя процесса — берётся из /proc)" });
                    st.acc = Some(a);
                }
                Err(e) => {
                    tracing::warn!("учёт трафика: {e}");
                    st.error = Some(e.to_string());
                }
            }
        } else if !tcfg.enabled && st.acc.is_some() {
            self.finish_all(st);
            st.acc = None;
            tracing::info!("учёт трафика выключен");
        }
        if !tcfg.enabled {
            st.error = None;
        }
        let now = now_ms();
        let (date, hour) = local(now);
        if date != st.day.date {
            self.save_day(st);
            st.day = Day { date, ..Default::default() };
            self.prune_days(tcfg.days);
        }
        let dt = st.last_tick.map(|t| t.elapsed().as_secs_f64()).filter(|d| *d > 0.2).unwrap_or(1.0);
        st.last_tick = Some(Instant::now());
        let Some(acc) = st.acc.take() else {
            st.rates.clear();
            return;
        };
        let boot = boot_ns();
        let to_ms = |ts: u64| now.saturating_sub(boot.saturating_sub(ts) / 1_000_000);

        // 1. Закрытые с прошлого раза.
        let closed: Vec<u64> = acc.closed.keys(1 << 17).into_iter().map(|k| { acc.closed.delete(&k); u64::from_ne_bytes(k[..8].try_into().unwrap()) }).collect();

        // 2. Новые адреса назначения.
        let names = self.names.lock().unwrap();
        let host_of = |ip: &IpAddr| if tcfg.hostnames { names.get(ip) } else { None };
        for (k, v) in acc.conns.entries(1 << 17) {
            let cookie = u64::from_ne_bytes(k[..8].try_into().unwrap());
            let fam = u32::from_ne_bytes(k[28..32].try_into().unwrap());
            let proto = (fam & 0xff) as u8;
            let ip = if fam >> 16 == 4 { IpAddr::from([k[8], k[9], k[10], k[11]]) } else {
                let v6 = std::net::Ipv6Addr::from(<[u8; 16]>::try_from(&k[8..24]).unwrap());
                v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6))
            };
            let port = u16::from_be_bytes([k[24], k[25]]);
            let tgid = i32::from_ne_bytes(v[0..4].try_into().unwrap());
            let uid = u32::from_ne_bytes(v[4..8].try_into().unwrap());
            let ts = u64::from_ne_bytes(v[8..16].try_into().unwrap());
            let comm = String::from_utf8_lossy(&v[16..32]).trim_end_matches('\0').to_string();
            if !st.socks.contains_key(&cookie) {
                let o = Self::owner(st, tgid, uid, &comm);
                st.socks.insert(cookie, Sock { owner: Some(o), tries: 0, proto, remote: None, also: Vec::new(), more: 0, seen: HashSet::new(), keys: Vec::new(), host: None, start: to_ms(ts), rx: 0, tx: 0, pend_rx: 0, pend_tx: 0, iface: String::new(), incoming: false, changed: Instant::now() });
            }
            let s = st.socks.get_mut(&cookie).unwrap();
            if s.owner.is_none() {
                s.owner = Some(Owner { app: app_of(tgid, &comm), comm: comm.clone(), pid: tgid, uid });
            }
            // Петля и «пробы» glibc (UDP connect на порт 0 — выбор адреса источника при getaddrinfo).
            if is_local(&ip) || port == 0 {
                s.keys.push(k);
                continue;
            }
            if s.seen.insert((ip, port)) {
                if s.remote.is_none() {
                    s.remote = Some((ip, port));
                    s.host = host_of(&ip);
                } else if s.also.len() < ALSO_MAX {
                    let h = host_of(&ip).map(|h| format!(" ({h})")).unwrap_or_default();
                    s.also.push(if ip.is_ipv6() { format!("[{ip}]:{port}{h}") } else { format!("{ip}:{port}{h}") });
                } else {
                    s.more += 1;
                }
            }
            // Много адресов (DHT): ключи сверх предела сразу убрать, чтобы таблица не росла.
            if s.keys.len() < ALSO_MAX + 2 {
                s.keys.push(k);
            } else {
                acc.conns.delete(&k);
                if s.seen.len() > 8192 {
                    s.seen.clear();
                }
            }
        }
        drop(names);

        // 3. Байты.
        let mut if_attr: HashMap<String, (u64, u64)> = HashMap::new();
        let mut rates: HashMap<(String, u32), (f64, f64)> = HashMap::new();
        let mut need_diag = false;
        for (k, v) in acc.bytes.entries(1 << 17) {
            let cookie = u64::from_ne_bytes(k[..8].try_into().unwrap());
            let rx = u64::from_ne_bytes(v[0..8].try_into().unwrap());
            let tx = u64::from_ne_bytes(v[8..16].try_into().unwrap());
            let ifidx = u32::from_ne_bytes(v[16..20].try_into().unwrap());
            let iface = ifname(ifidx, &mut st.ifnames);
            if let Some((o, old_if, _)) = st.recent.get(&cookie) {
                // Хвост закрытого сокета: прежнему владельцу, запись — прочь.
                let ifc = if iface.is_empty() { old_if.clone() } else { iface.clone() };
                let (app, uid) = (o.app.clone(), o.uid);
                st.day.credit(&app, uid, &ifc, rx, tx, hour);
                let a = if_attr.entry(ifc).or_default();
                a.0 += rx;
                a.1 += tx;
                acc.bytes.delete(&k);
                continue;
            }
            let s = st.socks.entry(cookie).or_insert_with(|| Sock { owner: None, tries: 0, proto: 0, remote: None, also: Vec::new(), more: 0, seen: HashSet::new(), keys: Vec::new(), host: None, start: now, rx: 0, tx: 0, pend_rx: 0, pend_tx: 0, iface: String::new(), incoming: false, changed: Instant::now() });
            let (drx, dtx) = (rx.saturating_sub(s.rx), tx.saturating_sub(s.tx));
            s.rx = rx;
            s.tx = tx;
            if !iface.is_empty() {
                s.iface = iface.clone();
            }
            if drx == 0 && dtx == 0 {
                continue;
            }
            s.changed = Instant::now();
            let a = if_attr.entry(iface.clone()).or_default();
            a.0 += drx;
            a.1 += dtx;
            match &s.owner {
                Some(o) => {
                    let (app, uid) = (o.app.clone(), o.uid);
                    st.day.credit(&app, uid, &iface, drx, dtx, hour);
                    let r = rates.entry((app, uid)).or_default();
                    r.0 += drx as f64 / dt;
                    r.1 += dtx as f64 / dt;
                }
                None => {
                    s.pend_rx += drx;
                    s.pend_tx += dtx;
                    need_diag = true;
                }
            }
        }

        // 4. Владельцы сокетов без connect: sock_diag (не чаще раза в 3 с).
        if need_diag && st.last_diag.is_none_or(|t| t.elapsed().as_secs() >= 3) {
            st.last_diag = Some(Instant::now());
            self.resolve_orphans(st, &mut rates, dt, hour);
        }

        // 5. Доля «системы»: канал насчитал больше, чем досталось программам.
        for ifc in uplinks {
            let base = PathBuf::from("/sys/class/net").join(ifc).join("statistics");
            let rd = |f: &str| std::fs::read_to_string(base.join(f)).ok().and_then(|s| s.trim().parse::<u64>().ok());
            let (Some(r), Some(t)) = (rd("rx_bytes"), rd("tx_bytes")) else { continue };
            if let Some(&(r0, t0)) = st.if_last.get(ifc) {
                let a = if_attr.get(ifc).copied().unwrap_or_default();
                // Счётчики канала — по кадрам с заголовками канального уровня, учёт — по IP: ±14 байт на пакет; мелочь не в счёт.
                let (srx, stx) = (r.saturating_sub(r0).saturating_sub(a.0), t.saturating_sub(t0).saturating_sub(a.1));
                let (srx, stx) = (if srx > 2048 { srx } else { 0 }, if stx > 2048 { stx } else { 0 });
                st.day.credit(APP_SYSTEM, 0, ifc, srx, stx, hour);
                if srx + stx > 0 {
                    let e = rates.entry((APP_SYSTEM.to_string(), 0)).or_default();
                    e.0 += srx as f64 / dt;
                    e.1 += stx as f64 / dt;
                }
            }
            st.if_last.insert(ifc.clone(), (r, t));
        }
        st.rates = rates;

        // 6. Закрытые — в журнал.
        let limit = tcfg.log_mb.max(1) as u64 * 1024 * 1024;
        for cookie in closed {
            let Some(mut s) = st.socks.remove(&cookie) else { continue };
            for k in &s.keys {
                acc.conns.delete(k);
            }
            let key = cookie.to_ne_bytes();
            if let Some(v) = acc.bytes.lookup(&key) {
                // Последние байты после чтения в шаге 3.
                let rx = u64::from_ne_bytes(v[0..8].try_into().unwrap());
                let tx = u64::from_ne_bytes(v[8..16].try_into().unwrap());
                if let Some(o) = &s.owner {
                    st.day.credit(&o.app, o.uid, &s.iface, rx.saturating_sub(s.rx), tx.saturating_sub(s.tx), hour);
                }
                s.rx = rx.max(s.rx);
                s.tx = tx.max(s.tx);
                acc.bytes.delete(&key);
            }
            if let Some(c) = Self::record(&s, Some(now)) {
                if let Some(o) = &s.owner {
                    st.day.rows.entry((o.app.clone(), o.uid, s.iface.clone())).or_default().conns += 1;
                }
                st.log.write(&c, limit);
            }
            if let Some(o) = s.owner {
                st.recent.insert(cookie, (o, s.iface, Instant::now()));
            }
        }
        st.recent.retain(|_, (_, _, t)| t.elapsed().as_secs() < 300);

        // 7. Сокеты, закрывшиеся без события (открытые до запуска учёта и т. п.):
        // давно без байтов и нет в sock_diag — закрыть.
        if st.last_sweep.elapsed().as_secs() >= 60 {
            st.last_sweep = Instant::now();
            let stale: Vec<u64> = st.socks.iter().filter(|(_, s)| s.changed.elapsed().as_secs() >= 120).map(|(c, _)| *c).collect();
            if !stale.is_empty() {
                let alive: HashSet<u64> = crate::diag::dump().into_iter().map(|d| d.cookie).collect();
                for cookie in stale.into_iter().filter(|c| !alive.contains(c)) {
                    let Some(s) = st.socks.remove(&cookie) else { continue };
                    for k in &s.keys {
                        acc.conns.delete(k);
                    }
                    acc.bytes.delete(&cookie.to_ne_bytes());
                    if let Some(c) = Self::record(&s, Some(now)) {
                        st.log.write(&c, limit);
                    }
                }
            }
        }
        st.log.flush();
        st.acc = Some(acc);
        if st.day.dirty && st.last_save.elapsed().as_secs() >= 60 {
            self.save_day(st);
        }
    }

    fn resolve_orphans(&self, st: &mut State, rates: &mut HashMap<(String, u32), (f64, f64)>, dt: f64, hour: usize) {
        let socks = crate::diag::dump();
        let listen = crate::diag::listening(&socks);
        let by_cookie: HashMap<u64, &crate::diag::Sock> = socks.iter().map(|s| (s.cookie, s)).collect();
        let mut want: HashSet<u64> = HashSet::new();
        for (cookie, s) in &st.socks {
            if s.owner.is_none() {
                if let Some(d) = by_cookie.get(cookie) {
                    if d.inode != 0 {
                        want.insert(d.inode);
                    }
                }
            }
        }
        let owners = crate::diag::owners(&want);
        let orphans: Vec<u64> = st.socks.iter().filter(|(_, s)| s.owner.is_none()).map(|(c, _)| *c).collect();
        for cookie in orphans {
            let d = by_cookie.get(&cookie);
            let pid = d.and_then(|d| owners.get(&d.inode)).copied();
            let found = match (d, pid) {
                (Some(d), Some(pid)) => Some((Self::owner(st, pid, d.uid, ""), Some((*d).clone()))),
                // Сокет ядра (нет inode) или процесса уже нет после нескольких попыток.
                (Some(d), None) if d.inode == 0 => Some((Owner { app: APP_SYSTEM.into(), comm: String::new(), pid: 0, uid: 0 }, Some((*d).clone()))),
                _ => None,
            };
            let s = st.socks.get_mut(&cookie).unwrap();
            let found = found.or_else(|| {
                s.tries += 1;
                (s.tries >= 3).then(|| (Owner { app: APP_SYSTEM.into(), comm: String::new(), pid: 0, uid: 0 }, None))
            });
            let Some((o, d)) = found else { continue };
            if let Some(d) = d {
                s.proto = d.proto;
                if !is_local(&d.remote) && d.rport != 0 {
                    s.remote = Some((d.remote, d.rport));
                    if self.cfg.read().unwrap().traffic.hostnames {
                        s.host = self.names.lock().unwrap().get(&d.remote);
                    }
                }
                // Входящее — TCP к слушающему порту (у UDP без connect не отличить).
                s.incoming = d.proto == libc::IPPROTO_TCP as u8 && listen.contains(&(d.proto, d.lport));
            }
            let (prx, ptx) = (std::mem::take(&mut s.pend_rx), std::mem::take(&mut s.pend_tx));
            st.day.credit(&o.app, o.uid, &s.iface, prx, ptx, hour);
            let r = rates.entry((o.app.clone(), o.uid)).or_default();
            r.0 += prx as f64 / dt.max(3.0);
            r.1 += ptx as f64 / dt.max(3.0);
            s.owner = Some(o);
        }
    }

    fn record(s: &Sock, end: Option<u64>) -> Option<Conn> {
        // Без адреса (только петля, сокеты ядра, UDP без connect, открытый
        // до запуска учёта) — не в журнал; байты уже в расходе программы.
        s.remote?;
        let o = s.owner.clone().unwrap_or(Owner { app: APP_SYSTEM.into(), comm: String::new(), pid: 0, uid: 0 });
        Some(Conn {
            start: s.start,
            end,
            app: o.app,
            comm: o.comm,
            pid: o.pid,
            uid: o.uid,
            proto: proto_name(s.proto),
            remote: s.remote.map(|r| r.0.to_string()).unwrap_or_default(),
            port: s.remote.map(|r| r.1).unwrap_or(0),
            host: s.host.clone(),
            iface: s.iface.clone(),
            rx: s.rx,
            tx: s.tx,
            incoming: s.incoming,
            more: s.more,
            also: s.also.clone(),
        })
    }

    /// Записать открытые соединения (остановка службы, выключение учёта).
    fn finish_all(&self, st: &mut State) {
        let limit = self.cfg.read().unwrap().traffic.log_mb.max(1) as u64 * 1024 * 1024;
        let now = now_ms();
        let socks: Vec<Sock> = st.socks.drain().map(|(_, s)| s).collect();
        for s in socks {
            if let Some(c) = Self::record(&s, Some(now)) {
                st.log.write(&c, limit);
            }
        }
        st.log.flush();
        self.save_day(st);
    }

    pub fn shutdown(&self) {
        let mut st = self.st.lock().unwrap();
        self.finish_all(&mut st);
    }

    fn save_day(&self, st: &mut State) {
        let dir = self.dir.join("usage");
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join(format!("{}.json.tmp", st.day.date));
        if let Ok(s) = serde_json::to_string(&st.day.to_usage()) {
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join(format!("{}.json", st.day.date)));
            }
        }
        st.day.dirty = false;
        st.last_save = Instant::now();
    }

    fn prune_days(&self, days: u32) {
        let cutoff = local(now_ms().saturating_sub(days.max(1) as u64 * 86_400_000)).0;
        for e in std::fs::read_dir(self.dir.join("usage")).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(d) = name.strip_suffix(".json") {
                if d < cutoff.as_str() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }

    // ─── Запросы ────────────────────────────────────────────────────────────

    pub fn snapshot(&self, uid: u32, all: bool) -> syn_netd::Traffic {
        let cfg = self.cfg.read().unwrap().traffic.clone();
        let st = self.st.lock().unwrap();
        let mut apps: HashMap<(String, u32), AppTraffic> = HashMap::new();
        let visible = |u: u32| all || u == uid;
        for ((app, u, iface), r) in &st.day.rows {
            if !visible(*u) {
                continue;
            }
            let a = apps.entry((app.clone(), *u)).or_insert_with(|| AppTraffic { app: app.clone(), uid: *u, ..Default::default() });
            a.rx_today += r.rx;
            a.tx_today += r.tx;
            a.conns_today += r.conns;
            if !iface.is_empty() {
                a.ifaces.push((iface.clone(), r.rx, r.tx));
            }
        }
        for ((app, u), (rx, tx)) in &st.rates {
            if !visible(*u) {
                continue;
            }
            let a = apps.entry((app.clone(), *u)).or_insert_with(|| AppTraffic { app: app.clone(), uid: *u, ..Default::default() });
            a.rx_bps = *rx;
            a.tx_bps = *tx;
        }
        for s in st.socks.values() {
            let Some(o) = &s.owner else { continue };
            if !visible(o.uid) || (s.remote.is_none() && s.rx + s.tx == 0) {
                continue;
            }
            let a = apps.entry((o.app.clone(), o.uid)).or_insert_with(|| AppTraffic { app: o.app.clone(), uid: o.uid, ..Default::default() });
            a.active += 1;
            if a.comm.is_empty() {
                a.comm = o.comm.clone();
            }
        }
        let mut apps: Vec<AppTraffic> = apps.into_values().collect();
        for a in &mut apps {
            a.ifaces.sort_by(|x, y| (y.1 + y.2).cmp(&(x.1 + x.2)));
        }
        apps.sort_by(|a, b| (b.rx_bps + b.tx_bps).total_cmp(&(a.rx_bps + a.tx_bps)).then((b.rx_today + b.tx_today).cmp(&(a.rx_today + a.tx_today))));
        let segs = st.log.segments();
        syn_netd::Traffic {
            enabled: cfg.enabled && st.acc.is_some(),
            error: st.error.clone(),
            apps,
            log_bytes: segs.iter().map(|s| s.2).sum(),
            log_limit: cfg.log_mb as u64 * 1024 * 1024,
            log_since: segs.first().map(|s| s.0),
            dns: self.names.lock().unwrap().live,
            all_users: all,
        }
    }

    pub fn connections(&self, q: &ConnQuery, uid: u32, all: bool) -> Vec<Conn> {
        let limit = q.limit.clamp(1, 20000) as usize;
        let mut out = Vec::new();
        let segs = {
            let mut st = self.st.lock().unwrap();
            st.log.flush();
            // Открытые сейчас.
            for s in st.socks.values() {
                if let Some(mut c) = Self::record(s, None) {
                    c.end = None;
                    if (all || c.uid == uid) && c.matches(q) {
                        out.push(c);
                    }
                }
            }
            st.log.segments()
        };
        out.sort_by(|a, b| b.start.cmp(&a.start));
        out.truncate(limit);
        if q.active_only {
            return out;
        }
        let active = out.len();
        // Журнал, с новых файлов.
        for (seg_start, path, _) in segs.into_iter().rev() {
            if out.len() >= limit {
                break;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let mut newest_end = 0;
            for line in text.lines().rev() {
                let Ok(c) = serde_json::from_str::<Conn>(line) else { continue };
                newest_end = newest_end.max(c.end.unwrap_or(c.start));
                if (all || c.uid == uid) && c.matches(q) {
                    out.push(c);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
            // Файлы старше начала промежутка дальше не смотреть.
            if q.from.is_some_and(|f| newest_end < f || seg_start < f && newest_end < f) {
                break;
            }
        }
        // Открытые и закрытые вперемешку — по времени начала.
        if out.len() > active {
            out.sort_by(|a, b| b.start.cmp(&a.start));
        }
        out
    }

    pub fn usage(&self, from: &str, to: &str, uid: u32, all: bool) -> Vec<DayUsage> {
        {
            let mut st = self.st.lock().unwrap();
            if st.day.dirty {
                self.save_day(&mut st);
            }
        }
        let mut days: Vec<DayUsage> = std::fs::read_dir(self.dir.join("usage"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let d = name.strip_suffix(".json")?.to_string();
                (d.as_str() >= from && d.as_str() <= to).then_some(e.path())
            })
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .filter_map(|s| serde_json::from_str::<DayUsage>(&s).ok())
            .collect();
        for d in &mut days {
            d.rows.retain(|r| all || r.uid == uid);
        }
        days.sort_by(|a, b| a.day.cmp(&b.day));
        days
    }

    pub fn clear(&self) {
        let mut st = self.st.lock().unwrap();
        st.log.cur = None;
        let _ = std::fs::remove_dir_all(self.dir.join("log"));
        let _ = std::fs::remove_dir_all(self.dir.join("usage"));
        let date = st.day.date.clone();
        st.day = Day { date, ..Default::default() };
        tracing::info!("журнал соединений и архив трафика очищены");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring() {
        let dir = std::env::temp_dir().join(format!("synnetd-ring-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = Log { dir: dir.clone(), cur: None };
        let limit = 4 * 1024 * 1024;
        let c = Conn { start: 1, app: "x".repeat(200), proto: "tcp".into(), remote: "1.2.3.4".into(), port: 443, ..Default::default() };
        for _ in 0..40000 {
            log.write(&c, limit);
        }
        log.flush();
        let total: u64 = log.segments().iter().map(|s| s.2).sum();
        assert!(total <= limit + 300, "{total}");
        assert!(total > limit / 2, "{total}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn app_name() {
        assert_eq!(app_of(-5, "curl"), "curl");
        assert_eq!(app_of(std::process::id() as i32, ""), std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy());
    }
}
