//! synfsd — демон журнала изменений файлов (см. `synfsd` lib).
//!
//! Поток fanotify читает события и копит их в памяти; раз в
//! [`FLUSH_EVERY`] сводка пишется в SQLite. Запросы — по сокету
//! [`synfsd::SOCKET`], ответы фильтруются по пользователю (`SO_PEERCRED`).
//! Настройки — `/etc/synshell/synfsd.toml`: исключённые программы и пути,
//! срок хранения; список программ меняется и через сокет (root, `wheel`).

mod fan;
mod procmon;
mod store;

use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use synfsd::{Record, Request, Response, Status};

const DB: &str = "/var/lib/synfsd/activity.db";
const CONFIG: &str = "/etc/synshell/synfsd.toml";
const FLUSH_EVERY: Duration = Duration::from_secs(5);
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
struct Config {
    /// Программы, чьи изменения не записываются: имя процесса, имя файла
    /// или полный путь (`journalctl`, `/usr/lib/systemd/systemd-journald`).
    exclude_programs: Vec<String>,
    /// Пути, изменения в которых не записываются; `*` — любая одна часть
    /// пути (`/home/*/.cache`).
    exclude_paths: Vec<String>,
    /// Сколько дней хранить записи.
    retention_days: u32,
    /// Не больше стольких записей (старые уходят первыми).
    max_records: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            exclude_programs: Vec::new(),
            exclude_paths: vec!["/var/lib/synfsd".into(), "/var/log/journal".into()],
            retention_days: 30,
            max_records: 2_000_000,
        }
    }
}

impl Config {
    fn load() -> Config {
        match std::fs::read_to_string(CONFIG) {
            Ok(t) => toml::from_str(&t).unwrap_or_else(|e| {
                tracing::warn!("{CONFIG}: {e}");
                Config::default()
            }),
            Err(_) => Config::default(),
        }
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(d) = Path::new(CONFIG).parent() {
            std::fs::create_dir_all(d)?;
        }
        let text = format!(
            "# synfsd — журнал изменений файлов. Меняется и из Проводника (Настройки → Журнал изменений).\n{}",
            toml::to_string_pretty(self)?
        );
        std::fs::write(CONFIG, text)?;
        Ok(())
    }

    fn excluded(&self, path: &str, exe: &str, comm: &str) -> bool {
        self.exclude_programs.iter().any(|p| synfsd::program_matches(p, exe, comm))
            || self.exclude_paths.iter().any(|p| path_matches(p, path))
    }
}

/// Путь внутри шаблона: части сравниваются по порядку, `*` — любая.
fn path_matches(pattern: &str, path: &str) -> bool {
    let mut pi = pattern.trim_end_matches('/').split('/');
    let mut si = path.split('/');
    loop {
        match (pi.next(), si.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(p), Some(s)) if p == "*" || p == s => {}
            _ => return false,
        }
    }
}

#[derive(Default)]
struct Stats {
    since: i64,
    events: u64,
    overflows: Arc<std::sync::atomic::AtomicU64>,
    filesystems: Vec<String>,
}

struct Shared {
    pending: Mutex<store::Pending>,
    store: Mutex<store::Store>,
    config: RwLock<Config>,
    stats: Mutex<Stats>,
}

/// В памяти больше стольких записей — сбросить, не дожидаясь срока.
const PENDING_MAX: usize = 20_000;

impl Shared {
    fn flush(&self) {
        let batch = self.pending.lock().unwrap().take();
        let big = batch.len() > 1000;
        if let Err(e) = self.store.lock().unwrap().write(batch) {
            tracing::warn!("запись журнала: {e:#}");
        }
        if big {
            // После всплеска (сборка, распаковка) — вернуть память системе.
            unsafe { libc::malloc_trim(0) };
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if unsafe { libc::geteuid() } != 0 {
        anyhow::bail!("synfsd нужны права root (fanotify на всю файловую систему)");
    }
    let config = Config::load();
    let mut fan = fan::Fan::new()?;
    let shared = Arc::new(Shared {
        pending: Mutex::new(store::Pending::default()),
        store: Mutex::new(store::Store::open(Path::new(DB))?),
        config: RwLock::new(config),
        stats: Mutex::new(Stats { since: now_ms(), filesystems: fan.watched.clone(), overflows: fan.overflows.clone(), events: 0 }),
    });

    // Сброс в базу и чистка старого.
    {
        let sh = shared.clone();
        std::thread::Builder::new().name("flush".into()).spawn(move || {
            let mut last_prune: Option<Instant> = None;
            loop {
                std::thread::sleep(FLUSH_EVERY);
                sh.flush();
                if last_prune.is_none_or(|t| t.elapsed() > PRUNE_EVERY) {
                    let (days, max) = {
                        let c = sh.config.read().unwrap();
                        (c.retention_days.max(1) as i64, c.max_records.max(1000))
                    };
                    match sh.store.lock().unwrap().prune(now_ms() - days * 86_400_000, max) {
                        Ok(n) if n > 0 => tracing::info!("удалено старых записей: {n}"),
                        Err(e) => tracing::warn!("чистка журнала: {e:#}"),
                        _ => {}
                    }
                    last_prune = Some(Instant::now());
                }
            }
        })?;
    }

    // Сокет.
    {
        let _ = std::fs::remove_file(synfsd::SOCKET);
        let listener = UnixListener::bind(synfsd::SOCKET)?;
        std::fs::set_permissions(synfsd::SOCKET, std::fs::Permissions::from_mode(0o666))?;
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

    tracing::info!("synfsd запущен");
    let sh = shared.clone();
    fan.run(|ev| {
        if sh.config.read().unwrap().excluded(&ev.path, &ev.proc_.exe, &ev.proc_.comm) {
            return;
        }
        let t = now_ms();
        sh.pending.lock().unwrap().add(Record {
            path: ev.path,
            old_path: ev.old_path,
            kind: ev.kind,
            is_dir: ev.is_dir,
            exe: ev.proc_.exe,
            comm: ev.proc_.comm,
            pid: ev.pid,
            uid: ev.proc_.uid,
            first: t,
            last: t,
            count: 1,
        });
        sh.stats.lock().unwrap().events += 1;
        if sh.pending.lock().unwrap().len() > PENDING_MAX {
            sh.flush();
        }
    })?;
    Ok(())
}

/// Кто на том конце сокета.
fn peer_uid(s: &UnixStream) -> Option<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    (r == 0).then_some(cred.uid)
}

/// Имя и домашняя папка пользователя.
fn user(uid: u32) -> Option<(String, PathBuf, u32)> {
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_string_lossy().into_owned();
    let home = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) }.to_string_lossy().into_owned();
    Some((name, PathBuf::from(home), unsafe { (*pw).pw_gid }))
}

