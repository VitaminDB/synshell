//! Запуски процессов (netlink proc connector): программа запоминается в
//! момент `exec`, пока процесс жив, и хранится ещё [`KEEP_AFTER_EXIT`]
//! после выхода. Иначе короткие `cp`, `mkdir`, `sh -c` успевали
//! завершиться раньше, чем событие fanotify прочитано, и в `/proc` их уже
//! не было.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::fan::Proc;

const KEEP_AFTER_EXIT: Duration = Duration::from_secs(30);

const CN_IDX_PROC: u32 = 1;
const CN_VAL_PROC: u32 = 1;
const PROC_CN_MCAST_LISTEN: u32 = 1;
const PROC_EVENT_EXEC: u32 = 0x0000_0002;
const PROC_EVENT_EXIT: u32 = 0x8000_0000;

struct Known {
    proc_: Proc,
    exited: Option<Instant>,
}

#[derive(Clone, Default)]
pub struct Procs(Arc<Mutex<HashMap<i32, Known>>>);

impl Procs {
    /// Что известно о процессе (живом или недавно завершившемся).
    pub fn get(&self, pid: i32) -> Option<Proc> {
        self.0.lock().unwrap().get(&pid).map(|k| k.proc_.clone())
    }

    fn exec(&self, pid: i32) {
        let p = read_proc(pid);
        if p.exe.is_empty() && p.comm.is_empty() {
            return;
        }
        self.0.lock().unwrap().insert(pid, Known { proc_: p, exited: None });
    }

    fn exit(&self, pid: i32) {
        let mut m = self.0.lock().unwrap();
        if let Some(k) = m.get_mut(&pid) {
            k.exited = Some(Instant::now());
        }
        if m.len() > 8192 {
            m.retain(|_, k| k.exited.is_none_or(|t| t.elapsed() < KEEP_AFTER_EXIT));
        }
    }
}

/// Программа процесса по `/proc` (пусто — процесса уже нет).
pub fn read_proc(pid: i32) -> Proc {
    let base = format!("/proc/{pid}");
    Proc {
        exe: std::fs::read_link(format!("{base}/exe"))
            .map(|e| {
                let s = e.to_string_lossy().into_owned();
                s.strip_suffix(" (deleted)").map(str::to_string).unwrap_or(s)
            })
            .unwrap_or_default(),
        comm: std::fs::read_to_string(format!("{base}/comm")).map(|c| c.trim().to_string()).unwrap_or_default(),
        uid: std::fs::metadata(&base).map(|m| m.uid()).unwrap_or(u32::MAX),
    }
}

/// Подписаться на события процессов и слушать их в своём потоке. Не
/// вышло (нет netlink-коннектора в ядре) — работаем только по `/proc`.
pub fn start() -> Procs {
    let procs = Procs::default();
    match subscribe() {
        Ok(fd) => {
            let p = procs.clone();
            let _ = std::thread::Builder::new().name("procmon".into()).spawn(move || listen(fd, p));
        }
        Err(e) => tracing::warn!("proc connector: {e} — короткие процессы могут остаться без имени"),
    }
    procs
}

fn subscribe() -> std::io::Result<i32> {
    unsafe {
        let fd = libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, libc::NETLINK_CONNECTOR);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut sa: libc::sockaddr_nl = std::mem::zeroed();
        sa.nl_family = libc::AF_NETLINK as u16;
        sa.nl_groups = CN_IDX_PROC;
        sa.nl_pid = 0;
        if libc::bind(fd, (&sa as *const libc::sockaddr_nl).cast(), std::mem::size_of::<libc::sockaddr_nl>() as u32) != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        // nlmsghdr (16) + cn_msg (20) + op (4).
        let mut msg = [0u8; 40];
        msg[0..4].copy_from_slice(&40u32.to_ne_bytes());
        msg[4..6].copy_from_slice(&(libc::NLMSG_DONE as u16).to_ne_bytes());
        msg[12..16].copy_from_slice(&(std::process::id()).to_ne_bytes());
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

fn listen(fd: i32, procs: Procs) {
    let mut buf = vec![0u8; 8192];
    loop {
        let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // ENOBUFS — потеряли часть событий, слушаем дальше.
            if e.raw_os_error() == Some(libc::ENOBUFS) {
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
            // nlmsghdr (16) + cn_msg (20) + proc_event: what (4), cpu (4),
            // timestamp (8), данные: pid (4), tgid (4)…
            let ev = &b[off + 36..off + len];
            if ev.len() >= 24 {
                let what = u32::from_ne_bytes(ev[0..4].try_into().unwrap());
                let pid = i32::from_ne_bytes(ev[16..20].try_into().unwrap());
                let tgid = i32::from_ne_bytes(ev[20..24].try_into().unwrap());
                match what {
                    PROC_EVENT_EXEC => procs.exec(tgid),
                    PROC_EVENT_EXIT if pid == tgid => procs.exit(tgid),
                    _ => {}
                }
            }
            off += (len + 3) & !3;
        }
    }
}
