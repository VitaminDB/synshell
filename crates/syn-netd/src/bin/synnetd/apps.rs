//! Программы со своим выбором канала. Процесс программы (и всё, что он
//! запускает) переносится в под-cgroup своей же cgroup —
//! `…/session-4.scope/synnet-both`. Сеанс logind, по которому polkit и
//! порталы узнают процесс, остаётся тем же (он определяется по первому
//! юниту пути), а при выходе из сеанса процессы завершаются вместе с ним.
//! На под-cgroup висит BPF-программа, метящая сокеты ([`crate::bpf`]).
//!
//! Перенос — сразу при `exec` (netlink proc connector) и пересмотром всех
//! процессов раз в [`RESCAN`] и при смене настроек. Потомок программы идёт
//! по выбору ближайшего предка с правилом; правило убрали — процессы
//! возвращаются в свою прежнюю cgroup.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use syn_netd::{AppPolicy, AppRunning, Config};

use crate::route::{MARK_BLOCK, MARK_BOTH, MARK_PRIMARY};

pub const RESCAN: Duration = Duration::from_secs(10);
const CG_ROOT: &str = "/sys/fs/cgroup";

/// Сведения о процессе.
struct Proc {
    ppid: i32,
    /// Путь cgroup (`/user.slice/…/session-4.scope`), без корня.
    cgroup: String,
    /// Правило, подошедшее к самому процессу: (имя программы, выбор).
    rule: Option<(String, AppPolicy)>,
}

fn read_proc(pid: i32, cfg: &Config) -> Option<Proc> {
    let base = format!("/proc/{pid}");
    let exe = std::fs::read_link(format!("{base}/exe")).ok()?.to_string_lossy().into_owned();
    let exe = exe.strip_suffix(" (deleted)").map(str::to_string).unwrap_or(exe);
    let comm = std::fs::read_to_string(format!("{base}/comm")).map(|c| c.trim().to_string()).unwrap_or_default();
    let stat = std::fs::read_to_string(format!("{base}/stat")).ok()?;
    // pid (comm) state ppid … — comm может содержать пробелы и скобки.
    let ppid = stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()?;
    let cgroup = std::fs::read_to_string(format!("{base}/cgroup")).ok()?.lines().find_map(|l| l.strip_prefix("0::").map(|s| s.trim().to_string()))?;
    let rule = if cfg.apps.is_empty() {
        None
    } else {
        let argv: Vec<String> = std::fs::read(format!("{base}/cmdline"))
            .map(|b| b.split(|c| *c == 0).filter(|s| !s.is_empty()).take(8).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
            .unwrap_or_default();
        cfg.apps.iter().find(|(name, _)| syn_netd::program_matches(name, &exe, &comm, &argv)).map(|(n, p)| (n.clone(), p.clone()))
    };
    Some(Proc { ppid, cgroup, rule })
}

/// Путь cgroup без своего хвоста `/synnet-*` и что там было.
fn split_ours(cg: &str) -> (&str, Option<AppPolicy>) {
    if let Some((base, last)) = cg.rsplit_once('/') {
        if let Some(p) = AppPolicy::from_cgroup_name(last) {
            return (if base.is_empty() { "/" } else { base }, Some(p));
        }
    }
    (cg, None)
}

fn cg_dir(cg: &str) -> PathBuf {
    Path::new(CG_ROOT).join(cg.trim_start_matches('/'))
}

/// Дочерние процессы (по всем потокам).
fn children(pid: i32) -> Vec<i32> {
    let mut v = Vec::new();
    for t in std::fs::read_dir(format!("/proc/{pid}/task")).into_iter().flatten().flatten() {
        if let Ok(s) = std::fs::read_to_string(t.path().join("children")) {
            v.extend(s.split_whitespace().filter_map(|x| x.parse::<i32>().ok()));
        }
    }
    v
}

/// Поставить метку `mark` всем сокетам IPv4/IPv6 процесса: копия
/// дескриптора через `pidfd_getfd` указывает на тот же сокет, `SO_MARK` на
/// ней меняет метку и у программы. Новые соединения и отправки UDP идут уже
/// по метке; установленные TCP-соединения остаются на своём канале (их
/// адрес источника не меняется).
fn mark_existing(pid: i32, mark: u32) {
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
    if pidfd < 0 {
        return;
    }
    for e in std::fs::read_dir(format!("/proc/{pid}/fd")).into_iter().flatten().flatten() {
        let is_sock = std::fs::read_link(e.path()).is_ok_and(|l| l.to_string_lossy().starts_with("socket:"));
        let Some(fd) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
        if !is_sock {
            continue;
        }
        let dup = unsafe { libc::syscall(libc::SYS_pidfd_getfd, pidfd, fd, 0) } as i32;
        if dup < 0 {
            continue;
        }
        let mut dom: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let ok = unsafe { libc::getsockopt(dup, libc::SOL_SOCKET, libc::SO_DOMAIN, (&mut dom as *mut libc::c_int).cast(), &mut len) } == 0;
        if ok && (dom == libc::AF_INET || dom == libc::AF_INET6) {
            let m = mark;
            unsafe { libc::setsockopt(dup, libc::SOL_SOCKET, libc::SO_MARK, (&m as *const u32).cast(), 4) };
        }
        unsafe { libc::close(dup) };
    }
    unsafe { libc::close(pidfd) };
}

/// Найти свои под-cgroup (`synnet-*`) в дереве.
fn find_ours(dir: &Path, depth: u32, out: &mut HashSet<PathBuf>) {
    if depth > 10 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let p = e.path();
        if e.file_name().to_str().and_then(AppPolicy::from_cgroup_name).is_some() {
            out.insert(p);
        } else {
            find_ours(&p, depth + 1, out);
        }
    }
}

