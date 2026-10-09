//! Демон: сеансы с устройствами, спаривание, состояние для оболочки.
//!
//! Соединяемся сами только со спаренными и с устройствами на кабеле USB;
//! чужие в Wi-Fi видны как «рядом», спаривание с ними начинает
//! пользователь. Из двух устройств соединение начинает то, у которого id
//! меньше, — чтобы не было встречных дублей. По кабелю и по Wi-Fi сразу
//! остаётся одно соединение — по USB.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use synshell_common::link::{
    Battery, DeviceKind, Event, PairPrompt, PeerInfo, RemoteNotification, SelfInfo, Status, Transport, UsbInfo,
};
use tokio::sync::{broadcast, mpsc, oneshot, Notify};

use crate::discovery::Announce;
use crate::identity::{Identity, Peers, Trusted};
use crate::proto::{self, Ctl, Hello, Note};
use synshell_tr::{n_, t};

pub struct Session {
    pub conn: quinn::Connection,
    pub transport: Transport,
    pub addr: SocketAddr,
    pub hello: Hello,
    pub ctl: mpsc::UnboundedSender<Ctl>,
    pub battery: Option<(u8, bool)>,
    pub gen: u64,
}

struct Nearby {
    name: String,
    kind: DeviceKind,
    addr: SocketAddr,
    transport: Transport,
    seen: Instant,
    /// С какого момента слышно без перерыва (дольше 20 с).
    since: Instant,
}

#[derive(Default)]
struct St {
    peers: Peers,
    sessions: HashMap<String, Session>,
    nearby: HashMap<String, Nearby>,
    prompts: Vec<PairPrompt>,
    waiters: HashMap<String, oneshot::Sender<bool>>,
    /// Спаривание начал пользователь (`synlink pair`).
    intents: HashSet<String>,
    /// Идёт исходящее соединение: когда начато и номер попытки (снимает запись только своя попытка).
    connecting: HashMap<String, (Instant, u64)>,
    connect_seq: u64,
    /// Последняя ошибка исходящего соединения — в журнал пишется только новая, видна в `synlink status`.
    connect_err: HashMap<String, String>,
    /// Спаривание идёт (второе соединение ждёт или отбрасывается).
    pairing: HashSet<String>,
    /// Неудачное спаривание: сами не соединяемся какое-то время.
    backoff: HashMap<String, Instant>,
    notes: VecDeque<RemoteNotification>,
    usb: UsbInfo,
    mounts: HashMap<String, String>,
    name: String,
    discoverable: bool,
    gen: u64,
}

pub struct Daemon {
    pub id: Identity,
    pub port: u16,
    pub ep: quinn::Endpoint,
    pub kind: DeviceKind,
    pub hello_base: Hello,
    st: Mutex<St>,
    pub events: broadcast::Sender<Event>,
    pub announce_now: Notify,
    pub notes_tx: mpsc::UnboundedSender<(String, Note)>,
    /// `[link] notifications` / `auto_mount` — меняются на лету (config.toml).
    pub notifications: std::sync::atomic::AtomicBool,
    pub auto_mount: std::sync::atomic::AtomicBool,
    /// `[link] clipboard` — общий буфер обмена.
    pub clipboard: std::sync::atomic::AtomicBool,
    /// Поток буфера обмена этой машины (`clipboard.rs`).
    pub clip: std::sync::OnceLock<crate::clipboard::Handle>,
    /// Телефон с погашенным экраном засыпает (`/run/syn-sleep/screen-off`, synmobile syn-sleepd):
    /// по Wi-Fi не анонсируемся и сеансов не держим — иначе keepalive QUIC с компьютера будил его
    /// каждые 3 с, а каждый анонс после самопробуждения приводил к новому соединению и повторам.
    pub sleeping: std::sync::atomic::AtomicBool,
    /// Звук трансляции, включённый отсюда (`audio.rs`).
    pub audio: crate::audio::Sessions,
}

/// Исходящая попытка без сеанса дольше этого считается зависшей.
const CONNECT_STALE: Duration = Duration::from_secs(30);

const SLEEP_FLAG: &str = "/run/syn-sleep/screen-off";
const SLEEP_INHIBIT: &str = "/run/syn-sleep/inhibit.d";

/// Телефон вот-вот уснёт — те же условия, что у syn-sleepd: экран погашен (флаг), прошла задержка «Глубокий сон
/// после блокировки» (вторая строка флага, с; не меньше 3 с) и нет запретов в `inhibit.d`. Сам флаг ещё не сон:
/// с задержкой в час связь по Wi-Fi рвалась сразу при блокировке.
fn sleep_due() -> bool {
    let Ok(meta) = std::fs::metadata(SLEEP_FLAG) else { return false };
    let delay = std::fs::read_to_string(SLEEP_FLAG)
        .ok()
        .and_then(|t| t.lines().nth(1).and_then(|l| l.trim().parse::<u64>().ok()))
        .unwrap_or(0)
        .max(3);
    let age = meta.modified().ok().and_then(|t| t.elapsed().ok()).unwrap_or_default();
    let inhibited = std::fs::read_dir(SLEEP_INHIBIT).is_ok_and(|mut d| d.next().is_some());
    age >= Duration::from_secs(delay) && !inhibited
}

