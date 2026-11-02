//! Камера компьютера через V4L2 (`/dev/video*`, веб-камеры UVC): поток MJPEG или YUYV в буферах mmap,
//! кадр — NV12 (полный диапазон, как у камер телефона). Только libc: собирается и на телефоне.

use std::ffi::CStr;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::convert;

const fn fourcc(s: &[u8; 4]) -> u32 {
    s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16 | (s[3] as u32) << 24
}

pub const MJPG: u32 = fourcc(b"MJPG");
pub const YUYV: u32 = fourcc(b"YUYV");
pub const NV12: u32 = fourcc(b"NV12");

const BUF_TYPE_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const CAP_VIDEO_CAPTURE: u32 = 1;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;
const FRMSIZE_DISCRETE: u32 = 1;

#[repr(C)]
struct Capability {
    driver: [u8; 16],
    card: [u8; 32],
    bus_info: [u8; 32],
    version: u32,
    capabilities: u32,
    device_caps: u32,
    reserved: [u32; 3],
}

#[repr(C)]
struct FmtDesc {
    index: u32,
    ty: u32,
    flags: u32,
    description: [u8; 32],
    pixelformat: u32,
    mbus_code: u32,
    reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    priv_: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

#[repr(C, align(8))]
struct Format {
    ty: u32,
    /// Объединение `fmt` у ядра выровнено на 8 (в нём есть указатели).
    _pad: u32,
    pix: PixFormat,
    rest: [u8; 200 - std::mem::size_of::<PixFormat>()],
}

#[repr(C)]
struct RequestBuffers {
    count: u32,
    ty: u32,
    memory: u32,
    capabilities: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Buffer {
    index: u32,
    ty: u32,
    bytesused: u32,
    flags: u32,
    field: u32,
    timestamp: libc::timeval,
    timecode: [u32; 4],
    sequence: u32,
    memory: u32,
    m: u64,
    length: u32,
    reserved2: u32,
    request_fd: u32,
}

#[repr(C)]
struct StreamParm {
    ty: u32,
    capability: u32,
    capturemode: u32,
    num: u32,
    den: u32,
    rest: [u8; 200 - 16],
}

#[repr(C)]
struct FrmSizeEnum {
    index: u32,
    pixel_format: u32,
    ty: u32,
    w: u32,
    h: u32,
    rest: [u32; 4],
    reserved: [u32; 2],
}

const fn ioc(dir: u32, nr: u32, size: usize) -> libc::c_ulong {
    ((dir << 30) | ((size as u32) << 16) | ((b'V' as u32) << 8) | nr) as libc::c_ulong
}
const RW: u32 = 3;
const W: u32 = 1;
const R: u32 = 2;
const QUERYCAP: libc::c_ulong = ioc(R, 0, std::mem::size_of::<Capability>());
const ENUM_FMT: libc::c_ulong = ioc(RW, 2, std::mem::size_of::<FmtDesc>());
const S_FMT: libc::c_ulong = ioc(RW, 5, std::mem::size_of::<Format>());
const REQBUFS: libc::c_ulong = ioc(RW, 8, std::mem::size_of::<RequestBuffers>());
const QUERYBUF: libc::c_ulong = ioc(RW, 9, std::mem::size_of::<Buffer>());
const QBUF: libc::c_ulong = ioc(RW, 15, std::mem::size_of::<Buffer>());
const DQBUF: libc::c_ulong = ioc(RW, 17, std::mem::size_of::<Buffer>());
const STREAMON: libc::c_ulong = ioc(W, 18, 4);
const STREAMOFF: libc::c_ulong = ioc(W, 19, 4);
const S_PARM: libc::c_ulong = ioc(RW, 22, std::mem::size_of::<StreamParm>());
const ENUM_FRAMESIZES: libc::c_ulong = ioc(RW, 74, std::mem::size_of::<FrmSizeEnum>());

fn xioctl<T>(fd: i32, req: libc::c_ulong, arg: &mut T) -> io::Result<()> {
    loop {
        // SAFETY: ioctl с указателем на живую структуру нужного размера
        let r = unsafe { libc::ioctl(fd, req as _, arg as *mut T) };
        if r >= 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

fn zeroed<T>() -> T {
    // SAFETY: структуры V4L2 — простые данные, нули допустимы
    unsafe { std::mem::zeroed() }
}

fn cstr(b: &[u8]) -> String {
    CStr::from_bytes_until_nul(b).map(|c| c.to_string_lossy().into_owned()).unwrap_or_default()
}

fn open_dev(path: &Path) -> io::Result<OwnedFd> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
    // SAFETY: open с путём C
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: свежий дескриптор
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Камера V4L2.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub path: PathBuf,
    pub name: String,
    pub bus: String,
    /// Форматы кадров и их размеры (по убыванию площади).
    pub formats: Vec<(u32, Vec<(u32, u32)>)>,
}

impl Device {
    fn probe(path: &Path) -> Option<Device> {
        let fd = open_dev(path).ok()?;
        let mut cap: Capability = zeroed();
        xioctl(fd.as_raw_fd(), QUERYCAP, &mut cap).ok()?;
        let caps = if cap.capabilities & CAP_DEVICE_CAPS != 0 { cap.device_caps } else { cap.capabilities };
        if caps & CAP_VIDEO_CAPTURE == 0 {
            return None;
        }
        let mut formats = Vec::new();
        for i in 0.. {
            let mut d: FmtDesc = zeroed();
            d.index = i;
            d.ty = BUF_TYPE_CAPTURE;
            if xioctl(fd.as_raw_fd(), ENUM_FMT, &mut d).is_err() {
                break;
            }
            if d.pixelformat != MJPG && d.pixelformat != YUYV && d.pixelformat != NV12 {
                continue;
            }
            let mut sizes = Vec::new();
            for j in 0..64 {
                let mut s: FrmSizeEnum = zeroed();
                s.index = j;
                s.pixel_format = d.pixelformat;
                if xioctl(fd.as_raw_fd(), ENUM_FRAMESIZES, &mut s).is_err() || s.ty != FRMSIZE_DISCRETE {
                    break;
                }
                sizes.push((s.w, s.h));
            }
            sizes.sort_by_key(|(w, h)| std::cmp::Reverse(*w as u64 * *h as u64));
            sizes.dedup();
            formats.push((d.pixelformat, sizes));
        }
        if formats.is_empty() {
            return None;
        }
        Some(Device { path: path.to_path_buf(), name: cstr(&cap.card), bus: cstr(&cap.bus_info), formats })
    }

    /// Все размеры (любого формата), по убыванию площади.
    pub fn sizes(&self) -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = self.formats.iter().flat_map(|f| f.1.iter().copied()).collect();
        v.sort_by_key(|(w, h)| std::cmp::Reverse(*w as u64 * *h as u64));
        v.dedup();
        v
    }

    /// Наибольший размер с соотношением сторон `aw:ah` не шире `max_w`.
    pub fn size_for(&self, aw: u32, ah: u32, max_w: u32) -> Option<(u32, u32)> {
        self.sizes().into_iter().find(|(w, h)| *w <= max_w && *w as u64 * ah as u64 == *h as u64 * aw as u64)
    }

    /// Формат для размера: YUYV до 640×480 (без распаковки), больше — MJPEG (USB 2 не тянет сырой 720p30).
    fn pick(&self, w: u32, h: u32) -> Option<u32> {
        let has = |f: u32| self.formats.iter().any(|(ff, s)| *ff == f && s.contains(&(w, h)));
        if has(NV12) {
            return Some(NV12);
        }
        if w * h <= 640 * 480 && has(YUYV) {
            return Some(YUYV);
        }
        [MJPG, YUYV].into_iter().find(|f| has(*f))
    }
}

/// Камеры V4L2 (по одному узлу на устройство).
pub fn devices() -> Vec<Device> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev")
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("video"))).collect())
        .unwrap_or_default();
    nodes.sort_by_key(|p| p.file_name().and_then(|n| n.to_str()).and_then(|n| n[5..].parse::<u32>().ok()).unwrap_or(u32::MAX));
    let mut out: Vec<Device> = Vec::new();
    for p in nodes {
        if let Some(d) = Device::probe(&p) {
            if !out.iter().any(|o| o.bus == d.bus && !d.bus.is_empty()) {
                out.push(d);
            }
        }
    }
    out
}

