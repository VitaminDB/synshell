//! Сеанс камеры: поток кадров источника ([`crate::capture`]) в фоне → превью (`LiveFrame`, RGBA с
//! поворотом под окно) и запись видео; управление камерой телефона (зум, точка фокуса, EV…) — сообщениями
//! CONTROL, повторяется после переоткрытия. Камера занята или недоступна — повтор раз в секунду.
//!
//! Пример (видео себя с записью):
//! ```ignore
//! let cam = synmedia::front_camera().unwrap();
//! let (w, h) = cam.video_size(16, 9, 1280).unwrap_or((1280, 720));
//! let eng = synmedia::Engine::new(|n| if let synmedia::Note::Frame = n { /* перерисовать LiveView */ });
//! eng.set_rotation(cam.sensor_rotation(), cam.front());
//! eng.open(synmedia::OpenSpec::video(cam, w, h), Default::default());
//! // LiveView::new(eng.preview()), после показа кадра — eng.frame_taken()
//! eng.start_recording(synmedia::Settings::new(path))?;
//! let voice = eng.track(1.0).unwrap();   // голос собеседника
//! let at = voice.play(&pcm, 24000);       // время куска в записи
//! let path = eng.stop_recording()?;
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use syngui::t;
use syngui::widgets::LiveFrame;

use crate::audio::Track;
use crate::capture::{self, Cam, CapEvent, Capture};
use crate::convert::{self, Src};
use crate::recorder::{self, Recorder};
use crate::syncamd::{self, Control, Meta};

/// Что открыть.
#[derive(Clone, Debug)]
pub struct OpenSpec {
    pub cam: Cam,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Режим видео (у камер телефона — поток под запись).
    pub video: bool,
    /// Поток JPEG сразу (снимок без перенастройки; камеры телефона).
    pub jpeg: Option<(u32, u32)>,
}

impl OpenSpec {
    /// Видео 30 к/с.
    pub fn video(cam: Cam, width: u32, height: u32) -> OpenSpec {
        OpenSpec { cam, width, height, fps: 30, video: true, jpeg: None }
    }
}

/// События сеанса (из фонового потока).
pub enum Note {
    /// Новый кадр превью.
    Frame,
    Meta(Meta),
    /// Камера включилась (размер кадра потока).
    Started(u32, u32),
    /// Состояние: занята, ошибка (текст), пусто — всё хорошо.
    Status(String),
}

/// Отвод кадров: каждый кадр потока (данные и время, нс) — до превью и записи.
pub type Tap = Arc<dyn Fn(&Src, u64) + Send + Sync>;

type NoteFn = Arc<dyn Fn(Note) + Send + Sync>;

