//! fanotify: метки на целые файловые системы (`FAN_MARK_FILESYSTEM`),
//! события с папкой и именем (`FAN_REPORT_DFID_NAME`) — путь собирается
//! из дескриптора папки (`open_by_handle_at`) и имени. Процесс берётся из
//! события (pid) и `/proc` сразу при чтении.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use synfsd::Kind;

/// Файловые системы с файлами пользователя (псевдо-ФС и сеть — нет).
const FS_TYPES: [&str; 10] = ["ext4", "ext3", "btrfs", "xfs", "f2fs", "bcachefs", "vfat", "exfat", "ntfs3", "jfs"];

const MASK: u64 = libc::FAN_CREATE | libc::FAN_DELETE | libc::FAN_RENAME | libc::FAN_MODIFY | libc::FAN_ONDIR;

/// Событие с собранным путём и процессом.
pub struct Event {
    pub kind: Kind,
    pub path: String,
    pub old_path: Option<String>,
    pub is_dir: bool,
    pub pid: i32,
    pub proc_: Proc,
}

#[derive(Clone, Default)]
pub struct Proc {
    pub exe: String,
    pub comm: String,
    pub uid: u32,
}

struct Mount {
    fsid: [i32; 2],
    fd: OwnedFd,
}

pub struct Fan {
    fd: OwnedFd,
    mounts: Vec<Mount>,
    pub watched: Vec<String>,
    own_pid: i32,
    /// Дескриптор папки → путь (папки переименовывают редко; на
    /// переименовании папки кэш сбрасывается).
    dirs: HashMap<Vec<u8>, (PathBuf, Instant)>,
    procs: HashMap<i32, (Proc, Instant)>,
    pub overflows: std::sync::Arc<std::sync::atomic::AtomicU64>,
    launched: crate::procmon::Procs,
}

const DIR_TTL: Duration = Duration::from_secs(5);
const PROC_TTL: Duration = Duration::from_secs(3);

fn fsid_of(path: &Path) -> Option<[i32; 2]> {
    let c = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    // `fsid_t` — два int без публичных полей.
    Some(unsafe { std::mem::transmute::<libc::fsid_t, [i32; 2]>(st.f_fsid) })
}

/// Точки монтирования дисковых ФС из `/proc/self/mountinfo` (поля с
/// пробелами экранированы как `\040`).
fn disk_mounts() -> Vec<PathBuf> {
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((left, right)) = line.split_once(" - ") else { continue };
        let fstype = right.split(' ').next().unwrap_or("");
        if !FS_TYPES.contains(&fstype) {
            continue;
        }
        let Some(mp) = left.split(' ').nth(4) else { continue };
        out.push(PathBuf::from(unescape(mp)));
    }
    out
}

