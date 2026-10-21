//! Клиент wlr-data-control истории буфера: следит за выбором (`selection`)
//! в своём потоке и своей Wayland-связи, читает содержимое и отдаёт его
//! наружу; по команде становится источником выбора — запись истории снова
//! в буфере.
//!
//! Что читается: файлы (`text/uri-list`), иначе картинка, если текста нет
//! (кнопка «Копировать изображение» браузера), иначе текст (и HTML рядом —
//! чтобы при возврате записи вставка была той же). Пароли (пометка
//! `x-kde-passwordManagerHint`) и свой же выбор (пометка [`MARK`]) не
//! читаются вовсе.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
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

/// Свой выбор (запись, возвращённая из истории) — обратно не читается.
pub const MARK: &str = "application/x-synshell-clipboard-history";
/// Пометка секрета (KeePassXC, synpass, KDE).
const SECRET_HINT: &str = "x-kde-passwordManagerHint";
/// Текст, пришедший с другого устройства через synlink.
const SYNLINK_MARK: &str = "application/x-synlink-clipboard";
pub const TEXT_MIMES: &[&str] = &["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"];
const MAX_TEXT: usize = 4 << 20;
const MAX_HTML: usize = 2 << 20;
/// Картинки предпочтительно в этом порядке.
const IMAGE_MIMES: &[&str] = &["image/png", "image/webp", "image/jpeg", "image/gif", "image/bmp", "image/avif"];

/// Картинки запоминать (`[clipboard] images`).
pub static IMAGES: AtomicBool = AtomicBool::new(true);
/// Предел картинки, байт (`[clipboard] max_image_mb`).
pub static MAX_IMAGE: AtomicU64 = AtomicU64::new(32 << 20);

/// Что прочитано из буфера.
#[derive(Debug, Clone)]
pub enum Content {
    Text { text: String, html: Option<Vec<u8>> },
    Image { mime: String, data: Vec<u8> },
    /// `text/uri-list` (по URI на строку).
    Files { uris: String },
}

#[derive(Debug, Clone)]
pub struct Captured {
    pub content: Content,
    /// Пришло с другого устройства (synlink).
    pub remote: bool,
}

/// Форматы источника: MIME → данные.
pub type Offer = Vec<(String, Arc<Vec<u8>>)>;

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::Sender<Offer>,
    wake: Arc<OwnedFd>,
}

impl Handle {
    /// Стать выбором с этими форматами.
    pub fn set(&self, offer: Offer) {
        let _ = self.tx.send(offer);
        let b = [1u8];
        unsafe { libc::write(self.wake.as_raw_fd(), b.as_ptr().cast(), 1) };
    }
}

fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Запустить поток; прочитанное приходит в `on` (из рабочих потоков).
pub fn start(on: impl Fn(Captured) + Send + Sync + 'static) -> Result<Handle> {
    let (tx, rx) = mpsc::channel();
    let (wake_r, wake_w) = pipe()?;
    unsafe { libc::fcntl(wake_r.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
    let on: Arc<dyn Fn(Captured) + Send + Sync> = Arc::new(on);
    std::thread::Builder::new().name("clip-history".into()).spawn(move || {
        let mut warned = false;
        loop {
            match run(&rx, &wake_r, &on) {
                Ok(()) => return,
                Err(e) => {
                    if !warned {
                        log::warn!("история буфера обмена: {e:#} — повтор через 5 с");
                        warned = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
            while rx.try_recv().is_ok() {}
        }
    })?;
    Ok(Handle { tx, wake: Arc::new(wake_w) })
}

struct St {
    on: Arc<dyn Fn(Captured) + Send + Sync>,
    offers: HashMap<ObjectId, Vec<String>>,
    sources: HashMap<ObjectId, (ZwlrDataControlSourceV1, Arc<Offer>)>,
    conn: Connection,
    gone: bool,
}

fn run(rx: &mpsc::Receiver<Offer>, wake: &OwnedFd, on: &Arc<dyn Fn(Captured) + Send + Sync>) -> Result<()> {
    let conn = Connection::connect_to_env().context("нет Wayland")?;
    let (globals, mut queue) = registry_queue_init::<St>(&conn).context("реестр Wayland")?;
    let qh = queue.handle();
    let manager: ZwlrDataControlManagerV1 = globals.bind(&qh, 1..=2, ()).context("композитор без wlr-data-control")?;
    let seat: WlSeat = globals.bind(&qh, 1..=7, ()).context("нет wl_seat")?;
    let device = manager.get_data_device(&seat, &qh, ());
    let mut st = St { on: on.clone(), offers: HashMap::new(), sources: HashMap::new(), conn: conn.clone(), gone: false };
    log::info!("история буфера обмена: подключена к композитору");
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
            while let Ok(o) = rx.try_recv() {
                let src = manager.create_data_source(&qh, ());
                for (m, _) in &o {
                    src.offer(m.clone());
                }
                src.offer(MARK.into());
                device.set_selection(Some(&src));
                st.sources.insert(src.id(), (src, Arc::new(o)));
            }
        }
    }
}

/// Что читать из предложенного.
enum Plan {
    Files(&'static str),
    Image(String),
    Text { text: &'static str, html: bool },
}

fn plan(mimes: &[String]) -> Option<Plan> {
    let has = |m: &str| mimes.iter().any(|x| x == m);
    if has("text/uri-list") {
        return Some(Plan::Files("text/uri-list"));
    }
    let text = TEXT_MIMES.iter().find(|m| has(m)).copied();
    if text.is_none() && IMAGES.load(Ordering::Relaxed) {
        let img = IMAGE_MIMES
            .iter()
            .find(|m| has(m))
            .map(|m| m.to_string())
            .or_else(|| mimes.iter().find(|m| m.starts_with("image/") && *m != "image/svg+xml").cloned());
        if let Some(m) = img {
            return Some(Plan::Image(m));
        }
    }
    text.map(|t| Plan::Text { text: t, html: has("text/html") })
}

fn on_selection(st: &mut St, o: Option<ZwlrDataControlOfferV1>) {
    let Some(o) = o else { return };
    let mimes = st.offers.remove(&o.id()).unwrap_or_default();
    if mimes.iter().any(|m| m == MARK || m == SECRET_HINT) {
        o.destroy();
        return;
    }
    let remote = mimes.iter().any(|m| m == SYNLINK_MARK);
    let Some(plan) = plan(&mimes) else {
        o.destroy();
        return;
    };
    // Запросы — здесь (поток Wayland), чтение — в отдельном потоке: источник
    // может быть и нашим, его `send` обслуживает этот же поток.
    let ask = |mime: &str| -> Option<OwnedFd> {
        let (r, w) = pipe().ok()?;
        o.receive(mime.to_string(), w.as_fd());
        Some(r)
    };
    let job: Box<dyn FnOnce() -> Option<Content> + Send> = match plan {
        Plan::Files(m) => {
            let Some(r) = ask(m) else { return o.destroy() };
            Box::new(move || {
                let uris = String::from_utf8(read_pipe(r, MAX_TEXT)?).ok()?;
                (!uris.trim().is_empty()).then_some(Content::Files { uris })
            })
        }
        Plan::Image(m) => {
            let Some(r) = ask(&m) else { return o.destroy() };
            let max = MAX_IMAGE.load(Ordering::Relaxed) as usize;
            Box::new(move || {
                let data = read_pipe(r, max)?;
                (!data.is_empty()).then_some(Content::Image { mime: m, data })
            })
        }
        Plan::Text { text, html } => {
            let Some(r) = ask(text) else { return o.destroy() };
            let rh = if html { ask("text/html") } else { None };
            Box::new(move || {
                let t = String::from_utf8_lossy(&read_pipe(r, MAX_TEXT)?).into_owned();
                if t.trim().is_empty() {
                    return None;
                }
                let html = rh.and_then(|r| read_pipe(r, MAX_HTML)).filter(|h| !h.is_empty());
                Some(Content::Text { text: t, html })
            })
        }
    };
    let _ = st.conn.flush();
    o.destroy();
    let on = st.on.clone();
    std::thread::spawn(move || {
        if let Some(content) = job() {
            on(Captured { content, remote });
        }
    });
}

/// Содержимое канала: не дольше 3 с, не больше `max` (больше — `None`).
fn read_pipe(r: OwnedFd, max: usize) -> Option<Vec<u8>> {
    let mut f = std::fs::File::from(r);
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 65536];
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
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
                if buf.len() > max {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(buf)
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
                let Some((_, offer)) = st.sources.get(&s.id()) else { return };
                let data = offer.iter().find(|(m, _)| *m == mime_type).map(|(_, d)| d.clone()).unwrap_or_default();
                // Читатель может не спешить — не держать поток Wayland.
                std::thread::spawn(move || {
                    let mut f = std::fs::File::from(fd);
                    let _ = f.write_all(&data);
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