struct Inner {
    preview: Arc<LiveFrame>,
    note: NoteFn,
    /// Номер сеанса: поток прежнего сеанса завершается, увидев другой.
    gen: AtomicU64,
    cap: Mutex<Option<Arc<dyn Capture>>>,
    ctl: Mutex<Control>,
    recorder: Mutex<Option<Recorder>>,
    /// Поворот превью по часовой и зеркало (фронтальная).
    rot: AtomicU32,
    mirror: AtomicBool,
    /// Кадров превью в пути к интерфейсу (не взят — следующий не готовим).
    pending: AtomicU32,
    /// Последний кадр превью (миниатюра снимка).
    last: Mutex<Option<(u32, u32, Arc<[u8]>)>>,
    tap: Mutex<Option<Tap>>,
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
                cap: Mutex::new(None),
                ctl: Mutex::new(Control::default()),
                recorder: Mutex::new(None),
                rot: AtomicU32::new(90),
                mirror: AtomicBool::new(false),
                pending: AtomicU32::new(0),
                last: Mutex::new(None),
                tap: Mutex::new(None),
            }),
        }
    }

    /// Отвод кадров (распознавание, серия кадров «Ночи»…); `None` — убрать.
    pub fn set_tap(&self, tap: Option<Tap>) {
        *self.inner.tap.lock().unwrap() = tap;
    }

    pub fn preview(&self) -> Arc<LiveFrame> {
        self.inner.preview.clone()
    }

    /// Интерфейс взял кадр превью.
    pub fn frame_taken(&self) {
        self.inner.pending.store(0, Ordering::Release);
    }

    /// Поворот превью по часовой и зеркало (по умолчанию 90°, как у датчиков телефона).
    pub fn set_rotation(&self, rot: u32, mirror: bool) {
        self.inner.rot.store(rot % 360, Ordering::Relaxed);
        self.inner.mirror.store(mirror, Ordering::Relaxed);
    }

    /// Последний кадр превью (RGBA).
    pub fn last_frame(&self) -> Option<(u32, u32, Arc<[u8]>)> {
        self.inner.last.lock().unwrap().clone()
    }

    /// Открытый поток (снимок у камер телефона: `capture().and_then(|c| c.syncamd().cloned())`).
    pub fn capture(&self) -> Option<Arc<dyn Capture>> {
        self.inner.cap.lock().unwrap().clone()
    }

    /// Открыть камеру (прежний сеанс закрывается).
    pub fn open(&self, spec: OpenSpec, ctl: Control) {
        let gen = self.inner.gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.shutdown_stream();
        *self.inner.ctl.lock().unwrap() = ctl;
        let inner = self.inner.clone();
        std::thread::Builder::new().name("camera".into()).spawn(move || session(inner, gen, spec)).ok();
    }

    /// Закрыть камеру (запись — остановить заранее).
    pub fn close(&self) {
        self.inner.gen.fetch_add(1, Ordering::SeqCst);
        self.shutdown_stream();
    }

    fn shutdown_stream(&self) {
        if let Some(s) = self.inner.cap.lock().unwrap().take() {
            s.shutdown();
        }
    }

    /// Изменить управление и отправить поля `mask` (камеры телефона).
    pub fn control(&self, mask: u32, f: impl FnOnce(&mut Control)) {
        let c = {
            let mut c = self.inner.ctl.lock().unwrap();
            f(&mut c);
            let mut c = *c;
            c.mask = mask;
            c
        };
        if let Some(s) = self.inner.cap.lock().unwrap().as_ref() {
            s.control(&c);
        }
    }

    pub fn ctl(&self) -> Control {
        *self.inner.ctl.lock().unwrap()
    }

    pub fn recording(&self) -> bool {
        self.inner.recorder.lock().unwrap().is_some()
    }

    /// Начать запись; кодер — `Recorder::encoder()` (через [`Engine::with_recorder`]).
    pub fn start_recording(&self, s: recorder::Settings) -> Result<()> {
        let cap = self.capture().ok_or_else(|| anyhow!("камера не включена"))?;
        let (w, h) = cap.size();
        let r = Recorder::start(w, h, s)?;
        *self.inner.recorder.lock().unwrap() = Some(r);
        Ok(())
    }

    pub fn with_recorder<R>(&self, f: impl FnOnce(&Recorder) -> R) -> Option<R> {
        self.inner.recorder.lock().unwrap().as_ref().map(f)
    }

    /// Заглушить микрофон в записи (см. [`crate::Recorder::set_mic_muted`]).
    pub fn mute_mic(&self, on: bool) {
        self.with_recorder(|r| r.set_mic_muted(on));
    }

    pub fn pause_recording(&self, on: bool) {
        self.with_recorder(|r| r.set_paused(on));
    }

    /// (длительность, мкс; байт; ошибка)
    pub fn record_stats(&self) -> Option<(u64, u64, Option<String>)> {
        self.with_recorder(|r| (r.shared.duration_us.load(Ordering::Relaxed), r.shared.bytes.load(Ordering::Relaxed), r.failed()))
    }

    /// Время записи сейчас (для транскрипта).
    pub fn record_position(&self) -> Option<Duration> {
        self.with_recorder(|r| r.position())
    }

    /// Внешняя дорожка звука текущей записи (голос собеседника).
    pub fn track(&self, gain: f32) -> Option<Track> {
        self.with_recorder(|r| r.track(gain)).flatten()
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
        let flags = syncamd::OPEN_META | if spec.video { syncamd::OPEN_VIDEO } else { 0 };
        let cap = match capture::open(&spec.cam, spec.width, spec.height, spec.fps, flags, spec.jpeg) {
            Ok(s) => s,
            Err(e) => {
                if !busy_said || !e.busy {
                    (inner.note)(Note::Status(if e.busy { t!("Камера занята другой программой").into() } else { e.text }));
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
        c.mask = syncamd::CTL_ZOOM | syncamd::CTL_EV | syncamd::CTL_TORCH | syncamd::CTL_MODE | syncamd::CTL_AWB | syncamd::CTL_MANUAL | syncamd::CTL_STAB | syncamd::CTL_AE_LOCK;
        cap.control(&c);
        *inner.cap.lock().unwrap() = Some(cap.clone());
        (inner.note)(Note::Status(String::new()));
        let (w, h) = cap.size();
        (inner.note)(Note::Started(w, h));
        busy_said = false;
        run(&inner, &*cap, gen);
        // поток кадров кончился: закрыли сами или камера остановилась
        let mut cur = inner.cap.lock().unwrap();
        if cur.as_ref().is_some_and(|s| Arc::ptr_eq(s, &cap)) {
            *cur = None;
        }
        drop(cur);
        if alive() {
            (inner.note)(Note::Status(t!("Камера остановилась — включаю снова…").into()));
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

fn run(inner: &Inner, cap: &dyn Capture, gen: u64) {
    let mut pool: Vec<Arc<[u8]>> = Vec::new();
    let timing = std::env::var_os("SYNCAMERA_TIMING").is_some();
    loop {
        let ev = match cap.next() {
            Ok(e) => e,
            Err(_) => return,
        };
        if inner.gen.load(Ordering::SeqCst) != gen {
            return;
        }
        let f = match ev {
            CapEvent::Frame(f) => f,
            CapEvent::Meta(m) => {
                (inner.note)(Note::Meta(m));
                continue;
            }
            CapEvent::Stopped => return,
        };
        // превью: только когда интерфейс взял прошлый кадр
        if inner.pending.load(Ordering::Acquire) == 0 {
            let rot = inner.rot.load(Ordering::Relaxed);
            let mirror = inner.mirror.load(Ordering::Relaxed);
            let mut shown = None;
            f.with_src(&mut |src| {
                // превью не больше ~2,2 Мп: 1080p и 1440×1080 — как есть, 1:1 и 4K — через пиксель
                let px = src.width * src.height;
                let step = (1..4).find(|k| px / (k * k) <= 2_200_000).unwrap_or(4);
                let (ow, oh) = convert::out_size(src.width, src.height, rot, step);
                let need = ow * oh * 4;
                // буфер из пула, который никто больше не держит
                let mut buf = match pool.iter().position(|b| Arc::strong_count(b) == 1 && b.len() == need) {
                    Some(i) => pool.swap_remove(i),
                    None => Arc::from(vec![0u8; need]),
                };
                let data = Arc::get_mut(&mut buf).expect("буфер превью свободен");
                let t = std::time::Instant::now();
                convert::to_rgba(src, rot, mirror, step, data);
                if timing {
                    eprintln!("превью {ow}×{oh} шаг {step}: {:.1} мс", t.elapsed().as_secs_f64() * 1e3);
                }
                shown = Some((ow as u32, oh as u32, buf));
            });
            if let Some((ow, oh, buf)) = shown {
                inner.preview.set(ow, oh, buf.clone());
                *inner.last.lock().unwrap() = Some((ow, oh, buf.clone()));
                pool.push(buf);
                pool.truncate(4);
                inner.pending.store(1, Ordering::Release);
                (inner.note)(Note::Frame);
            }
        }
        let tap = inner.tap.lock().unwrap().clone();
        if let Some(tap) = tap {
            let ts = f.ts_ns();
            f.with_src(&mut |src| tap(src, ts));
        }
        if let Some(r) = inner.recorder.lock().unwrap().as_mut() {
            r.push(f);
        }
    }
}