pub type D = Arc<Daemon>;

impl Daemon {
    pub fn new(id: Identity, cfg: synshell_common::config::Config) -> Result<(D, mpsc::UnboundedReceiver<(String, Note)>)> {
        let link = cfg.link.clone();
        let ep = crate::net::endpoint(&id, link.port)?;
        let kind = crate::identity::device_kind(&cfg);
        let name = if link.name.trim().is_empty() { crate::identity::default_name() } else { link.name.trim().to_string() };
        let (user, uid, home) = crate::ssh::whoami();
        let hello_base = Hello {
            proto: proto::PROTO,
            id: id.id.clone(),
            name: name.clone(),
            kind,
            user,
            uid,
            home,
            ssh_key: crate::ssh::ensure_key().map_err(|e| tracing::warn!(?e, "ключ ssh")).ok(),
            ssh_host_key: crate::ssh::host_key(),
            usb_gadget: crate::netif::is_gadget(),
        };
        let (events, _) = broadcast::channel(256);
        let (notes_tx, notes_rx) = mpsc::unbounded_channel();
        let st = St { peers: Peers::load(), name, discoverable: link.discoverable, ..Default::default() };
        let d = Arc::new(Self {
            id,
            port: link.port,
            ep,
            kind,
            hello_base,
            st: Mutex::new(st),
            events,
            announce_now: Notify::new(),
            notes_tx,
            notifications: std::sync::atomic::AtomicBool::new(link.notifications),
            auto_mount: std::sync::atomic::AtomicBool::new(link.auto_mount),
            clipboard: std::sync::atomic::AtomicBool::new(link.clipboard),
            clip: std::sync::OnceLock::new(),
            sleeping: std::sync::atomic::AtomicBool::new(false),
            audio: Default::default(),
        });
        Ok((d, notes_rx))
    }

    pub fn self_info(&self) -> SelfInfo {
        let st = self.st.lock().unwrap();
        SelfInfo { id: self.id.id.clone(), name: st.name.clone(), kind: self.kind, discoverable: st.discoverable }
    }

    fn hello(&self) -> Hello {
        let mut h = self.hello_base.clone();
        h.name = self.st.lock().unwrap().name.clone();
        h
    }

    // ─── состояние ─────────────────────────────────────────────────────

    pub fn status(&self) -> Status {
        let audio: HashSet<String> = self.audio.lock().unwrap().keys().cloned().collect();
        let st = self.st.lock().unwrap();
        let mut peers: Vec<PeerInfo> = Vec::new();
        for t in st.peers.peers.values() {
            let s = st.sessions.get(&t.id);
            let n = st.nearby.get(&t.id);
            peers.push(PeerInfo {
                id: t.id.clone(),
                name: s.map(|s| s.hello.name.clone()).unwrap_or_else(|| t.name.clone()),
                kind: t.kind,
                paired: true,
                connected: s.is_some(),
                transport: s.map(|s| s.transport).or(n.map(|n| n.transport)),
                address: s.map(|s| s.addr.ip().to_string()).or(n.map(|n| n.addr.ip().to_string())),
                rtt_ms: s.map(|s| s.conn.rtt().as_secs_f32() * 1000.0),
                battery: s.and_then(|s| s.battery).map(|(percent, charging)| Battery { percent, charging }),
                user: Some(t.user.clone()),
                home: Some(t.home.clone()),
                mount: st.mounts.get(&t.id).cloned(),
                ssh_host: Some(crate::ssh::host_alias(t)),
                last_seen: t.last_seen,
                audio: audio.contains(&t.id),
                heard_ago: n.map(|n| n.seen.elapsed().as_secs() as u32),
                link_note: if s.is_some() { None } else { Self::link_note(&st, &t.id) },
            });
        }
        for (id, n) in &st.nearby {
            if st.peers.peers.contains_key(id) || n.seen.elapsed() > Duration::from_secs(20) {
                continue;
            }
            let s = st.sessions.get(id);
            peers.push(PeerInfo {
                id: id.clone(),
                name: n.name.clone(),
                kind: n.kind,
                paired: false,
                connected: false,
                transport: Some(s.map(|s| s.transport).unwrap_or(n.transport)),
                address: Some(n.addr.ip().to_string()),
                ..Default::default()
            });
        }
        // Соединённые — первыми, потом спаренные, потом рядом.
        peers.sort_by_key(|p| (!p.connected, !p.paired, p.name.to_lowercase()));
        Status { me: SelfInfo { id: self.id.id.clone(), name: st.name.clone(), kind: self.kind, discoverable: st.discoverable }, usb: st.usb.clone(), peers, prompts: st.prompts.clone() }
    }

