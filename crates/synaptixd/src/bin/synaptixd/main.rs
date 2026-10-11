mod config;
mod devices;
mod manager;
mod registry;
mod runners;

use std::ffi::CStr;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use synaptixd::wire::{self, Mapped};
use synaptixd::{CtcReply, Envelope, Error, ErrorCode, FdRef, Hello, Reply, Request, SettingsChange};

use manager::Manager;

fn is_admin(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return false;
        }
        let name = (*pw).pw_name;
        let base = (*pw).pw_gid;
        let grp = libc::getgrnam(c"wheel".as_ptr());
        if grp.is_null() {
            return false;
        }
        let wheel = (*grp).gr_gid;
        if base == wheel {
            return true;
        }
        let mut groups = vec![0 as libc::gid_t; 256];
        let mut n = groups.len() as libc::c_int;
        if libc::getgrouplist(name, base, groups.as_mut_ptr(), &mut n) < 0 {
            return false;
        }
        let _ = CStr::from_ptr(name);
        groups[..n as usize].contains(&wheel)
    }
}

fn data<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn pcm_from(fds: &mut [Option<OwnedFd>], r: FdRef, samples: usize) -> Result<Mapped, Error> {
    let fd = fds
        .get_mut(r.fd as usize)
        .and_then(Option::take)
        .ok_or_else(|| Error::new(ErrorCode::Protocol, "нет файла с аудио"))?;
    Mapped::map(fd, samples * 4).map_err(|e| Error::new(ErrorCode::Protocol, e.to_string()))
}

fn denied() -> Error {
    Error::new(ErrorCode::Denied, "менять параметры нейросетей могут root и группа wheel")
}

struct Conn {
    mgr: Arc<Manager>,
    sock: OwnedFd,
    admin: bool,
    opened: Vec<String>,
    current: u64,
}

