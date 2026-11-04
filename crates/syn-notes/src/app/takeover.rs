//! Последний запуск забирает управление. Запуск `syn-notes` (из Проводника,
//! меню, ассистента) сначала отдаёт команду хозяину через сокет. Если
//! хозяин не ответил (завис), он завершается, и хозяином становится новый
//! процесс. Копии без сокета, оставшиеся от прежних сбоев, тоже
//! завершаются: они держали проекты (блокировку `.syn`), и новый процесс
//! не мог их открыть («Проект открыт в другой программе»).
//!
//! Решение «отдать или стать хозяином» принимается под блокировкой
//! `syn-notes.start.lock` и держится до подъёма сокета: два одновременных
//! запуска не становятся хозяевами оба и не завершают друг друга.
//!
//! Хозяина завершают SIGTERM: [`install_term_handler`] записывает
//! несохранённое и выходит. Через 4 с без выхода — SIGKILL.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use syn_notes_proto::socket_path;

/// Сколько ждать ответа хозяина, прежде чем считать его зависшим. Команды
/// хозяин исполняет на главном потоке; дольше этого он занят, только если
/// завис.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Блокировка запуска. Держится, пока жив объект: процесс-хозяин хранит
/// её до подъёма сокета.
pub struct StartGuard(#[allow(dead_code)] Option<File>);

fn start_lock_path() -> PathBuf {
    let sock = socket_path();
    let mut name = sock.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".start.lock");
    sock.with_file_name(name)
}

