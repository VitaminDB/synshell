//! synnetd — служба нескольких подключений к интернету (см. [`syn_netd`]).
//!
//! Потоки: цикл маршрутизации (по событиям `ip monitor` и раз в
//! [`TICK`]), запуски процессов (proc connector), пересмотр процессов,
//! сокет [`syn_netd::SOCKET`], сигналы (остановка — убрать свои правила и
//! вернуть процессы на место).
//!
//! Переменные для проверки: `SYNNETD_STATE_DIR` (каталог настроек, по
//! умолчанию /var/lib/synnetd), `SYNNETD_SOCKET`.

mod apps;
mod bpf;
mod dhcp;
mod route;

use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;

use syn_netd::{Config, Request, Response, Status, Uplink};

const TICK: Duration = Duration::from_secs(3);

fn state_dir() -> PathBuf {
    std::env::var_os("SYNNETD_STATE_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/var/lib/synnetd"))
}

fn socket_path() -> String {
    std::env::var("SYNNETD_SOCKET").unwrap_or_else(|_| syn_netd::SOCKET.to_string())
}

fn load_config() -> Config {
    std::fs::read_to_string(state_dir().join("config.toml")).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

fn save_config(c: &Config) -> anyhow::Result<()> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("config.toml.tmp");
    std::fs::write(&tmp, toml::to_string_pretty(c)?)?;
    std::fs::rename(tmp, dir.join("config.toml"))?;
    Ok(())
}

/// Будильник цикла маршрутизации.
#[derive(Default)]
struct Wake {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Wake {
    fn ring(&self) {
        *self.flag.lock().unwrap() = true;
        self.cv.notify_all();
    }

    fn wait(&self, t: Duration) {
        let g = self.flag.lock().unwrap();
        let (mut g, _) = self.cv.wait_timeout_while(g, t, |f| !*f).unwrap();
        *g = false;
    }
}

struct Shared {
    cfg: Arc<RwLock<Config>>,
    apps: apps::Apps,
    net: Mutex<(Vec<Uplink>, bool, Option<String>)>,
    wake: Wake,
    rescan: Wake,
}

impl Shared {
    fn status(&self, uid: u32) -> Status {
        let (uplinks, active, mut note) = self.net.lock().unwrap().clone();
        if note.is_none() {
            note = self.apps.error.lock().unwrap().clone();
        }
        Status { config: self.cfg.read().unwrap().clone(), uplinks, active, note, running: self.apps.running(), can_edit: can_edit(uid) }
    }
}

/// Управление работающей службой из командной строки.
fn ctl(args: &[String]) -> anyhow::Result<()> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if a.first() == Some(&"status") {
        let st = syn_netd::status()?;
        println!("{}", serde_json::to_string_pretty(&st)?);
        return Ok(());
    }
    let mut c = syn_netd::status()?.config;
    match a.as_slice() {
        ["mode", "primary"] => c.mode = syn_netd::Mode::Primary,
        ["mode", "both"] => c.mode = syn_netd::Mode::Both,
        ["primary", "auto"] => c.primary.clear(),
        ["primary", iface] => c.primary = iface.to_string(),
        ["app", name, "system"] => {
            c.apps.remove(*name);
        }
        ["app", name, p] => {
            let p = syn_netd::AppPolicy::from_id(p).ok_or_else(|| anyhow::anyhow!("выбор: primary, both или system"))?;
            c.apps.insert(name.to_string(), p);
        }
        ["usb-dhcp", "on"] => c.usb_dhcp = true,
        ["usb-dhcp", "off"] => c.usb_dhcp = false,
        _ => anyhow::bail!(
            "synnetd                         — служба (root)\n\
             synnetd status                  — состояние (JSON)\n\
             synnetd mode primary|both       — основной канал или все сразу\n\
             synnetd primary <iface>|auto    — основной канал\n\
             synnetd app <имя> primary|both|system — выбор для программы\n\
             synnetd usb-dhcp on|off         — поднимать USB-модемы самой службой"
        ),
    }
    let st = syn_netd::set_config(c)?;
    println!("{}", serde_json::to_string_pretty(&st)?);
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        return ctl(&args);
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if unsafe { libc::geteuid() } != 0 {
        anyhow::bail!("synnetd нужны права root (маршруты, cgroup, BPF)");
    }
    let cfg = Arc::new(RwLock::new(load_config()));
    let shared = Arc::new(Shared {
        apps: apps::Apps::new(cfg.clone()),
        cfg,
        net: Mutex::new((Vec::new(), false, None)),
        wake: Wake::default(),
        rescan: Wake::default(),
    });

    // Остаток прежнего запуска.
    route::teardown();

