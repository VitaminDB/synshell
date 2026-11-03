//! Запись видео: кадры источника → кодер ([`crate::encoder`]: V4L2 M2M телефона или ffmpeg компьютера,
//! H.264/HEVC) → MP4 (libavformat); звук — микрофон и внешние дорожки ([`crate::audio::Track`]) → AAC.
//!
//! Кадры кодируются как их отдаёт датчик, поворот под ориентацию — матрицей отображения потока (`tkhd` в
//! MP4, как у камер телефонов). Кадр копируется в буфер кодера в потоке кодера и сразу отпускается
//! (буфер возвращается источнику). Метки времени видео — время кадров источника, паузы вырезаются; звук
//! идёт непрерывным счётчиком отсчётов от первого кадра. Таймлапс — каждый N-й кадр с шагом 1/fps, без
//! звука.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::audio::{Audio, Track};
use crate::capture::CamFrame;
use crate::encoder::{self, VideoEncoder};
use crate::mux::{Mux, Writer};

/// Параметры записи.
#[derive(Clone, Debug)]
pub struct Settings {
    pub path: PathBuf,
    /// Поворот показа по часовой (градусы) — в матрицу отображения MP4.
    pub rot: u32,
    pub fps: u32,
    pub bitrate: u32,
    pub hevc: bool,
    /// Писать микрофон.
    pub audio: bool,
    /// Дорожка звука есть и без микрофона: внешние дорожки ([`Recorder::track`]).
    pub tracks: bool,
    /// Таймлапс: брать каждый N-й кадр (1 — обычная запись).
    pub timelapse: u32,
    pub encoder: encoder::Kind,
}

impl Settings {
    pub fn new(path: impl Into<PathBuf>) -> Settings {
        Settings { path: path.into(), rot: 0, fps: 30, bitrate: 8_000_000, hevc: false, audio: true, tracks: true, timelapse: 1, encoder: encoder::Kind::Auto }
    }
}

/// Общее состояние записи (поток кодера ↔ программа).
#[derive(Default)]
pub struct Shared {
    pub paused: AtomicBool,
    /// Микрофон в запись не идёт (тишина), дорожки — идут: пока звучит
    /// собеседник из динамика, его эхо не попадает в запись.
    pub mic_muted: AtomicBool,
    resumed: AtomicBool,
    /// Длительность записанного, мкс.
    pub duration_us: AtomicU64,
    pub bytes: AtomicU64,
    pub error: Mutex<Option<String>>,
    /// Кадров записано / пропущено: очередь к кодеру полна, кодер занят.
    frames: AtomicU64,
    drop_queue: AtomicU64,
    drop_enc: AtomicU64,
}

enum Job {
    Frame(Box<dyn CamFrame>),
    Stop,
}

pub struct Recorder {
    tx: SyncSender<Job>,
    pub shared: Arc<Shared>,
    thread: Option<JoinHandle<Result<PathBuf>>>,
    every: u32,
    count: u32,
    mixer: Option<Arc<crate::audio::Mixer>>,
    encoder: String,
}

fn boottime_ns() -> u64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: запись в живую структуру
    unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut t) };
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