impl Conn {
    fn send(&self, reply: &Reply, fds: &[OwnedFd]) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(reply).unwrap_or_default();
        let borrowed: Vec<_> = fds.iter().map(|f| f.as_fd()).collect();
        wire::send(self.sock.as_fd(), &bytes, &borrowed)
    }

    fn dispatch(&mut self, req: Request, fds: Vec<OwnedFd>) -> Result<(Value, Vec<OwnedFd>), Error> {
        let mgr = self.mgr.clone();
        let mut fds: Vec<Option<OwnedFd>> = fds.into_iter().map(Some).collect();
        let none = Vec::new;
        Ok(match req {
            Request::Hello { client } => {
                tracing::debug!("клиент {client}");
                (data(Hello { version: synaptixd::VERSION, devices: devices::list() }), none())
            }
            Request::List => (data(mgr.list()), none()),
            Request::Status => (data(mgr.status()), none()),
            Request::Rescan => {
                mgr.rescan();
                (data(mgr.list()), none())
            }
            Request::Open { model } => {
                let info = mgr.open(&model)?;
                self.opened.push(model);
                (data(info), none())
            }
            Request::Close { model } => {
                if let Some(i) = self.opened.iter().position(|m| *m == model) {
                    self.opened.swap_remove(i);
                    mgr.close(&model);
                }
                (Value::Null, none())
            }
            Request::Load { model } => {
                drop(mgr.acquire(&model)?);
                (data(mgr.entry(&model)?), none())
            }
            Request::Unload { model } => {
                if !self.admin {
                    return Err(denied());
                }
                mgr.unload(&model);
                (data(mgr.entry(&model)?), none())
            }
            Request::Set { model, device, pinned, idle_unload_min, preload } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set(&model, SettingsChange { device, pinned, idle_unload_min, preload })?), none())
            }
            Request::Ctc { model, pcm, samples, sample_rate, hidden } => {
                let pcm = pcm_from(&mut fds, pcm, samples)?;
                let r = mgr.run(&model, |m| m.ctc(pcm.f32s(), sample_rate, hidden))?;
                let mut out = vec![wire::memfd_f32("logprobs", &r.logprobs)?];
                let hidden = match &r.hidden {
                    Some(h) => {
                        out.push(wire::memfd_f32("hidden", h)?);
                        Some(FdRef { fd: 1 })
                    }
                    None => None,
                };
                let reply = CtcReply {
                    frames: r.frames,
                    vocab: r.vocab,
                    frame_sec: r.frame_sec,
                    logprobs: FdRef { fd: 0 },
                    hidden,
                    hidden_dim: r.hidden_dim,
                };
                (data(reply), out)
            }
            Request::MfaAlign { model, pcm, samples, words, options } => {
                let pcm = pcm_from(&mut fds, pcm, samples)?;
                let al = mgr.run(&model, |m| m.mfa_align(pcm.f32s(), &words, options.as_ref()))?;
                (data(al), none())
            }
            Request::Transcribe { model, pcm, samples, sample_rate, language } => {
                let pcm = pcm_from(&mut fds, pcm, samples)?;
                let t = mgr.run(&model, |m| m.transcribe(pcm.f32s(), sample_rate, language.as_deref()))?;
                (data(t), none())
            }
            Request::Generate { model, messages, max_tokens, temperature } => {
                let sock = self.sock.as_fd();
                let id = self.current;
                let send = |piece: &str| {
                    let r = Reply { id, event: Some(synaptixd::Event::Token { text: piece.to_string() }), ..Default::default() };
                    let _ = wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]);
                };
                let g = mgr.run(&model, |m| m.generate(&messages, max_tokens, temperature, &send))?;
                (data(g), none())
            }
            Request::Subscribe => unreachable!(),
        })
    }

    fn subscribe(&self, id: u64) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.mgr.subscribe(tx);
        let ok = Reply { id, ok: Some(true), ..Default::default() };
        if self.send(&ok, &[]).is_err() {
            return;
        }
        for ev in rx {
            let r = Reply { id, event: Some(ev), ..Default::default() };
            if self.send(&r, &[]).is_err() {
                return;
            }
        }
    }

    fn serve(mut self) {
        loop {
            let (bytes, fds) = match wire::recv(self.sock.as_fd()) {
                Ok(x) => x,
                Err(_) => break,
            };
            let env: Envelope = match serde_json::from_slice(&bytes) {
                Ok(e) => e,
                Err(e) => {
                    let r = Reply { id: 0, ok: Some(false), error: Some(e.to_string()), code: Some(ErrorCode::Protocol), ..Default::default() };
                    if self.send(&r, &[]).is_err() {
                        break;
                    }
                    continue;
                }
            };
            if matches!(env.req, Request::Subscribe) {
                self.subscribe(env.id);
                break;
            }
            let id = env.id;
            self.current = id;
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.dispatch(env.req, fds)))
                .unwrap_or_else(|_| Err(Error::new(ErrorCode::Failed, "сбой модели")));
            let sent = match res {
                Ok((data, out)) => self.send(&Reply { id, ok: Some(true), data, ..Default::default() }, &out),
                Err(e) => self.send(
                    &Reply { id, ok: Some(false), error: Some(e.message), code: Some(e.code), ..Default::default() },
                    &[],
                ),
            };
            if sent.is_err() {
                break;
            }
        }
        for id in std::mem::take(&mut self.opened) {
            self.mgr.close(&id);
        }
    }
}

fn raise_nofile() {
    unsafe {
        let mut r: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) == 0 && r.rlim_cur < r.rlim_max {
            r.rlim_cur = r.rlim_max;
            libc::setrlimit(libc::RLIMIT_NOFILE, &r);
        }
    }
}

fn main() -> anyhow::Result<()> {
    raise_nofile();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "synaptixd=info".into()))
        .init();
    devices::init();
    let cfg = config::Config::load();
    if let Some(dir) = cfg.socket.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = wire::listen(&cfg.socket)?;
    std::fs::set_permissions(&cfg.socket, std::fs::Permissions::from_mode(0o666))?;
    tracing::info!("слушаю {}", cfg.socket.display());
    let mgr = Manager::new(cfg);
    {
        let m = mgr.clone();
        std::thread::Builder::new().name("monitor".into()).spawn(move || m.monitor())?;
        let m = mgr.clone();
        std::thread::Builder::new().name("preload".into()).spawn(move || m.preload())?;
    }
    loop {
        let sock = match wire::accept(listener.as_fd()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("accept: {e}");
                continue;
            }
        };
        let admin = wire::peer(sock.as_fd()).map(|p| is_admin(p.uid)).unwrap_or(false);
        let conn = Conn { mgr: mgr.clone(), sock, admin, opened: Vec::new(), current: 0 };
        let _ = std::thread::Builder::new().name("conn".into()).spawn(move || conn.serve());
    }
}