    fn link_note(st: &St, id: &str) -> Option<String> {
        if st.pairing.contains(id) {
            return Some(t!("идёт спаривание").into());
        }
        if let Some((t, _)) = st.connecting.get(id) {
            return Some(t!("соединяюсь {v} с", v = t.elapsed().as_secs()));
        }
        if let Some(t) = st.backoff.get(id).filter(|t| t.elapsed() < Duration::from_secs(60)) {
            return Some(t!("пауза после неудачного спаривания ещё {v} с", v = 60 - t.elapsed().as_secs()));
        }
        st.connect_err.get(id).cloned()
    }

    pub fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    pub fn emit_status(&self) {
        self.emit(Event::Status { status: self.status() });
    }

    pub fn peer_info(&self, id: &str) -> Option<PeerInfo> {
        self.status().peers.into_iter().find(|p| p.id == id)
    }

    /// Устройство по id, имени или началу id → id.
    pub fn resolve(&self, key: &str) -> Option<String> {
        let st = self.st.lock().unwrap();
        if let Some(t) = st.peers.find(key) {
            return Some(t.id.clone());
        }
        if st.sessions.contains_key(key) || st.nearby.contains_key(key) {
            return Some(key.to_string());
        }
        let lower = key.to_lowercase();
        if let Some((id, _)) = st.nearby.iter().find(|(id, n)| id.starts_with(key) || n.name.to_lowercase() == lower) {
            return Some(id.clone());
        }
        // phone / desktop / laptop / tablet — если такое рядом одно.
        let kind: DeviceKind = serde_json::from_value(serde_json::Value::String(lower)).ok()?;
        let mut it = st.nearby.iter().filter(|(_, n)| n.kind == kind);
        let first = it.next().map(|(id, _)| id.clone());
        if it.next().is_some() {
            None
        } else {
            first
        }
    }

    /// Соединение с устройством (спаренным и соединённым).
    pub fn conn(&self, id: &str) -> Option<(quinn::Connection, Trusted)> {
        let st = self.st.lock().unwrap();
        let t = st.peers.peers.get(id)?.clone();
        let s = st.sessions.get(id)?;
        Some((s.conn.clone(), t))
    }

    /// Версия протокола и способ связи соединённого устройства.
    pub fn session_info(&self, id: &str) -> Option<(u32, synshell_common::link::Transport)> {
        let st = self.st.lock().unwrap();
        st.sessions.get(id).map(|s| (s.hello.proto, s.transport))
    }

    pub fn trusted(&self, id: &str) -> Option<Trusted> {
        self.st.lock().unwrap().peers.peers.get(id).cloned()
    }

    pub fn trusted_all(&self) -> Vec<Trusted> {
        self.st.lock().unwrap().peers.peers.values().cloned().collect()
    }

    pub fn set_mount(&self, id: &str, path: Option<String>) {
        {
            let mut st = self.st.lock().unwrap();
            match path {
                Some(p) => st.mounts.insert(id.to_string(), p),
                None => st.mounts.remove(id),
            };
        }
        self.emit_status();
    }

    pub fn mount_of(&self, id: &str) -> Option<String> {
        self.st.lock().unwrap().mounts.get(id).cloned()
    }