/// Поток кадров камеры.
pub struct Stream {
    fd: OwnedFd,
    bufs: Vec<(*mut u8, usize)>,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    stride: u32,
    stop: AtomicBool,
}

// SAFETY: отображения только читаются; ioctl на дескрипторе потокобезопасны
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

impl Stream {
    /// Открыть камеру с ближайшим к `w × h` размером и частотой `fps`.
    pub fn open(dev: &Device, w: u32, h: u32, fps: u32) -> io::Result<Stream> {
        let (w, h) = dev
            .sizes()
            .into_iter()
            .min_by_key(|(sw, sh)| ((*sw as i64 - w as i64).abs() + (*sh as i64 - h as i64).abs(), std::cmp::Reverse(*sw)))
            .ok_or_else(|| io::Error::other("у камеры нет размеров кадра"))?;
        let format = dev.pick(w, h).ok_or_else(|| io::Error::other("нет подходящего формата кадров"))?;
        let fd = open_dev(&dev.path)?;
        let raw = fd.as_raw_fd();
        let mut f: Format = zeroed();
        f.ty = BUF_TYPE_CAPTURE;
        f.pix.width = w;
        f.pix.height = h;
        f.pix.pixelformat = format;
        f.pix.field = 1; // V4L2_FIELD_NONE
        xioctl(raw, S_FMT, &mut f)?;
        let mut p: StreamParm = zeroed();
        p.ty = BUF_TYPE_CAPTURE;
        p.num = 1;
        p.den = fps.max(1);
        let _ = xioctl(raw, S_PARM, &mut p);
        let mut rb = RequestBuffers { count: 4, ty: BUF_TYPE_CAPTURE, memory: MEMORY_MMAP, capabilities: 0, flags: 0 };
        xioctl(raw, REQBUFS, &mut rb)?;
        let mut bufs = Vec::new();
        for i in 0..rb.count {
            let mut b: Buffer = zeroed();
            b.index = i;
            b.ty = BUF_TYPE_CAPTURE;
            b.memory = MEMORY_MMAP;
            xioctl(raw, QUERYBUF, &mut b)?;
            // SAFETY: отображение буфера драйвера по его смещению
            let ptr = unsafe { libc::mmap(std::ptr::null_mut(), b.length as usize, libc::PROT_READ, libc::MAP_SHARED, raw, (b.m & 0xffff_ffff) as libc::off_t) };
            if ptr == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            bufs.push((ptr as *mut u8, b.length as usize));
            xioctl(raw, QBUF, &mut b)?;
        }
        let mut t = BUF_TYPE_CAPTURE as i32;
        xioctl(raw, STREAMON, &mut t)?;
        let stride = if f.pix.bytesperline > 0 { f.pix.bytesperline } else { w * 2 };
        Ok(Stream { fd, bufs, width: f.pix.width & !1, height: f.pix.height & !1, format: f.pix.pixelformat, stride, stop: AtomicBool::new(false) })
    }