pub fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Fan {
    pub fn new() -> anyhow::Result<Fan> {
        let flags = libc::FAN_CLASS_NOTIF | libc::FAN_CLOEXEC | libc::FAN_REPORT_DFID_NAME | libc::FAN_UNLIMITED_QUEUE;
        let fd = unsafe { libc::fanotify_init(flags, (libc::O_RDONLY | libc::O_LARGEFILE) as u32) };
        if fd < 0 {
            bail!("fanotify_init: {}", std::io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let mut fan = Fan {
            fd,
            mounts: Vec::new(),
            watched: Vec::new(),
            own_pid: std::process::id() as i32,
            dirs: HashMap::new(),
            procs: HashMap::new(),
            overflows: Default::default(),
            launched: crate::procmon::start(),
        };
        for mp in disk_mounts() {
            if let Err(e) = fan.mark(&mp) {
                tracing::warn!("{}: {e:#}", mp.display());
            }
        }
        if fan.watched.is_empty() {
            bail!("ни одной файловой системы не удалось поставить под наблюдение");
        }
        Ok(fan)
    }

    fn mark(&mut self, mp: &Path) -> anyhow::Result<()> {
        let c = CString::new(mp.as_os_str().as_encoded_bytes())?;
        let r = unsafe {
            libc::fanotify_mark(self.fd.as_raw_fd(), libc::FAN_MARK_ADD | libc::FAN_MARK_FILESYSTEM, MASK, libc::AT_FDCWD, c.as_ptr())
        };
        if r != 0 {
            bail!("fanotify_mark: {}", std::io::Error::last_os_error());
        }
        let fsid = fsid_of(mp).context("statfs")?;
        let mfd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
        if mfd < 0 {
            bail!("open: {}", std::io::Error::last_os_error());
        }
        self.mounts.push(Mount { fsid, fd: unsafe { OwnedFd::from_raw_fd(mfd) } });
        self.watched.push(mp.display().to_string());
        tracing::info!("наблюдаю {}", mp.display());
        Ok(())
    }

    /// Читать события (блокируясь) и отдавать их `f`.
    pub fn run(&mut self, mut f: impl FnMut(Event)) -> anyhow::Result<()> {
        // Выровненный буфер: заголовки читаются по смещениям.
        let mut buf = vec![0u64; 32 * 1024];
        loop {
            let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len() * 8) };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                bail!("read: {e}");
            }
            let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), n as usize) };
            self.parse(bytes, &mut f);
        }
    }

    fn parse(&mut self, b: &[u8], f: &mut impl FnMut(Event)) {
        const META: usize = std::mem::size_of::<libc::fanotify_event_metadata>();
        let mut off = 0;
        while off + META <= b.len() {
            let m: libc::fanotify_event_metadata = unsafe { std::ptr::read_unaligned(b[off..].as_ptr().cast()) };
            let len = m.event_len as usize;
            if m.vers != libc::FANOTIFY_METADATA_VERSION || len < META || off + len > b.len() {
                break;
            }
            if m.fd >= 0 {
                unsafe { libc::close(m.fd) };
            }
            if m.mask & libc::FAN_Q_OVERFLOW != 0 {
                self.overflows.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                tracing::warn!("очередь fanotify переполнилась — часть событий потеряна");
            } else if m.pid != self.own_pid {
                self.event(&m, &b[off + m.metadata_len as usize..off + len], f);
            }
            off += len;
        }
    }

    fn event(&mut self, m: &libc::fanotify_event_metadata, mut info: &[u8], f: &mut impl FnMut(Event)) {
        let mut path = None;
        let mut old = None;
        while info.len() >= 4 {
            let ty = info[0];
            let len = u16::from_ne_bytes([info[2], info[3]]) as usize;
            if len < 4 || len > info.len() {
                break;
            }
            let rec = &info[..len];
            match ty {
                libc::FAN_EVENT_INFO_TYPE_DFID_NAME | libc::FAN_EVENT_INFO_TYPE_NEW_DFID_NAME => path = self.resolve(rec),
                libc::FAN_EVENT_INFO_TYPE_OLD_DFID_NAME => old = self.resolve(rec),
                _ => {}
            }
            info = &info[len..];
        }
        let Some(path) = path else { return };
        let is_dir = m.mask & libc::FAN_ONDIR != 0;
        let kind = if m.mask & libc::FAN_RENAME != 0 {
            if is_dir {
                // Пути внутри переименованной папки сменились.
                self.dirs.clear();
            }
            Kind::Renamed
        } else if m.mask & libc::FAN_CREATE != 0 {
            Kind::Created
        } else if m.mask & libc::FAN_DELETE != 0 {
            Kind::Deleted
        } else if m.mask & libc::FAN_MODIFY != 0 {
            Kind::Modified
        } else {
            return;
        };
        if is_dir && kind == Kind::Deleted {
            self.dirs.clear();
        }
        let proc_ = self.proc_info(m.pid);
        f(Event {
            kind,
            path: path.to_string_lossy().into_owned(),
            old_path: if kind == Kind::Renamed { old.map(|o| o.to_string_lossy().into_owned()) } else { None },
            is_dir,
            pid: m.pid,
            proc_,
        });
    }

    /// Запись `fanotify_event_info_fid` + имя: заголовок (4), fsid (8),
    /// `file_handle` (4 + 4 + handle_bytes), имя с нулём на конце.
    fn resolve(&mut self, rec: &[u8]) -> Option<PathBuf> {
        if rec.len() < 4 + 8 + 8 {
            return None;
        }
        let fsid = [i32::from_ne_bytes(rec[4..8].try_into().ok()?), i32::from_ne_bytes(rec[8..12].try_into().ok()?)];
        let hb = u32::from_ne_bytes(rec[12..16].try_into().ok()?) as usize;
        let handle_end = 20 + hb;
        if rec.len() < handle_end {
            return None;
        }
        let handle = &rec[12..handle_end];
        let name_raw = &rec[handle_end..];
        let name_end = name_raw.iter().position(|&c| c == 0).unwrap_or(name_raw.len());
        let name = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&name_raw[..name_end]);
        let dir = self.dir_path(fsid, handle)?;
        Some(if name.is_empty() || name == "." { dir } else { dir.join(name) })
    }

    fn dir_path(&mut self, fsid: [i32; 2], handle: &[u8]) -> Option<PathBuf> {
        let mut key = Vec::with_capacity(8 + handle.len());
        key.extend_from_slice(&fsid[0].to_ne_bytes());
        key.extend_from_slice(&fsid[1].to_ne_bytes());
        key.extend_from_slice(handle);
        if let Some((p, at)) = self.dirs.get(&key) {
            if at.elapsed() < DIR_TTL {
                return Some(p.clone());
            }
        }
        // Дескриптор подходит к любой точке монтирования той же ФС; у
        // подтомов btrfs fsid свой — тогда пробуем все.
        let mut order: Vec<usize> = (0..self.mounts.len()).collect();
        order.sort_by_key(|&i| self.mounts[i].fsid != fsid);
        let mut h = handle.to_vec();
        let p = order.into_iter().find_map(|i| {
            let fd = unsafe {
                libc::open_by_handle_at(self.mounts[i].fd.as_raw_fd(), h.as_mut_ptr().cast(), libc::O_PATH | libc::O_CLOEXEC)
            };
            if fd < 0 {
                return None;
            }
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            let link = std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()?;
            let s = link.to_string_lossy();
            Some(PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s)))
        })?;
        if self.dirs.len() > 50_000 {
            self.dirs.clear();
        }
        self.dirs.insert(key, (p.clone(), Instant::now()));
        Some(p)
    }

    fn proc_info(&mut self, pid: i32) -> Proc {
        if let Some((p, at)) = self.procs.get(&pid) {
            if at.elapsed() < PROC_TTL {
                return p.clone();
            }
        }
        // Процесс уже завершился — что запомнили при его запуске.
        let mut p = crate::procmon::read_proc(pid);
        if p.exe.is_empty() {
            if let Some(known) = self.launched.get(pid) {
                p = known;
            }
        }
        if self.procs.len() > 4096 {
            self.procs.retain(|_, (_, at)| at.elapsed() < PROC_TTL);
        }
        self.procs.insert(pid, (p.clone(), Instant::now()));
        p
    }
}
