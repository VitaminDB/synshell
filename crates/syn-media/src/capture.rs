//! Источник кадров: служба камеры syncamd (телефон; кадры — dma-buf службы, возвращаются ей, когда кадр
//! больше не нужен), камера media controller с сырым Bayer и программным ISP (mainline-ядра телефонов,
//! [`crate::mcam`]) или камера V4L2 (компьютер); у двух последних кадр — свой буфер NV12. Кадр отдаёт данные через
//! [`CamFrame::with_src`] и возвращает буфер источнику, когда его отпускают (`Drop`).

use std::io;
use std::sync::{Arc, Mutex};

use crate::convert::Src;
use crate::syncamd::{self, Control, Meta};
use crate::{mcam, v4l2};

/// Номера камер без службы syncamd (у службы свои, с нуля).
pub const LOCAL_ID: u32 = 100;

/// Кадр источника.
pub trait CamFrame: Send {
    /// Время кадра, нс (отсчёт источника; для записи важна только разница).
    fn ts_ns(&self) -> u64;
    /// Данные кадра (NV12 или NV21) на время вызова.
    fn with_src(&self, f: &mut dyn FnMut(&Src));
}

pub enum CapEvent {
    Frame(Box<dyn CamFrame>),
    Meta(Meta),
    Stopped,
}

/// Открытый поток кадров.
pub trait Capture: Send + Sync {
    /// Следующее событие (блокирует).
    fn next(&self) -> io::Result<CapEvent>;
    /// Размер кадра.
    fn size(&self) -> (u32, u32);
    /// Управление (зум, фокус, EV…) — у камер телефона.
    fn control(&self, _c: &Control) {}
    /// Разбудить `next` в другом потоке и закрыть поток.
    fn shutdown(&self);
    /// Поток службы syncamd (снимок, точный формат).
    fn syncamd(&self) -> Option<&Arc<syncamd::Stream>> {
        None
    }
}

/// Камера: телефона (syncamd), сырой Bayer через media controller или компьютера (V4L2).
#[derive(Clone, Debug, PartialEq)]
pub enum Cam {
    Syncamd(syncamd::Camera),
    Mcam(mcam::Device),
    /// Камера V4L2 и её номер.
    V4l2(v4l2::Device, u32),
}

impl Cam {
    /// Фронтальная: у телефона — по службе, у компьютера встроенная веб-камера смотрит на человека.
    pub fn front(&self) -> bool {
        match self {
            Cam::Syncamd(c) => c.front(),
            Cam::Mcam(d) => d.front,
            Cam::V4l2(..) => true,
        }
    }

    /// Номер камеры (у службы — её, у остальных — от [`LOCAL_ID`]).
    pub fn id(&self) -> u32 {
        match self {
            Cam::Syncamd(c) => c.id,
            Cam::Mcam(d) => d.caps.id,
            Cam::V4l2(_, id) => *id,
        }
    }

    /// Снимок делает служба (JPEG с датчика полного размера); иначе снимок — кадр потока.
    pub fn service_snapshot(&self) -> bool {
        matches!(self, Cam::Syncamd(_))
    }

    /// Возможности в виде описания камеры syncamd: размеры потока и снимка, EV, ISO, выдержка, фокус…
    /// (интерфейс камеры один для всех источников).
    pub fn caps(&self) -> syncamd::Camera {
        match self {
            Cam::Syncamd(c) => *c,
            Cam::Mcam(d) => d.caps,
            Cam::V4l2(d, id) => {
                // SAFETY: структура из целых и float, нули допустимы
                let mut c: syncamd::Camera = unsafe { std::mem::zeroed() };
                c.id = *id;
                c.facing = syncamd::FACING_FRONT;
                let sizes = d.sizes();
                let n = sizes.len().min(c.sizes.len());
                c.nsizes = n as u32;
                for (i, s) in sizes.iter().take(n).enumerate() {
                    c.sizes[i] = syncamd::Size { width: s.0, height: s.1, min_frame_ns: 33_333_333 };
                }
                let nj = sizes.len().min(c.jpeg.len());
                c.njpeg = nj as u32;
                for (i, s) in sizes.iter().take(nj).enumerate() {
                    c.jpeg[i] = [s.0, s.1];
                }
                if let Some(&(w, h)) = sizes.first() {
                    (c.sensor_width, c.sensor_height, c.photo_width, c.photo_height, c.full_width, c.full_height) = (w, h, w, h, w, h);
                    c.active = [0, 0, w as i32, h as i32];
                }
                c.max_zoom = 1.0;
                c
            }
        }
    }