    /// Следующий кадр в NV12 (`width × height`, без отступов) и его время, нс (CLOCK_MONOTONIC);
    /// `None` — поток остановлен (`shutdown`). Битый кадр MJPEG пропускается.
    pub fn next_nv12(&self, out: &mut Vec<u8>) -> io::Result<Option<u64>> {
        let raw = self.fd.as_raw_fd();
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let mut pfd = libc::pollfd { fd: raw, events: libc::POLLIN, revents: 0 };
            // SAFETY: poll одного дескриптора
            let n = unsafe { libc::poll(&mut pfd, 1, 200) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if n == 0 {
                continue;
            }
            if pfd.revents & (libc::POLLERR | libc::POLLHUP) != 0 && pfd.revents & libc::POLLIN == 0 {
                return Err(io::Error::other("камера отключилась"));
            }
            let mut b: Buffer = zeroed();
            b.ty = BUF_TYPE_CAPTURE;
            b.memory = MEMORY_MMAP;
            match xioctl(raw, DQBUF, &mut b) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            }
            let ts = b.timestamp.tv_sec as u64 * 1_000_000_000 + b.timestamp.tv_usec as u64 * 1000;
            let (ptr, len) = self.bufs[b.index as usize];
            // SAFETY: буфер у нас до QBUF
            let data = unsafe { std::slice::from_raw_parts(ptr, (b.bytesused as usize).min(len)) };
            let (w, h) = (self.width as usize, self.height as usize);
            out.resize(w * h * 3 / 2, 0);
            let ok = match self.format {
                YUYV => {
                    convert::yuyv_to_nv12(data, self.stride as usize, w, h, out);
                    true
                }
                NV12 => {
                    let s = self.stride as usize;
                    for y in 0..h {
                        out[y * w..(y + 1) * w].copy_from_slice(&data[y * s..y * s + w]);
                    }
                    let uv = s * self.height as usize;
                    for y in 0..h / 2 {
                        out[w * h + y * w..w * h + (y + 1) * w].copy_from_slice(&data[uv + y * s..uv + y * s + w]);
                    }
                    true
                }
                _ => match decode_mjpeg(data) {
                    Some(rgb) if rgb.width() as usize >= w && rgb.height() as usize >= h => {
                        convert::rgb_to_nv12(rgb.as_raw(), w, h, out);
                        true
                    }
                    _ => false,
                },
            };
            let _ = xioctl(raw, QBUF, &mut b);
            if ok {
                return Ok(Some(ts));
            }
        }
    }

    /// Разбудить `next_nv12` в другом потоке.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let mut t = BUF_TYPE_CAPTURE as i32;
        let _ = xioctl(self.fd.as_raw_fd(), STREAMOFF, &mut t);
        for (p, len) in &self.bufs {
            // SAFETY: отображения из open
            unsafe { libc::munmap(*p as *mut libc::c_void, *len) };
        }
    }
}