    // Сигналы: убрать за собой.
    {
        let sh = shared.clone();
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTERM);
            libc::sigaddset(&mut set, libc::SIGINT);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }
        std::thread::Builder::new().name("signals".into()).spawn(move || {
            let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
            unsafe {
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGTERM);
                libc::sigaddset(&mut set, libc::SIGINT);
            }
            let mut sig = 0;
            unsafe { libc::sigwait(&set, &mut sig) };
            tracing::info!("остановка: свои правила убраны, программы возвращены");
            route::teardown();
            sh.apps.release_all();
            let _ = std::fs::remove_file(socket_path());
            std::process::exit(0);
        })?;
    }

    // Изменения сети.
    {
        let sh = shared.clone();
        std::thread::Builder::new().name("monitor".into()).spawn(move || loop {
            match std::process::Command::new("ip").args(["monitor", "link", "address", "route"]).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn() {
                Ok(mut child) => {
                    if let Some(out) = child.stdout.take() {
                        for _ in BufReader::new(out).lines().map_while(Result::ok) {
                            sh.wake.ring();
                        }
                    }
                    let _ = child.wait();
                }
                Err(e) => tracing::warn!("ip monitor: {e}"),
            }
            std::thread::sleep(Duration::from_secs(5));
        })?;
    }

    // Запуски процессов.
    match procmon_subscribe() {
        Ok(fd) => {
            let sh = shared.clone();
            std::thread::Builder::new().name("procmon".into()).spawn(move || procmon_listen(fd, &sh))?;
        }
        Err(e) => tracing::warn!("proc connector: {e} — программы переносятся только пересмотром раз в {} с", apps::RESCAN.as_secs()),
    }

    // Пересмотр процессов.
    {
        let sh = shared.clone();
        std::thread::Builder::new().name("rescan".into()).spawn(move || loop {
            sh.apps.rescan();
            sh.rescan.wait(apps::RESCAN);
        })?;
    }

    // Сокет.
    {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666))?;
        let sh = shared.clone();
        std::thread::Builder::new().name("socket".into()).spawn(move || {
            for conn in listener.incoming().flatten() {
                let sh = sh.clone();
                let _ = std::thread::Builder::new().name("client".into()).spawn(move || {
                    if let Err(e) = serve(&sh, conn) {
                        tracing::debug!("клиент: {e}");
                    }
                });
            }
        })?;
    }

    tracing::info!("synnetd запущен");
    let mut dhcp = dhcp::Dhcp::default();
    let ours = dhcp.ours();
    let mut last: Option<route::Plan> = None;
    loop {
        let cfg = shared.cfg.read().unwrap().clone();
        dhcp.tick(cfg.usb_dhcp);
        let snap = route::read();
        let plan = route::plan(&snap, &cfg);
        if last.as_ref().map(|l| &l.cmds) != Some(&plan.cmds) {
            let errs = route::apply(&plan);
            for e in &errs {
                tracing::warn!("{e}");
            }
            if plan.active() {
                tracing::info!(
                    "каналы: {} · основной {} · режим {:?}",
                    snap.v4.iter().chain(snap.v6.iter()).map(|g| g.iface.as_str()).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", "),
                    plan.primary.as_deref().unwrap_or("-"),
                    cfg.mode
                );
            } else {
                tracing::info!("свои правила сняты ({})", plan.note.as_deref().unwrap_or(""));
            }
        }
        let o = ours.lock().unwrap().clone();
        let uplinks = route::uplinks(&snap, &plan, &|i| o.iter().any(|x| x == i));
        *shared.net.lock().unwrap() = (uplinks, plan.active(), plan.note.clone());
        last = Some(plan);
        shared.wake.wait(TICK);
        // Пачка событий (свои же изменения, новый адрес) — одним проходом.
        std::thread::sleep(Duration::from_millis(300));
    }
}

// ─── Запуски процессов (proc connector) ─────────────────────────────────────

const CN_IDX_PROC: u32 = 1;
const CN_VAL_PROC: u32 = 1;
const PROC_CN_MCAST_LISTEN: u32 = 1;
const PROC_EVENT_FORK: u32 = 1;
const PROC_EVENT_EXEC: u32 = 2;
const PROC_EVENT_EXIT: u32 = 0x8000_0000;

