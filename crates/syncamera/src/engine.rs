//! Сеанс камеры: поток кадров syncamd в фоне → превью (`LiveFrame`, RGBA с поворотом под окно) и запись
//! видео; управление (зум, точка фокуса, EV…) — сообщениями CONTROL того же соединения, повторяется после
//! переоткрытия. Камера занята или служба недоступна — повтор раз в секунду.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use syngui::widgets::LiveFrame;

use crate::convert::{self, Src};
use crate::proto::{self, Camera, Control, Meta, Stream};
use crate::recorder::{self, Recorder};

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
}

type NoteFn = Arc<dyn Fn(Note) + Send + Sync>;

struct Inner {
    preview: Arc<LiveFrame>,
    note: NoteFn,
    /// Номер сеанса: поток прежнего сеанса завершается, увидев другой.
    gen: AtomicU64,
    stream: Mutex<Option<Arc<Stream>>>,
    ctl: Mutex<Control>,
    recorder: Mutex<Option<Recorder>>,
    /// Поворот превью по часовой и зеркало (фронтальная).
    rot: AtomicU32,
    mirror: AtomicBool,
    /// Кадров превью в пути к интерфейсу (не взят — следующий не готовим).
    pending: AtomicU32,
    /// Последний кадр превью (миниатюра снимка).
    last: Mutex<Option<(u32, u32, Arc<[u8]>)>>,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl PartialEq for Engine {
    fn eq(&self, o: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &o.inner)
    }
}

impl Engine {
    pub fn new(note: impl Fn(Note) + Send + Sync + 'static) -> Engine {
        Engine {
            inner: Arc::new(Inner {
                preview: LiveFrame::new(),
                note: Arc::new(note),
                gen: AtomicU64::new(0),
                stream: Mutex::new(None),
                ctl: Mutex::new(Control::default()),
                recorder: Mutex::new(None),
                rot: AtomicU32::new(90),
                mirror: AtomicBool::new(false),
                pending: AtomicU32::new(0),
                last: Mutex::new(None),
            }),
        }
    }

    pub fn preview(&self) -> Arc<LiveFrame> {
        self.inner.preview.clone()
    }

    /// Интерфейс взял кадр превью.
    pub fn frame_taken(&self) {
        self.inner.pending.store(0, Ordering::Release);
    }

    pub fn set_rotation(&self, rot: u32, mirror: bool) {
        self.inner.rot.store(rot % 360, Ordering::Relaxed);
        self.inner.mirror.store(mirror, Ordering::Relaxed);
    }

    /// Последний кадр превью (RGBA).
    pub fn last_frame(&self) -> Option<(u32, u32, Arc<[u8]>)> {
        self.inner.last.lock().unwrap().clone()
    }

    /// Открыть камеру (прежний сеанс закрывается).
    pub fn open(&self, spec: OpenSpec, ctl: Control) {
        let gen = self.inner.gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.shutdown_stream();
        *self.inner.ctl.lock().unwrap() = ctl;
        let inner = self.inner.clone();
        std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || session(inner, gen, spec))
            .ok();
    }

    /// Закрыть камеру (запись — остановить заранее).
    pub fn close(&self) {
        self.inner.gen.fetch_add(1, Ordering::SeqCst);
        self.shutdown_stream();
    }

    fn shutdown_stream(&self) {
        if let Some(s) = self.inner.stream.lock().unwrap().take() {
            s.shutdown();
        }
    }

    /// Изменить управление и отправить поля `mask`.
    pub fn control(&self, mask: u32, f: impl FnOnce(&mut Control)) {
        let c = {
            let mut c = self.inner.ctl.lock().unwrap();
            f(&mut c);
            let mut c = *c;
            c.mask = mask;
            c
        };
        if let Some(s) = self.inner.stream.lock().unwrap().as_ref() {
            s.control(&c);
        }
    }

    pub fn ctl(&self) -> Control {
        *self.inner.ctl.lock().unwrap()
    }

    pub fn recording(&self) -> bool {
        self.inner.recorder.lock().unwrap().is_some()
    }

    pub fn start_recording(&self, s: recorder::Settings) -> Result<()> {
        let stream = self.inner.stream.lock().unwrap().clone().ok_or_else(|| anyhow!("камера не включена"))?;
        let r = Recorder::start(stream, s)?;
        *self.inner.recorder.lock().unwrap() = Some(r);
        Ok(())
    }

    pub fn pause_recording(&self, on: bool) {
        if let Some(r) = self.inner.recorder.lock().unwrap().as_ref() {
            r.set_paused(on);
        }
    }

    /// (длительность, мкс; байт; ошибка)
    pub fn record_stats(&self) -> Option<(u64, u64, Option<String>)> {
        self.inner.recorder.lock().unwrap().as_ref().map(|r| {
            (r.shared.duration_us.load(Ordering::Relaxed), r.shared.bytes.load(Ordering::Relaxed), r.failed())
        })
    }

    pub fn stop_recording(&self) -> Result<PathBuf> {
        let r = self.inner.recorder.lock().unwrap().take().ok_or_else(|| anyhow!("запись не идёт"))?;
        r.stop()
    }
}