/// Таблицы Хаффмана по умолчанию (JPEG, приложение K): веб-камеры часто шлют MJPEG без DHT.
const DHT: &[u8] = &[
    0xFF, 0xC4, 0x01, 0xA2, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x01, 0x00, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02,
    0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13,
    0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
    0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44,
    0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73,
    0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
    0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5,
    0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9,
    0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0x11, 0x00, 0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04,
    0x04, 0x00, 0x01, 0x02, 0x77, 0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22,
    0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0, 0x15, 0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1,
    0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77,
    0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3,
    0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8,
    0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4,
    0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA,
];

/// Есть ли в JPEG таблица Хаффмана до начала скана.
fn has_dht(d: &[u8]) -> bool {
    let mut i = 2;
    while i + 4 <= d.len() && d[i] == 0xFF {
        let m = d[i + 1];
        if m == 0xC4 {
            return true;
        }
        if m == 0xDA {
            return false;
        }
        i += 2 + u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
    }
    false
}

fn decode_mjpeg(data: &[u8]) -> Option<image::RgbImage> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let fixed;
    let data = if has_dht(data) {
        data
    } else {
        let mut v = Vec::with_capacity(data.len() + DHT.len());
        v.extend_from_slice(&data[..2]);
        v.extend_from_slice(DHT);
        v.extend_from_slice(&data[2..]);
        fixed = v;
        &fixed
    };
    image::load_from_memory_with_format(data, image::ImageFormat::Jpeg).ok().map(|i| i.to_rgb8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes() {
        assert_eq!(std::mem::size_of::<Capability>(), 104);
        assert_eq!(std::mem::size_of::<FmtDesc>(), 64);
        assert_eq!(std::mem::size_of::<Format>(), 208);
        assert_eq!(std::mem::size_of::<RequestBuffers>(), 20);
        assert_eq!(std::mem::size_of::<Buffer>(), 88);
        assert_eq!(std::mem::size_of::<StreamParm>(), 204);
        assert_eq!(std::mem::size_of::<FrmSizeEnum>(), 44);
        assert_eq!(DHT.len(), 420);
    }
}
