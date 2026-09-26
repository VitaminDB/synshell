//! Звук: собственный микшер поверх потокового `AudioPlayer` syngui.
//!
//! Один выходной поток 48 кГц моно. В него микшируются фоновая музыка
//! (зациклена с кроссфейдом) и короткие эффекты (несколько голосов
//! одновременно, с изменяемой высотой — «лестница» комбо). Эффекты вшиты в
//! бинарник (`assets/sfx`, синтез — `tools/gen_sfx.py`), музыка — `assets/music/bgm.wav`
//! (24 кГц моно, сгенерирована ACE-Step через `synaptix music`).
//!
//! Доступ глобальный: `audio::init(..)` один раз, дальше `audio::play(Sfx::Place)`.
//! Без `init` (тесты) все вызовы — no-op.

use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use syngui::audio::AudioPlayer;

pub const RATE: u32 = 48_000;
/// Опережение генерации относительно реального времени (задержка эффектов).
#[cfg(target_os = "android")]
const LEAD_SEC: f32 = 0.06;
#[cfg(not(target_os = "android"))]
const LEAD_SEC: f32 = 0.07;
const CHUNK: usize = 480; // 10 мс
const MUSIC_GAIN: f32 = 0.34;
const SFX_GAIN: f32 = 0.9;
const LOOP_XFADE_SEC: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sfx {
    Pick,
    Place,
    Back,
    Deny,
    Clear1,
    Clear2,
    Clear3,
    Clear4,
    Combo,
    Perfect,
    Best,
    Over,
    Refill,
    Click,
    Wood,
    Crystal,
    Gold,
    Lava,
}

impl Sfx {
    pub const ALL: [Sfx; 18] = [
        Sfx::Pick,
        Sfx::Place,
        Sfx::Back,
        Sfx::Deny,
        Sfx::Clear1,
        Sfx::Clear2,
        Sfx::Clear3,
        Sfx::Clear4,
        Sfx::Combo,
        Sfx::Perfect,
        Sfx::Best,
        Sfx::Over,
        Sfx::Refill,
        Sfx::Click,
        Sfx::Wood,
        Sfx::Crystal,
        Sfx::Gold,
        Sfx::Lava,
    ];

    fn bytes(self) -> &'static [u8] {
        match self {
            Sfx::Pick => include_bytes!("../assets/sfx/pick.wav"),
            Sfx::Place => include_bytes!("../assets/sfx/place.wav"),
            Sfx::Back => include_bytes!("../assets/sfx/back.wav"),
            Sfx::Deny => include_bytes!("../assets/sfx/deny.wav"),
            Sfx::Clear1 => include_bytes!("../assets/sfx/clear1.wav"),
            Sfx::Clear2 => include_bytes!("../assets/sfx/clear2.wav"),
            Sfx::Clear3 => include_bytes!("../assets/sfx/clear3.wav"),
            Sfx::Clear4 => include_bytes!("../assets/sfx/clear4.wav"),
            Sfx::Combo => include_bytes!("../assets/sfx/combo.wav"),
            Sfx::Perfect => include_bytes!("../assets/sfx/perfect.wav"),
            Sfx::Best => include_bytes!("../assets/sfx/best.wav"),
            Sfx::Over => include_bytes!("../assets/sfx/over.wav"),
            Sfx::Refill => include_bytes!("../assets/sfx/refill.wav"),
            Sfx::Click => include_bytes!("../assets/sfx/click.wav"),
            Sfx::Wood => include_bytes!("../assets/sfx/wood.wav"),
            Sfx::Crystal => include_bytes!("../assets/sfx/crystal.wav"),
            Sfx::Gold => include_bytes!("../assets/sfx/gold.wav"),
            Sfx::Lava => include_bytes!("../assets/sfx/lava.wav"),
        }
    }

    /// Относительная громкость эффекта.
    fn gain(self) -> f32 {
        match self {
            Sfx::Pick => 0.55,
            Sfx::Place => 0.9,
            Sfx::Back => 0.5,
            Sfx::Deny => 0.6,
            Sfx::Click => 0.6,
            Sfx::Refill => 0.55,
            Sfx::Combo => 0.7,
            Sfx::Wood | Sfx::Crystal | Sfx::Gold | Sfx::Lava => 0.75,
            _ => 1.0,
        }
    }

    /// Эффект очистки по числу линий.
    pub fn clear(lines: usize) -> Sfx {
        match lines {
            0 | 1 => Sfx::Clear1,
            2 => Sfx::Clear2,
            3 => Sfx::Clear3,
            _ => Sfx::Clear4,
        }
    }
}

