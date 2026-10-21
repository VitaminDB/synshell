//! Звук: собственный микшер поверх потокового `AudioPlayer` syngui.
//!
//! Один выходной поток 48 кГц моно. В него микшируются фоновая музыка
//! (зациклена с кроссфейдом) и короткие эффекты (несколько голосов
//! одновременно, с изменяемой высотой — «лестница» комбо). Эффекты вшиты в
//! бинарник (`assets/sfx`, синтез — `tools/gen_sfx.py`), музыка — `assets/music/bgm.wav`
//! (24 кГц моно, сгенерирована ACE-Step через `synaptix music`).
//!
//! Голосовые реплики (`Voice`, английский, VoxCPM2 через `tools/gen_voice.sh`)
//! идут отдельным каналом: одна реплика за раз, музыка на это время приглушается.
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
    Metal,
    Stone,
    Velvet,
    Glass,
    Ice,
    Neon,
    Candy,
    Chime,
    Harp,
}

impl Sfx {
    pub const ALL: [Sfx; 27] = [
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
        Sfx::Metal,
        Sfx::Stone,
        Sfx::Velvet,
        Sfx::Glass,
        Sfx::Ice,
        Sfx::Neon,
        Sfx::Candy,
        Sfx::Chime,
        Sfx::Harp,
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
            Sfx::Metal => include_bytes!("../assets/sfx/metal.wav"),
            Sfx::Stone => include_bytes!("../assets/sfx/stone.wav"),
            Sfx::Velvet => include_bytes!("../assets/sfx/velvet.wav"),
            Sfx::Glass => include_bytes!("../assets/sfx/glass.wav"),
            Sfx::Ice => include_bytes!("../assets/sfx/ice.wav"),
            Sfx::Neon => include_bytes!("../assets/sfx/neon.wav"),
            Sfx::Candy => include_bytes!("../assets/sfx/candy.wav"),
            Sfx::Chime => include_bytes!("../assets/sfx/chime.wav"),
            Sfx::Harp => include_bytes!("../assets/sfx/harp.wav"),
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
            Sfx::Metal | Sfx::Stone | Sfx::Velvet | Sfx::Glass | Sfx::Ice | Sfx::Neon | Sfx::Candy => 0.7,
            Sfx::Chime | Sfx::Harp => 0.8,
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

/// Голосовые реплики (английский; `assets/voice`, 24 кГц моно).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Voice {
    Welcome,
    Nice,
    GoodOne,
    Beautiful,
    Wonderful,
    Excellent,
    Amazing,
    Incredible,
    Magnificent,
    OnFire,
    Unstoppable,
    YouBeautiful,
    Perfect,
    Flawless,
    NewRecord,
    GameOver,
}

impl Voice {
    pub const ALL: [Voice; 16] = [
        Voice::Welcome,
        Voice::Nice,
        Voice::GoodOne,
        Voice::Beautiful,
        Voice::Wonderful,
        Voice::Excellent,
        Voice::Amazing,
        Voice::Incredible,
        Voice::Magnificent,
        Voice::OnFire,
        Voice::Unstoppable,
        Voice::YouBeautiful,
        Voice::Perfect,
        Voice::Flawless,
        Voice::NewRecord,
        Voice::GameOver,
    ];

    fn bytes(self) -> &'static [u8] {
        match self {
            Voice::Welcome => include_bytes!("../assets/voice/welcome.wav"),
            Voice::Nice => include_bytes!("../assets/voice/nice.wav"),
            Voice::GoodOne => include_bytes!("../assets/voice/good_one.wav"),
            Voice::Beautiful => include_bytes!("../assets/voice/beautiful.wav"),
            Voice::Wonderful => include_bytes!("../assets/voice/wonderful.wav"),
            Voice::Excellent => include_bytes!("../assets/voice/excellent.wav"),
            Voice::Amazing => include_bytes!("../assets/voice/amazing.wav"),
            Voice::Incredible => include_bytes!("../assets/voice/incredible.wav"),
            Voice::Magnificent => include_bytes!("../assets/voice/magnificent.wav"),
            Voice::OnFire => include_bytes!("../assets/voice/on_fire.wav"),
            Voice::Unstoppable => include_bytes!("../assets/voice/unstoppable.wav"),
            Voice::YouBeautiful => include_bytes!("../assets/voice/you_beautiful.wav"),
            Voice::Perfect => include_bytes!("../assets/voice/perfect.wav"),
            Voice::Flawless => include_bytes!("../assets/voice/flawless.wav"),
            Voice::NewRecord => include_bytes!("../assets/voice/new_record.wav"),
            Voice::GameOver => include_bytes!("../assets/voice/game_over.wav"),
        }
    }