pub struct Apps {
    cfg: Arc<RwLock<Config>>,
    /// Ядро загружает BPF-программы (проверено при запуске).
    bpf_ok: bool,
    /// Программы меток по значению метки — загружаются, когда понадобились.
    progs: Mutex<HashMap<u32, Arc<OwnedFd>>>,
    /// Под-cgroup, где уже есть программа (чтобы не спрашивать ядро каждый раз).
    attached: Mutex<HashSet<PathBuf>>,
    running: Mutex<Vec<AppRunning>>,
    pub error: Mutex<Option<String>>,
    /// Метки по процессу (корневая cgroup) и кто в таблице сейчас.
    tgid: Option<crate::bpf::TgidMarks>,
    marked: Mutex<HashMap<i32, u32>>,
}

fn mark_of(p: Option<&AppPolicy>) -> u32 {
    match p {
        Some(AppPolicy::Both) => MARK_BOTH,
        Some(AppPolicy::Primary) => MARK_PRIMARY,
        Some(AppPolicy::Iface(i)) => syn_netd::iface_mark(i),
        Some(AppPolicy::Block) => MARK_BLOCK,
        None => 0,
    }
}

impl Apps {
    pub fn new(cfg: Arc<RwLock<Config>>) -> Apps {
        let mut progs = HashMap::new();
        let bpf_ok = match crate::bpf::load(MARK_BOTH) {
            Ok(p) => {
                progs.insert(MARK_BOTH, Arc::new(p));
                true
            }
            Err(e) => {
                tracing::warn!("BPF: {e} — выбор канала для программ недоступен");
                false
            }
        };
        let error = if bpf_ok { None } else { Some("bpf".to_string()) };
        // Свои cgroup прежних запусков.
        let mut known = HashSet::new();
        find_ours(Path::new(CG_ROOT), 0, &mut known);
        let tgid = match crate::bpf::TgidMarks::new() {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!("метки по процессу: {e} — короткие программы могут успеть соединиться до переноса");
                None
            }
        };
        Apps { cfg, bpf_ok, progs: Mutex::new(progs), attached: Mutex::new(known), running: Mutex::new(Vec::new()), error: Mutex::new(error), tgid, marked: Mutex::new(HashMap::new()) }
    }

    /// Занести процесс в таблицу меток (0 — убрать).
    fn set_mark(&self, pid: i32, mark: u32) {
        let Some(t) = &self.tgid else { return };
        let mut m = self.marked.lock().unwrap();
        if mark == 0 {
            if m.remove(&pid).is_some() {
                t.remove(pid);
            }
        } else if m.get(&pid) != Some(&mark) {
            t.set(pid, mark);
            m.insert(pid, mark);
        }
    }

    /// Процесс создал потомка: тот наследует метку.
    pub fn on_fork(&self, parent: i32, child: i32) {
        let mark = self.marked.lock().unwrap().get(&parent).copied();
        if let Some(m) = mark {
            self.set_mark(child, m);
        }
    }

    pub fn on_exit(&self, pid: i32) {
        self.set_mark(pid, 0);
    }

    pub fn running(&self) -> Vec<AppRunning> {
        self.running.lock().unwrap().clone()
    }

    /// Программа, ставящая метку `mark`.
    fn prog(&self, mark: u32) -> Option<Arc<OwnedFd>> {
        if !self.bpf_ok {
            return None;
        }
        let mut p = self.progs.lock().unwrap();
        if let Some(f) = p.get(&mark) {
            return Some(f.clone());
        }
        match if mark == MARK_BLOCK { crate::bpf::load_deny() } else { crate::bpf::load(mark) } {
            Ok(f) => {
                let f = Arc::new(f);
                p.insert(mark, f.clone());
                Some(f)
            }
            Err(e) => {
                tracing::warn!("BPF: {e}");
                None
            }
        }
    }

    /// Перенести процесс туда, где он должен быть.
    fn place(&self, pid: i32, cur: &str, want: Option<AppPolicy>) -> bool {
        self.set_mark(pid, mark_of(want.as_ref()));
        let (base, now) = split_ours(cur);
        if now == want {
            return true;
        }
        let target = match &want {
            Some(p) => {
                let Some(prog) = self.prog(mark_of(Some(p))) else { return false };
                let path = if base == "/" { format!("/{}", p.cgroup_name()) } else { format!("{base}/{}", p.cgroup_name()) };
                let dir = cg_dir(&path);
                if !dir.exists() {
                    if let Err(e) = std::fs::create_dir(&dir) {
                        tracing::debug!("{}: {e}", dir.display());
                        return false;
                    }
                }
                if !self.attached.lock().unwrap().contains(&dir) {
                    if let Err(e) = crate::bpf::attach(&dir, &prog) {
                        tracing::warn!("BPF на {}: {e}", dir.display());
                        let _ = std::fs::remove_dir(&dir);
                        return false;
                    }
                    self.attached.lock().unwrap().insert(dir.clone());
                }
                dir
            }
            None => cg_dir(base),
        };
        match std::fs::write(target.join("cgroup.procs"), pid.to_string()) {
            Ok(()) => {
                tracing::debug!("pid {pid}: {cur} → {}", target.display());
                // Сокеты, открытые до переноса (перенос идёт через 1–5 мс
                // после exec, а программа успевает спросить DNS или открыть
                // соединение), — пометить так же.
                mark_existing(pid, mark_of(want.as_ref()));
                // Ушли из своей под-cgroup — убрать её, если опустела.
                if now.is_some() {
                    let _ = std::fs::remove_dir(cg_dir(cur));
                }
                true
            }
            Err(e) => {
                tracing::debug!("pid {pid} → {}: {e}", target.display());
                false
            }
        }
    }

    /// Процесс только что запустил программу.
    pub fn on_exec(&self, pid: i32) {
        let cfg = self.cfg.read().unwrap();
        if cfg.apps.is_empty() {
            return;
        }
        let Some(p) = read_proc(pid, &cfg) else { return };
        drop(cfg);
        if let Some((_, policy)) = p.rule {
            if self.place(pid, &p.cgroup, Some(policy.clone())) {
                // Потомки, успевшие запуститься до переноса.
                let cfg = Config::default();
                let mut todo = children(pid);
                let mut n = 0;
                while let Some(c) = todo.pop() {
                    n += 1;
                    if n > 256 {
                        break;
                    }
                    if let Some(q) = read_proc(c, &cfg) {
                        self.place(c, &q.cgroup, Some(policy.clone()));
                    }
                    todo.extend(children(c));
                }
            }
        }
    }

    /// Пересмотреть все процессы.
    pub fn rescan(&self) {
        let cfg = self.cfg.read().unwrap().clone();
        let me = std::process::id() as i32;
        let mut procs: HashMap<i32, Proc> = HashMap::new();
        for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
            if pid <= 1 || pid == me {
                continue;
            }
            if let Some(p) = read_proc(pid, &cfg) {
                procs.insert(pid, p);
            }
        }
        let mut counts: BTreeMap<(String, String), u32> = BTreeMap::new();
        let mut dirs: HashSet<String> = HashSet::new();
        for (&pid, p) in &procs {
            // Выбор ближайшего предка с правилом.
            let mut want = None;
            let mut cur = pid;
            for _ in 0..64 {
                let Some(q) = procs.get(&cur) else { break };
                if let Some(r) = &q.rule {
                    want = Some(r.clone());
                    break;
                }
                if q.ppid <= 1 {
                    break;
                }
                cur = q.ppid;
            }
            let (_, now) = split_ours(&p.cgroup);
            if now.is_none() && want.is_none() {
                // Правило убрали, а процесс ещё в таблице меток.
                self.set_mark(pid, 0);
                continue;
            }
            if now.is_some() {
                dirs.insert(p.cgroup.clone());
            }
            if self.place(pid, &p.cgroup, want.as_ref().map(|w| w.1.clone())) {
                if let Some((name, pol)) = want {
                    *counts.entry((name, pol.id())).or_default() += 1;
                }
            }
        }
        // Опустевшие свои cgroup.
        self.prune(dirs.into_iter().map(|d| cg_dir(&d)));
        *self.running.lock().unwrap() = counts.into_iter().map(|((app, policy), n)| AppRunning { app, policy, processes: n }).collect();
    }

    /// Убрать опустевшие свои cgroup (и все известные).
    fn prune(&self, extra: impl Iterator<Item = PathBuf>) {
        let mut all: HashSet<PathBuf> = self.attached.lock().unwrap().clone();
        all.extend(extra);
        for dir in all {
            let empty = std::fs::read_to_string(dir.join("cgroup.procs")).is_ok_and(|s| s.trim().is_empty());
            if !dir.exists() || (empty && std::fs::remove_dir(&dir).is_ok()) {
                self.attached.lock().unwrap().remove(&dir);
            }
        }
    }

    /// Вернуть все процессы в их прежние cgroup (остановка службы).
    pub fn release_all(&self) {
        let cfg = Config::default();
        for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
            if let Some(p) = read_proc(pid, &cfg) {
                if split_ours(&p.cgroup).1.is_some() {
                    self.place(pid, &p.cgroup, None);
                }
            }
        }
        self.prune(std::iter::empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split() {
        assert_eq!(split_ours("/user.slice/session-4.scope/synnet-both"), ("/user.slice/session-4.scope", Some(AppPolicy::Both)));
        assert_eq!(split_ours("/user.slice/session-4.scope/synnet-primary"), ("/user.slice/session-4.scope", Some(AppPolicy::Primary)));
        assert_eq!(split_ours("/user.slice/session-4.scope"), ("/user.slice/session-4.scope", None));
        assert_eq!(split_ours("/synnet-both"), ("/", Some(AppPolicy::Both)));
        assert_eq!(split_ours("/u/s.scope/synnet-if-enp128s20f0u1"), ("/u/s.scope", Some(AppPolicy::Iface("enp128s20f0u1".into()))));
        assert_eq!(split_ours("/x/synnet-weird"), ("/x/synnet-weird", None));
    }
}
