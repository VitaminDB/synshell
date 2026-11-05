//! Источник кадров: служба камеры syncamd (телефон; кадры — dma-buf службы, возвращаются ей, когда кадр
//! больше не нужен) или камера V4L2 (компьютер; кадр — свой буфер NV12). Кадр отдаёт данные через
//! [`CamFrame::with_src`] и возвращает буфер источнику, когда его отпускают (`Drop`).

use std::io;
use std::sync::{Arc, Mutex};

use crate::convert::Src;
use crate::syncamd::{self, Control, Meta};
use crate::v4l2;

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

/// Камера: телефона (syncamd) или компьютера (V4L2).
#[derive(Clone, Debug, PartialEq)]
pub enum Cam {
    Syncamd(syncamd::Camera),
    V4l2(v4l2::Device),
}

impl Cam {
    /// Фронтальная: у телефона — по службе, у компьютера встроенная веб-камера смотрит на человека.
    pub fn front(&self) -> bool {
        match self {
            Cam::Syncamd(c) => c.front(),
            Cam::V4l2(_) => true,
        }
    }

    pub fn name(&self) -> String {
        match self {
            Cam::Syncamd(c) => format!("{} {}", if c.front() { "front" } else { "back" }, c.id),
            Cam::V4l2(d) => d.name.clone(),
        }
    }

    /// Наибольший размер кадра видео с соотношением `aw:ah` не шире `max_w`.
    pub fn video_size(&self, aw: u32, ah: u32, max_w: u32) -> Option<(u32, u32)> {
        match self {
            Cam::Syncamd(c) => c.yuv_for(aw, ah, max_w),
            Cam::V4l2(d) => d.size_for(aw, ah, max_w),
        }
    }

    /// Поворот датчика относительно экрана в естественном положении (по часовой): превью и запись
    /// поворачиваются на него. Камеры компьютера — без поворота.
    pub fn sensor_rotation(&self) -> u32 {
        match self {
            Cam::Syncamd(c) => c.orientation.rem_euclid(360) as u32,
            Cam::V4l2(_) => 0,
        }
    }
}

/// Камеры этой машины: служба syncamd, если она есть, иначе V4L2.
pub fn cameras() -> Vec<Cam> {
    if std::path::Path::new(syncamd::SOCKET).exists() {
        if let Ok(c) = syncamd::cameras() {
            if !c.is_empty() {
                return c.into_iter().map(Cam::Syncamd).collect();
            }
        }
    }
    v4l2::devices().into_iter().map(Cam::V4l2).collect()
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
        Cam::V4l2(d) => match v4l2::Stream::open(d, w, h, fps) {
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