/// Фоновые треки (YuE2, 22.05 кГц моно): имя и данные.
pub const TRACKS: &[(&str, &[u8])] = &[
    ("Шкатулка", include_bytes!("../assets/music/track1.wav")),
    ("Лоу-фай", include_bytes!("../assets/music/track2.wav")),
    ("Вечер", include_bytes!("../assets/music/track3.wav")),
    ("Кафе", include_bytes!("../assets/music/track4.wav")),
    ("Ночной город", include_bytes!("../assets/music/track5.wav")),
];

pub fn track_count() -> usize {
    TRACKS.len()
}

static CURRENT_TRACK: AtomicUsize = AtomicUsize::new(0);

/// Трек, который сейчас играет (в режиме «по кругу» меняется сам).
pub fn current_track() -> usize {
    CURRENT_TRACK.load(Ordering::Relaxed)
}

pub fn track_name(i: usize) -> &'static str {
    TRACKS[i % TRACKS.len()].0
}

enum Cmd {
    Play { sfx: Sfx, rate: f32 },
    Music(bool),
    Track(usize),
    LoopAll(bool),
    SfxOn(bool),
    Suspend,
    Resume,
}

struct Handle {
    tx: Sender<Cmd>,
}

static HANDLE: OnceLock<Mutex<Option<Handle>>> = OnceLock::new();

fn handle() -> &'static Mutex<Option<Handle>> {
    HANDLE.get_or_init(|| Mutex::new(None))
}

fn send(cmd: Cmd) {
    if let Ok(g) = handle().lock() {
        if let Some(h) = g.as_ref() {
            let _ = h.tx.send(cmd);
        }
    }
}

/// Запустить звук (один раз при старте приложения).
pub fn init(music_on: bool, sfx_on: bool, track: usize, loop_all: bool) {
    let (tx, rx) = mpsc::channel();
    CURRENT_TRACK.store(track % TRACKS.len(), Ordering::Relaxed);
    let spawned = thread::Builder::new()
        .name("elle-audio-mixer".into())
        .spawn(move || Mixer::run(rx, music_on, sfx_on, track, loop_all));
    match spawned {
        Ok(_) => {
            if let Ok(mut g) = handle().lock() {
                *g = Some(Handle { tx });
            }
        }
        Err(e) => log::warn!("audio: не удалось запустить микшер: {e}"),
    }
}

pub fn play(sfx: Sfx) {
    send(Cmd::Play { sfx, rate: 1.0 });
}

/// Эффект со сдвигом высоты в полутонах.
pub fn play_pitched(sfx: Sfx, semitones: f32) {
    send(Cmd::Play { sfx, rate: 2f32.powf(semitones / 12.0) });
}

pub fn set_music(on: bool) {
    send(Cmd::Music(on));
}

/// Переключить трек (с плавным затуханием и нарастанием).
pub fn set_track(i: usize) {
    send(Cmd::Track(i % TRACKS.len()));
}

/// Режим «все треки по кругу».
pub fn set_loop_all(on: bool) {
    send(Cmd::LoopAll(on));
}

pub fn set_sfx(on: bool) {
    send(Cmd::SfxOn(on));
}

/// Приложение ушло в фон: остановить вывод (на Android поток всё равно отберут).
pub fn suspend() {
    send(Cmd::Suspend);
}

pub fn resume() {
    send(Cmd::Resume);
}

// ── декодирование ──────────────────────────────────────────────────────────

/// WAV → моно f32 на частоте `RATE` (линейная передискретизация при нужде).
pub fn decode_wav(bytes: &[u8]) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).ok()?;
    let spec = reader.spec();
    let ch = spec.channels.max(1) as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = ((1u64 << (spec.bits_per_sample - 1)) - 1) as f32;
            reader.samples::<i32>().filter_map(|s| s.ok()).map(|s| s as f32 / max).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
    };
    let mono: Vec<f32> = if ch == 1 {
        raw
    } else {
        raw.chunks(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
    };
    Some(resample_linear(&mono, spec.sample_rate, RATE))
}

pub fn resample_linear(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = ((x.len() as f64) / ratio) as usize;
    (0..n)
        .map(|i| {
            let pos = i as f64 * ratio;
            let k = pos as usize;
            let t = (pos - k as f64) as f32;
            let a = x[k.min(x.len() - 1)];
            let b = x[(k + 1).min(x.len() - 1)];
            a + (b - a) * t
        })
        .collect()
}

