mod config;
mod defaults;
mod library;
mod devices;
mod flow;
mod gen;
mod hangup;
mod stages;
mod manager;
mod registry;
mod runners;

use std::ffi::CStr;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use synaptixd::wire::{self, Mapped};
use synaptixd::{CtcReply, Envelope, Error, ErrorCode, FdRef, Hello, ModelKind, Params, Reply, Request, SettingsChange};

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
            Request::Set { model, device, pinned, idle_unload_min, preload, quant, params } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set(&model, SettingsChange { device, pinned, idle_unload_min, preload, quant, params })?), none())
            }
            Request::Defaults => (data(mgr.defaults()), none()),
            Request::SetDefault { kind, model } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set_default(kind, model)?), none())
            }
            Request::Resolve { kind } => (data(mgr.entry(&mgr.resolve(kind)?)?), none()),
            Request::Libraries => (data(mgr.libraries()), none()),
            Request::AddLibrary { path, name } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.add_library(&path, name)?), none())
            }
            Request::RemoveLibrary { path } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.remove_library(&path)?), none())
            }
            Request::SetDefaultLibrary { path } => {
                if !self.admin {
                    return Err(denied());
                }
                (data(mgr.set_default_library(&path)?), none())
            }
            Request::DeleteModel { model } => {
                if !self.admin {
                    return Err(denied());
                }
                mgr.delete_model(&model)?;
                (Value::Null, none())
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
            Request::Image { model, prompt, negative, width, height, steps, guidance, seed, images, init, strength } => {
                let p = synaptixd::ImageParams { negative, width, height, steps, guidance, seed, images, init, strength };
                let sock = self.sock.as_fd();
                let id = self.current;
                // Клиент ушёл (отправка не прошла) — отмена генерации.
                let mut send = |step: usize, total: usize| -> bool {
                    let r = Reply { id, event: Some(synaptixd::Event::Progress { step, total }), ..Default::default() };
                    wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_ok()
                };
                let r = gen::image(&mgr, &model, prompt, p, &mut send)?;
                let fd = wire::memfd("image", &r.png)?;
                let reply = synaptixd::ImageReply { width: r.width, height: r.height, seed: r.seed, label: r.label, png: FdRef { fd: 0 }, bytes: r.png.len() };
                (data(reply), vec![fd])
            }
            Request::Video { model, prompt, negative, width, height, seconds, steps, guidance, seed, first_frame, last_frame, refs, audio, source_video, retake, extend, control_video, control, two_stage, strength } => {
                let p = synaptixd::VideoParams { negative, width, height, seconds, steps, guidance, seed, first_frame, last_frame, refs, audio, source_video, retake, extend, control_video, control, two_stage, strength };
                let raw = self.sock.as_raw_fd();
                let id = self.current;
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let send = |step: usize, total: usize| {
                    let r = Reply { id, event: Some(synaptixd::Event::Progress { step, total }), ..Default::default() };
                    let sock = unsafe { std::os::fd::BorrowedFd::borrow_raw(raw) };
                    if wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_err() {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                };
                let r = gen::video(&mgr, &model, prompt, p, &cancel, &send)?;
                let fd = wire::memfd("video", &r.mp4)?;
                let o = r.out;
                let reply = synaptixd::VideoReply { width: o.width, height: o.height, frames: o.frames, fps: o.fps, seed: r.seed, label: o.label, mp4: FdRef { fd: 0 }, bytes: r.mp4.len() };
                (data(reply), vec![fd])
            }
            Request::Music { model, prompt, lyrics, seconds, seed, steps } => {
                let sock = self.sock.as_fd();
                let id = self.current;
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let c2 = cancel.clone();
                let mut send = |step: usize, total: usize| {
                    let r = Reply { id, event: Some(synaptixd::Event::Progress { step, total }), ..Default::default() };
                    if wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_err() {
                        c2.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                };
                let r = gen::music(&mgr, &model, prompt, lyrics, seconds, seed, steps, cancel.clone(), &mut send)?;
                let fd = wire::memfd("music", &r.wav)?;
                let o = r.out;
                let seconds = o.samples.len() as f32 / (o.sample_rate.max(1) as f32 * o.channels.max(1) as f32);
                let reply = synaptixd::MusicReply { seconds, sample_rate: o.sample_rate, channels: o.channels, seed: r.seed, label: o.label, wav: FdRef { fd: 0 }, bytes: r.wav.len() };
                (data(reply), vec![fd])
            }
            Request::Embed { model, texts } => {
                let model = mgr.resolve_id(&model, ModelKind::Embed)?;
                let e = mgr.run(&model, |m| m.embed(&texts))?;
                (data(e), none())
            }
            Request::Transcribe { model, pcm, samples, sample_rate, language } => {
                let model = mgr.resolve_id(&model, ModelKind::Asr)?;
                let mut call = Params::new();
                if let Some(v) = language {
                    call.set("language", Some(v.into()));
                }
                let language = mgr.params(&model, &call).text("language").map(String::from);
                let pcm = pcm_from(&mut fds, pcm, samples)?;
                let t = mgr.run(&model, |m| m.transcribe(pcm.f32s(), sample_rate, language.as_deref()))?;
                (data(t), none())
            }
            Request::Speak { model, text, language, instruct, speed, steps, params, fresh } => {
                let model = mgr.resolve_id(&model, ModelKind::Tts)?;
                let mut call = params;
                if let Some(v) = language {
                    call.set("language", Some(v.into()));
                }
                if let Some(v) = instruct {
                    call.set("voice", Some(v.into()));
                }
                if let Some(v) = speed {
                    call.set("speed", Some(v.into()));
                }
                if let Some(v) = steps {
                    call.set("steps", Some((v as i64).into()));
                }
                let params = mgr.params(&model, &call);
                let (wav, sr, seed) = mgr.run(&model, |m| m.speak(&text, &params, fresh))?;
                let fd = wire::memfd_f32("speech", &wav)?;
                (data(synaptixd::SpeechReply { samples: wav.len(), sample_rate: sr, audio: FdRef { fd: 0 }, seed }), vec![fd])
            }
            Request::Generate { model, messages, max_tokens, temperature, params, tools } => {
                let model = mgr.resolve_id(&model, ModelKind::Llm)?;
                let mut call = params;
                if let Some(v) = max_tokens {
                    call.set("max_tokens", Some((v as i64).into()));
                }
                if let Some(v) = temperature {
                    call.set("temperature", Some(v.into()));
                }
                // Параметры запроса, а не настройки модели: схема их не знает.
                let request_only: Vec<(&str, synaptixd::ParamValue)> =
                    ["session", "stop", "max_image_tokens"].into_iter().filter_map(|k| call.get(k).cloned().map(|v| (k, v))).collect();
                let mut params = mgr.params(&model, &call);
                for (k, v) in request_only {
                    params.set(k, Some(v));
                }
                let sock = self.sock.as_fd();
                let id = self.current;
                // Отмена: клиент закрыл сокет (кнопка «Стоп») — ход обрывается и
                // на префилле, и между токенами.
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let _watch = {
                    let c = cancel.clone();
                    hangup::watch(sock.as_raw_fd(), move || c.store(true, std::sync::atomic::Ordering::SeqCst))
                };
                let send = |ev: synaptixd::Event| {
                    let r = Reply { id, event: Some(ev), ..Default::default() };
                    if wire::send(sock, &serde_json::to_vec(&r).unwrap_or_default(), &[]).is_err() {
                        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                };
                let g = mgr.run(&model, |m| m.generate(&messages, &params, &tools, &send, &cancel))?;
                (data(g), none())
            }
            Request::Tokenize { model, texts } => {
                let model = mgr.resolve_id(&model, ModelKind::Llm)?;
                let counts = mgr.run(&model, |m| m.tokenize(&texts))?;
                (data(synaptixd::TokenCounts { counts }), none())
            }
            Request::LlmInfo { model } => {
                let model = mgr.resolve_id(&model, ModelKind::Llm)?;
                let info = mgr.run(&model, |m| m.llm_info())?;
                (data(info), none())
            }
            Request::Flow { flow, inputs, files } => flow::serve(&mgr, self.sock.as_fd(), self.current, flow, inputs, files, fds.into_iter().flatten().collect())?,
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
