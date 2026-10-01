//! Демон `syndroidd`: состояние контейнера, фоновые задания, сокет управления.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::api::{Job, Request, Response, Session, State, Status};
use crate::config::Config;
use crate::{container, images, paths};

#[derive(Default)]
struct Inner {
    state: Option<State>,
    init_pid: Option<i32>,
    session: Option<Session>,
    android_version: Option<String>,
    error: Option<String>,
    /// Номер запуска: фоновые потоки старого запуска не трогают новый.
    generation: u64,
    jobs: Vec<Job>,
}

pub struct Daemon {
    inner: Mutex<Inner>,
    changed: Condvar,
    next_job: AtomicU64,
}

impl Daemon {
    fn state(&self) -> State {
        self.inner.lock().unwrap().state.unwrap_or(State::Stopped)
    }

    fn status(&self) -> Status {
        let i = self.inner.lock().unwrap();
        Status {
            state: i.state.unwrap_or(State::Stopped),
            image: Config::load().active,
            init_pid: i.init_pid,
            session: i.session.clone(),
            uptime: i.init_pid.and_then(container::uptime_of),
            android_version: i.android_version.clone(),
            jobs: i.jobs.clone(),
            error: i.error.clone(),
        }
    }

    fn set_state(&self, s: State) {
        self.inner.lock().unwrap().state = Some(s);
        self.changed.notify_all();
    }

