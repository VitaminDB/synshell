//! Федерация: модели спаренных устройств (synlink) в списке этой службы.
//!
//! Через локальный сокет synlink служба следит, какие спаренные устройства
//! на связи, открывает к их службам мост (`Request::Synaptix` synlink,
//! кадры [`synaptixd::bridge::Frame`]) и берёт их список моделей. Удалённые
//! модели видны с id `<устройство>:<id>` и `origin: remote`. Запрос к такой
//! модели (или к модели по умолчанию, если удалённое устройство
//! предпочтительнее) пересылается целиком: файлы запроса и входы-пути едут
//! байтами, ответы и события — обратно как есть. Связь пропала до ответа —
//! запрос выполняется здесь, если здесь есть подходящая модель.
//!
//! Программы ничего не меняют: тот же `Client::connect()`.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use synaptixd::bridge::{fd_bytes, Frame, BLOB_PATH};
use synaptixd::wire;
use synaptixd::{Envelope, Error, ErrorCode, Event, FederationDevice, FederationInfo, ModelEntry, ModelKind, Origin, Reply, Request};

use crate::config::{FedConfig, Prefer};
use crate::manager::Manager;

static FED: OnceLock<Arc<Fed>> = OnceLock::new();

pub fn get() -> Option<&'static Arc<Fed>> {
    FED.get()
}

pub fn init(mgr: Arc<Manager>, cfg: FedConfig, state_dir: PathBuf) {
    if !cfg.enabled {
        tracing::info!("федерация выключена");
        return;
    }
    let prefer_file = state_dir.join("federation-prefer");
    let prefer = std::fs::read_to_string(&prefer_file).ok().and_then(|s| Prefer::parse(s.trim())).unwrap_or(cfg.prefer);
    let f = Arc::new(Fed { cfg, prefer: Mutex::new(prefer), prefer_file, mgr, st: Mutex::new(State::default()) });
    if FED.set(f.clone()).is_ok() {
        let _ = std::thread::Builder::new().name("federation".into()).spawn(move || f.run());
    }
}

pub struct Fed {
    cfg: FedConfig,
    prefer: Mutex<Prefer>,
    prefer_file: PathBuf,
    mgr: Arc<Manager>,
    st: Mutex<State>,
}

#[derive(Default)]
struct State {
    sock: Option<PathBuf>,
    /// Вид этой машины по synlink (`desktop`, `phone`…).
    me_kind: String,
    /// По id synlink; отключённые остаются (их префикс узнаётся).
    peers: HashMap<String, Peer>,
}

#[derive(Clone)]
struct Peer {
    id: String,
    name: String,
    slug: String,
    kind: String,
    connected: bool,
    /// Модели той службы (id без префикса).
    models: Vec<ModelEntry>,
    /// Поколение соединения: поток устройства живёт, пока оно то же.
    epoch: u64,
}

/// Куда идёт запрос.
pub enum Route {
    Local,
    Remote(Target),
    Fail(Error),
}

#[derive(Clone)]
pub struct Target {
    pub peer: String,
    pub slug: String,
    pub name: String,
    /// Id модели на той стороне (пусто — её модель по умолчанию).
    pub model: String,
    /// Модель названа с префиксом устройства: без связи — ошибка, не резерв.
    pub explicit: bool,
}

/// Чем кончилась пересылка.
pub enum Forwarded {
    Done,
    /// Связь не сложилась до первого ответа: клиент ничего не получил.
    Link(String),
    /// Клиент ушёл.
    Gone,
}

fn slugify(name: &str, kind: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        kind.to_string()
    } else {
        s
    }
}

impl Prefer {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "auto" => Prefer::Auto,
            "remote" => Prefer::Remote,
            "local" => Prefer::Local,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Prefer::Auto => "auto",
            Prefer::Remote => "remote",
            Prefer::Local => "local",
        }
    }
}