fn session(inner: Arc<Inner>, gen: u64, spec: OpenSpec) {
    let alive = || inner.gen.load(Ordering::SeqCst) == gen;
    let mut busy_said = false;
    while alive() {
        let flags = proto::OPEN_META | if spec.video { proto::OPEN_VIDEO } else { 0 };
        let mut r = Stream::open(spec.cam.id, spec.width, spec.height, spec.fps, flags, spec.jpeg);
        if let (Err(e), Some(_)) = (&r, spec.jpeg) {
            // поток JPEG рядом с большим видео HAL может не принять — без него (снимок перенастроит)
            if !e.busy {
                r = Stream::open(spec.cam.id, spec.width, spec.height, spec.fps, flags, None);
            }
        }
        let stream = match r {
            Ok(s) => Arc::new(s),
            Err(e) => {
                if !busy_said || !e.busy {
                    (inner.note)(Note::Status(if e.busy { "Камера занята другой программой".into() } else { e.text }));
                    busy_said = true;
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        if !alive() {
            return;
        }
        // управление — сразу (зум, EV, режим… сохраняются между переоткрытиями)
        let mut c = *inner.ctl.lock().unwrap();
        c.mask = proto::CTL_ZOOM | proto::CTL_EV | proto::CTL_TORCH | proto::CTL_MODE | proto::CTL_AWB | proto::CTL_MANUAL | proto::CTL_STAB | proto::CTL_AE_LOCK;
        stream.control(&c);
        *inner.stream.lock().unwrap() = Some(stream.clone());
        (inner.note)(Note::Status(String::new()));
        (inner.note)(Note::Started(stream.info.width, stream.info.height));
        busy_said = false;
        run(&inner, &stream, gen);
        // поток кадров кончился: закрыли сами или камера остановилась
        let mut cur = inner.stream.lock().unwrap();
        if cur.as_ref().is_some_and(|s| Arc::ptr_eq(s, &stream)) {
            *cur = None;
        }
        drop(cur);
        if alive() {
            (inner.note)(Note::Status("Камера остановилась — включаю снова…".into()));
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

fn run(inner: &Inner, stream: &Arc<Stream>, gen: u64) {
    let info = stream.info;
    let mut pool: Vec<Arc<[u8]>> = Vec::new();
    loop {
        let ev = match stream.next() {
            Ok(e) => e,
            Err(_) => return,
        };
        if inner.gen.load(Ordering::SeqCst) != gen {
            if let proto::Event::Frame(f) = ev {
                stream.release(f.buf);
            }
            return;
        }
        let f = match ev {
            proto::Event::Frame(f) => f,
            proto::Event::Meta(m) => {
                (inner.note)(Note::Meta(m));
                continue;
            }
            proto::Event::Stopped => return,
        };
        // превью: только когда интерфейс взял прошлый кадр
        if inner.pending.load(Ordering::Acquire) == 0 {
            let rot = inner.rot.load(Ordering::Relaxed);
            let mirror = inner.mirror.load(Ordering::Relaxed);
            let step = if info.width.max(info.height) > 2000 { 2 } else { 1 };
            let (ow, oh) = convert::out_size(info.width as usize, info.height as usize, rot, step);
            let need = ow * oh * 4;
            // буфер из пула, который никто больше не держит
            let mut buf = match pool.iter().position(|b| Arc::strong_count(b) == 1 && b.len() == need) {
                Some(i) => pool.swap_remove(i),
                None => Arc::from(vec![0u8; need]),
            };
            let data = Arc::get_mut(&mut buf).expect("буфер превью свободен");
            stream.begin(f.buf);
            let src = Src {
                data: stream.data(f.buf),
                width: info.width as usize,
                height: info.height as usize,
                stride: info.stride as usize,
                scanlines: info.scanlines as usize,
                vu: info.format == proto::FMT_NV21,
            };
            convert::to_rgba(&src, rot, mirror, step, data);
            stream.end(f.buf);
            inner.preview.set(ow as u32, oh as u32, buf.clone());
            *inner.last.lock().unwrap() = Some((ow as u32, oh as u32, buf.clone()));
            pool.push(buf);
            pool.truncate(4);
            inner.pending.store(1, Ordering::Release);
            (inner.note)(Note::Frame);
        }
        let taken = inner.recorder.lock().unwrap().as_mut().is_some_and(|r| r.push(f));
        if !taken {
            stream.release(f.buf);
        }
    }
}