    fn start(self: &Arc<Self>, session: Session) -> Result<()> {
        {
            let mut i = self.inner.lock().unwrap();
            match i.state.unwrap_or(State::Stopped) {
                State::Stopped => {}
                s => bail!("Android уже {s:?}"),
            }
            i.state = Some(State::Starting);
            i.error = None;
            i.android_version = None;
            i.generation += 1;
        }
        let cfg = Config::load();
        let running = match container::start(&cfg, &session) {
            Ok(r) => r,
            Err(e) => {
                let mut i = self.inner.lock().unwrap();
                i.state = Some(State::Stopped);
                i.error = Some(format!("{e:#}"));
                self.changed.notify_all();
                return Err(e);
            }
        };
        let gen = {
            let mut i = self.inner.lock().unwrap();
            i.init_pid = Some(running.init_pid);
            i.session = Some(session);
            i.generation
        };
        tracing::info!("контейнер запущен, init = {}", running.init_pid);
        // Ожидание выхода контейнера → уборка
        let me = self.clone();
        let network = cfg.network;
        std::thread::spawn(move || {
            let container::Running { mut starter, dnsmasq, .. } = running;
            let st = starter.wait();
            tracing::info!("контейнер завершился: {st:?}");
            container::cleanup(dnsmasq, network);
            let mut i = me.inner.lock().unwrap();
            if i.generation == gen {
                if i.state == Some(State::Starting) {
                    i.error = Some("Android завершился во время загрузки (см. /run/syndroid/container.log, dmesg)".into());
                }
                i.state = Some(State::Stopped);
                i.init_pid = None;
            }
            drop(i);
            me.changed.notify_all();
        });
        // Ожидание загрузки Android
        let me = self.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let (pid, alive) = {
                let i = me.inner.lock().unwrap();
                (i.init_pid, i.generation == gen && i.state == Some(State::Starting))
            };
            let (Some(pid), true) = (pid, alive) else { break };
            if container::getprop(pid, "sys.boot_completed") == "1" {
                let ver = container::getprop(pid, "ro.build.version.release");
                let mut i = me.inner.lock().unwrap();
                if i.generation == gen && i.state == Some(State::Starting) {
                    i.state = Some(State::Running);
                    i.android_version = Some(ver);
                    tracing::info!("Android загрузился");
                }
                drop(i);
                me.changed.notify_all();
                break;
            }
        });
        Ok(())
    }

    fn stop(&self) -> Result<()> {
        let pid = {
            let mut i = self.inner.lock().unwrap();
            match (i.state.unwrap_or(State::Stopped), i.init_pid) {
                (State::Stopped, _) => return Ok(()),
                (_, Some(pid)) => {
                    i.state = Some(State::Stopping);
                    pid
                }
                (_, None) => bail!("нет PID контейнера"),
            }
        };
        let _ = container::freeze(false);
        // Данные Android — на ФС хоста: SIGKILL init'у равен выдёргиванию питания только для самого Android,
        // записанное в файлы остаётся. Сначала просим Android сбросить кэши.
        if let Ok(mut c) = container::command_in(pid, &["/system/bin/sync".into()]) {
            let _ = c.status();
        }
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let mut i = self.inner.lock().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while i.state != Some(State::Stopped) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                bail!("контейнер не остановился за 20 с");
            }
            i = self.changed.wait_timeout(i, left).unwrap().0;
        }
        Ok(())
    }

    fn freeze(&self, on: bool) -> Result<()> {
        let s = self.state();
        match (on, s) {
            (true, State::Running | State::Starting) => {
                container::freeze(true)?;
                self.set_state(State::Frozen);
            }
            (false, State::Frozen) => {
                container::freeze(false)?;
                let booted = self.inner.lock().unwrap().android_version.is_some();
                self.set_state(if booted { State::Running } else { State::Starting });
            }
            _ => bail!("нельзя: Android {s:?}"),
        }
        Ok(())
    }

    /// Фоновое задание с ходом выполнения в `Status::jobs`.
    fn job(self: &Arc<Self>, title: &str, f: impl FnOnce(images::Progress) -> Result<()> + Send + 'static) -> u64 {
        let id = self.next_job.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut i = self.inner.lock().unwrap();
            i.jobs.retain(|j| !j.finished);
            i.jobs.push(Job { id, title: title.into(), ..Default::default() });
        }
        let me = self.clone();
        std::thread::spawn(move || {
            let upd = |step: &str, done: u64, total: u64| {
                let mut i = me.inner.lock().unwrap();
                if let Some(j) = i.jobs.iter_mut().find(|j| j.id == id) {
                    j.step = step.into();
                    j.done = done;
                    j.total = total;
                }
            };
            let r = f(&upd);
            let mut i = me.inner.lock().unwrap();
            if let Some(j) = i.jobs.iter_mut().find(|j| j.id == id) {
                j.finished = true;
                j.error = r.err().map(|e| format!("{e:#}"));
                if let Some(e) = &j.error {
                    tracing::warn!("задание «{}»: {e}", j.title);
                }
            }
        });
        id
    }

    fn handle(self: &Arc<Self>, req: Request, peer: Peer) -> Result<Response> {
        let mutating = !matches!(req, Request::Status | Request::Images | Request::GetConfig | Request::CheckUpdates);
        if mutating && !peer.admin {
            bail!("нет прав: нужен root или группа wheel/android");
        }
        Ok(match req {
            Request::Status => Response::Status(self.status()),
            Request::Start { session } => {
                if peer.uid != 0 && peer.uid != session.uid {
                    bail!("нельзя запустить Android для чужого сеанса");
                }
                self.start(session)?;
                Response::Ok
            }
            Request::Stop => {
                self.stop()?;
                Response::Ok
            }
            Request::Restart => {
                let session = self.inner.lock().unwrap().session.clone().context("Android ещё не запускался")?;
                self.stop()?;
                self.start(session)?;
                Response::Ok
            }
            Request::Freeze => {
                self.freeze(true)?;
                Response::Ok
            }
            Request::Unfreeze => {
                self.freeze(false)?;
                Response::Ok
            }
            Request::Images => Response::Images { sets: images::list(), active: Config::load().active },
            Request::CheckUpdates => {
                let (system, vendor) = images::ota_latest(&Config::load())?;
                let installed = images::exists(&images::set_name(&system.filename));
                Response::Updates { system, vendor, installed }
            }
            Request::FetchImages => {
                let id = self.job("Загрузка образов Android", |p| {
                    let mut c = Config::load();
                    let set = images::fetch(&c, None, p)?;
                    if c.active.is_none() {
                        c.active = Some(set.name);
                        c.save()?;
                    }
                    Ok(())
                });
                Response::Job { id }
            }
            Request::ImportImages { system, vendor, name } => {
                let id = self.job("Импорт образов Android", move |p| {
                    let set = images::import(system.as_ref(), vendor.as_ref(), name.as_deref(), p)?;
                    let mut c = Config::load();
                    if c.active.is_none() {
                        c.active = Some(set.name);
                        c.save()?;
                    }
                    Ok(())
                });
                Response::Job { id }
            }
            Request::UseImages { name } => {
                if !images::exists(&name) {
                    bail!("нет набора «{name}»");
                }
                let mut c = Config::load();
                c.active = Some(name);
                c.save()?;
                Response::Ok
            }
            Request::RemoveImages { name } => {
                let mut c = Config::load();
                if c.active.as_deref() == Some(&name) {
                    if self.state() != State::Stopped {
                        bail!("набор используется запущенным Android");
                    }
                    c.active = None;
                    c.save()?;
                }
                images::remove(&name)?;
                Response::Ok
            }
            Request::GetConfig => Response::Config { config: Config::load() },
            Request::SetConfig { config } => {
                config.save()?;
                Response::Ok
            }
        })
    }
}

