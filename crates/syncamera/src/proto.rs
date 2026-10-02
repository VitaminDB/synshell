//! Протокол службы камеры syncamd платформы (arch-mobile-port `camera/syncam.h`, v3): сокет
//! `/run/syncam/syncam.sock` (SOCK_SEQPACKET, группа video), одно сообщение — одна структура C.
//!
//! Поток кадров: HELLO → CAMERAS, OPEN → STARTED + dma-buf буферов (SCM_RIGHTS), дальше FRAME/RELEASE
//! по номеру буфера; CONTROL — управление (зум, точка фокуса, EV…), META — состояние 3A. Снимок —
//! SNAPSHOT → PHOTO + memfd с JPEG, отдельным соединением (ответ не перемешивается с кадрами).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

pub const SOCKET: &str = "/run/syncam/syncam.sock";
const MAGIC: u32 = 0x6d61_6373;
const VERSION: u32 = 4;
const MAX_CAMS: usize = 8;
const MAX_SIZES: usize = 32;

pub const SC_HELLO: u32 = 1;
pub const SC_CAMERAS: u32 = 2;
pub const SC_OPEN: u32 = 3;
pub const SC_STARTED: u32 = 4;
pub const SC_ERROR: u32 = 5;
pub const SC_FRAME: u32 = 6;
pub const SC_RELEASE: u32 = 7;
pub const SC_STOPPED: u32 = 8;
pub const SC_SNAPSHOT: u32 = 11;
pub const SC_PHOTO: u32 = 12;
pub const SC_CONTROL: u32 = 13;
pub const SC_META: u32 = 14;

pub const FLASH_OFF: u32 = 0;

pub const FMT_NV21: u32 = 1;
pub const ERR_BUSY: u32 = 1;
pub const FACING_FRONT: i32 = 1;

pub const OPEN_VIDEO: u32 = 1;
pub const OPEN_META: u32 = 2;
pub const SNAP_NO_AF: u32 = 1;

pub const CTL_ZOOM: u32 = 1;
pub const CTL_POINT: u32 = 2;
pub const CTL_EV: u32 = 4;
pub const CTL_TORCH: u32 = 8;
pub const CTL_MODE: u32 = 16;
pub const CTL_AE_LOCK: u32 = 32;
pub const CTL_AWB: u32 = 64;
pub const CTL_MANUAL: u32 = 128;
pub const CTL_STAB: u32 = 256;

pub const MODE_PHOTO: u32 = 0;
pub const MODE_VIDEO: u32 = 1;
pub const MODE_NIGHT: u32 = 2;

pub const AF_FOCUSED: u32 = 2;
pub const AF_FAILED: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
    pub min_frame_ns: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub id: u32,
    pub facing: i32,
    pub orientation: i32,
    pub logical: u32,
    pub sensor_width: u32,
    pub sensor_height: u32,
    pub focal_um: u32,
    pub flash: u32,
    pub nsizes: u32,
    pub sizes: [Size; MAX_SIZES],
    pub photo_width: u32,
    pub photo_height: u32,
    pub full_width: u32,
    pub full_height: u32,
    pub njpeg: u32,
    pub jpeg: [[u32; 2]; 16],
    pub max_zoom: f32,
    pub ev_min: i32,
    pub ev_max: i32,
    pub ev_step_num: i32,
    pub ev_step_den: i32,
    pub active: [i32; 4],
    pub af: u32,
    pub stabilization: u32,
    pub iso_min: i32,
    pub iso_max: i32,
    pub exposure_min: i64,
    pub exposure_max: i64,
    pub min_focus: f32,
    pub scenes: u32,
    pub nfull: u32,
    pub full: [[u32; 2]; 8],
}

impl Camera {
    pub fn front(&self) -> bool {
        self.facing == FACING_FRONT
    }

    pub fn sizes(&self) -> &[Size] {
        &self.sizes[..(self.nsizes as usize).min(MAX_SIZES)]
    }

    pub fn jpegs(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.jpeg[..(self.njpeg as usize).min(16)].iter().map(|j| (j[0], j[1]))
    }

    /// Наибольший выход YUV с соотношением сторон `aw:ah`, не больше `max_w` по ширине.
    pub fn yuv_for(&self, aw: u32, ah: u32, max_w: u32) -> Option<(u32, u32)> {
        self.sizes()
            .iter()
            .filter(|s| s.width <= max_w && s.width as u64 * ah as u64 == s.height as u64 * aw as u64)
            .max_by_key(|s| s.width)
            .map(|s| (s.width, s.height))
    }

