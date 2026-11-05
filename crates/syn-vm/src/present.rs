//! Слой кадров игр поверх экрана гостя: сервер паравиртуальной видеокарты `syn-gpu`
//! (проект qemu-vulkan-driver, docs/present.md, версия 1) отдаёт кадры swapchain из BAR2
//! (memfd хоста), syn-vm рисует их поверх кадра VNC в месте окна игры.
//!
//! Сокет `$XDG_RUNTIME_DIR/syn-gpu/<машина>.present.sock`; нет сокета — слоя нет, всё как
//! раньше. Сообщения: `u32 type, u32 len`, полезная нагрузка (little-endian, repr(C));
//! дескриптор BAR2 — SCM_RIGHTS при HELLO.

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
const RELEASE: u32 = 16;
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
    updated: Instant,
}

/// Клиент слоя одной машины.
pub struct Present {
    layers: Mutex<HashMap<u64, Layer>>,
    alive: AtomicBool,
}

impl Present {
    /// Слой, который показывать: последняя обновлённая видимая поверхность.
    pub fn visible(&self) -> Option<Layer> {
        self.layers.lock().unwrap().values().filter(|l| l.show).max_by_key(|l| l.updated).cloned()
    }

    pub fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}

fn socket(id: &str) -> PathBuf {
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    run.join("syn-gpu").join(format!("{id}.present.sock"))
}

/// Запустить клиента: ждёт сокет, переподключается; `on_change` — новый кадр или слой.
pub fn start(id: &str, on_change: Arc<dyn Fn() + Send + Sync>) -> Arc<Present> {
    let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true) });
    let me = p.clone();
    let path = socket(id);
    std::thread::Builder::new()
        .name("syn-gpu-present".into())
        .spawn(move || {
            while me.alive.load(Ordering::Relaxed) {
                if path.exists() {
                    match UnixStream::connect(&path) {
                        Ok(s) => {
                            if let Err(e) = run(&me, s, &on_change) {
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

fn run(p: &Present, mut s: UnixStream, on_change: &Arc<dyn Fn() + Send + Sync>) -> Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(1)))?;
    let mut heap: Option<Heap> = None;
    while p.alive.load(Ordering::Relaxed) {
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
                let pixels = heap.as_ref().and_then(|hp| copy_frame(hp, offset, width, height, stride, format));
                // Кадр прочитан (или негоден) — буфер свободен.
                send(&mut s, RELEASE, &[surface.to_le_bytes(), seq.to_le_bytes()].concat())?;
                if let Some(px) = pixels {
                    let mut layers = p.layers.lock().unwrap();
                    let l = layers.entry(surface).or_insert_with(|| Layer {
                        frame: LiveFrame::new(),
                        rect: (0, 0, 0, 0),
                        show: false,
                        fullscreen: false,
                        updated: Instant::now(),
                    });
                    l.frame.set(width, height, px.into());
                    l.rect = (x, y, w, h);
                    l.show = flags & 1 != 0;
                    l.fullscreen = flags & 2 != 0;
                    l.updated = Instant::now();
                    drop(layers);
                    on_change();
                }
            }
            GONE => {
                let surface = u64_at(&payload, 0);
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
        let p = Arc::new(Present { layers: Mutex::new(HashMap::new()), alive: AtomicBool::new(true) });
        let changed = Arc::new(AtomicBool::new(false));
        let ch = changed.clone();
        let p2 = p.clone();
        let t = std::thread::spawn(move || {
            let cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ch.store(true, Ordering::Relaxed));
            let _ = run(&p2, cli, &cb);
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
