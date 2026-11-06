//! Слой кадров игр поверх экрана гостя: сервер паравиртуальной видеокарты `syn-gpu`
//! (проект syn-virt, docs/present.md, версия 1) отдаёт кадры swapchain из BAR2
//! (memfd хоста), syn-vm рисует их поверх кадра VNC в месте окна игры.
//!
//! Сокет `$XDG_RUNTIME_DIR/syn-gpu/<машина>.present.sock`; нет сокета — слоя нет, всё как
//! раньше. Сообщения: `u32 type, u32 len`, полезная нагрузка (little-endian, repr(C));
//! дескриптор BAR2 — SCM_RIGHTS при HELLO.
//!
//! Версия 2 (без копий): если окно рисует на той же видеокарте, что и syn-gpu, клиент шлёт
//! CAPS с UUID карты; сервер отдаёт кольцо изображений Vulkan (IMAGE + fd OPAQUE_FD), а FRAME с
//! флагом GPU_IMAGE называет номер изображения. syngui переписывает кадр в текстуру на GPU
//! (`LiveFrame::set_external`), RELEASE уходит, когда GPU дочитал. Импорт не удался — CAPS с
//! нулями, и сервер возвращается к BAR2.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use syngui::widgets::LiveFrame;

const HELLO: u32 = 1;
const FRAME: u32 = 2;
const GONE: u32 = 3;
/// Сервер → клиент: аппаратный курсор гостя (`Cursor`, пиксели BGRA в BAR2).
const CURSOR: u32 = 5;
/// Сервер → клиент: сообщение пользователю (`Notice { kind, _pad, bytes, free }`).
const NOTICE: u32 = 6;
/// Notice.kind: GPU хоста отказал гостю в видеопамяти.
pub const OUT_OF_VIDEO_MEMORY: u32 = 1;

/// Сообщение сервера syn-gpu пользователю.
#[derive(Clone, Copy, Debug)]
pub struct Notice {
    pub kind: u32,
    pub bytes: u64,
    pub free: u64,
}
const IMAGE: u32 = 4;
const RELEASE: u32 = 16;
const CAPS: u32 = 18;
/// Клиент → сервер: желаемое разрешение экрана гостя (`Screen { width, height }`), «Разрешение под окно».
const MODE: u32 = 19;
/// Клиент → сервер: частота монитора окна в мГц (`Refresh { millihertz, _pad }`) — VSync гостя от хоста.
const REFRESH: u32 = 20;
/// FRAME.flags: кадр в изображении Vulkan (offset — его номер).
const GPU_IMAGE: u32 = 1 << 2;
/// FRAME.flags: рабочий стол гостя (DWM на карте syn-gpu, сервер с `--display`).
const DESKTOP: u32 = 1 << 3;
/// DRM fourcc: байты B,G,R,A (`AR24` — с альфой, `XR24` — без).
const AR24: u32 = u32::from_le_bytes(*b"AR24");
const XR24: u32 = u32::from_le_bytes(*b"XR24");

/// Поверхность игры: кадр и место окна на экране гостя.
#[derive(Clone)]
pub struct Layer {
    pub frame: Arc<LiveFrame>,
    /// x, y, w, h — физические пиксели экрана гостя.
    pub rect: (i32, i32, u32, u32),
    /// Окно игры видимо и активно (flags бит 0).
    pub show: bool,
    pub fullscreen: bool,
    /// Рабочий стол гостя (FRAME.flags DESKTOP), а не окно игры.
    pub desktop: bool,
    updated: Instant,
}

/// Клиент слоя одной машины.
pub struct Present {
    layers: Mutex<HashMap<u64, Layer>>,
    alive: AtomicBool,
    /// Желаемое разрешение (окно) — уходит сразу и после каждого (пере)подключения.
    mode: Mutex<Option<(u32, u32)>>,
    refresh: Mutex<Option<u32>>,
    tx: Mutex<Option<Arc<Mutex<UnixStream>>>>,
    /// Номер своего указателя syngui для курсора гостя (0 — не ставить: проверка без окна).
    cursor_id: u32,
    on_notice: Option<Arc<dyn Fn(Notice) + Send + Sync>>,
}