    /// Наибольший JPEG с соотношением сторон `aw:ah`.
    pub fn jpeg_for(&self, aw: u32, ah: u32) -> Option<(u32, u32)> {
        self.jpegs().filter(|(w, h)| *w as u64 * ah as u64 == *h as u64 * aw as u64).max_by_key(|(w, h)| *w as u64 * *h as u64)
    }

    /// Размеры фото 4:3 от меньшего к большему: обычный JPEG и все полного разрешения (50, 108, 200 Мп…).
    pub fn photo_sizes(&self) -> Vec<(u32, u32)> {
        let mut v: Vec<(u32, u32)> = self.jpeg_for(4, 3).into_iter().collect();
        for f in self.full[..(self.nfull as usize).min(8)].iter().rev() {
            if !v.contains(&(f[0], f[1])) {
                v.push((f[0], f[1]));
            }
        }
        v.sort_by_key(|(w, h)| *w as u64 * *h as u64);
        v
    }

    /// Размер фото 4:3 для выбора в мегапикселях (0 — обычный); ближайший из доступных.
    pub fn photo_size_for(&self, mp: u32) -> Option<(u32, u32)> {
        let v = self.photo_sizes();
        if mp == 0 {
            return v.first().copied();
        }
        v.into_iter().min_by_key(|s| (megapixels(*s) as i64 - mp as i64).abs())
    }

    pub fn ev_step(&self) -> f32 {
        if self.ev_step_den == 0 {
            0.0
        } else {
            self.ev_step_num as f32 / self.ev_step_den as f32
        }
    }
}

#[repr(C)]
struct Hello {
    ty: u32,
    magic: u32,
    version: u32,
}

#[repr(C)]
struct Cameras {
    ty: u32,
    n: u32,
    cams: [Camera; MAX_CAMS],
}

#[repr(C)]
struct Open {
    ty: u32,
    camera: u32,
    width: u32,
    height: u32,
    fps: u32,
    flags: u32,
    jpeg_width: u32,
    jpeg_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Started {
    pub ty: u32,
    pub camera: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub scanlines: u32,
    pub format: u32,
    pub size: u32,
    pub fps: u32,
    pub nbuf: u32,
}

#[repr(C)]
struct ErrorMsg {
    ty: u32,
    code: u32,
    text: [u8; 96],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Frame {
    ty: u32,
    buf: u32,
    seq: u32,
    pad: u32,
    ts_ns: u64,
}

#[repr(C)]
struct Release {
    ty: u32,
    buf: u32,
}

#[repr(C)]
struct Snapshot {
    ty: u32,
    camera: u32,
    flash: u32,
    width: u32,
    height: u32,
    rotation: i32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Photo {
    ty: u32,
    size: u32,
    width: u32,
    height: u32,
}

/// Управление камерой (SC_CONTROL); применяются поля из `mask`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Control {
    ty: u32,
    pub mask: u32,
    pub zoom: f32,
    pub x: f32,
    pub y: f32,
    pub ev: i32,
    pub torch: u32,
    pub mode: u32,
    pub ae_lock: u32,
    pub awb: u32,
    pub iso: i32,
    pad: u32,
    pub exposure_ns: i64,
    pub focus: f32,
    pub stabilization: u32,
}

impl Default for Control {
    fn default() -> Self {
        Self {
            ty: SC_CONTROL,
            mask: 0,
            zoom: 1.0,
            x: -1.0,
            y: -1.0,
            ev: 0,
            torch: 0,
            mode: MODE_PHOTO,
            ae_lock: 0,
            awb: 1,
            iso: 0,
            pad: 0,
            exposure_ns: 0,
            focus: -1.0,
            stabilization: 0,
        }
    }
}

/// Мегапиксели «как в описании телефона»: 16384×12288 — 200, а не 201; 12000×9000 — 108; 4096×3072 — 12.
pub fn megapixels((w, h): (u32, u32)) -> u32 {
    let px = w as u64 * h as u64;
    if px < 50_000_000 {
        return (px / 1_000_000) as u32;
    }
    let mp = ((px + 500_000) / 1_000_000) as u32;
    let r10 = (mp + 5) / 10 * 10;
    if mp >= 50 && (mp as i64 - r10 as i64).abs() * 100 <= r10 as i64 {
        r10
    } else {
        mp
    }
}

/// Состояние 3A (SC_META).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Meta {
    ty: u32,
    pub af: u32,
    pub ae_state: u32,
    pub iso: i32,
    pub exposure_ns: i64,
    pub focus: f32,
    pad: u32,
}

fn as_bytes<T>(v: &T) -> &[u8] {
    // SAFETY: структуры repr(C) из целых и float, читаем ровно их размер
    unsafe { std::slice::from_raw_parts(v as *const T as *const u8, std::mem::size_of::<T>()) }
}

fn connect() -> io::Result<OwnedFd> {
    // SAFETY: обычные вызовы сокетов; дескриптор сразу во владении OwnedFd
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = OwnedFd::from_raw_fd(fd);
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (d, s) in addr.sun_path.iter_mut().zip(SOCKET.bytes()) {
            *d = s as libc::c_char;
        }
        let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        if libc::connect(fd.as_raw_fd(), &addr as *const _ as *const libc::sockaddr, len) < 0 {
            return Err(io::Error::other(format!("служба камеры недоступна: {}", io::Error::last_os_error())));
        }
        Ok(fd)
    }
}