    /// Ранг реплики: новая реплика перебивает звучащую только при большем ранге.
    pub fn rank(self) -> u8 {
        match self {
            Voice::Nice | Voice::GoodOne | Voice::Welcome => 1,
            Voice::Beautiful | Voice::Wonderful | Voice::Excellent => 2,
            Voice::Amazing | Voice::Incredible | Voice::Magnificent => 3,
            Voice::OnFire | Voice::Unstoppable | Voice::YouBeautiful => 4,
            Voice::Perfect | Voice::Flawless | Voice::NewRecord | Voice::GameOver => 5,
        }
    }
}

const VOICE_GAIN: f32 = 1.0;
/// Приглушение музыки во время реплики (≈ −7 дБ).
const DUCK_GAIN: f32 = 0.45;

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
    Say(Voice),
    VoiceOn(bool),
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
    init_with_voice(music_on, sfx_on, true, track, loop_all);
}

pub fn init_with_voice(music_on: bool, sfx_on: bool, voice_on: bool, track: usize, loop_all: bool) {
    let (tx, rx) = mpsc::channel();
    CURRENT_TRACK.store(track % TRACKS.len(), Ordering::Relaxed);
    let spawned = thread::Builder::new()
        .name("syn-blocks-audio".into())
        .spawn(move || Mixer::run(rx, music_on, sfx_on, voice_on, track, loop_all));
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

/// Голосовая реплика (одна за раз; перебивает звучащую только при большем ранге).
pub fn say(v: Voice) {
    send(Cmd::Say(v));
}

pub fn set_voice(on: bool) {
    send(Cmd::VoiceOn(on));
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

struct SfxVoice {
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
    voices: Vec<SfxVoice>,
    /// Клипы реплик (декодируются при первом воспроизведении).
    speech: Vec<Option<Vec<f32>>>,
    /// Звучащая реплика: (индекс в `Voice::ALL`, позиция).
    speaking: Option<(usize, usize)>,
    voice_on: bool,
    /// Текущее приглушение музыки (1 — нет, DUCK_GAIN — реплика).
    duck: f32,
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

    fn run(rx: Receiver<Cmd>, music_on: bool, sfx_on: bool, voice_on: bool, track: usize, loop_all: bool) {
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
            speech: vec![None; Voice::ALL.len()],
            speaking: None,
            voice_on,
            duck: 1.0,
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
                self.voices.push(SfxVoice { sfx: idx, pos: 0.0, rate, gain: sfx.gain() * SFX_GAIN });
            }
            Cmd::Say(v) => {
                if !self.voice_on || self.suspended {
                    return;
                }
                let idx = Voice::ALL.iter().position(|x| *x == v).unwrap_or(0);
                if let Some((cur, _)) = self.speaking {
                    if Voice::ALL[cur].rank() >= v.rank() {
                        return;
                    }
                }
                if self.speech[idx].is_none() {
                    self.speech[idx] = Some(decode_wav(v.bytes()).unwrap_or_default());
                }
                self.speaking = Some((idx, 0));
            }
            Cmd::VoiceOn(on) => {
                self.voice_on = on;
                if !on {
                    self.speaking = None;
                }
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
                self.speaking = None;
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
        let duck_target = if self.speaking.is_some() { DUCK_GAIN } else { 1.0 };
        if !self.music.is_empty() {
            for s in buf.iter_mut() {
                self.music_gain += (target_gain - self.music_gain) * rate;
                self.duck += (duck_target - self.duck) * 0.0004;
                if self.music_gain > 0.0005 {
                    *s += self.music[self.music_pos] * MUSIC_GAIN * self.music_gain * self.duck;
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
        // реплика
        if let Some((idx, pos)) = self.speaking {
            let clip = self.speech[idx].as_deref().unwrap_or(&[]);
            let mut p = pos;
            for s in buf.iter_mut() {
                if p >= clip.len() {
                    break;
                }
                *s += clip[p] * VOICE_GAIN;
                p += 1;
            }
            self.speaking = if p >= clip.len() { None } else { Some((idx, p)) };
        }
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
    fn all_voice_clips_decode() {
        for v in Voice::ALL {
            let pcm = decode_wav(v.bytes()).expect("voice wav decodes");
            assert!(pcm.len() > RATE as usize / 10, "{v:?} короче 0.1 с");
            assert!(pcm.len() < RATE as usize * 4, "{v:?} длиннее 4 с");
            assert!(v.rank() >= 1);
        }
    }

    #[test]
    fn noop_without_init() {
        play(Sfx::Place);
        say(Voice::Nice);
        set_voice(false);
        set_music(false);
        suspend();
        resume();
    }
}