/// Сделать из трека бесшовный цикл: хвост длиной `xfade` наложить на начало.
pub fn make_loop(x: &[f32], xfade: usize) -> Vec<f32> {
    if x.len() <= xfade * 2 || xfade == 0 {
        return x.to_vec();
    }
    let body = x.len() - xfade;
    let mut out = Vec::with_capacity(body);
    for i in 0..body {
        if i < xfade {
            let t = i as f32 / xfade as f32;
            let (a, b) = ((0.5 * std::f32::consts::PI * t).sin(), (0.5 * std::f32::consts::PI * t).cos());
            out.push(x[i] * a + x[body + i] * b);
        } else {
            out.push(x[i]);
        }
    }
    out
}

// ── микшер ─────────────────────────────────────────────────────────────────

struct Voice {
    sfx: usize,
    pos: f32,
    rate: f32,
    gain: f32,
}

struct Mixer {
    sfx: Vec<Vec<f32>>,
    music: Vec<f32>,
    music_cache: Vec<Option<Vec<f32>>>,
    track: usize,
    /// Ожидающий трек: текущий затухает, потом подмена.
    pending_track: Option<usize>,
    loop_all: bool,
    music_pos: usize,
    music_on: bool,
    sfx_on: bool,
    music_gain: f32,
    voices: Vec<Voice>,
    out: Option<(AudioPlayer, Sender<Vec<f32>>)>,
    started: Instant,
    produced: u64,
    suspended: bool,
}

impl Mixer {
    fn load_track(i: usize) -> Vec<f32> {
        decode_wav(TRACKS[i % TRACKS.len()].1)
            .map(|m| make_loop(&m, (LOOP_XFADE_SEC * RATE as f32) as usize))
            .unwrap_or_default()
    }

    fn run(rx: Receiver<Cmd>, music_on: bool, sfx_on: bool, track: usize, loop_all: bool) {
        let sfx: Vec<Vec<f32>> = Sfx::ALL.iter().map(|s| decode_wav(s.bytes()).unwrap_or_default()).collect();
        let track = track % TRACKS.len();
        let music = Self::load_track(track);
        let mut cache: Vec<Option<Vec<f32>>> = vec![None; TRACKS.len()];
        cache[track] = Some(music.clone());
        let mut m = Mixer {
            sfx,
            music,
            music_cache: cache,
            track,
            pending_track: None,
            loop_all,
            music_pos: 0,
            music_on,
            sfx_on,
            music_gain: if music_on { 1.0 } else { 0.0 },
            voices: Vec::new(),
            out: None,
            started: Instant::now(),
            produced: 0,
            suspended: false,
        };
        m.open();
        loop {
            match rx.try_recv() {
                Ok(cmd) => m.handle(cmd),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => break,
            }
            m.pump();
            thread::sleep(Duration::from_millis(4));
        }
    }

    fn open(&mut self) {
        if self.out.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        match AudioPlayer::start_streaming(rx, RATE) {
            Ok(p) => {
                self.out = Some((p, tx));
                self.started = Instant::now();
                self.produced = 0;
            }
            Err(e) => log::warn!("audio: вывод недоступен: {e}"),
        }
    }