/// Может ли пользователь менять исключения: root или группа `wheel`.
fn can_edit(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let Some((name, _, gid)) = user(uid) else { return false };
    let wheel = unsafe { libc::getgrnam(c"wheel".as_ptr()) };
    if wheel.is_null() {
        return false;
    }
    let wheel_gid = unsafe { (*wheel).gr_gid };
    let Ok(cname) = std::ffi::CString::new(name) else { return false };
    let mut groups = vec![0 as libc::gid_t; 256];
    let mut n = groups.len() as libc::c_int;
    if unsafe { libc::getgrouplist(cname.as_ptr(), gid, groups.as_mut_ptr(), &mut n) } < 0 {
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
        Ok(req) => handle(sh, uid, req).unwrap_or_else(|e| Response { ok: false, error: Some(format!("{e:#}")), ..Default::default() }),
        Err(e) => Response { ok: false, error: Some(format!("запрос: {e}")), ..Default::default() },
    };
    let mut out = serde_json::to_string(&resp)?;
    out.push('\n');
    (&conn).write_all(out.as_bytes())?;
    Ok(())
}

fn handle(sh: &Shared, uid: u32, req: Request) -> anyhow::Result<Response> {
    let programs = |sh: &Shared| sh.config.read().unwrap().exclude_programs.clone();
    Ok(match req {
        Request::History { path, children, limit } => {
            // Свежее (ещё в памяти) — тоже в ответ.
            sh.flush();
            let only = if uid == 0 {
                None
            } else {
                let home = user(uid).map(|(_, h, _)| h.to_string_lossy().into_owned()).unwrap_or_else(|| "/nonexistent".into());
                Some((uid, home))
            };
            let items = sh.store.lock().unwrap().history(&path, children, limit, only)?;
            Response { ok: true, items, ..Default::default() }
        }
        Request::Status => {
            let st = sh.stats.lock().unwrap();
            let status = Status {
                since: st.since,
                events: st.events,
                overflows: st.overflows.load(std::sync::atomic::Ordering::Relaxed),
                filesystems: st.filesystems.clone(),
                records: sh.store.lock().unwrap().count() + sh.pending.lock().unwrap().len() as u64,
            };
            Response { ok: true, status: Some(status), programs: programs(sh), can_edit: can_edit(uid), ..Default::default() }
        }
        Request::Exclusions => Response { ok: true, programs: programs(sh), can_edit: can_edit(uid), ..Default::default() },
        Request::ExcludeProgram { program } | Request::IncludeProgram { program } if program.trim().is_empty() => {
            anyhow::bail!("пустое имя программы")
        }
        Request::ExcludeProgram { program } => {
            edit(sh, uid, |c| {
                let p = program.trim().to_string();
                if !c.exclude_programs.contains(&p) {
                    c.exclude_programs.push(p);
                }
            })?;
            Response { ok: true, programs: programs(sh), can_edit: true, ..Default::default() }
        }
        Request::IncludeProgram { program } => {
            edit(sh, uid, |c| c.exclude_programs.retain(|p| p != program.trim()))?;
            Response { ok: true, programs: programs(sh), can_edit: true, ..Default::default() }
        }
    })
}

fn edit(sh: &Shared, uid: u32, f: impl FnOnce(&mut Config)) -> anyhow::Result<()> {
    if !can_edit(uid) {
        anyhow::bail!("менять исключения могут только администраторы (группа wheel)");
    }
    let mut c = sh.config.write().unwrap();
    f(&mut c);
    c.save()?;
    tracing::info!("исключённые программы: {:?}", c.exclude_programs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions() {
        assert!(path_matches("/home/*/.cache", "/home/u/.cache/x"));
        assert!(!path_matches("/home/*/.cache", "/home/u/.cachex"));
        assert!(path_matches("/var/lib/synfsd", "/var/lib/synfsd"));
        let c = Config { exclude_programs: vec!["journalctl".into(), "/usr/bin/vim".into()], ..Default::default() };
        assert!(c.excluded("/x", "/usr/bin/journalctl", "journalctl"));
        assert!(c.excluded("/x", "/usr/bin/vim", "vim"));
        assert!(!c.excluded("/x", "/opt/vim", "vim"));
        // `comm` обрезан до 15 байт.
        assert!(synfsd::program_matches("systemd-journald", "", "systemd-journal"));
    }
}