fn send<T>(fd: RawFd, msg: &T) -> io::Result<()> {
    let b = as_bytes(msg);
    // SAFETY: буфер живёт на время вызова
    let n = unsafe { libc::send(fd, b.as_ptr().cast(), b.len(), libc::MSG_NOSIGNAL) };
    if n == b.len() as isize {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Принять сообщение и дескрипторы SCM_RIGHTS.
fn recv(fd: RawFd, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> io::Result<usize> {
    let mut ctl = [0u8; 512];
    loop {
        // SAFETY: буферы живут на время вызова; дескрипторы из CMSG сразу во владении OwnedFd
        unsafe {
            let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
            let mut mh: libc::msghdr = std::mem::zeroed();
            mh.msg_iov = &mut iov;
            mh.msg_iovlen = 1;
            mh.msg_control = ctl.as_mut_ptr().cast();
            mh.msg_controllen = ctl.len() as _;
            let n = libc::recvmsg(fd, &mut mh, libc::MSG_CMSG_CLOEXEC);
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            let mut cm = libc::CMSG_FIRSTHDR(&mh);
            while !cm.is_null() {
                if (*cm).cmsg_level == libc::SOL_SOCKET && (*cm).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(cm) as *const RawFd;
                    let count = ((*cm).cmsg_len as usize - libc::CMSG_LEN(0) as usize) / std::mem::size_of::<RawFd>();
                    for i in 0..count {
                        fds.push(OwnedFd::from_raw_fd(*data.add(i)));
                    }
                }
                cm = libc::CMSG_NXTHDR(&mh, cm);
            }
            return Ok(n as usize);
        }
    }
}

fn read_struct<T: Copy>(buf: &[u8]) -> T {
    // SAFETY: вызывающий проверил длину; read_unaligned — буфер байтов без выравнивания
    unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const T) }
}

fn error_text(buf: &[u8], n: usize) -> (u32, String) {
    if n >= std::mem::size_of::<ErrorMsg>() {
        let e: ErrorMsg = unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const ErrorMsg) };
        let end = e.text.iter().position(|&c| c == 0).unwrap_or(e.text.len());
        (e.code, String::from_utf8_lossy(&e.text[..end]).into_owned())
    } else {
        (0, "ошибка службы камеры".into())
    }
}

fn hello(fd: RawFd) -> io::Result<Vec<Camera>> {
    send(fd, &Hello { ty: SC_HELLO, magic: MAGIC, version: VERSION })?;
    let mut buf = vec![0u8; std::mem::size_of::<Cameras>() + 64];
    let mut fds = Vec::new();
    let n = recv(fd, &mut buf, &mut fds)?;
    if n == 0 {
        return Err(io::Error::other("служба камеры другой версии (обновите syncamd)"));
    }
    if n < std::mem::size_of::<Cameras>() || u32::from_ne_bytes(buf[..4].try_into().unwrap()) != SC_CAMERAS {
        return Err(io::Error::other("неожиданный ответ службы камеры"));
    }
    // SAFETY: длина проверена
    let list: Box<Cameras> = unsafe {
        let mut b = Box::<Cameras>::new_uninit();
        std::ptr::copy_nonoverlapping(buf.as_ptr(), b.as_mut_ptr() as *mut u8, std::mem::size_of::<Cameras>());
        b.assume_init()
    };
    Ok(list.cams[..(list.n as usize).min(MAX_CAMS)].to_vec())
}