#[derive(Clone, Copy)]
struct Peer {
    uid: u32,
    admin: bool,
}

fn peer_of(s: &UnixStream) -> Peer {
    let mut cred = libc::ucred { pid: 0, uid: u32::MAX, gid: u32::MAX };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    use std::os::fd::AsRawFd;
    let r = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    if r != 0 {
        return Peer { uid: u32::MAX, admin: false };
    }
    Peer { uid: cred.uid, admin: cred.uid == 0 || in_admin_group(cred.uid, cred.gid) }
}

/// Пользователь в группе wheel или android (по /etc/group и основной группе).
fn in_admin_group(uid: u32, gid: u32) -> bool {
    let user = std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|s| s.lines().find(|l| l.split(':').nth(2) == Some(&uid.to_string())).map(|l| l.split(':').next().unwrap_or("").to_string()))
        .unwrap_or_default();
    std::fs::read_to_string("/etc/group").ok().is_some_and(|s| {
        s.lines().any(|l| {
            let f: Vec<&str> = l.split(':').collect();
            f.len() >= 4
                && (f[0] == "wheel" || f[0] == "android")
                && (f[2] == gid.to_string() || f[3].split(',').any(|m| m == user))
        })
    })
}

fn serve(d: Arc<Daemon>, s: UnixStream) {
    let peer = peer_of(&s);
    let mut w = match s.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    for line in BufReader::new(s).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(req) => d.handle(req, peer).unwrap_or_else(|e| Response::Error { message: format!("{e:#}") }),
            Err(e) => Response::Error { message: format!("запрос: {e}") },
        };
        let mut out = serde_json::to_string(&resp).unwrap_or_default();
        out.push('\n');
        if w.write_all(out.as_bytes()).is_err() {
            break;
        }
    }
}

pub fn run() -> Result<()> {
    if unsafe { libc::getuid() } != 0 {
        bail!("syndroidd запускается от root (syndroid.service)");
    }
    std::fs::create_dir_all(paths::RUN)?;
    let _ = std::fs::remove_file(paths::SOCKET);
    let l = UnixListener::bind(paths::SOCKET).context(paths::SOCKET)?;
    std::fs::set_permissions(paths::SOCKET, std::fs::Permissions::from_mode(0o666))?;
    // Остатки прошлого запуска (демон упал с работающим контейнером): контейнер без хозяина убить.
    if let Ok(procs) = std::fs::read_to_string(std::path::Path::new(paths::CGROUP).join("cgroup.procs")) {
        for pid in procs.lines().filter_map(|l| l.parse::<i32>().ok()) {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        std::thread::sleep(Duration::from_millis(500));
        container::cleanup(None, true);
    }
    let d = Arc::new(Daemon { inner: Mutex::new(Inner::default()), changed: Condvar::new(), next_job: AtomicU64::new(0) });
    // SIGTERM (остановка службы) — остановить Android
    {
        let d = d.clone();
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTERM);
            libc::sigaddset(&mut set, libc::SIGINT);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }
        std::thread::spawn(move || {
            let mut sig = 0;
            unsafe { libc::sigwait(&set, &mut sig) };
            tracing::info!("сигнал {sig}: остановка");
            let _ = d.stop();
            let _ = std::fs::remove_file(paths::SOCKET);
            std::process::exit(0);
        });
    }
    tracing::info!("syndroidd слушает {}", paths::SOCKET);
    for s in l.incoming() {
        match s {
            Ok(s) => {
                let d = d.clone();
                std::thread::spawn(move || serve(d, s));
            }
            Err(e) => tracing::warn!("accept: {e}"),
        }
    }
    Ok(())
}