    pub fn configure(&self, name: Option<String>, discoverable: Option<bool>) {
        {
            let mut st = self.st.lock().unwrap();
            if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
                st.name = n.trim().to_string();
            }
            if let Some(v) = discoverable {
                st.discoverable = v;
            }
        }
        self.announce_now.notify_one();
        self.emit_status();
    }

    pub fn push_note(&self, n: RemoteNotification) {
        let mut st = self.st.lock().unwrap();
        st.notes.push_front(n);
        st.notes.truncate(200);
    }

    pub fn notes(&self, device: Option<&str>) -> Vec<RemoteNotification> {
        let st = self.st.lock().unwrap();
        st.notes.iter().filter(|n| device.is_none_or(|d| n.device == d)).cloned().collect()
    }

    /// Разослать управляющее сообщение всем соединённым спаренным.
    pub fn broadcast_ctl(&self, msg: Ctl) {
        let st = self.st.lock().unwrap();
        for s in st.sessions.values() {
            if st.peers.peers.contains_key(&s.hello.id) {
                let _ = s.ctl.send(msg.clone());
            }
        }
    }

    /// Текст буфера обмена — всем соединённым, кто его понимает (кроме `except`).
    pub fn share_clipboard(&self, text: String, secret: bool, except: Option<&str>) {
        if !self.clipboard.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let st = self.st.lock().unwrap();
        for s in st.sessions.values() {
            if s.hello.proto >= proto::PROTO_CLIPBOARD && st.peers.peers.contains_key(&s.hello.id) && Some(s.hello.id.as_str()) != except {
                let _ = s.ctl.send(Ctl::Clipboard { text: text.clone(), secret });
            }
        }
    }

    // ─── поиск и соединение ────────────────────────────────────────────

    /// Услышали анонс.
    pub async fn heard(self: D, a: Announce, from: SocketAddr) {
        let ifaces = crate::netif::list();
        let transport = crate::netif::transport_for(from.ip(), &ifaces);
        let addr = SocketAddr::new(from.ip(), a.port);
        let (connect, fresh) = {
            let mut st = self.st.lock().unwrap();
            let fresh = st.nearby.get(&a.id).is_none_or(|n| n.seen.elapsed() > Duration::from_secs(20));
            // Уже видели по USB — Wi-Fi-анонс не перебивает транспорт.
            // (Пока кабель есть и USB-анонсы свежие; время не продлеваем —
            // пропадут USB-анонсы, запись перейдёт на Wi-Fi.)
            let cable = st.usb.cable;
            let keep_usb = st.nearby.get(&a.id).is_some_and(|n| {
                cable && n.transport == Transport::Usb && transport == Transport::Wifi && n.seen.elapsed() < Duration::from_secs(10)
            });
            if !keep_usb {
                let since = st.nearby.get(&a.id).filter(|_| !fresh).map_or_else(Instant::now, |n| n.since);
                st.nearby.insert(a.id.clone(), Nearby { name: a.name.clone(), kind: a.kind, addr, transport, seen: Instant::now(), since });
            }
            // Устройство переименовалось (Параметры, имя модели при загрузке) — имя в анонсе новее, чем в
            // приветствии открытого сеанса и в списке спаренных.
            let mut renamed = false;
            if let Some(t) = st.peers.peers.get_mut(&a.id).filter(|t| t.name != a.name) {
                t.name = a.name.clone();
                renamed = true;
            }
            if renamed {
                st.peers.save();
            }
            if let Some(s) = st.sessions.get_mut(&a.id).filter(|s| s.hello.name != a.name) {
                s.hello.name = a.name.clone();
                renamed = true;
            }
            let fresh = fresh || renamed;
            let trusted = st.peers.peers.contains_key(&a.id);
            let session = st.sessions.get(&a.id);
            let want = match session {
                Some(s) => s.transport == Transport::Wifi && transport == Transport::Usb,
                None => trusted || transport == Transport::Usb,
            };
            // Начинает сторона с меньшим id; если та молчит (слышим давно, сеанса нет) — начинаем сами.
            let initiator = self.id.id < a.id || trusted && st.nearby.get(&a.id).is_some_and(|n| n.since.elapsed() > Duration::from_secs(20));
            let cooling = st.backoff.get(&a.id).is_some_and(|t| t.elapsed() < Duration::from_secs(60));
            let asleep = transport == Transport::Wifi && self.sleeping.load(std::sync::atomic::Ordering::Relaxed);
            // Попытка дольше CONNECT_STALE без сеанса — зависла (не должна: все шаги до сеанса с тайм-аутами), но
            // раньше такая запись навсегда запирала соединение до перезапуска демона. Спаривание ждёт человека — не трогаем.
            let busy = st.connecting.get(&a.id).is_some_and(|(t, _)| t.elapsed() < CONNECT_STALE) || st.pairing.contains(&a.id);
            (want && initiator && !cooling && !asleep && !busy, fresh)
        };
        if fresh {
            tracing::info!(peer = %a.name, id = %a.id, %addr, transport = transport.title(), "устройство слышно");
            self.emit_status();
        }
        if connect {
            tokio::spawn(self.clone().connect(a.id, addr));
        }
    }

    pub async fn connect(self: D, id: String, addr: SocketAddr) {
        let seq = {
            let mut st = self.st.lock().unwrap();
            if st.connecting.get(&id).is_some_and(|(t, _)| t.elapsed() < CONNECT_STALE) || st.pairing.contains(&id) {
                return;
            }
            if let Some((t, _)) = st.connecting.get(&id) {
                tracing::warn!(id, secs = t.elapsed().as_secs(), "прежняя попытка соединения зависла — начинаю новую");
            }
            st.connect_seq += 1;
            let seq = st.connect_seq;
            st.connecting.insert(id.clone(), (Instant::now(), seq));
            seq
        };
        let res = async {
            let conn = self.ep.connect(addr, "synlink")?;
            let conn = tokio::time::timeout(Duration::from_secs(6), conn).await.context("тайм-аут")??;
            Ok::<_, anyhow::Error>(conn)
        }
        .await;
        match res {
            Ok(conn) => {
                if let Err(e) = self.clone().run_session(conn, true).await {
                    tracing::info!(%addr, "сеанс: {e:#}");
                    self.connect_failed(&id, t!("сеанс: {e}", e = format!("{:#}", e)));
                }
            }
            Err(e) => self.connect_failed(&id, t!("соединение с {addr}: {e}", addr = addr, e = format!("{:#}", e))),
        }
        let mut st = self.st.lock().unwrap();
        if st.connecting.get(&id).is_some_and(|(_, s)| *s == seq) {
            st.connecting.remove(&id);
        }
    }

    /// Неудачная попытка: в журнал — только когда ошибка сменилась (анонсы идут каждые 4 с).
    fn connect_failed(&self, id: &str, err: String) {
        let mut st = self.st.lock().unwrap();
        if st.connect_err.get(id) != Some(&err) {
            tracing::info!(id, "не соединилось: {err}");
            st.connect_err.insert(id.to_string(), err);
        }
    }

    pub async fn accept_loop(self: D) {
        while let Some(inc) = self.ep.accept().await {
            if crate::netif::usb_disabled() && crate::netif::in_usb_subnet(inc.remote_address().ip()) {
                inc.refuse();
                continue;
            }
            // Спим (экран погашен): по Wi-Fi никого не принимаем — иначе компьютер будит нас повторами
            if self.sleeping.load(std::sync::atomic::Ordering::Relaxed) && !crate::netif::in_usb_subnet(inc.remote_address().ip()) {
                inc.refuse();
                continue;
            }
            let d = self.clone();
            tokio::spawn(async move {
                match inc.await {
                    Ok(conn) => {
                        let addr = conn.remote_address();
                        if let Err(e) = d.run_session(conn, false).await {
                            tracing::info!(%addr, "входящий сеанс: {e:#}");
                        }
                    }
                    Err(e) => tracing::debug!("входящее соединение: {e}"),
                }
            });
        }
    }

    /// Пользователь начал спаривание с устройством рядом.
    pub async fn pair(self: D, id: String) -> Result<()> {
        let addr = {
            let mut st = self.st.lock().unwrap();
            if st.peers.peers.contains_key(&id) {
                bail!("уже спарено");
            }
            st.intents.insert(id.clone());
            st.backoff.remove(&id);
            st.nearby.get(&id).map(|n| n.addr).context("устройство не найдено рядом")?
        };
        tokio::spawn(self.clone().connect(id, addr));
        Ok(())
    }

    pub fn pair_reply(&self, id: &str, accept: bool) -> Result<()> {
        let tx = self.st.lock().unwrap().waiters.remove(id).context("нет запроса спаривания")?;
        let _ = tx.send(accept);
        Ok(())
    }

    pub fn unpair(&self, id: &str) -> Result<()> {
        let (t, sess) = {
            let mut st = self.st.lock().unwrap();
            let t = st.peers.peers.remove(id).context("не спарено")?;
            st.peers.save();
            (t, st.sessions.remove(id))
        };
        crate::ssh::unauthorize(&t.id);
        crate::ssh::write_config(&self.trusted_all());
        if let Some(s) = sess {
            let _ = s.ctl.send(Ctl::Forget);
            let conn = s.conn.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                conn.close(0u32.into(), b"unpair");
            });
        }
        crate::fuse::unmount(self, id);
        self.emit_status();
        Ok(())
    }

    pub fn disconnect(&self, id: &str) -> Result<()> {
        let s = self.st.lock().unwrap().sessions.get(id).map(|s| s.conn.clone()).context("не соединено")?;
        s.close(0u32.into(), b"bye");
        Ok(())
    }

    // ─── сеанс ─────────────────────────────────────────────────────────

    async fn run_session(self: D, conn: quinn::Connection, outgoing: bool) -> Result<()> {
        let fp = crate::net::peer_fingerprint(&conn).context("нет сертификата")?;
        let pid = crate::identity::id_of(&fp);
        if pid == self.id.id {
            conn.close(0u32.into(), b"self");
            bail!("соединение с самим собой");
        }
        let (mut tx, mut rx) = if outgoing {
            conn.open_bi().await?
        } else {
            tokio::time::timeout(Duration::from_secs(5), conn.accept_bi()).await.context("нет управляющего потока")??
        };
        proto::send(&mut tx, &Ctl::Hello(self.hello())).await?;
        let hello = match tokio::time::timeout(Duration::from_secs(5), proto::recv::<Ctl>(&mut rx)).await?? {
            Some(Ctl::Hello(h)) => h,
            _ => bail!("нет приветствия"),
        };
        if hello.id != pid {
            conn.close(1u32.into(), b"id");
            bail!("id не совпадает с сертификатом");
        }
        let ifaces = crate::netif::list();
        let transport = crate::netif::transport_for(conn.remote_address().ip(), &ifaces);

        // Доверие.
        let trusted = self.st.lock().unwrap().peers.peers.get(&pid).is_some_and(|t| t.fingerprint == fp);
        proto::send(&mut tx, &if trusted { Ctl::Trusted } else { Ctl::PairRequest }).await?;
        let they_trust = match tokio::time::timeout(Duration::from_secs(5), proto::recv::<Ctl>(&mut rx)).await?? {
            Some(Ctl::Trusted) => true,
            Some(Ctl::PairRequest) => false,
            _ => bail!("нет ответа о доверии"),
        };
        if !(trusted && they_trust) {
            self.pairing(&conn, &mut tx, &mut rx, &hello, &fp, trusted, transport, outgoing).await?;
        }

        // Сеанс установлен.
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<Ctl>();
        let gen = {
            let mut st = self.st.lock().unwrap();
            st.gen += 1;
            let gen = st.gen;
            // Новое соединение заменяет старое: его начинают, только когда
            // старое для той стороны мёртво (выдернули кабель — она заметила
            // раньше нас) или ради переезда на USB.
            if let Some(old) = st.sessions.get(&pid) {
                tracing::info!(peer = %hello.name, from = old.transport.title(), to = transport.title(), "соединение заменено");
                old.conn.close(0u32.into(), b"replaced");
            }
            if let Some(t) = st.peers.peers.get_mut(&pid) {
                t.last_seen = Some(crate::identity::now());
                t.name = hello.name.clone();
                t.user = hello.user.clone();
                t.uid = hello.uid;
                t.home = hello.home.clone();
                if hello.ssh_host_key.is_some() {
                    t.ssh_host_key = hello.ssh_host_key.clone();
                }
                let a = conn.remote_address().to_string();
                t.addrs.retain(|x| *x != a);
                t.addrs.insert(0, a);
                t.addrs.truncate(4);
            }
            st.peers.save();
            // Сеанс есть — новое соединение (переезд на USB) больше не блокируется.
            st.connecting.remove(&pid);
            st.connect_err.remove(&pid);
            st.sessions.insert(
                pid.clone(),
                Session { conn: conn.clone(), transport, addr: conn.remote_address(), hello: hello.clone(), ctl: ctl_tx.clone(), battery: None, gen },
            );
            gen
        };
        tracing::info!(peer = %hello.name, id = %pid, transport = transport.title(), "соединено");
        crate::ssh::write_config(&self.trusted_all());
        let _ = ctl_tx.send(Ctl::State { battery: crate::netif::battery() });
        if let Some(info) = self.peer_info(&pid) {
            self.emit(Event::Connected { device: info });
        }
        self.emit_status();
        if self.auto_mount.load(std::sync::atomic::Ordering::Relaxed) {
            let d = self.clone();
            let id = pid.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::fuse::mount(&d, &id).await {
                    tracing::warn!(id, "файлы устройства не смонтированы: {e:#}");
                }
            });
        }

        // Писатель управляющего потока.
        let writer = tokio::spawn(async move {
            while let Some(m) = ctl_rx.recv().await {
                if proto::send(&mut tx, &m).await.is_err() {
                    break;
                }
            }
        });
        // Входящие вызовы.
        let calls = {
            let (d, conn, pid) = (self.clone(), conn.clone(), pid.clone());
            tokio::spawn(async move {
                while let Ok((s, r)) = conn.accept_bi().await {
                    let (d, pid) = (d.clone(), pid.clone());
                    tokio::spawn(async move {
                        if let Err(e) = crate::rpc::serve(d, pid, s, r).await {
                            tracing::debug!("вызов: {e:#}");
                        }
                    });
                }
            })
        };
        // Читатель управляющего потока.
        loop {
            let msg = match proto::recv::<Ctl>(&mut rx).await {
                Ok(Some(m)) => m,
                _ => break,
            };
            match msg {
                Ctl::State { battery } => {
                    if let Some(s) = self.st.lock().unwrap().sessions.get_mut(&pid) {
                        s.battery = battery;
                    }
                    self.emit_status();
                }
                Ctl::Notification(n) => {
                    let _ = self.notes_tx.send((pid.clone(), n));
                }
                Ctl::Ping(x) => {
                    let _ = ctl_tx.send(Ctl::Pong(x));
                }
                Ctl::Clipboard { text, secret } => {
                    if self.clipboard.load(std::sync::atomic::Ordering::Relaxed) {
                        tracing::debug!(peer = %hello.name, len = text.len(), secret, "буфер обмена с устройства");
                        if let Some(c) = self.clip.get() {
                            c.set(text.clone(), secret);
                        }
                        // Третьим устройствам — тоже (они могут быть не соединены с той стороной).
                        self.share_clipboard(text, secret, Some(&pid));
                    }
                }
                Ctl::Forget => {
                    tracing::info!(peer = %hello.name, "устройство забыло нас");
                    let removed = self.st.lock().unwrap().peers.peers.remove(&pid).is_some();
                    if removed {
                        self.st.lock().unwrap().peers.save();
                        crate::ssh::unauthorize(&pid);
                        crate::ssh::write_config(&self.trusted_all());
                    }
                    conn.close(0u32.into(), b"forget");
                    break;
                }
                _ => {}
            }
        }
        writer.abort();
        calls.abort();
        conn.close(0u32.into(), b"end");
        let removed = {
            let mut st = self.st.lock().unwrap();
            if st.sessions.get(&pid).is_some_and(|s| s.gen == gen) {
                st.sessions.remove(&pid);
                true
            } else {
                false
            }
        };
        if removed {
            tracing::info!(peer = %hello.name, "соединение закрыто");
            crate::fuse::unmount(&self, &pid);
            if let Some(mut info) = self.peer_info(&pid) {
                info.name = hello.name.clone();
                self.emit(Event::Disconnected { device: info });
            }
            self.emit_status();
        }
        Ok(())
    }

    /// Спаривание внутри соединения: каждая сторона решает сама (или
    /// спрашивает пользователя), обе должны согласиться.
    #[allow(clippy::too_many_arguments)]
    async fn pairing(
        &self,
        conn: &quinn::Connection,
        tx: &mut quinn::SendStream,
        rx: &mut quinn::RecvStream,
        hello: &Hello,
        fp: &str,
        trusted: bool,
        transport: Transport,
        outgoing: bool,
    ) -> Result<()> {
        let pid = hello.id.clone();
        if !self.st.lock().unwrap().pairing.insert(pid.clone()) {
            conn.close(0u32.into(), b"busy");
            bail!("спаривание с этим устройством уже идёт");
        }
        let res = self.pairing_inner(conn, tx, rx, hello, fp, trusted, transport, outgoing).await;
        {
            let mut st = self.st.lock().unwrap();
            st.pairing.remove(&pid);
            if res.is_err() {
                st.backoff.insert(pid.clone(), Instant::now());
            } else {
                st.backoff.remove(&pid);
            }
        }
        res
    }

    #[allow(clippy::too_many_arguments)]
    async fn pairing_inner(
        &self,
        conn: &quinn::Connection,
        tx: &mut quinn::SendStream,
        rx: &mut quinn::RecvStream,
        hello: &Hello,
        fp: &str,
        trusted: bool,
        transport: Transport,
        outgoing: bool,
    ) -> Result<()> {
        let pid = hello.id.clone();
        let intent = self.st.lock().unwrap().intents.remove(&pid);
        let code = proto::pair_code(&self.id.fingerprint, fp);
        let gadget = self.hello_base.usb_gadget;
        let discoverable = self.st.lock().unwrap().discoverable;
        enum Decide {
            Accept,
            Ask,
            Show,
            Reject(&'static str),
        }
        let decide = if trusted {
            Decide::Accept
        } else if transport == Transport::Usb {
            // По кабелю: подтверждает телефон (гаджет), компьютер — сразу.
            if gadget {
                Decide::Ask
            } else {
                Decide::Accept
            }
        } else if intent && outgoing {
            // Сами начали: показываем код для сверки, решает та сторона.
            Decide::Show
        } else if discoverable {
            Decide::Ask
        } else {
            Decide::Reject(n_!("машина скрыта"))
        };
        let prompt = PairPrompt {
            id: pid.clone(),
            name: hello.name.clone(),
            kind: hello.kind,
            transport,
            code: if transport == Transport::Wifi { Some(code.clone()) } else { None },
            outgoing: matches!(decide, Decide::Show),
        };
        let mine = match decide {
            Decide::Accept => true,
            Decide::Reject(why) => {
                let _ = proto::send(tx, &Ctl::PairReject(synshell_tr::t(why))).await;
                conn.close(0u32.into(), b"reject");
                bail!("спаривание отклонено: {why}");
            }
            Decide::Show => {
                self.add_prompt(prompt.clone(), None);
                true
            }
            Decide::Ask => {
                let (wtx, wrx) = oneshot::channel();
                self.add_prompt(prompt.clone(), Some(wtx));
                crate::notify::pair_prompt(&prompt);
                let ok = tokio::select! {
                    r = wrx => r.unwrap_or(false),
                    _ = tokio::time::sleep(Duration::from_secs(90)) => false,
                    _ = conn.closed() => false,
                };
                self.drop_prompt(&pid);
                ok
            }
        };
        proto::send(tx, &if mine { Ctl::PairAccept } else { Ctl::PairReject(t!("пользователь отказал").into()) }).await?;
        let theirs = tokio::select! {
            r = proto::recv::<Ctl>(rx) => r?,
            _ = tokio::time::sleep(Duration::from_secs(100)) => None,
        };
        self.drop_prompt(&pid);
        let ok = mine && matches!(theirs, Some(Ctl::PairAccept));
        if !ok {
            let why = match theirs {
                Some(Ctl::PairReject(w)) => w,
                _ if !mine => t!("отклонено здесь").into(),
                _ => t!("нет ответа").into(),
            };
            self.emit(Event::Paired { device: pid.clone(), name: hello.name.clone(), ok: false, message: Some(why.clone()) });
            conn.close(0u32.into(), b"reject");
            bail!("спаривание не состоялось: {why}");
        }
        let t = Trusted {
            id: pid.clone(),
            name: hello.name.clone(),
            kind: hello.kind,
            fingerprint: fp.to_string(),
            user: hello.user.clone(),
            uid: hello.uid,
            home: hello.home.clone(),
            ssh_key: hello.ssh_key.clone(),
            ssh_host_key: hello.ssh_host_key.clone(),
            paired_at: crate::identity::now(),
            last_seen: Some(crate::identity::now()),
            addrs: vec![conn.remote_address().to_string()],
        };
        if let Some(k) = &t.ssh_key {
            if let Err(e) = crate::ssh::authorize(&t.id, &t.name, k) {
                tracing::warn!(?e, "authorized_keys");
            }
        }
        {
            let mut st = self.st.lock().unwrap();
            st.peers.peers.insert(pid.clone(), t);
            st.peers.save();
        }
        tracing::info!(peer = %hello.name, transport = transport.title(), "спарено");
        self.emit(Event::Paired { device: pid, name: hello.name.clone(), ok: true, message: None });
        Ok(())
    }

    fn add_prompt(&self, p: PairPrompt, waiter: Option<oneshot::Sender<bool>>) {
        {
            let mut st = self.st.lock().unwrap();
            st.prompts.retain(|x| x.id != p.id);
            st.prompts.push(p.clone());
            if let Some(w) = waiter {
                st.waiters.insert(p.id.clone(), w);
            }
        }
        self.emit(Event::PairPrompt { prompt: p });
        self.emit_status();
    }

    fn drop_prompt(&self, id: &str) {
        crate::notify::close_prompt(id);
        let changed = {
            let mut st = self.st.lock().unwrap();
            let n = st.prompts.len();
            st.prompts.retain(|x| x.id != id);
            st.waiters.remove(id);
            n != st.prompts.len()
        };
        if changed {
            self.emit_status();
        }
    }

    // ─── фон ───────────────────────────────────────────────────────────

    /// Кабель, батарея, давно не слышанные устройства.
    pub async fn ticker(self: D) {
        let mut n = 0u64;
        let mut last_battery = None;
        let cfg_path = synshell_common::paths::config_file();
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut cfg_mtime = mtime(&cfg_path);
        loop {
            // config.toml изменился («Параметры → Связь с устройствами») — применить [link].
            let m = mtime(&cfg_path);
            if m != cfg_mtime {
                cfg_mtime = m;
                let (cfg, _) = synshell_common::config::Config::load();
                let l = cfg.link;
                use std::sync::atomic::Ordering::Relaxed;
                self.notifications.store(l.notifications, Relaxed);
                self.auto_mount.store(l.auto_mount, Relaxed);
                self.clipboard.store(l.clipboard, Relaxed);
                let name = if l.name.trim().is_empty() { crate::identity::default_name() } else { l.name.trim().to_string() };
                if name != self.self_info().name || l.discoverable != self.self_info().discoverable {
                    tracing::info!(%name, discoverable = l.discoverable, "[link] изменён");
                    self.configure(Some(name), Some(l.discoverable));
                }
            }
            // Телефон засыпает — сеансы по Wi-Fi закрыть, анонсы прекратить; проснулся — анонс сразу.
            let sleeping = sleep_due();
            if self.sleeping.swap(sleeping, std::sync::atomic::Ordering::Relaxed) != sleeping {
                if sleeping {
                    let st = self.st.lock().unwrap();
                    for s in st.sessions.values().filter(|s| s.transport == Transport::Wifi) {
                        tracing::info!(peer = %s.hello.name, "экран погашен — соединение по Wi-Fi закрыто на время сна");
                        s.conn.close(0u32.into(), b"sleep");
                    }
                } else {
                    self.announce_now.notify_one();
                }
            }
            let ifaces = crate::netif::list();
            let mut usb = crate::netif::usb_info(&ifaces);
            let changed = {
                let mut st = self.st.lock().unwrap();
                // Телефон без ADSP не узнаёт о выдернутом кабеле: UDC так и
                // остаётся `configured`. Кабель есть, пока по USB идут анонсы
                // или сеанс.
                if self.hello_base.usb_gadget && usb.cable {
                    let alive = st.sessions.values().any(|s| s.transport == Transport::Usb)
                        || st.nearby.values().any(|n| n.transport == Transport::Usb && n.seen.elapsed() < Duration::from_secs(12));
                    // Только что запустились — ещё никого не слышали: верим UDC.
                    usb.cable = alive || n < 5;
                }
                let changed = st.usb != usb;
                if st.usb.interface != usb.interface || st.usb.address != usb.address {
                    // Новый интерфейс — анонсироваться сразу.
                    self.announce_now.notify_one();
                }
                // Кабель выдернули — сеанс по USB закрыть сразу, не ждать тайм-аута
                // QUIC: переподключение по Wi-Fi пойдёт по ближайшему анонсу.
                if st.usb.cable && !usb.cable {
                    for s in st.sessions.values().filter(|s| s.transport == Transport::Usb) {
                        tracing::info!(peer = %s.hello.name, "кабель USB отключён — соединение по USB закрыто");
                        s.conn.close(0u32.into(), b"usb-gone");
                    }
                    // Анонс по Wi-Fi тут же — не ждать очередного.
                    self.announce_now.notify_one();
                }
                st.usb = usb;
                st.nearby.retain(|_, v| v.seen.elapsed() < Duration::from_secs(60));
                changed
            };
            if changed {
                self.emit_status();
            }
            if n % 15 == 0 {
                let b = crate::netif::battery();
                if b != last_battery || n % 150 == 0 {
                    last_battery = b;
                    self.broadcast_ctl(Ctl::State { battery: b });
                }
                // Задержка меняется — оболочке раз в полминуты хватит.
                self.emit_status();
            }
            n += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}