    pub fn name(&self) -> String {
        match self {
            Cam::Syncamd(c) => format!("{} {}", if c.front() { "front" } else { "back" }, c.id),
            Cam::Mcam(d) => d.name.clone(),
            Cam::V4l2(d, _) => d.name.clone(),
        }
    }

    /// Наибольший размер кадра видео с соотношением `aw:ah` не шире `max_w`.
    pub fn video_size(&self, aw: u32, ah: u32, max_w: u32) -> Option<(u32, u32)> {
        match self {
            Cam::Syncamd(c) => c.yuv_for(aw, ah, max_w),
            Cam::Mcam(d) => d.size_for(aw, ah, max_w),
            Cam::V4l2(d, _) => d.size_for(aw, ah, max_w),
        }
    }

    /// Поворот датчика относительно экрана в естественном положении (по часовой): превью и запись
    /// поворачиваются на него. Камеры компьютера — без поворота.
    pub fn sensor_rotation(&self) -> u32 {
        match self {
            Cam::Syncamd(c) => c.orientation.rem_euclid(360) as u32,
            Cam::Mcam(d) => d.rotation,
            Cam::V4l2(..) => 0,
        }
    }
}

/// Камеры этой машины: служба syncamd, если она есть, иначе сенсоры media controller и камеры V4L2.
pub fn cameras() -> Vec<Cam> {
    if std::path::Path::new(syncamd::SOCKET).exists() {
        if let Ok(c) = syncamd::cameras() {
            if !c.is_empty() {
                return c.into_iter().map(Cam::Syncamd).collect();
            }
        }
    }
    let mut v: Vec<Cam> = mcam::devices(LOCAL_ID).into_iter().map(Cam::Mcam).collect();
    let n = v.len() as u32;
    v.extend(v4l2::devices().into_iter().enumerate().map(|(i, d)| Cam::V4l2(d, LOCAL_ID + n + i as u32)));
    v
}

/// Фронтальная камера (для видео себя), иначе первая.
pub fn front_camera() -> Option<Cam> {
    let all = cameras();
    all.iter().find(|c| c.front()).cloned().or_else(|| all.into_iter().next())
}

pub struct OpenError {
    /// Камера занята другой программой.
    pub busy: bool,
    pub text: String,
}

/// Открыть поток кадров камеры.
pub fn open(cam: &Cam, w: u32, h: u32, fps: u32, flags: u32, jpeg: Option<(u32, u32)>) -> Result<Arc<dyn Capture>, OpenError> {
    match cam {
        Cam::Syncamd(c) => {
            let mut r = syncamd::Stream::open(c.id, w, h, fps, flags, jpeg);
            if let (Err(e), Some(_)) = (&r, jpeg) {
                // поток JPEG рядом с большим видео HAL может не принять — без него (снимок перенастроит)
                if !e.busy {
                    r = syncamd::Stream::open(c.id, w, h, fps, flags, None);
                }
            }
            r.map(|s| Arc::new(SyncamdCapture(Arc::new(s))) as Arc<dyn Capture>).map_err(|e| OpenError { busy: e.busy, text: e.text })
        }
        Cam::Mcam(d) => match mcam::Stream::open(d, w, h, flags & syncamd::OPEN_VIDEO != 0) {
            Ok(s) => Ok(Arc::new(McamCapture { s, pool: Arc::new(Mutex::new(Vec::new())) })),
            Err(e) => Err(OpenError { busy: e.busy, text: format!("{}: {}", d.name, e.text) }),
        },
        Cam::V4l2(d, _) => match v4l2::Stream::open(d, w, h, fps) {
            Ok(s) => Ok(Arc::new(V4l2Capture { s, pool: Arc::new(Mutex::new(Vec::new())) })),
            Err(e) => Err(OpenError { busy: e.raw_os_error() == Some(libc::EBUSY), text: format!("{}: {e}", d.path.display()) }),
        },
    }
}

// ─── syncamd ────────────────────────────────────────────────────────────────