fn is_big(kind: &str) -> bool {
    matches!(kind, "desktop" | "laptop")
}

/// Сокет synlink: из настроек, `SYNLINK_SOCKET`, иначе первый живой
/// `/run/user/<uid>/synlink.sock` (служба — root, synlink — у пользователя).
fn find_synlink(cfg: &FedConfig) -> Option<PathBuf> {
    if let Some(p) = &cfg.synlink_socket {
        return Some(p.clone());
    }
    if let Some(p) = std::env::var_os("SYNLINK_SOCKET") {
        return Some(PathBuf::from(p));
    }
    let mut dirs: Vec<(u32, PathBuf)> = std::fs::read_dir("/run/user")
        .ok()?
        .flatten()
        .filter_map(|e| Some((e.file_name().to_str()?.parse().ok()?, e.path().join("synlink.sock"))))
        .filter(|(_, p): &(u32, PathBuf)| p.exists())
        .collect();
    dirs.sort();
    dirs.into_iter().find(|(uid, _)| *uid >= 1000).map(|(_, p)| p)
}

/// Одно соединение с synlink: запрос строкой JSON, ответ строкой.
fn link_request(path: &Path, req: &Value) -> io::Result<Value> {
    let s = UnixStream::connect(path)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut w = s.try_clone()?;
    writeln!(w, "{req}")?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(io::Error::other)
}

/// Мост к службе устройства через synlink.
pub struct Bridge {
    r: BufReader<UnixStream>,
    w: UnixStream,
}

impl Bridge {
    pub fn open(sock: &Path, device: &str) -> io::Result<Self> {
        let s = UnixStream::connect(sock)?;
        s.set_read_timeout(Some(Duration::from_secs(15)))?;
        let mut w = s.try_clone()?;
        writeln!(w, "{}", json!({ "request": "synaptix", "device": device }))?;
        let mut r = BufReader::new(s);
        let mut line = String::new();
        r.read_line(&mut line)?;
        let v: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
        if v["response"] != "ok" {
            let msg = v["message"].as_str().unwrap_or("synlink не открыл мост").to_string();
            return Err(io::Error::new(io::ErrorKind::ConnectionRefused, msg));
        }
        r.get_ref().set_read_timeout(None)?;
        Ok(Self { r, w })
    }

    pub fn send(&mut self, f: &Frame) -> io::Result<()> {
        f.write_to(&mut self.w)
    }

    pub fn recv(&mut self) -> io::Result<Option<Frame>> {
        Frame::read_from(&mut self.r)
    }

    /// Копия сокета (оборвать мост из другого потока).
    fn handle(&self) -> io::Result<UnixStream> {
        self.w.try_clone()
    }

    /// Запрос без файлов → итоговый ответ (события пропускаются).
    fn call(&mut self, req: Request) -> io::Result<Reply> {
        let env = Envelope { id: 1, req };
        self.send(&Frame::new(serde_json::to_vec(&env)?))?;
        loop {
            let f = self.recv()?.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "мост закрыт"))?;
            let r: Reply = serde_json::from_slice(&f.json).map_err(io::Error::other)?;
            if r.ok.is_some() {
                return Ok(r);
            }
        }
    }
}

fn model_kind(req: &Request) -> Option<(&str, Option<ModelKind>)> {
    use Request as R;
    Some(match req {
        R::Open { model } | R::Load { model } => (model, None),
        R::Ctc { model, .. } => (model, Some(ModelKind::Ctc)),
        R::MfaAlign { model, .. } => (model, Some(ModelKind::Mfa)),
        R::Image { model, .. } => (model, Some(ModelKind::Image)),
        R::Video { model, .. } => (model, Some(ModelKind::Video)),
        R::Music { model, .. } => (model, Some(ModelKind::Music)),
        R::Embed { model, .. } => (model, Some(ModelKind::Embed)),
        R::Transcribe { model, .. } => (model, Some(ModelKind::Asr)),
        R::Speak { model, .. } => (model, Some(ModelKind::Tts)),
        R::Generate { model, .. } | R::Tokenize { model, .. } | R::LlmInfo { model } => (model, Some(ModelKind::Llm)),
        R::Rerank { model, .. } => (model, Some(ModelKind::Rerank)),
        R::Resolve { kind } => ("", Some(*kind)),
        _ => return None,
    })
}