    fn close(&mut self) {
        if let Some((p, tx)) = self.out.take() {
            drop(tx);
            p.stop();
        }
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Play { sfx, rate } => {
                if !self.sfx_on || self.suspended {
                    return;
                }
                let idx = Sfx::ALL.iter().position(|s| *s == sfx).unwrap_or(0);
                if self.voices.len() >= 12 {
                    self.voices.remove(0);
                }
                self.voices.push(Voice { sfx: idx, pos: 0.0, rate, gain: sfx.gain() * SFX_GAIN });
            }
            Cmd::Music(on) => self.music_on = on,
            Cmd::Track(i) => {
                if i != self.track {
                    self.pending_track = Some(i);
                }
            }
            Cmd::LoopAll(on) => self.loop_all = on,
            Cmd::SfxOn(on) => {
                self.sfx_on = on;
                if !on {
                    self.voices.clear();
                }
            }
            Cmd::Suspend => {
                self.suspended = true;
                self.voices.clear();
                self.close();
            }
            Cmd::Resume => {
                self.suspended = false;
                self.open();
            }
        }
    }

    fn pump(&mut self) {
        if self.suspended {
            return;
        }
        if self.out.is_none() {
            // устройство могло появиться позже — пробуем не чаще раза в секунду
            if self.started.elapsed() > Duration::from_secs(1) {
                self.started = Instant::now();
                self.open();
            }
            return;
        }
        let elapsed = self.started.elapsed().as_secs_f32();
        let target = ((elapsed + LEAD_SEC) * RATE as f32) as u64;
        while self.produced < target {
            let chunk = self.render(CHUNK);
            self.produced += CHUNK as u64;
            let dead = match &self.out {
                Some((_, tx)) => tx.send(chunk).is_err(),
                None => true,
            };
            if dead {
                self.close();
                return;
            }
        }
    }

    fn render(&mut self, n: usize) -> Vec<f32> {
        let mut buf = vec![0.0f32; n];
        // смена трека: дождаться затухания, подменить буфер
        if let Some(i) = self.pending_track {
            if self.music_gain < 0.01 || !self.music_on {
                if self.music_cache[i].is_none() {
                    self.music_cache[i] = Some(Self::load_track(i));
                }
                self.music = self.music_cache[i].clone().unwrap_or_default();
                self.music_pos = 0;
                self.track = i;
                self.pending_track = None;
                CURRENT_TRACK.store(i, Ordering::Relaxed);
            }
        }
        // музыка с плавным включением/выключением
        let target_gain = if self.music_on && self.pending_track.is_none() { 1.0 } else { 0.0 };
        let rate = if self.pending_track.is_some() { 0.00025 } else { 0.00008 };
        if !self.music.is_empty() {
            for s in buf.iter_mut() {
                self.music_gain += (target_gain - self.music_gain) * rate;
                if self.music_gain > 0.0005 {
                    *s += self.music[self.music_pos] * MUSIC_GAIN * self.music_gain;
                }
                self.music_pos += 1;
                if self.music_pos >= self.music.len() {
                    self.music_pos = 0;
                    if self.loop_all && self.pending_track.is_none() && TRACKS.len() > 1 {
                        self.pending_track = Some((self.track + 1) % TRACKS.len());
                    }
                }
            }
        }
        // эффекты
        let sfx = &self.sfx;
        for v in self.voices.iter_mut() {
            let src = &sfx[v.sfx];
            for s in buf.iter_mut() {
                let k = v.pos as usize;
                if k + 1 >= src.len() {
                    break;
                }
                let t = v.pos - k as f32;
                let sample = src[k] + (src[k + 1] - src[k]) * t;
                *s += sample * v.gain;
                v.pos += v.rate;
            }
        }
        self.voices.retain(|v| (v.pos as usize) + 1 < sfx[v.sfx].len());
        // мягкое ограничение
        for s in buf.iter_mut() {
            if s.abs() > 0.8 {
                *s = s.signum() * (0.8 + (s.abs() - 0.8) / (1.0 + (s.abs() - 0.8) * 3.0)).min(1.0);
            }
        }
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sfx_decode_at_48k() {
        for s in Sfx::ALL {
            let pcm = decode_wav(s.bytes()).expect("wav decodes");
            assert!(!pcm.is_empty(), "{s:?} пустой");
            assert!(pcm.len() < RATE as usize * 4, "{s:?} длиннее 4 с");
            assert!(pcm.iter().all(|v| v.abs() <= 1.0001));
        }
    }

    #[test]
    fn music_decodes_and_loops() {
        assert_eq!(track_count(), 5);
        for (i, (name, bytes)) in TRACKS.iter().enumerate() {
            assert!(!name.is_empty());
            let m = decode_wav(bytes).expect("track decodes");
            assert!(m.len() > RATE as usize * 20, "трек {i} короче 20 с");
        }
        let m = decode_wav(TRACKS[0].1).expect("music decodes");
        let l = make_loop(&m, RATE as usize);
        assert_eq!(l.len(), m.len() - RATE as usize);
        // на стыке нет скачка больше, чем внутри трека
        let joint = (l[0] - l[l.len() - 1]).abs();
        assert!(joint < 0.5, "скачок на стыке {joint}");
    }

    #[test]
    fn resample_keeps_duration() {
        let x: Vec<f32> = (0..24_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let y = resample_linear(&x, 24_000, 48_000);
        assert!((y.len() as i64 - 48_000).abs() <= 2);
        assert!((y[100] - x[50]).abs() < 1e-3);
    }

    #[test]
    fn noop_without_init() {
        play(Sfx::Place);
        set_music(false);
        suspend();
        resume();
    }
}