pub struct SyncamdCapture(pub Arc<syncamd::Stream>);

struct SyncamdFrame {
    s: Arc<syncamd::Stream>,
    f: syncamd::FrameRef,
}

impl CamFrame for SyncamdFrame {
    fn ts_ns(&self) -> u64 {
        self.f.ts_ns
    }

    fn with_src(&self, f: &mut dyn FnMut(&Src)) {
        let info = self.s.info;
        self.s.begin(self.f.buf);
        f(&Src {
            data: self.s.data(self.f.buf),
            width: info.width as usize,
            height: info.height as usize,
            stride: info.stride as usize,
            scanlines: info.scanlines as usize,
            vu: info.format == syncamd::FMT_NV21,
        });
        self.s.end(self.f.buf);
    }
}

impl Drop for SyncamdFrame {
    fn drop(&mut self) {
        self.s.release(self.f.buf);
    }
}

impl Capture for SyncamdCapture {
    fn next(&self) -> io::Result<CapEvent> {
        Ok(match self.0.next()? {
            syncamd::Event::Frame(f) => CapEvent::Frame(Box::new(SyncamdFrame { s: self.0.clone(), f })),
            syncamd::Event::Meta(m) => CapEvent::Meta(m),
            syncamd::Event::Stopped => CapEvent::Stopped,
        })
    }

    fn size(&self) -> (u32, u32) {
        (self.0.info.width, self.0.info.height)
    }

    fn control(&self, c: &Control) {
        self.0.control(c);
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }

    fn syncamd(&self) -> Option<&Arc<syncamd::Stream>> {
        Some(&self.0)
    }
}

// ─── V4L2 ───────────────────────────────────────────────────────────────────

struct V4l2Capture {
    s: v4l2::Stream,
    /// Свободные буферы NV12 (кадр, отпущенный записью или превью, возвращается сюда).
    pool: Arc<Mutex<Vec<Vec<u8>>>>,
}

struct V4l2Frame {
    data: Vec<u8>,
    w: u32,
    h: u32,
    ts: u64,
    pool: Option<Arc<Mutex<Vec<Vec<u8>>>>>,
}

impl CamFrame for V4l2Frame {
    fn ts_ns(&self) -> u64 {
        self.ts
    }

    fn with_src(&self, f: &mut dyn FnMut(&Src)) {
        f(&Src { data: &self.data, width: self.w as usize, height: self.h as usize, stride: self.w as usize, scanlines: self.h as usize, vu: false });
    }
}

impl Drop for V4l2Frame {
    fn drop(&mut self) {
        if let Some(p) = &self.pool {
            let mut p = p.lock().unwrap();
            if p.len() < 4 {
                p.push(std::mem::take(&mut self.data));
            }
        }
    }
}

impl Capture for V4l2Capture {
    fn next(&self) -> io::Result<CapEvent> {
        let mut buf = self.pool.lock().unwrap().pop().unwrap_or_default();
        match self.s.next_nv12(&mut buf)? {
            Some(ts) => Ok(CapEvent::Frame(Box::new(V4l2Frame { data: buf, w: self.s.width, h: self.s.height, ts, pool: Some(self.pool.clone()) }))),
            None => Ok(CapEvent::Stopped),
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.s.width, self.s.height)
    }

    fn shutdown(&self) {
        self.s.shutdown();
    }
}

// ─── media controller (сырой Bayer) ─────────────────────────────────────────

struct McamCapture {
    s: mcam::Stream,
    pool: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Capture for McamCapture {
    fn next(&self) -> io::Result<CapEvent> {
        if let Some(m) = self.s.take_meta() {
            return Ok(CapEvent::Meta(m));
        }
        let mut buf = self.pool.lock().unwrap().pop().unwrap_or_default();
        match self.s.next_nv12(&mut buf)? {
            Some(ts) => Ok(CapEvent::Frame(Box::new(V4l2Frame { data: buf, w: self.s.width, h: self.s.height, ts, pool: Some(self.pool.clone()) }))),
            None => Ok(CapEvent::Stopped),
        }
    }

    fn size(&self) -> (u32, u32) {
        (self.s.width, self.s.height)
    }

    fn control(&self, c: &Control) {
        self.s.control(c);
    }

    fn shutdown(&self) {
        self.s.shutdown();
    }
}