impl Recorder {
    /// Начать запись кадров размера `width × height` (кадры — [`Recorder::push`]).
    pub fn start(width: u32, height: u32, s: Settings) -> Result<Recorder> {
        let (w, h) = (width & !1, height & !1);
        let enc = encoder::open(s.encoder, w, h, s.hevc, s.bitrate, s.fps)?;
        let encoder = enc.name().to_string();
        if let Some(d) = s.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mux = Arc::new(Mutex::new(Mux::new(&s.path, w, h, s.hevc, s.rot)?));
        let shared = Arc::new(Shared::default());
        let t0 = Arc::new(AtomicU64::new(0));
        let audio = if (s.audio || s.tracks) && s.timelapse <= 1 {
            match Audio::start(mux.clone(), shared.clone(), t0.clone(), s.audio) {
                Ok(a) => Some(a),
                Err(e) => {
                    tracing::warn!("звук для видео: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        let mixer = audio.as_ref().map(|a| a.mixer.clone());
        let (tx, rx) = sync_channel::<Job>(3);
        let sh = shared.clone();
        let every = s.timelapse.max(1);
        let thread = std::thread::Builder::new().name("video-enc".into()).spawn(move || {
            let r = encode_loop(enc, rx, &mux, &sh, &s, &t0, audio);
            if let Err(e) = &r {
                *sh.error.lock().unwrap() = Some(format!("{e:#}"));
                let _ = std::fs::remove_file(&s.path);
            }
            r.map(|_| s.path.clone())
        })?;
        Ok(Recorder { tx, shared, thread: Some(thread), every, count: 0, mixer, encoder })
    }

    /// Кадр для записи. false — не взят (пауза, очередь полна, пропуск таймлапса); кадр отпускается.
    pub fn push(&mut self, f: Box<dyn CamFrame>) -> bool {
        if self.shared.paused.load(Ordering::Relaxed) {
            return false;
        }
        self.count += 1;
        if !(self.count - 1).is_multiple_of(self.every) {
            return false;
        }
        match self.tx.try_send(Job::Frame(f)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.shared.drop_queue.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Заглушить микрофон в записи (дорожки и видео пишутся дальше).
    pub fn set_mic_muted(&self, on: bool) {
        self.shared.mic_muted.store(on, Ordering::SeqCst);
    }

    pub fn set_paused(&self, on: bool) {
        self.shared.paused.store(on, Ordering::SeqCst);
        if !on {
            self.shared.resumed.store(true, Ordering::SeqCst);
        }
    }

    pub fn failed(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// Чем кодируется видео (`v4l2m2m`, `h264_nvenc`, `libx264`…).
    pub fn encoder(&self) -> &str {
        &self.encoder
    }

    /// Сколько уже записано (без пауз) — метка для транскрипта и пометок.
    pub fn position(&self) -> Duration {
        let v = Duration::from_micros(self.shared.duration_us.load(Ordering::Relaxed));
        match &self.mixer {
            Some(m) => v.max(Duration::from_micros(m.position_us())),
            None => v,
        }
    }

    /// Новая внешняя дорожка звука (голос собеседника): её куски смешиваются с микрофоном.
    /// `None` — записи без звука (таймлапс, нет AAC).
    pub fn track(&self, gain: f32) -> Option<Track> {
        self.mixer.as_ref().map(|m| Track::new(m.clone(), gain))
    }

    /// Остановить и дописать файл; путь готового видео.
    pub fn stop(mut self) -> Result<PathBuf> {
        let _ = self.tx.send(Job::Stop);
        match self.thread.take().map(|t| t.join()) {
            Some(Ok(r)) => r,
            _ => Err(anyhow!("поток записи упал")),
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            let _ = self.tx.send(Job::Stop);
            let _ = t.join();
        }
    }
}

fn encode_loop(
    mut enc: Box<dyn VideoEncoder>,
    rx: Receiver<Job>,
    mux: &Mutex<Mux>,
    sh: &Shared,
    s: &Settings,
    t0: &AtomicU64,
    audio: Option<Audio>,
) -> Result<()> {
    let frame_ns = 1_000_000_000 / s.fps.max(1) as u64;
    let (mut first_ts, mut last_ts, mut paused_ns, mut n) = (0u64, 0u64, 0u64, 0u64);
    let mut w = Writer::new(mux, sh, s.hevc, (frame_ns / 1000) as i64);
    let timing = std::env::var_os("SYNCAMERA_TIMING").is_some();
    let mut result = Ok(());
    loop {
        let job = match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(j) => Some(j),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            Err(_) => break,
        };
        // готовое — сразу в файл
        if let Err(e) = enc.poll(0, &mut |d, ts, k| w.put(d, ts, k)) {
            result = Err(e);
            break;
        }
        let f = match job {
            Some(Job::Frame(f)) => f,
            Some(Job::Stop) => break,
            None => continue,
        };
        // метка времени: время кадра без пауз; таймлапс — ровный шаг
        let ts = f.ts_ns();
        let (first, mut paused) = if n == 0 { (ts, 0) } else { (first_ts, paused_ns) };
        if n > 0 && sh.resumed.load(Ordering::SeqCst) && ts > last_ts + frame_ns {
            paused += ts - last_ts - frame_ns;
        }
        let pts_us = if s.timelapse > 1 { n * frame_ns / 1000 } else { ts.saturating_sub(first + paused) / 1000 };
        let t = std::time::Instant::now();
        let mut r = Ok(false);
        f.with_src(&mut |src| r = enc.encode(src, pts_us, n == 0, &mut |d, ts, k| w.put(d, ts, k)));
        drop(f);
        match r {
            Err(e) => {
                result = Err(e);
                break;
            }
            Ok(false) => {
                sh.drop_enc.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            Ok(true) => {}
        }
        if timing {
            eprintln!("в кодер: {:.1} мс", t.elapsed().as_secs_f64() * 1e3);
        }
        if n == 0 {
            first_ts = ts;
            t0.store(boottime_ns().max(1), Ordering::SeqCst);
        }
        sh.resumed.store(false, Ordering::SeqCst);
        paused_ns = paused;
        last_ts = ts;
        n += 1;
        sh.frames.store(n, Ordering::Relaxed);
    }
    if result.is_ok() {
        result = enc.finish(&mut |d, ts, k| w.put(d, ts, k));
    }
    // кадры, оставшиеся в очереди, — отпустить
    while rx.try_recv().is_ok() {}
    drop(enc);
    if let Some(a) = audio {
        a.stop();
    }
    let mut m = mux.lock().unwrap();
    if !w.started && result.is_ok() {
        result = Err(anyhow!("не записано ни одного кадра"));
    }
    let fin = m.finish();
    tracing::info!(
        "запись: кадров {}, пропущено — очередь {}, кодер {}",
        sh.frames.load(Ordering::Relaxed),
        sh.drop_queue.load(Ordering::Relaxed),
        sh.drop_enc.load(Ordering::Relaxed)
    );
    result.and(fin)
}