/// Номер указателя syngui (`CursorIcon::Custom`) для курсора гостя машины.
pub fn cursor_id(id: &str) -> u32 {
    let h = id.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    0x5356_0000 | (h & 0xffff)
}

impl Present {
    /// Слой, который показывать: последняя обновлённая видимая поверхность.
    pub fn visible(&self) -> Option<Layer> {
        self.layers.lock().unwrap().values().filter(|l| l.show).max_by_key(|l| l.updated).cloned()
    }

    /// Видимое окно игры (не рабочий стол).
    pub fn game(&self) -> Option<Layer> {
        self.layers.lock().unwrap().values().filter(|l| l.show && !l.desktop).max_by_key(|l| l.updated).cloned()
    }

    /// Рабочий стол гостя — последний кадр DESKTOP.
    pub fn desktop(&self) -> Option<Layer> {
        self.layers.lock().unwrap().values().filter(|l| l.desktop).max_by_key(|l| l.updated).cloned()
    }

    pub fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Частота монитора окна (мГц): по ней сервер отсчитывает VSync и кадры гостя.
    pub fn request_refresh(&self, mhz: u32) {
        if *self.refresh.lock().unwrap() == Some(mhz) {
            return;
        }
        *self.refresh.lock().unwrap() = Some(mhz);
        if let Some(tx) = self.tx.lock().unwrap().clone() {
            if send(&mut tx.lock().unwrap(), REFRESH, &[mhz.to_le_bytes(), 0u32.to_le_bytes()].concat()).is_ok() {
                tracing::info!("syn-gpu present: частота монитора {:.3} Гц", mhz as f64 / 1000.0);
            }
        }
    }

    /// Разрешение экрана гостя под окно (физические пиксели).
    pub fn request_mode(&self, w: u32, h: u32) {
        if *self.mode.lock().unwrap() == Some((w, h)) {
            return;
        }
        *self.mode.lock().unwrap() = Some((w, h));
        if let Some(tx) = self.tx.lock().unwrap().clone() {
            let b = [w.to_le_bytes(), h.to_le_bytes()].concat();
            if send(&mut tx.lock().unwrap(), MODE, &b).is_ok() {
                tracing::info!("syn-gpu present: разрешение под окно {w}×{h}");
            }
        }
    }
}

fn socket(id: &str) -> PathBuf {
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    run.join("syn-gpu").join(format!("{id}.present.sock"))
}