/// Список камер.
pub fn cameras() -> io::Result<Vec<Camera>> {
    let fd = connect()?;
    hello(fd.as_raw_fd())
}

/// Кадр потока: буфер `buf` занят, пока его не вернули `Stream::release`.
#[derive(Clone, Copy, Debug)]
pub struct FrameRef {
    pub buf: u32,
    pub ts_ns: u64,
}

pub enum Event {
    Frame(FrameRef),
    Meta(Meta),
    Stopped,
}

/// Поток кадров камеры (одно соединение, подписка).
pub struct Stream {
    fd: OwnedFd,
    pub info: Started,
    /// Отображения буферов (только чтение).
    maps: Vec<(*mut u8, usize, OwnedFd)>,
}

// SAFETY: отображения только читаются, дескриптор сокета потокобезопасен
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

#[repr(C)]
struct DmaBufSync {
    flags: u64,
}
const DMA_BUF_SYNC_READ: u64 = 1;
const DMA_BUF_SYNC_START: u64 = 0;
const DMA_BUF_SYNC_END: u64 = 4;
const DMA_BUF_SYNC_WRITE: u64 = 2;
// _IOW('b', 0, struct dma_buf_sync)
const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;

pub fn dma_sync(fd: RawFd, start: bool, write: bool) {
    let mut s = DmaBufSync {
        flags: (if write { DMA_BUF_SYNC_WRITE } else { DMA_BUF_SYNC_READ }) | if start { DMA_BUF_SYNC_START } else { DMA_BUF_SYNC_END },
    };
    // SAFETY: ioctl с указателем на живую структуру
    unsafe {
        libc::ioctl(fd, DMA_BUF_IOCTL_SYNC as _, &mut s);
    }
}

pub struct OpenError {
    pub busy: bool,
    pub text: String,
}

impl Stream {
    /// Открыть камеру: размер и частота потока YUV, флаги OPEN_*, сразу ли с потоком JPEG.
    pub fn open(camera: u32, w: u32, h: u32, fps: u32, flags: u32, jpeg: Option<(u32, u32)>) -> Result<Stream, OpenError> {
        let err = |e: io::Error| OpenError { busy: false, text: e.to_string() };
        let fd = connect().map_err(err)?;
        hello(fd.as_raw_fd()).map_err(err)?;
        let (jw, jh) = jpeg.unwrap_or((0, 0));
        send(fd.as_raw_fd(), &Open { ty: SC_OPEN, camera, width: w, height: h, fps, flags, jpeg_width: jw, jpeg_height: jh }).map_err(err)?;
        let mut buf = [0u8; 256];
        let mut fds = Vec::new();
        let n = recv(fd.as_raw_fd(), &mut buf, &mut fds).map_err(err)?;
        let ty = if n >= 4 { u32::from_ne_bytes(buf[..4].try_into().unwrap()) } else { 0 };
        if ty == SC_ERROR {
            let (code, text) = error_text(&buf, n);
            return Err(OpenError { busy: code == ERR_BUSY, text });
        }
        if ty != SC_STARTED || n < std::mem::size_of::<Started>() {
            return Err(OpenError { busy: false, text: "камера не включилась".into() });
        }
        let info: Started = read_struct(&buf);
        if info.format != FMT_NV21 && info.format != 2 {
            return Err(OpenError { busy: false, text: format!("формат кадров {} не поддерживается", info.format) });
        }
        let mut maps = Vec::new();
        for f in fds {
            // SAFETY: отображение dma-buf только для чтения на размер буфера из STARTED
            let p = unsafe { libc::mmap(std::ptr::null_mut(), info.size as usize, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0) };
            if p == libc::MAP_FAILED {
                return Err(OpenError { busy: false, text: format!("mmap буфера: {}", io::Error::last_os_error()) });
            }
            maps.push((p as *mut u8, info.size as usize, f));
        }
        Ok(Stream { fd, info, maps })
    }