impl Fed {
    fn run(self: Arc<Self>) {
        loop {
            if let Some(path) = find_synlink(&self.cfg) {
                self.st.lock().unwrap().sock = Some(path.clone());
                if let Err(e) = self.follow(&path) {
                    tracing::debug!("synlink ({}): {e}", path.display());
                }
            }
            self.lose_all();
            std::thread::sleep(Duration::from_secs(10));
        }
    }

    /// Подписка на synlink: состояние устройств.
    fn follow(self: &Arc<Self>, path: &Path) -> io::Result<()> {
        let s = UnixStream::connect(path)?;
        let mut w = s.try_clone()?;
        writeln!(w, "{}", json!({ "request": "subscribe" }))?;
        let r = BufReader::new(s);
        for line in r.lines() {
            let v: Value = match serde_json::from_str(&line?) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match v["event"].as_str() {
                Some("status") => self.apply_status(&v["status"]),
                Some("connected" | "disconnected" | "paired") => {
                    if let Ok(st) = link_request(path, &json!({ "request": "status" })) {
                        self.apply_status(&st["status"]);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn allowed(&self, id: &str, name: &str) -> bool {
        self.cfg.devices.is_empty() || self.cfg.devices.iter().any(|d| d == id || d.eq_ignore_ascii_case(name) || id.starts_with(d.as_str()))
    }

    fn apply_status(self: &Arc<Self>, status: &Value) {
        let mut spawn = Vec::new();
        let mut changed = false;
        {
            let mut st = self.st.lock().unwrap();
            st.me_kind = status["me"]["kind"].as_str().unwrap_or("desktop").to_string();
            let empty = Vec::new();
            let mut live = std::collections::HashSet::new();
            for p in status["peers"].as_array().unwrap_or(&empty) {
                let (Some(id), Some(name)) = (p["id"].as_str(), p["name"].as_str()) else { continue };
                if !p["paired"].as_bool().unwrap_or(false) || !self.allowed(id, name) {
                    continue;
                }
                let kind = p["kind"].as_str().unwrap_or("desktop").to_string();
                let connected = p["connected"].as_bool().unwrap_or(false);
                let mut slug = slugify(name, &kind);
                if st.peers.values().any(|o| o.id != id && o.slug == slug) {
                    slug = format!("{slug}-{}", &id[..id.len().min(4)]);
                }
                let e = st.peers.entry(id.to_string()).or_insert_with(|| Peer {
                    id: id.to_string(),
                    name: name.to_string(),
                    slug: slug.clone(),
                    kind: kind.clone(),
                    connected: false,
                    models: Vec::new(),
                    epoch: 0,
                });
                e.name = name.to_string();
                e.kind = kind;
                if connected {
                    live.insert(id.to_string());
                    if !e.connected {
                        e.connected = true;
                        e.epoch += 1;
                        spawn.push((id.to_string(), e.epoch));
                    }
                }
            }
            for p in st.peers.values_mut() {
                if p.connected && !live.contains(&p.id) {
                    p.connected = false;
                    p.epoch += 1;
                    changed |= !p.models.is_empty();
                    p.models.clear();
                }
            }
        }
        if changed {
            self.mgr.notify(Event::ModelsChanged);
        }
        for (id, epoch) in spawn {
            let me = self.clone();
            let _ = std::thread::Builder::new().name("federation-peer".into()).spawn(move || me.watch_peer(id, epoch));
        }
    }

    fn lose_all(&self) {
        let mut changed = false;
        {
            let mut st = self.st.lock().unwrap();
            st.sock = None;
            for p in st.peers.values_mut() {
                changed |= !p.models.is_empty();
                p.connected = false;
                p.epoch += 1;
                p.models.clear();
            }
        }
        if changed {
            self.mgr.notify(Event::ModelsChanged);
        }
    }

    fn current(&self, id: &str, epoch: u64) -> bool {
        self.st.lock().unwrap().peers.get(id).is_some_and(|p| p.connected && p.epoch == epoch)
    }

    fn sock(&self) -> Option<PathBuf> {
        self.st.lock().unwrap().sock.clone()
    }

    fn set_models(&self, id: &str, epoch: u64, models: Vec<ModelEntry>) {
        let changed = {
            let mut st = self.st.lock().unwrap();
            match st.peers.get_mut(id) {
                Some(p) if p.epoch == epoch && p.models != models => {
                    tracing::info!("федерация: {} — моделей {}", p.name, models.len());
                    p.models = models;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.mgr.notify(Event::ModelsChanged);
        }
    }

    fn fetch_list(&self, id: &str) -> io::Result<Vec<ModelEntry>> {
        let sock = self.sock().ok_or_else(|| io::Error::other("нет synlink"))?;
        let mut b = Bridge::open(&sock, id)?;
        let r = b.call(Request::List)?;
        if r.ok != Some(true) {
            return Err(io::Error::other(r.error.unwrap_or_default()));
        }
        serde_json::from_value(r.data).map_err(io::Error::other)
    }

    /// Пока устройство на связи: его список и подписка на изменения.
    fn watch_peer(self: Arc<Self>, id: String, epoch: u64) {
        while self.current(&id, epoch) {
            match self.fetch_list(&id) {
                Ok(list) => self.set_models(&id, epoch, list),
                Err(e) => {
                    tracing::debug!("федерация {id}: список: {e}");
                    std::thread::sleep(Duration::from_secs(20));
                    continue;
                }
            }
            // Подписка: список той службы изменился — взять снова.
            let sub = self.sock().ok_or_else(|| io::Error::other("нет synlink")).and_then(|s| Bridge::open(&s, &id));
            let Ok(mut b) = sub else {
                std::thread::sleep(Duration::from_secs(20));
                continue;
            };
            if b.send(&Frame::new(serde_json::to_vec(&Envelope { id: 1, req: Request::Subscribe }).unwrap_or_default())).is_err() {
                continue;
            }
            loop {
                let Ok(Some(f)) = b.recv() else { break };
                let Ok(r) = serde_json::from_slice::<Reply>(&f.json) else { continue };
                if matches!(r.event, Some(Event::ModelsChanged)) {
                    if let Ok(list) = self.fetch_list(&id) {
                        self.set_models(&id, epoch, list);
                    }
                }
                if !self.current(&id, epoch) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    }

    /// Удалённые модели для списка службы.
    pub fn entries(&self) -> Vec<ModelEntry> {
        let st = self.st.lock().unwrap();
        let mut out = Vec::new();
        for p in st.peers.values().filter(|p| p.connected) {
            for m in &p.models {
                let mut e = m.clone();
                e.id = format!("{}:{}", p.slug, m.id);
                e.origin = Origin::Remote { device: p.slug.clone(), name: p.name.clone() };
                out.push(e);
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn info(&self) -> FederationInfo {
        let st = self.st.lock().unwrap();
        let mut devices: Vec<FederationDevice> = st
            .peers
            .values()
            .map(|p| FederationDevice { device: p.slug.clone(), name: p.name.clone(), kind: p.kind.clone(), connected: p.connected, models: p.models.len() as u32 })
            .collect();
        devices.sort_by(|a, b| a.device.cmp(&b.device));
        FederationInfo { enabled: true, prefer: self.prefer.lock().unwrap().id().into(), synlink: st.sock.is_some(), devices }
    }

    pub fn set_prefer(&self, p: &str) -> Result<(), Error> {
        let p = Prefer::parse(p).ok_or_else(|| Error::new(ErrorCode::Protocol, format!("prefer: auto | remote | local, не «{p}»")))?;
        *self.prefer.lock().unwrap() = p;
        if let Some(d) = self.prefer_file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(&self.prefer_file, p.id());
        self.mgr.notify(Event::ModelsChanged);
        Ok(())
    }

    fn prefers_remote(&self, me: &str, peer: &Peer) -> bool {
        match *self.prefer.lock().unwrap() {
            Prefer::Remote => true,
            Prefer::Local => false,
            // Слабая машина отдаёт работу сильной.
            Prefer::Auto => !is_big(me) && is_big(&peer.kind),
        }
    }

    fn target(p: &Peer, model: &str, explicit: bool) -> Target {
        Target { peer: p.id.clone(), slug: p.slug.clone(), name: p.name.clone(), model: model.to_string(), explicit }
    }

    /// Куда направить запрос программы.
    pub fn route(&self, req: &Request) -> Route {
        let Some((model, kind)) = model_kind(req) else { return Route::Local };
        let model = model.trim();
        if model.starts_with('/') {
            return Route::Local;
        }
        let st = self.st.lock().unwrap();
        // Сильные устройства — первыми.
        let mut peers: Vec<&Peer> = st.peers.values().collect();
        peers.sort_by_key(|p| (!is_big(&p.kind), p.slug.clone()));
        // `<устройство>:<id>`
        if let Some((slug, rest)) = model.split_once(':') {
            if let Some(p) = peers.iter().find(|p| p.slug == slug) {
                if !p.connected {
                    return Route::Fail(Error::new(ErrorCode::Unavailable, format!("устройство «{}» не на связи", p.name)));
                }
                return Route::Remote(Self::target(p, rest, true));
            }
        }
        let live = || peers.iter().filter(|p| p.connected);
        let has = |p: &Peer| -> bool {
            p.models.iter().any(|m| match model {
                "" | "auto" | "default" => kind.is_some_and(|k| m.kind == k),
                id => m.id == id,
            })
        };
        let local = match model {
            "" | "auto" | "default" => kind.is_some_and(|k| self.mgr.resolve(k).is_ok()),
            id => self.mgr.entry(id).is_ok(),
        };
        let me = st.me_kind.as_str();
        if let Some(p) = live().find(|p| self.prefers_remote(me, p) && has(p)) {
            return Route::Remote(Self::target(p, model, false));
        }
        if local {
            return Route::Local;
        }
        match live().find(|p| has(p)) {
            Some(p) => Route::Remote(Self::target(p, model, false)),
            None => Route::Local,
        }
    }

    /// Переслать запрос на устройство и передать программе ответы и события.
    #[allow(clippy::too_many_arguments)]
    pub fn forward(
        &self,
        bridges: &mut HashMap<String, Bridge>,
        client: BorrowedFd,
        cred: Option<wire::Peer>,
        id: u64,
        req: &Request,
        fds: &[OwnedFd],
        to: &Target,
    ) -> Forwarded {
        let patch_entry = matches!(req, Request::Load { .. } | Request::Resolve { .. });
        let frame = match build_frame(id, req, fds, to, cred) {
            Ok(f) => f,
            Err(e) => return self.reply_err(client, id, ErrorCode::Protocol, e.to_string()),
        };
        let Some(sock) = self.sock() else { return Forwarded::Link("synlink не запущен".into()) };
        // Мост живёт всё соединение программы; оборвался — один новый.
        let mut tries = 0;
        let bridge = loop {
            tries += 1;
            let ok = match bridges.get_mut(&to.peer) {
                Some(b) => b.send(&frame).is_ok(),
                None => match Bridge::open(&sock, &to.peer) {
                    Ok(mut b) => {
                        let sent = b.send(&frame).is_ok();
                        bridges.insert(to.peer.clone(), b);
                        sent
                    }
                    Err(e) => return Forwarded::Link(format!("{}: {e}", to.name)),
                },
            };
            if ok {
                break bridges.get_mut(&to.peer).unwrap();
            }
            bridges.remove(&to.peer);
            if tries >= 2 {
                return Forwarded::Link(format!("{}: мост оборвался", to.name));
            }
        };
        // Программа закрыла сокет (отмена) — оборвать мост: та служба
        // увидит закрытие и остановит работу.
        let _watch = bridge.handle().ok().map(|h| {
            crate::hangup::watch(client.as_raw_fd(), move || {
                let _ = h.shutdown(std::net::Shutdown::Both);
            })
        });
        let mut sent = false;
        loop {
            let f = match bridge.recv() {
                Ok(Some(f)) => f,
                other => {
                    bridges.remove(&to.peer);
                    let why = match other {
                        Err(e) => e.to_string(),
                        _ => "мост закрыт".into(),
                    };
                    if !sent {
                        return Forwarded::Link(format!("{}: {why}", to.name));
                    }
                    return self.reply_err(client, id, ErrorCode::Unavailable, format!("связь с «{}» прервалась: {why}", to.name));
                }
            };
            let mut json = f.json;
            let mut fin = false;
            if let Ok(mut r) = serde_json::from_slice::<Reply>(&json) {
                fin = r.ok.is_some();
                if fin && patch_entry && r.ok == Some(true) && r.data.get("id").is_some() {
                    if let Ok(mut e) = serde_json::from_value::<ModelEntry>(r.data.clone()) {
                        e.id = format!("{}:{}", to.slug, e.id);
                        e.origin = Origin::Remote { device: to.slug.clone(), name: to.name.clone() };
                        r.data = serde_json::to_value(e).unwrap_or(Value::Null);
                        json = serde_json::to_vec(&r).unwrap_or(json);
                    }
                }
            }
            let out = Frame { json, blobs: f.blobs };
            if out.send_to(client).is_err() {
                return Forwarded::Gone;
            }
            sent = true;
            if fin {
                return Forwarded::Done;
            }
        }
    }

    fn reply_err(&self, client: BorrowedFd, id: u64, code: ErrorCode, msg: String) -> Forwarded {
        let r = Reply { id, ok: Some(false), error: Some(msg), code: Some(code), ..Default::default() };
        match wire::send(client, &serde_json::to_vec(&r).unwrap_or_default(), &[]) {
            Ok(()) => Forwarded::Done,
            Err(_) => Forwarded::Gone,
        }
    }
}

/// Прочитать файл с правами программы-клиента.
fn read_as(path: &str, cred: Option<wire::Peer>) -> io::Result<Vec<u8>> {
    match cred {
        Some(c) => crate::as_peer(c.uid, c.gid, || std::fs::read(path)),
        None => std::fs::read(path),
    }
    .map_err(|e| io::Error::new(e.kind(), format!("{path}: {e}")))
}

/// Входной путь → файл в кадре (`@blob:<n>/<имя>`).
fn embed(v: &mut Value, blobs: &mut Vec<Vec<u8>>, cred: Option<wire::Peer>) -> io::Result<()> {
    let Some(p) = v.as_str() else { return Ok(()) };
    if p.is_empty() || p.starts_with(BLOB_PATH) {
        return Ok(());
    }
    let data = read_as(p, cred)?;
    let name = Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    *v = Value::String(format!("{BLOB_PATH}{}/{name}", blobs.len()));
    blobs.push(data);
    Ok(())
}

fn build_frame(id: u64, req: &Request, fds: &[OwnedFd], to: &Target, cred: Option<wire::Peer>) -> io::Result<Frame> {
    let mut v = serde_json::to_value(Envelope { id, req: req.clone() })?;
    if v.get("model").is_some() {
        v["model"] = Value::String(to.model.clone());
    }
    let mut blobs: Vec<Vec<u8>> = fds.iter().map(|f| fd_bytes(f.as_fd())).collect::<io::Result<_>>()?;
    match v["op"].as_str().unwrap_or("") {
        "generate" => {
            if let Some(msgs) = v["messages"].as_array_mut() {
                for m in msgs {
                    if let Some(media) = m["media"].as_array_mut() {
                        for x in media {
                            embed(&mut x["path"], &mut blobs, cred)?;
                        }
                    }
                }
            }
        }
        "image" => {
            if let Some(a) = v["images"].as_array_mut() {
                for x in a {
                    embed(x, &mut blobs, cred)?;
                }
            }
            embed(&mut v["init"], &mut blobs, cred)?;
        }
        "video" => {
            for k in ["first_frame", "last_frame", "audio", "source_video", "control_video"] {
                embed(&mut v[k], &mut blobs, cred)?;
            }
            if let Some(a) = v["refs"].as_array_mut() {
                for x in a {
                    embed(x, &mut blobs, cred)?;
                }
            }
        }
        _ => {}
    }
    Ok(Frame { json: serde_json::to_vec(&v)?, blobs })
}

/// На стороне устройства: `@blob:<n>/<имя>` в запросе — файлы во временном
/// каталоге (служба читает входы по путям). Возвращает созданные файлы.
pub fn materialize(v: &mut Value, fds: &[OwnedFd], dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut made = Vec::new();
    walk(v, &mut |s: &mut String| -> io::Result<()> {
        let Some(rest) = s.strip_prefix(BLOB_PATH) else { return Ok(()) };
        let Some((n, name)) = rest.split_once('/') else { return Ok(()) };
        let Some(fd) = n.parse::<usize>().ok().and_then(|n| fds.get(n)) else { return Ok(()) };
        std::fs::create_dir_all(dir)?;
        let name: String = name.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let k = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = dir.join(format!("{}-{k}-{name}", std::process::id()));
        std::fs::write(&path, fd_bytes(fd.as_fd())?)?;
        *s = path.to_string_lossy().into_owned();
        made.push(path);
        Ok(())
    })?;
    Ok(made)
}

fn walk(v: &mut Value, f: &mut dyn FnMut(&mut String) -> io::Result<()>) -> io::Result<()> {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter_mut().try_for_each(|x| walk(x, f)),
        Value::Object(o) => o.values_mut().try_for_each(|x| walk(x, f)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Мой Desktop", "desktop"), "мой-desktop");
        assert_eq!(slugify("archlinux", "desktop"), "archlinux");
        assert_eq!(slugify("  ", "phone"), "phone");
    }

    #[test]
    fn blobs_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("cat.png");
        std::fs::write(&src, b"PNGDATA").unwrap();
        let req = Request::Image {
            model: "desktop:flux".into(),
            prompt: "x".into(),
            negative: None,
            width: None,
            height: None,
            steps: None,
            guidance: None,
            seed: None,
            images: vec![src.to_string_lossy().into_owned()],
            init: None,
            strength: None,
        };
        let to = Target { peer: "p".into(), slug: "desktop".into(), name: "D".into(), model: "flux".into(), explicit: true };
        let f = build_frame(5, &req, &[], &to, None).unwrap();
        let mut v: Value = serde_json::from_slice(&f.json).unwrap();
        assert_eq!(v["model"], "flux");
        assert_eq!(v["images"][0], "@blob:0/cat.png");
        let fds = f.memfds().unwrap();
        let out = dir.path().join("in");
        let made = materialize(&mut v, &fds, &out).unwrap();
        assert_eq!(made.len(), 1);
        let p = v["images"][0].as_str().unwrap();
        assert!(p.ends_with("cat.png"));
        assert_eq!(std::fs::read(p).unwrap(), b"PNGDATA");
    }
}