/// Запустить клиента: ждёт сокет, переподключается; `on_change` — новый кадр или слой.
pub fn start(id: &str, on_change: Arc<dyn Fn() + Send + Sync>, on_notice: Arc<dyn Fn(Notice) + Send + Sync>) -> Arc<Present> {
    let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: cursor_id(id), on_notice: Some(on_notice) });
    let me = p.clone();
    let path = socket(id);
    std::thread::Builder::new()
        .name("syn-gpu-present".into())
        .spawn(move || {
            while me.alive.load(Ordering::Relaxed) {
                if path.exists() {
                    match UnixStream::connect(&path) {
                        Ok(s) => {
                            if let Err(e) = run(&me, s, &on_change, true) {
                                tracing::debug!("syn-gpu present: {e}");
                            }
                            me.layers.lock().unwrap().clear();
                            on_change();
                        }
                        Err(e) => tracing::debug!("syn-gpu present {}: {e}", path.display()),
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .ok();
    p
}

/// Проверка без окна: подключиться, дождаться видимого кадра игры (`desktop` — кадра рабочего
/// стола) до `wait`, вернуть слой.
pub fn grab(id: &str, wait: Duration, desktop: bool) -> Result<Layer> {
    let path = socket(id);
    let s = UnixStream::connect(&path).with_context(|| path.display().to_string())?;
    let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None });
    let p2 = p.clone();
    let t = std::thread::spawn(move || {
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        run(&p2, s, &cb, false)
    });
    let until = Instant::now() + wait;
    let layer = loop {
        if let Some(l) = if desktop { p.desktop() } else { p.visible() } {
            break Some(l);
        }
        if t.is_finished() || Instant::now() > until {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    p.close();
    let _ = t.join();
    if desktop {
        return layer.with_context(|| syngui::t!("нет кадра рабочего стола (драйвер syn-gpu в госте ещё не загрузился?)"));
    }
    layer.with_context(|| syngui::t!("нет видимого кадра игры (окно игры свёрнуто или не активно?)"))
}

/// Отображение BAR2 только для чтения.
struct Heap {
    ptr: *const u8,
    len: usize,
}

unsafe impl Send for Heap {}

impl Drop for Heap {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut _, self.len) };
    }
}

/// Номера импортированных изображений — свои у каждого (по ним syngui кэширует импорт).
static IMAGE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// `gpu` — окно (может читать кадры из изображений GPU); проверка без окна — только BAR2.
fn run(p: &Present, mut s: UnixStream, on_change: &Arc<dyn Fn() + Send + Sync>, gpu: bool) -> Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut heap: Option<Heap> = None;
    // Запись RELEASE из потока GPU syngui (кадр дочитан) — отдельной копией сокета.
    let tx = Arc::new(Mutex::new(s.try_clone()?));
    *p.tx.lock().unwrap() = Some(tx.clone());
    if let Some((w, h)) = *p.mode.lock().unwrap() {
        send(&mut tx.lock().unwrap(), MODE, &[w.to_le_bytes(), h.to_le_bytes()].concat())?;
    }
    if let Some(mhz) = *p.refresh.lock().unwrap() {
        send(&mut tx.lock().unwrap(), REFRESH, &[mhz.to_le_bytes(), 0u32.to_le_bytes()].concat())?;
    }
    // CAPS уже отправлен (UUID карты окна известен только после создания GPU окна).
    let mut caps_sent = !gpu;
    // Импорт не удался — до конца соединения кадры через BAR2.
    let gpu_failed = Arc::new(AtomicBool::new(false));
    let mut images: HashMap<(u64, u32), Arc<syngui::gpu::external::ExternalImage>> = HashMap::new();
    // Последний поставленный курсор: (seq, видим).
    let mut cursor_state: Option<(u32, bool)> = None;
    while p.alive.load(Ordering::Relaxed) {
        if !caps_sent {
            if let Some((dev, drv)) = syngui::gpu::external::device_uuids() {
                let mut b = Vec::with_capacity(40);
                b.extend_from_slice(&2u32.to_le_bytes());
                b.extend_from_slice(&0u32.to_le_bytes());
                b.extend_from_slice(&dev);
                b.extend_from_slice(&drv);
                send(&mut tx.lock().unwrap(), CAPS, &b)?;
                caps_sent = true;
                tracing::info!("syn-gpu present: окно на видеокарте с UUID {:02x?} — прошу кадры без копий", &dev[..4]);
            }
        }
        let (ty, payload, fd) = match read_msg(&mut s) {
            Ok(m) => m,
            Err(e) if is_timeout(&e) => continue,
            Err(e) => return Err(e),
        };
        match ty {
            HELLO => {
                let fd = fd.context("HELLO без дескриптора BAR2")?;
                let version = u32_at(&payload, 0);
                let size = u64_at(&payload, 8) as usize;
                if version != 1 {
                    unsafe { libc::close(fd) };
                    bail!("неизвестная версия протокола present: {version}");
                }
                let ptr = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, fd, 0) };
                unsafe { libc::close(fd) };
                if ptr == libc::MAP_FAILED {
                    bail!("mmap BAR2: {}", std::io::Error::last_os_error());
                }
                heap = Some(Heap { ptr: ptr as *const u8, len: size });
            }
            FRAME => {
                let surface = u64_at(&payload, 0);
                let seq = u64_at(&payload, 8);
                let offset = u64_at(&payload, 16) as usize;
                let (width, height, stride, format) = (u32_at(&payload, 24), u32_at(&payload, 28), u32_at(&payload, 32), u32_at(&payload, 36));
                let (x, y) = (u32_at(&payload, 40) as i32, u32_at(&payload, 44) as i32);
                let (w, h, flags) = (u32_at(&payload, 48), u32_at(&payload, 52), u32_at(&payload, 56));
                let release = [surface.to_le_bytes(), seq.to_le_bytes()].concat();
                if flags & GPU_IMAGE != 0 {
                    let Some(img) = images.get(&(surface, offset as u32)).cloned() else {
                        send(&mut tx.lock().unwrap(), RELEASE, &release)?;
                        continue;
                    };
                    let (tx2, failed) = (tx.clone(), gpu_failed.clone());
                    // Буфер свободен, когда GPU окна дочитал кадр (или кадр заменён, не показавшись).
                    let ext = syngui::gpu::external::ExternalFrame::new(img, move |ok| {
                        let mut t = tx2.lock().unwrap();
                        let _ = send(&mut t, RELEASE, &release);
                        if !ok && !failed.swap(true, Ordering::Relaxed) {
                            tracing::warn!("syn-gpu present: импорт изображения GPU не удался — кадры через BAR2");
                            let _ = send(&mut t, CAPS, &[2u32.to_le_bytes(), 0u32.to_le_bytes()].concat().into_iter().chain([0u8; 32]).collect::<Vec<u8>>());
                        }
                    });
                    let mut layers = p.layers.lock().unwrap();
                    let l = layers.entry(surface).or_insert_with(|| Layer {
                        frame: LiveFrame::new(),
                        rect: (0, 0, 0, 0),
                        show: false,
                        fullscreen: false,
                        desktop: false,
                        updated: Instant::now(),
                    });
                    l.frame.set_external(ext);
                    l.rect = (x, y, w, h);
                    l.show = flags & 1 != 0;
                    l.fullscreen = flags & 2 != 0;
                    l.desktop = flags & DESKTOP != 0;
                    l.updated = Instant::now();
                    drop(layers);
                    on_change();
                    continue;
                }
                let pixels = heap.as_ref().and_then(|hp| copy_frame(hp, offset, width, height, stride, format));
                // Кадр прочитан (или негоден) — буфер свободен.
                send(&mut tx.lock().unwrap(), RELEASE, &release)?;
                if let Some(px) = pixels {
                    let mut layers = p.layers.lock().unwrap();
                    let l = layers.entry(surface).or_insert_with(|| Layer {
                        frame: LiveFrame::new(),
                        rect: (0, 0, 0, 0),
                        show: false,
                        fullscreen: false,
                        desktop: false,
                        updated: Instant::now(),
                    });
                    l.frame.set(width, height, px.into());
                    l.rect = (x, y, w, h);
                    l.show = flags & 1 != 0;
                    l.fullscreen = flags & 2 != 0;
                    l.desktop = flags & DESKTOP != 0;
                    l.updated = Instant::now();
                    drop(layers);
                    on_change();
                }
            }
            IMAGE => {
                let fd = fd.context("IMAGE без дескриптора")?;
                let owned = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
                let surface = u64_at(&payload, 0);
                let index = u32_at(&payload, 8);
                let img = syngui::gpu::external::ExternalImage {
                    id: IMAGE_ID.fetch_add(1, Ordering::Relaxed),
                    fd: owned,
                    format: u32_at(&payload, 12) as i32,
                    width: u32_at(&payload, 16),
                    height: u32_at(&payload, 20),
                    flags: u32_at(&payload, 24),
                    usage: u32_at(&payload, 28),
                    size: u64_at(&payload, 32),
                };
                tracing::info!("syn-gpu present: поверхность {surface:#x}: изображение {index} {}×{}", img.width, img.height);
                images.insert((surface, index), Arc::new(img));
            }
            NOTICE if payload.len() >= 24 => {
                let n = Notice { kind: u32_at(&payload, 0), bytes: u64_at(&payload, 8), free: u64_at(&payload, 16) };
                tracing::warn!("syn-gpu: сообщение {n:?}");
                if let Some(f) = &p.on_notice {
                    f(n);
                }
            }
            CURSOR if p.cursor_id != 0 && payload.len() >= 48 => {
                let (seq, visible) = (u32_at(&payload, 0), u32_at(&payload, 4) != 0);
                if cursor_state == Some((seq, visible)) {
                    continue;
                }
                cursor_state = Some((seq, visible));
                let (hx, hy, w, h, stride) = (u32_at(&payload, 16), u32_at(&payload, 20), u32_at(&payload, 24), u32_at(&payload, 28), u32_at(&payload, 32) as usize);
                let off = u64_at(&payload, 40) as usize;
                let img = if !visible || w == 0 || h == 0 {
                    None
                } else {
                    heap.as_ref().and_then(|hp| {
                        let (w, h) = (w.min(256) as usize, h.min(256) as usize);
                        if stride < w * 4 || off.checked_add(stride * (h - 1) + w * 4)? > hp.len {
                            return None;
                        }
                        let src = unsafe { std::slice::from_raw_parts(hp.ptr.add(off), stride * (h - 1) + w * 4) };
                        let mut rgba = vec![0u8; w * h * 4];
                        for row in 0..h {
                            for (d, s) in rgba[row * w * 4..(row + 1) * w * 4].chunks_exact_mut(4).zip(src[row * stride..row * stride + w * 4].chunks_exact(4)) {
                                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
                            }
                        }
                        Some(syngui::input::CursorImage { width: w as u16, height: h as u16, hot_x: hx.min(w as u32 - 1) as u16, hot_y: hy.min(h as u32 - 1) as u16, rgba })
                    })
                };
                syngui::input::set_custom_cursor(p.cursor_id, img);
            }
            GONE => {
                let surface = u64_at(&payload, 0);
                images.retain(|k, _| k.0 != surface);
                if p.layers.lock().unwrap().remove(&surface).is_some() {
                    on_change();
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// BGRA (AR24/XR24) из BAR2 → RGBA для LiveFrame.
fn copy_frame(hp: &Heap, offset: usize, width: u32, height: u32, stride: u32, format: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || (format != AR24 && format != XR24) {
        return None;
    }
    let (w, h, stride) = (width as usize, height as usize, stride as usize);
    if stride < w * 4 || offset.checked_add(stride.checked_mul(h - 1)?.checked_add(w * 4)?)? > hp.len {
        return None;
    }
    let src = unsafe { std::slice::from_raw_parts(hp.ptr.add(offset), stride * (h - 1) + w * 4) };
    let mut out = vec![0u8; w * h * 4];
    let opaque = format == XR24;
    for row in 0..h {
        let s = &src[row * stride..row * stride + w * 4];
        let d = &mut out[row * w * 4..(row + 1) * w * 4];
        for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
            dp[0] = sp[2];
            dp[1] = sp[1];
            dp[2] = sp[0];
            dp[3] = if opaque { 255 } else { sp[3] };
        }
    }
    Some(out)
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    b.get(i..i + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
}

fn is_timeout(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>().is_some_and(|e| matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

fn send(s: &mut UnixStream, ty: u32, payload: &[u8]) -> Result<()> {
    let mut b = Vec::with_capacity(8 + payload.len());
    b.extend_from_slice(&ty.to_le_bytes());
    b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    b.extend_from_slice(payload);
    s.write_all(&b)?;
    Ok(())
}

/// Сообщение: заголовок читается recvmsg (с ним может прийти дескриптор), нагрузка — следом.
fn read_msg(s: &mut UnixStream) -> Result<(u32, Vec<u8>, Option<RawFd>)> {
    let mut hdr = [0u8; 8];
    let mut cmsg = [0u8; 64];
    let mut iov = libc::iovec { iov_base: hdr.as_mut_ptr() as *mut _, iov_len: hdr.len() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg.as_mut_ptr() as *mut _;
    msg.msg_controllen = cmsg.len() as _;
    let n = unsafe { libc::recvmsg(s.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if n == 0 {
        bail!("syn-gpu закрыл соединение");
    }
    let mut fd = None;
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                fd = Some(std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const RawFd));
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    let got = n as usize;
    if got < 8 {
        // Остаток заголовка — обычным чтением (без тайм-аута посередине сообщения).
        s.set_read_timeout(None)?;
        s.read_exact(&mut hdr[got..])?;
    }
    let ty = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(hdr[4..8].try_into().unwrap()) as usize;
    if len > 1 << 20 {
        bail!("сообщение syn-gpu слишком длинное: {len}");
    }
    let mut payload = vec![0u8; len];
    s.set_read_timeout(None)?;
    s.read_exact(&mut payload)?;
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    Ok((ty, payload, fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::FromRawFd;

    #[test]
    fn notice_reaches_callback() {
        let (mut srv, cli) = UnixStream::pair().unwrap();
        let got: Arc<Mutex<Vec<(u32, u64)>>> = Arc::default();
        let g = got.clone();
        let p = Present {
            layers: Mutex::new(HashMap::new()),
            alive: AtomicBool::new(true),
            mode: Mutex::new(None),
            refresh: Mutex::new(None),
            tx: Mutex::new(None),
            cursor_id: 0,
            on_notice: Some(Arc::new(move |n: Notice| g.lock().unwrap().push((n.kind, n.bytes)))),
        };
        let mut m = Vec::new();
        m.extend_from_slice(&NOTICE.to_le_bytes());
        m.extend_from_slice(&24u32.to_le_bytes());
        m.extend_from_slice(&OUT_OF_VIDEO_MEMORY.to_le_bytes());
        m.extend_from_slice(&0u32.to_le_bytes());
        m.extend_from_slice(&(256u64 << 20).to_le_bytes());
        m.extend_from_slice(&0u64.to_le_bytes());
        srv.write_all(&m).unwrap();
        drop(srv);
        let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        let _ = run(&p, cli, &cb, false);
        assert_eq!(*got.lock().unwrap(), vec![(OUT_OF_VIDEO_MEMORY, 256 << 20)]);
    }

    /// Сервер-заглушка: HELLO с memfd, FRAME 2×1 в BGRA → слой RGBA, RELEASE в ответ, GONE.
    #[test]
    fn hello_frame_release_gone() {
        let (mut srv, cli) = UnixStream::pair().unwrap();
        let memfd = unsafe { libc::memfd_create(c"bar2".as_ptr(), 0) };
        let mut f = unsafe { std::fs::File::from_raw_fd(memfd) };
        let mut bar = vec![0u8; 4096];
        bar[256..264].copy_from_slice(&[10, 20, 30, 0, 40, 50, 60, 0]); // два пикселя B,G,R,X
        f.write_all(&bar).unwrap();
        // HELLO с дескриптором.
        let mut hello = Vec::new();
        hello.extend_from_slice(&HELLO.to_le_bytes());
        hello.extend_from_slice(&16u32.to_le_bytes());
        hello.extend_from_slice(&1u32.to_le_bytes());
        hello.extend_from_slice(&0u32.to_le_bytes());
        hello.extend_from_slice(&4096u64.to_le_bytes());
        send_fd(&srv, &hello, f.as_raw_fd());
        // FRAME: surface 7, seq 1, offset 256, 2×1, stride 8, XR24, x=100,y=50,w=2,h=1, flags 1.
        let mut fr = Vec::new();
        for v in [7u64, 1, 256] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        for v in [2u32, 1, 8, XR24, 100, 50, 2, 1, 1, 0] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        send(&mut srv, FRAME, &fr).unwrap();
        let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None });
        let changed = Arc::new(AtomicBool::new(false));
        let ch = changed.clone();
        let p2 = p.clone();
        let t = std::thread::spawn(move || {
            let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ch.store(true, Ordering::Relaxed));
            let _ = run(&p2, cli, &cb, false);
        });
        // RELEASE.
        let mut rel = [0u8; 24];
        srv.read_exact(&mut rel).unwrap();
        assert_eq!(u32_at(&rel, 0), RELEASE);
        assert_eq!((u64_at(&rel, 8), u64_at(&rel, 16)), (7, 1));
        std::thread::sleep(Duration::from_millis(50));
        let l = p.visible().expect("слой");
        assert_eq!(l.rect, (100, 50, 2, 1));
        let (w, h, px, _) = l.frame.latest();
        assert_eq!((w, h), (2, 1));
        assert_eq!(&px[..8], &[30, 20, 10, 255, 60, 50, 40, 255]);
        assert!(changed.load(Ordering::Relaxed));
        // GONE.
        send(&mut srv, GONE, &7u64.to_le_bytes()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(p.visible().is_none());
        p.close();
        drop(srv);
        t.join().unwrap();
    }

    /// v2: IMAGE (fd) → FRAME с GPU_IMAGE → слой с кадром во внешнем изображении; RELEASE —
    /// только когда кадр отпущен (здесь — GONE убирает слой), а не сразу после FRAME.
    #[test]
    fn gpu_image_release_on_drop() {
        let (mut srv, cli) = UnixStream::pair().unwrap();
        let memfd = unsafe { libc::memfd_create(c"img".as_ptr(), 0) };
        // IMAGE: surface 7, index 1, B8G8R8A8_UNORM (44), 640×480, MUTABLE_FORMAT (8), usage 6, size 1 МиБ.
        let mut im = Vec::new();
        im.extend_from_slice(&IMAGE.to_le_bytes());
        im.extend_from_slice(&40u32.to_le_bytes());
        im.extend_from_slice(&7u64.to_le_bytes());
        for v in [1u32, 44, 640, 480, 8, 6] {
            im.extend_from_slice(&v.to_le_bytes());
        }
        im.extend_from_slice(&(1u64 << 20).to_le_bytes());
        send_fd(&srv, &im, memfd);
        unsafe { libc::close(memfd) };
        let mut fr = Vec::new();
        for v in [7u64, 5, 1] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        for v in [640u32, 480, 0, XR24, 10, 20, 640, 480, 1 | GPU_IMAGE, 0] {
            fr.extend_from_slice(&v.to_le_bytes());
        }
        send(&mut srv, FRAME, &fr).unwrap();
        let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true), mode: Mutex::new(None), refresh: Mutex::new(None), tx: Mutex::new(None), cursor_id: 0, on_notice: None });
        let p2 = p.clone();
        let t = std::thread::spawn(move || {
            let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            let _ = run(&p2, cli, &cb, false);
        });
        std::thread::sleep(Duration::from_millis(100));
        let l = p.visible().expect("слой");
        assert_eq!(l.rect, (10, 20, 640, 480));
        assert_eq!(l.frame.size(), (640, 480));
        assert!(l.frame.latest().2.is_empty(), "кадр во внешнем изображении — без пикселей в памяти");
        drop(l);
        // Пока слой держит кадр, RELEASE нет.
        srv.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut rel = [0u8; 24];
        assert!(srv.read_exact(&mut rel).is_err());
        send(&mut srv, GONE, &7u64.to_le_bytes()).unwrap();
        srv.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        srv.read_exact(&mut rel).unwrap();
        assert_eq!(u32_at(&rel, 0), RELEASE);
        assert_eq!((u64_at(&rel, 8), u64_at(&rel, 16)), (7, 5));
        p.close();
        drop(srv);
        t.join().unwrap();
    }

    fn send_fd(s: &UnixStream, bytes: &[u8], fd: RawFd) {
        let mut cmsg = [0u8; 64];
        let mut iov = libc::iovec { iov_base: bytes.as_ptr() as *mut _, iov_len: bytes.len() };
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cmsg.as_mut_ptr() as *mut _;
        msg.msg_controllen = unsafe { libc::CMSG_SPACE(4) } as _;
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(4) as _;
            std::ptr::write_unaligned(libc::CMSG_DATA(c) as *mut RawFd, fd);
            assert!(libc::sendmsg(s.as_raw_fd(), &msg, 0) > 0);
        }
    }
}
