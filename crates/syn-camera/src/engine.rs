//! Сеанс камеры «Камеры»: общий сеанс `syn_media::Engine` (превью, управление, запись) плюс своё поверх
//! отвода кадров — поиск QR-кодов (режим «Фото») и серия кадров полного размера («Ночь»).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use syngui::widgets::LiveFrame;
use syn_media::convert::Src;
use syn_media::recorder;
use syn_media::syncamd::{Camera, Control, Meta};

use crate::night;

/// Что открыть.
#[derive(Clone, Debug)]
pub struct OpenSpec {
    pub cam: Camera,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video: bool,
    /// Поток JPEG сразу (снимок без перенастройки).
    pub jpeg: Option<(u32, u32)>,
}

/// События сеанса для интерфейса (из фонового потока).
pub enum Note {
    /// Новый кадр превью.
    Frame,
    Meta(Meta),
    /// Камера включилась (размер кадра потока).
    Started(u32, u32),
    /// Состояние: занята, ошибка (текст), пусто — всё хорошо.
    Status(String),
    /// В кадре QR-код (содержимое).
    Qr(String),
}

struct Burst {
    want: usize,
    frames: Vec<night::Frame>,
    tx: std::sync::mpsc::Sender<(Vec<night::Frame>, bool)>,
}

struct Extra {
    /// Серия кадров полного размера («Ночь»): сколько нужно, собранные, куда отдать.
    burst: Mutex<Option<Burst>>,
    /// Искать QR-коды в кадре (режим «Фото»).
    qr: AtomicBool,
    qr_tx: Mutex<Option<std::sync::mpsc::SyncSender<(Vec<u8>, usize, usize)>>>,
    qr_tick: AtomicU32,
}

#[derive(Clone)]
pub struct Engine {
    e: syn_media::Engine,
    x: Arc<Extra>,
    note: Arc<dyn Fn(Note) + Send + Sync>,
}

impl PartialEq for Engine {
    fn eq(&self, o: &Self) -> bool {
        self.e == o.e
    }
}

impl Engine {
    pub fn new(note: impl Fn(Note) + Send + Sync + 'static) -> Engine {
        let note: Arc<dyn Fn(Note) + Send + Sync> = Arc::new(note);
        let n2 = note.clone();
        let e = syn_media::Engine::new(move |n| {
            n2(match n {
                syn_media::Note::Frame => Note::Frame,
                syn_media::Note::Meta(m) => Note::Meta(m),
                syn_media::Note::Started(w, h) => Note::Started(w, h),
                syn_media::Note::Status(s) => Note::Status(s),
            })
        });
        let x = Arc::new(Extra { burst: Mutex::new(None), qr: AtomicBool::new(false), qr_tx: Mutex::new(None), qr_tick: AtomicU32::new(0) });
        let xt = x.clone();
        e.set_tap(Some(Arc::new(move |src: &Src, _ts: u64| tap(&xt, src))));
        Engine { e, x, note }
    }

    /// Искать QR-коды (фоновый поток распознаёт уменьшенную яркость раз в ~0,3 с).
    pub fn set_qr(&self, on: bool) {
        self.x.qr.store(on, Ordering::Relaxed);
        let mut tx = self.x.qr_tx.lock().unwrap();
        if on && tx.is_none() {
            let (t, rx) = std::sync::mpsc::sync_channel::<(Vec<u8>, usize, usize)>(1);
            *tx = Some(t);
            let note = self.note.clone();
            std::thread::Builder::new()
                .name("qr".into())
                .spawn(move || {
                    while let Ok((y, w, h)) = rx.recv() {
                        if let Some(s) = detect_qr(&y, w, h) {
                            note(Note::Qr(s));
                        }
                    }
                })
                .ok();
        }
    }

    pub fn preview(&self) -> Arc<LiveFrame> {
        self.e.preview()
    }

    pub fn frame_taken(&self) {
        self.e.frame_taken()
    }

    pub fn set_rotation(&self, rot: u32, mirror: bool) {
        self.e.set_rotation(rot, mirror)
    }