    /// Следующее событие (блокирует).
    pub fn next(&self) -> io::Result<Event> {
        let mut buf = [0u8; 128];
        loop {
            let mut fds = Vec::new();
            let n = recv(self.fd.as_raw_fd(), &mut buf, &mut fds)?;
            if n == 0 {
                return Ok(Event::Stopped);
            }
            let ty = u32::from_ne_bytes(buf[..4].try_into().unwrap());
            match ty {
                SC_FRAME if n >= std::mem::size_of::<Frame>() => {
                    let f: Frame = read_struct(&buf);
                    return Ok(Event::Frame(FrameRef { buf: f.buf, ts_ns: f.ts_ns }));
                }
                SC_META if n >= std::mem::size_of::<Meta>() => return Ok(Event::Meta(read_struct(&buf))),
                SC_STOPPED => return Ok(Event::Stopped),
                _ => {}
            }
        }
    }

    /// Данные буфера (между `begin` и `end` — синхронизация кэша dma-buf).
    pub fn data(&self, buf: u32) -> &[u8] {
        let (p, len, _) = &self.maps[buf as usize];
        // SAFETY: отображение живёт, пока жив Stream
        unsafe { std::slice::from_raw_parts(*p, *len) }
    }

    pub fn begin(&self, buf: u32) {
        dma_sync(self.maps[buf as usize].2.as_raw_fd(), true, false);
    }

    pub fn end(&self, buf: u32) {
        dma_sync(self.maps[buf as usize].2.as_raw_fd(), false, false);
    }

    pub fn release(&self, buf: u32) {
        let _ = send(self.fd.as_raw_fd(), &Release { ty: SC_RELEASE, buf });
    }

    pub fn control(&self, c: &Control) {
        let mut c = *c;
        c.ty = SC_CONTROL;
        let _ = send(self.fd.as_raw_fd(), &c);
    }

    /// Разбудить `next` в другом потоке: закрыть приём.
    pub fn shutdown(&self) {
        // SAFETY: shutdown живого сокета
        unsafe {
            libc::shutdown(self.fd.as_raw_fd(), libc::SHUT_RDWR);
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        for (p, len, _) in &self.maps {
            // SAFETY: отображения из open
            unsafe {
                libc::munmap(*p as *mut libc::c_void, *len);
            }
        }
    }
}

/// Снимок: JPEG целиком. `rotation` — поворот телефона по часовой, градусы.
pub fn snapshot(camera: u32, flash: u32, size: Option<(u32, u32)>, rotation: i32, flags: u32) -> io::Result<Vec<u8>> {
    let fd = connect()?;
    hello(fd.as_raw_fd())?;
    let (w, h) = size.unwrap_or((0, 0));
    send(fd.as_raw_fd(), &Snapshot { ty: SC_SNAPSHOT, camera, flash, width: w, height: h, rotation, flags })?;
    let mut buf = [0u8; 256];
    let mut fds = Vec::new();
    let n = recv(fd.as_raw_fd(), &mut buf, &mut fds)?;
    let ty = if n >= 4 { u32::from_ne_bytes(buf[..4].try_into().unwrap()) } else { 0 };
    if ty == SC_ERROR {
        return Err(io::Error::other(error_text(&buf, n).1));
    }
    if ty != SC_PHOTO || n < std::mem::size_of::<Photo>() || fds.is_empty() {
        return Err(io::Error::other("снимок не получен"));
    }
    let ph: Photo = read_struct(&buf);
    let mfd = &fds[0];
    let mut out = vec![0u8; ph.size as usize];
    let mut off = 0usize;
    while off < out.len() {
        // SAFETY: pread в живой буфер
        let r = unsafe { libc::pread(mfd.as_raw_fd(), out[off..].as_mut_ptr().cast(), out.len() - off, off as libc::off_t) };
        if r <= 0 {
            return Err(io::Error::other("снимок: чтение memfd"));
        }
        off += r as usize;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn megapixels_like_spec() {
        assert_eq!(super::megapixels((16384, 12288)), 200);
        assert_eq!(super::megapixels((12000, 9000)), 108);
        assert_eq!(super::megapixels((8192, 6144)), 50);
        assert_eq!(super::megapixels((4096, 3072)), 12);
        assert_eq!(super::megapixels((5184, 3904)), 20);
    }
}
