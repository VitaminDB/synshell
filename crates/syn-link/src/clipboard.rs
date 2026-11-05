//! Общий буфер обмена (`[link] clipboard`): текст, скопированный на этой
//! машине, уходит соединённым устройствам, пришедший оттуда — кладётся в
//! буфер здесь.
//!
//! Буфер читается и пишется клиентом wlr-data-control (synwm даёт его всем
//! клиентам) в своём потоке: выбор (`selection`) меняется — содержимое
//! читается в отдельном потоке (источник может быть и нашим, его `send`
//! обслуживает этот же поток) и уходит демону. Пришедший текст становится
//! нашим источником с пометкой [`MARK`], по которой свой же выбор не
//! читается обратно. Повтор того, что уже передано (`synced`), не уходит —
//! так текст не ходит по кругу.
//!
//! Секреты (менеджеры паролей помечают их `x-kde-passwordManagerHint`)
//! передаются с пометкой; очистили буфер с секретом здесь — он очищается и
//! там (если там его ещё не заменили). Обычный текст при очистке остаётся:
//! выбор пропадает и когда закрывается программа, из которой копировали.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self as dev, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self as offer, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self as source, ZwlrDataControlSourceV1},
};

/// Свой источник — его выбор не читается обратно.
const MARK: &str = "application/x-synlink-clipboard";
/// Пометка секрета (KDE, KeePassXC, synpass): истории буфера его не хранят.
pub const SECRET_HINT: &str = "x-kde-passwordManagerHint";
const TEXT_MIMES: &[&str] = &["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"];
/// Больше не передаём: буфер — для текста, не для файлов.
const MAX_TEXT: usize = 4 << 20;

/// Текст буфера этой машины (пустой — секрет очищен).
#[derive(Debug, Clone)]
pub struct Local {
    pub text: String,
    pub secret: bool,
}

enum Cmd {
    Set { text: String, secret: bool },
    /// Очистить, если в буфере всё ещё переданный секрет.
    ClearSecret,
}

#[derive(Default)]
struct Shared {
    /// Что сейчас в буфере здесь (известный нам текст).
    current: Option<String>,
    /// Последнее переданное в любую сторону.
    synced: Option<String>,
    synced_secret: bool,
}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Cmd>,
    wake: Arc<OwnedFd>,
}

impl Handle {
    /// Пришло с другого устройства.
    pub fn set(&self, text: String, secret: bool) {
        self.send(if text.is_empty() { Cmd::ClearSecret } else { Cmd::Set { text, secret } });
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
        let _ = nix_write(self.wake.as_raw_fd(), &[1]);
    }
}