    pub fn last_frame(&self) -> Option<(u32, u32, Arc<[u8]>)> {
        self.e.last_frame()
    }

    pub fn open(&self, spec: OpenSpec, ctl: Control) {
        let s = syn_media::OpenSpec { cam: syn_media::Cam::Syncamd(spec.cam), width: spec.width, height: spec.height, fps: spec.fps, video: spec.video, jpeg: spec.jpeg };
        self.e.open(s, ctl)
    }

    pub fn close(&self) {
        self.e.close()
    }

    pub fn control(&self, mask: u32, f: impl FnOnce(&mut Control)) {
        self.e.control(mask, f)
    }

    pub fn ctl(&self) -> Control {
        self.e.ctl()
    }

    /// Собрать `n` следующих кадров потока целиком; ответ — кадры и «пары цветности V,U» (NV21).
    pub fn burst(&self, n: usize) -> std::sync::mpsc::Receiver<(Vec<night::Frame>, bool)> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.x.burst.lock().unwrap() = Some(Burst { want: n.max(1), frames: Vec::new(), tx });
        rx
    }

    pub fn recording(&self) -> bool {
        self.e.recording()
    }

    pub fn start_recording(&self, s: recorder::Settings) -> Result<()> {
        self.e.start_recording(s)
    }

    pub fn pause_recording(&self, on: bool) {
        self.e.pause_recording(on)
    }

    /// (длительность, мкс; байт; ошибка)
    pub fn record_stats(&self) -> Option<(u64, u64, Option<String>)> {
        self.e.record_stats()
    }

    pub fn stop_recording(&self) -> Result<PathBuf> {
        self.e.stop_recording()
    }
}

/// Отвод кадра: серия «Ночи» и уменьшенная яркость для QR.
fn tap(x: &Extra, src: &Src) {
    {
        let mut b = x.burst.lock().unwrap();
        if let Some(bu) = b.as_mut() {
            bu.frames.push(night::Frame::copy_from(src.data, src.width, src.height, src.stride, src.scanlines));
            if bu.frames.len() >= bu.want {
                let bu = b.take().unwrap();
                let _ = bu.tx.send((bu.frames, src.vu));
            }
        }
    }
    // QR: уменьшенная яркость (до ~720 по ширине) — фоновому потоку, если он свободен
    let tick = x.qr_tick.fetch_add(1, Ordering::Relaxed) + 1;
    if x.qr.load(Ordering::Relaxed) && tick % 10 == 0 {
        if let Some(tx) = x.qr_tx.lock().unwrap().as_ref() {
            let k = src.width.div_ceil(720).max(1);
            let (w, h) = (src.width / k, src.height / k);
            let mut y = vec![0u8; w * h];
            for r in 0..h {
                let row = &src.data[r * k * src.stride..];
                for c in 0..w {
                    y[r * w + c] = row[c * k];
                }
            }
            let _ = tx.try_send((y, w, h));
        }
    }
}

pub fn detect_qr(y: &[u8], w: usize, h: usize) -> Option<String> {
    let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, yy| y[yy * w + x]);
    img.detect_grids().into_iter().find_map(|g| g.decode().ok().map(|(_, s)| s))
}
#[cfg(test)]
mod tests {
    #[test]
    fn qr_from_luma() {
        let code = qrcode::QrCode::new(b"https://example.org/syncamera").unwrap();
        let n = code.width();
        let colors = code.to_colors();
        // код 6 px на модуль с полями, по центру серого кадра 720×540
        let (w, h, k, off) = (720usize, 540usize, 6usize, 120usize);
        let mut y = vec![150u8; w * h];
        for (i, c) in colors.iter().enumerate() {
            let (mx, my) = (i % n, i / n);
            let v = if *c == qrcode::Color::Dark { 20 } else { 235 };
            for dy in 0..k {
                for dx in 0..k {
                    y[(off + my * k + dy) * w + off + mx * k + dx] = v;
                }
            }
        }
        // поле вокруг кода — светлое
        assert_eq!(super::detect_qr(&y, w, h).as_deref(), Some("https://example.org/syncamera"));
    }
}