fn procmon_subscribe() -> std::io::Result<i32> {
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_CONNECTOR);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        sa.nl_groups = CN_IDX_PROC;
        if libc::bind(fd, (&sa as *const libc::sockaddr_nl).cast(), std::mem::size_of::<libc::sockaddr_nl>() as u32) != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        // nlmsghdr (16) + cn_msg (20) + op (4).
        let mut msg = [0u8; 40];
        msg[0..4].copy_from_slice(&40u32.to_ne_bytes());
        msg[4..6].copy_from_slice(&(libc::NLMSG_DONE as u16).to_ne_bytes());
        msg[12..16].copy_from_slice(&std::process::id().to_ne_bytes());
        msg[16..20].copy_from_slice(&CN_IDX_PROC.to_ne_bytes());
        msg[20..24].copy_from_slice(&CN_VAL_PROC.to_ne_bytes());
        msg[32..34].copy_from_slice(&4u16.to_ne_bytes());
        msg[36..40].copy_from_slice(&PROC_CN_MCAST_LISTEN.to_ne_bytes());
        if libc::send(fd, msg.as_ptr().cast(), msg.len(), 0) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(fd)
    }
}

fn procmon_listen(fd: i32, sh: &Shared) {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted || e.raw_os_error() == Some(libc::ENOBUFS) {
                continue;
            }
            tracing::warn!("proc connector: {e}");
            return;
        }
        let b = &buf[..n as usize];
        let mut off = 0;
        while off + 16 <= b.len() {
            let len = u32::from_ne_bytes(b[off..off + 4].try_into().unwrap()) as usize;
            if len < 16 || off + len > b.len() {
                break;
            }
            // nlmsghdr (16) + cn_msg (20) + proc_event: what, cpu, timestamp, pid, tgid.
            let ev = &b[off + 36..off + len];
            let at = |o: usize| ev.get(o..o + 4).map(|b| i32::from_ne_bytes(b.try_into().unwrap()));
            match at(0).map(|w| w as u32) {
                Some(PROC_EVENT_EXEC) => {
                    if let Some(tgid) = at(20) {
                        sh.apps.on_exec(tgid);
                    }
                }
                // fork: parent pid, parent tgid, child pid, child tgid.
                Some(PROC_EVENT_FORK) => {
                    if let (Some(ptgid), Some(cpid), Some(ctgid)) = (at(20), at(24), at(28)) {
                        if cpid == ctgid {
                            sh.apps.on_fork(ptgid, ctgid);
                        }
                    }
                }
                Some(PROC_EVENT_EXIT) => {
                    if let (Some(pid), Some(tgid)) = (at(16), at(20)) {
                        if pid == tgid {
                            sh.apps.on_exit(tgid);
                        }
                    }
                }
                _ => {}
            }
            off += (len + 3) & !3;
        }
    }
}

// ─── Сокет ──────────────────────────────────────────────────────────────────

fn peer_uid(s: &UnixStream) -> Option<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) };
    (r == 0).then_some(cred.uid)
}

/// Может ли пользователь менять настройки: root или группа `wheel`.
fn can_edit(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return false;
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_owned();
    let gid = unsafe { (*pw).pw_gid };
    let wheel = unsafe { libc::getgrnam(c"wheel".as_ptr()) };
    if wheel.is_null() {
        return false;
    }
    let wheel_gid = unsafe { (*wheel).gr_gid };
    let mut groups = vec![0 as libc::gid_t; 256];
    let mut n = groups.len() as libc::c_int;
    if unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut n) } < 0 {
        return false;
    }
    groups[..n.max(0) as usize].contains(&wheel_gid)
}

fn serve(sh: &Shared, conn: UnixStream) -> anyhow::Result<()> {
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    let uid = peer_uid(&conn).unwrap_or(u32::MAX);
    let mut line = String::new();
    BufReader::new(&conn).read_line(&mut line)?;
    let resp = match serde_json::from_str::<Request>(&line) {
        Ok(Request::Status) => Response { ok: true, error: None, status: Some(sh.status(uid)) },
        Ok(Request::SetConfig { config }) => {
            if !can_edit(uid) {
                Response { ok: false, error: Some("менять настройки сети могут root и группа wheel".into()), status: None }
            } else {
                match save_config(&config) {
                    Ok(()) => {
                        tracing::info!("настройки: режим {:?}, основной «{}», программ {}", config.mode, config.primary, config.apps.len());
                        *sh.cfg.write().unwrap() = config;
                        sh.wake.ring();
                        sh.rescan.ring();
                        // Дать циклу применить, чтобы ответ уже был свежим.
                        std::thread::sleep(Duration::from_millis(600));
                        Response { ok: true, error: None, status: Some(sh.status(uid)) }
                    }
                    Err(e) => Response { ok: false, error: Some(format!("{e:#}")), status: None },
                }
            }
        }
        Err(e) => Response { ok: false, error: Some(format!("запрос: {e}")), status: None },
    };
    let mut out = serde_json::to_string(&resp)?;
    out.push('\n');
    (&conn).write_all(out.as_bytes())?;
    Ok(())
}