/// Занять блокировку запуска (ждёт до 30 с, потом идёт без неё).
pub fn start_lock() -> StartGuard {
    let path = start_lock_path();
    let Ok(file) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path) else {
        return StartGuard(None);
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        // SAFETY: flock над своим открытым дескриптором.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return StartGuard(Some(file));
        }
        if Instant::now() > deadline {
            tracing::warn!("{}: блокировка запуска не отпущена за 30 с", path.display());
            return StartGuard(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Итог попытки отдать команду.
pub enum Forward {
    /// Хозяин (pid) принял команду — этому процессу делать нечего.
    Done(Option<u32>),
    /// Хозяина нет (или он завершён как зависший) — стать хозяином.
    BecomeHost,
}

/// Отдать запрос хозяину; зависшего — завершить.
pub fn forward(req: &serde_json::Value) -> Forward {
    let Ok(stream) = UnixStream::connect(socket_path()) else {
        return Forward::BecomeHost;
    };
    let pid = peer_pid(&stream);
    match exchange(stream, req) {
        Ok(()) => Forward::Done(pid),
        Err(e) => {
            tracing::warn!("хозяин заметок (pid {pid:?}) не ответил: {e} — забираю управление");
            if let Some(pid) = pid.filter(|p| *p != std::process::id()) {
                terminate(&[pid]);
            }
            let _ = std::fs::remove_file(socket_path());
            Forward::BecomeHost
        }
    }
}

fn exchange(mut s: UnixStream, req: &serde_json::Value) -> std::io::Result<()> {
    s.set_read_timeout(Some(REPLY_TIMEOUT))?;
    s.set_write_timeout(Some(REPLY_TIMEOUT))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    if reply.trim().is_empty() {
        return Err(std::io::Error::other("пустой ответ"));
    }
    Ok(())
}

/// pid процесса на том конце сокета.
fn peer_pid(s: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: getsockopt пишет не больше len байт в cred.
    let rc = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    (rc == 0 && cred.pid > 0).then_some(cred.pid as u32)
}

/// Прежние копии syn-notes того же сеанса (тот же HOME, каталог сеанса и
/// сокет), запущенные раньше этого процесса. Снимки (`--screenshot`) и
/// проверки с другим HOME/сокетом не трогаются; более поздние запуски
/// ждут блокировку запуска и отдадут команду новому хозяину сами.
pub fn strays() -> Vec<u32> {
    let me = std::process::id();
    let Some(my_start) = start_time(me) else { return Vec::new() };
    let my_env = session_env(Path::new("/proc/self"));
    let my_uid = uid(Path::new("/proc/self"));
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|&pid| pid != me)
        .filter(|&pid| {
            let p = PathBuf::from(format!("/proc/{pid}"));
            std::fs::read_to_string(p.join("comm")).is_ok_and(|c| c.trim() == "syn-notes")
                && uid(&p) == my_uid
                && start_time(pid).is_some_and(|t| t < my_start)
                && !std::fs::read(p.join("cmdline")).is_ok_and(|c| c.split(|b| *b == 0).any(|a| a == b"--screenshot"))
                && session_env(&p) == my_env
        })
        .collect()
}

fn uid(proc: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(proc.join("status")).ok()?;
    s.lines().find(|l| l.starts_with("Uid:"))?.split_whitespace().nth(1)?.parse().ok()
}

/// Время запуска процесса (тики с загрузки, поле 22 `/proc/pid/stat`).
fn start_time(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Имя процесса в скобках может содержать пробелы — считать после «)».
    let rest = &s[s.rfind(')')? + 2..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn session_env(proc: &Path) -> Vec<Option<Vec<u8>>> {
    const KEYS: [&[u8]; 3] = [b"HOME=", b"XDG_RUNTIME_DIR=", b"SYN_NOTES_SOCKET="];
    let env = std::fs::read(proc.join("environ")).unwrap_or_default();
    KEYS.iter()
        .map(|k| env.split(|b| *b == 0).find_map(|kv| kv.strip_prefix(*k).map(<[u8]>::to_vec)))
        .collect()
}

/// Завершить процессы: SIGTERM (запишут несохранённое), через 4 с — SIGKILL.
pub fn terminate(pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let alive = |pid: u32| Path::new(&format!("/proc/{pid}")).exists() && !is_zombie(pid);
    for &pid in pids {
        tracing::info!("завершаю прежнюю копию syn-notes (pid {pid})");
        // SAFETY: сигнал процессу по pid.
        unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline && pids.iter().any(|&p| alive(p)) {
        std::thread::sleep(Duration::from_millis(50));
    }
    for &pid in pids.iter().filter(|&&p| alive(p)) {
        tracing::warn!("syn-notes (pid {pid}) не вышел по SIGTERM — SIGKILL");
        // SAFETY: сигнал процессу по pid.
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && pids.iter().any(|&p| alive(p)) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.get(s.rfind(')')? + 2..).and_then(|r| r.chars().next()))
        .is_some_and(|c| c == 'Z' || c == 'X')
}

static TERM_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_term(_: libc::c_int) {
    let fd = TERM_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let b = 1u8;
        // SAFETY: write — async-signal-safe.
        unsafe { libc::write(fd, (&b as *const u8).cast(), 1) };
    }
}

/// SIGTERM/SIGINT/SIGHUP: записать несохранённое (правки страниц — в
/// очередь, очередь — на диск) и выйти. Главный поток не ответил за 3 с —
/// пишется то, что уже в очереди.
pub fn install_term_handler() {
    let mut fds = [0i32; 2];
    // SAFETY: pipe2 заполняет два дескриптора.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return;
    }
    TERM_FD.store(fds[1], Ordering::Relaxed);
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: обработчик только пишет байт в канал.
        unsafe { libc::signal(sig, on_term as *const () as libc::sighandler_t) };
    }
    let rd = fds[0];
    std::thread::Builder::new()
        .name("notes-term".into())
        .spawn(move || {
            let mut b = 0u8;
            // SAFETY: чтение одного байта из своего канала.
            while unsafe { libc::read(rd, (&mut b as *mut u8).cast(), 1) } != 1 {}
            tracing::info!("syn-notes: сигнал завершения — записываю несохранённое");
            let (tx, rx) = std::sync::mpsc::channel();
            syngui::async_runtime::run_on_main_thread(move || {
                let ctx = syngui::context_provider::use_context::<crate::notes::NotesCtx>();
                crate::notes::autosave::enqueue_dirty(ctx);
                let _ = tx.send(crate::notes::autosave::flush_all());
            });
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(Err(e)) => tracing::error!("запись при выходе: {e}"),
                Ok(Ok(())) => {}
                Err(_) => {
                    if let Err(e) = crate::notes::autosave::flush_all() {
                        tracing::error!("запись при выходе: {e}");
                    }
                }
            }
            std::process::exit(0);
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_start_time_is_known() {
        assert!(start_time(std::process::id()).is_some());
        assert!(strays().iter().all(|&p| p != std::process::id()));
    }
}
