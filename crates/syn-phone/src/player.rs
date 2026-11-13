//! Прослушивание записей разговоров на вкладке «Записи»: `syngui::video::AudioFilePlayer` (тот же, что у Диктофона),
//! огибающая для волны — ffmpeg → моно 8 кГц → пик на каждые 20 мс (волна строится в фоне, звук играет сразу).
//! Плеер живёт в статике (не в сигнале): сигналы — выбранная запись, игра/пауза, позиция, длительность, волна.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::audio::AudioBuffer;
use syngui::prelude::*;
use syngui::video::AudioFilePlayer;

use crate::state::St;

/// Отсчётов огибающей в секунду (столбик волны — максимум из них по корзине).
const ENV_RATE: u32 = 50;
const DECODE_RATE: u32 = 8000;

static PLAYER: Mutex<Option<AudioFilePlayer>> = Mutex::new(None);
/// Поколение открытия: устаревшие потоки (открытие, волна, позиция) прошлой записи молча выходят.
static GEN: AtomicU64 = AtomicU64::new(0);

/// Состояние проигрывателя для интерфейса.
#[derive(Clone, Copy)]
pub struct Play {
    pub sel: RwSignal<Option<PathBuf>>,
    pub playing: RwSignal<bool>,
    pub pos: RwSignal<f64>,
    pub dur: RwSignal<f64>,
    pub wave: RwSignal<Option<Arc<AudioBuffer>>>,
    /// Запись, удаление которой ждёт подтверждения.
    pub confirm_delete: RwSignal<Option<PathBuf>>,
}

impl Play {
    pub fn new() -> Self {
        Play { sel: use_signal(None), playing: use_signal(false), pos: use_signal(0.0), dur: use_signal(0.0), wave: use_signal(None), confirm_delete: use_signal(None) }
    }
}

fn with_player<R>(f: impl FnOnce(&mut AudioFilePlayer) -> R) -> Option<R> {
    PLAYER.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(f)
}

/// Выбрать запись и играть её; та же — пауза/продолжить.
pub fn play(st: St, path: PathBuf) {
    let p = st.play;
    if p.sel.get_untracked().as_ref() == Some(&path) && PLAYER.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        toggle(st);
        return;
    }
    let gen = GEN.fetch_add(1, Ordering::AcqRel) + 1;
    drop_player();
    p.sel.set(Some(path.clone()));
    p.pos.set(0.0);
    p.dur.set(st.recordings.get_untracked().iter().find(|r| r.path == path).map(|r| r.duration).unwrap_or(0.0));
    p.wave.set(None);
    p.confirm_delete.set(None);
    let wpath = path.clone();
    std::thread::spawn(move || {
        if let Some(w) = envelope(&wpath) {
            run_on_main_thread(move || {
                if GEN.load(Ordering::Acquire) == gen {
                    p.wave.set(Some(Arc::new(w)));
                }
            });
        }
    });
    std::thread::spawn(move || {
        let r = AudioFilePlayer::open(&path.to_string_lossy());
        if GEN.load(Ordering::Acquire) != gen {
            return;
        }
        match r {
            Ok(pl) => {
                let d = pl.duration_sec();
                *PLAYER.lock().unwrap_or_else(|e| e.into_inner()) = Some(pl);
                run_on_main_thread(move || {
                    if d > 0.0 {
                        p.dur.set(d);
                    }
                    p.playing.set(true);
                });
                ticker(st, gen);
            }
            Err(e) => {
                let e = e.to_string();
                run_on_main_thread(move || {
                    p.sel.set(None);
                    st.toast(t!("Не удалось открыть запись: {e}", e = e));
                });
            }
        }
    });
}

/// Позиция на экране — 10 раз в секунду, пока запись открыта; конец — пауза в начале.
fn ticker(st: St, gen: u64) {
    let p = st.play;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        if GEN.load(Ordering::Acquire) != gen {
            return;
        }
        let Some((pos, ended, paused)) = with_player(|g| (g.position_sec(), g.is_ended(), g.is_paused())) else { return };
        run_on_main_thread(move || {
            if GEN.load(Ordering::Acquire) != gen {
                return;
            }
            if ended {
                p.playing.set(false);
                p.pos.set(p.dur.get_untracked());
            } else {
                if (p.pos.get_untracked() - pos).abs() > 0.05 {
                    p.pos.set(pos);
                }
                if p.playing.get_untracked() == paused {
                    p.playing.set(!paused);
                }
            }
        });
    }
}

pub fn toggle(st: St) {
    let p = st.play;
    let r = with_player(|g| {
        if g.is_ended() {
            let _ = g.seek(0.0);
            g.play();
        } else if g.is_paused() {
            g.play();
        } else {
            g.pause();
        }
        !g.is_paused()
    });
    match r {
        Some(on) => p.playing.set(on),
        None => {
            if let Some(s) = p.sel.get_untracked() {
                play(st, s);
            }
        }
    }
}

/// Перемотка на `sec` (в границах записи).
pub fn seek(st: St, sec: f64) {
    let p = st.play;
    let d = p.dur.get_untracked();
    let sec = if d > 0.0 { sec.clamp(0.0, d) } else { sec.max(0.0) };
    p.pos.set(sec);
    std::thread::spawn(move || {
        with_player(|g| {
            let _ = g.seek(sec);
            if g.is_ended() || g.is_paused() {
                // перемотали после конца — продолжить с новой точки
                g.play();
            }
        });
    });
    p.playing.set(true);
}

pub fn skip(st: St, by: f64) {
    seek(st, st.play.pos.get_untracked() + by);
}

fn drop_player() {
    if let Some(old) = PLAYER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        std::thread::spawn(move || drop(old));
    }
}

/// Закрыть проигрыватель (ушли с вкладки, удалили запись).
pub fn close(st: St) {
    GEN.fetch_add(1, Ordering::AcqRel);
    drop_player();
    let p = st.play;
    p.sel.set(None);
    p.playing.set(false);
    p.pos.set(0.0);
    p.wave.set(None);
}

/// Огибающая записи для волны: пик |x| на каждые 1/ENV_RATE с.
fn envelope(path: &Path) -> Option<AudioBuffer> {
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", &DECODE_RATE.to_string(), "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let per = (DECODE_RATE / ENV_RATE) as usize;
    let mut env = Vec::new();
    let mut buf = vec![0u8; per * 4];
    loop {
        let mut got = 0;
        while got < buf.len() {
            match out.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(_) => break,
            }
        }
        if got < 4 {
            break;
        }
        let peak = buf[..got - got % 4].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]).abs()).fold(0f32, f32::max);
        env.push(peak);
        if got < buf.len() {
            break;
        }
    }
    let _ = child.wait();
    if env.is_empty() {
        return None;
    }
    // разговор тихий относительно полной шкалы — волна по своему максимуму
    let max = env.iter().copied().fold(0f32, f32::max).max(1e-4);
    let pcm: Vec<f32> = env.iter().map(|v| (v / max).min(1.0)).collect();
    Some(AudioBuffer { pcm: pcm.into(), sample_rate: ENV_RATE, channels: 1 })
}

/// Длительность файла (для списка), секунды.
pub fn probe_duration(path: &Path) -> f64 {
    syngui::video::probe_audio(&path.to_string_lossy()).map(|m| m.duration_sec).unwrap_or(0.0)
}