fn nix_write(fd: i32, b: &[u8]) -> std::io::Result<usize> {
    let n = unsafe { libc::write(fd, b.as_ptr().cast(), b.len()) };
    if n < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Запустить поток буфера; текст этой машины приходит в `out`.
pub fn start(out: tokio::sync::mpsc::UnboundedSender<Local>) -> Result<Handle> {
    let (tx, rx) = mpsc::channel();
    let (wake_r, wake_w) = pipe()?;
    unsafe { libc::fcntl(wake_r.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
    std::thread::Builder::new().name("clipboard".into()).spawn(move || {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut warned = false;
        loop {
            match run(&rx, &wake_r, &out, &shared) {
                Ok(()) => return,
                Err(e) => {
                    if !warned {
                        tracing::warn!("общий буфер обмена: {e:#} — повтор через 5 с");
                        warned = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
            // Пока соединения не было, пришедшее устарело.
            while rx.try_recv().is_ok() {}
        }
    })?;
    Ok(Handle { tx, wake: Arc::new(wake_w) })
}

struct St {
    out: tokio::sync::mpsc::UnboundedSender<Local>,
    shared: Arc<Mutex<Shared>>,
    offers: HashMap<ObjectId, Vec<String>>,
    sources: HashMap<ObjectId, (ZwlrDataControlSourceV1, Arc<String>)>,
    conn: Connection,
    gone: bool,
}

fn run(rx: &mpsc::Receiver<Cmd>, wake: &OwnedFd, out: &tokio::sync::mpsc::UnboundedSender<Local>, shared: &Arc<Mutex<Shared>>) -> Result<()> {
    let conn = Connection::connect_to_env().context("нет Wayland")?;
    let (globals, mut queue) = registry_queue_init::<St>(&conn).context("реестр Wayland")?;
    let qh = queue.handle();
    let manager: ZwlrDataControlManagerV1 = globals.bind(&qh, 1..=2, ()).context("композитор без wlr-data-control")?;
    let seat: WlSeat = globals.bind(&qh, 1..=7, ()).context("нет wl_seat")?;
    let device = manager.get_data_device(&seat, &qh, ());
    let mut st = St { out: out.clone(), shared: shared.clone(), offers: HashMap::new(), sources: HashMap::new(), conn: conn.clone(), gone: false };
    tracing::info!("общий буфер обмена: подключён к композитору");
    loop {
        queue.dispatch_pending(&mut st)?;
        if st.gone {
            anyhow::bail!("устройство буфера закрыто");
        }
        conn.flush()?;
        let Some(guard) = queue.prepare_read() else { continue };
        let mut fds = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if n < 0 {
            drop(guard);
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.into());
        }
        if fds[0].revents != 0 {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.into()),
            }
        } else {
            drop(guard);
        }
        if fds[1].revents != 0 {
            let mut buf = [0u8; 64];
            while unsafe { libc::read(wake.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {}
            while let Ok(cmd) = rx.try_recv() {
                apply(&mut st, &manager, &device, &qh, cmd);
            }
        }
    }
}

fn apply(st: &mut St, manager: &ZwlrDataControlManagerV1, device: &ZwlrDataControlDeviceV1, qh: &QueueHandle<St>, cmd: Cmd) {
    match cmd {
        Cmd::Set { text, secret } => {
            {
                let mut sh = st.shared.lock().unwrap();
                sh.current = Some(text.clone());
                sh.synced = Some(text.clone());
                sh.synced_secret = secret;
            }
            let src = manager.create_data_source(qh, ());
            for m in TEXT_MIMES {
                src.offer(m.to_string());
            }
            src.offer(MARK.into());
            if secret {
                src.offer(SECRET_HINT.into());
            }
            device.set_selection(Some(&src));
            st.sources.insert(src.id(), (src, Arc::new(text)));
        }
        Cmd::ClearSecret => {
            let mut sh = st.shared.lock().unwrap();
            if sh.synced_secret && sh.synced.is_some() && sh.current == sh.synced {
                sh.current = None;
                sh.synced = None;
                sh.synced_secret = false;
                drop(sh);
                device.set_selection(None);
                tracing::debug!("секрет из общего буфера очищен");
            }
        }
    }
}

/// Выбор сменился на чужой источник.
fn on_selection(st: &mut St, o: Option<ZwlrDataControlOfferV1>) {
    let Some(o) = o else {
        let mut sh = st.shared.lock().unwrap();
        sh.current = None;
        // Очистили секрет — и там тоже.
        if sh.synced_secret && sh.synced.is_some() {
            sh.synced = None;
            sh.synced_secret = false;
            let _ = st.out.send(Local { text: String::new(), secret: true });
        }
        return;
    };
    let mimes = st.offers.remove(&o.id()).unwrap_or_default();
    if mimes.iter().any(|m| m == MARK) {
        o.destroy();
        return;
    }
    let Some(mime) = TEXT_MIMES.iter().find(|m| mimes.iter().any(|x| x == *m)) else {
        // Не текст (картинка, файлы) — не передаём.
        st.shared.lock().unwrap().current = None;
        o.destroy();
        return;
    };
    let secret = mimes.iter().any(|m| m == SECRET_HINT);
    let Ok((r, w)) = pipe() else {
        o.destroy();
        return;
    };
    o.receive(mime.to_string(), w.as_fd());
    let _ = st.conn.flush();
    drop(w);
    o.destroy();
    let (out, shared) = (st.out.clone(), st.shared.clone());
    std::thread::spawn(move || {
        let Some(text) = read_pipe(r) else { return };
        let mut sh = shared.lock().unwrap();
        sh.current = Some(text.clone());
        if text.is_empty() || sh.synced.as_deref() == Some(text.as_str()) {
            return;
        }
        sh.synced = Some(text.clone());
        sh.synced_secret = secret;
        drop(sh);
        let _ = out.send(Local { text, secret });
    });
}

/// Содержимое из канала источника: не дольше 2 с, не больше [`MAX_TEXT`].
fn read_pipe(r: OwnedFd) -> Option<String> {
    let mut f = std::fs::File::from(r);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16384];
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return None;
        }
        let mut p = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        if unsafe { libc::poll(&mut p, 1, left.as_millis() as i32) } <= 0 {
            return None;
        }
        match f.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > MAX_TEXT {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    String::from_utf8(buf).ok()
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for St {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSeat, ()> for St {
    fn event(_: &mut Self, _: &WlSeat, _: wayland_client::protocol::wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrDataControlManagerV1, ()> for St {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for St {
    fn event(st: &mut Self, _: &ZwlrDataControlDeviceV1, ev: dev::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            dev::Event::DataOffer { id } => {
                st.offers.insert(id.id(), Vec::new());
            }
            dev::Event::Selection { id } => on_selection(st, id),
            dev::Event::PrimarySelection { id } => {
                if let Some(o) = id {
                    st.offers.remove(&o.id());
                    o.destroy();
                }
            }
            dev::Event::Finished => st.gone = true,
            _ => {}
        }
    }

    event_created_child!(St, ZwlrDataControlDeviceV1, [
        dev::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for St {
    fn event(st: &mut Self, o: &ZwlrDataControlOfferV1, ev: offer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let offer::Event::Offer { mime_type } = ev {
            if let Some(v) = st.offers.get_mut(&o.id()) {
                v.push(mime_type);
            }
        }
    }
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for St {
    fn event(st: &mut Self, s: &ZwlrDataControlSourceV1, ev: source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            source::Event::Send { mime_type, fd } => {
                let Some((_, text)) = st.sources.get(&s.id()) else { return };
                let data = if mime_type == SECRET_HINT { Arc::new("secret".to_string()) } else { text.clone() };
                // Читатель может не спешить — не держать поток Wayland.
                std::thread::spawn(move || {
                    let mut f = std::fs::File::from(fd);
                    let _ = f.write_all(data.as_bytes());
                });
            }
            source::Event::Cancelled => {
                if let Some((src, _)) = st.sources.remove(&s.id()) {
                    src.destroy();
                }
            }
            _ => {}
        }
    }
}
