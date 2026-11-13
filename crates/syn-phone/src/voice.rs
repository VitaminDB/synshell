//! Звук разговора: отвод palaudiod (`/run/palaudio/voice-tap`) — строка JSON с форматом, дальше PCM s16le
//! (левый канал — свой микрофон, правый — собеседник). Из него — уровни голосов для визуализации и запись
//! разговора (ffmpeg → Opus, моно: оба голоса). На устройствах с голосом через ALSA q6voice (call-audio, способ alsa:
//! SM-T295 и подобные) отвода собеседника нет — звук берётся с микрофона через PipeWire: на громкой связи в нём оба
//! голоса, визуализация — одна дорожка «Разговор». Ни того ни другого — запись и визуализация недоступны.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub const TAP: &str = "/run/palaudio/voice-tap";
/// Настройки звука звонка способа alsa (call-audio): есть — запись и уровни с микрофона.
pub const ALSA_CONF: &str = "/etc/synmobile/call-audio.conf";
/// Частота записи с микрофона.
const MIC_RATE: u32 = 16000;
/// Усиление записи с микрофона: во время звонка кодек стоит на голосовых уровнях стока (ADC1 6, цифра 0 дБ) —
/// на 18 дБ тише записи вне звонка (docs/27 §27.33); +12 дБ возвращает речь к уровню диктофона.
const MIC_GAIN: i32 = 4;
/// Уровней на секунду визуализации.
pub const LEVELS_PER_SEC: usize = 20;
const KEEP: usize = 600;

pub fn available() -> bool {
    std::env::var_os("SYNPHONE_DEMO_TAP").is_some() || Path::new(TAP).exists() || Path::new(ALSA_CONF).exists()
}

/// Источник — одна дорожка (микрофон, без отвода собеседника): визуализация одной полосой.
pub fn mono_source() -> bool {
    std::env::var_os("SYNPHONE_DEMO_TAP").is_none() && !Path::new(TAP).exists() && Path::new(ALSA_CONF).exists()
}

/// Уровни голосов (0..1, громкость в «слышимой» шкале), новые в конце.
#[derive(Default)]
pub struct Vis {
    /// Собеседник.
    pub them: VecDeque<f32>,
    /// Свой микрофон.
    pub me: VecDeque<f32>,
    /// Звук идёт.
    pub live: bool,
    /// Одна дорожка (микрофон, оба голоса вместе): `them` пустой.
    pub mono: bool,
    pub last: Option<Instant>,
}

static VIS: Mutex<Vis> = Mutex::new(Vis { them: VecDeque::new(), me: VecDeque::new(), live: false, mono: false, last: None });

pub fn vis() -> MutexGuard<'static, Vis> {
    VIS.lock().unwrap()
}

struct Rec {
    child: Child,
    stdin: Option<ChildStdin>,
    path: PathBuf,
}

static REC: Mutex<Option<Rec>> = Mutex::new(None);
/// Звонок идёт — поток отвода держит соединение.
static CALL: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static RATE: Mutex<u32> = Mutex::new(16000);

/// Звонок начался/кончился: отвод включается на время звонка.
pub fn set_call(on: bool) {
    CALL.store(on, Ordering::SeqCst);
    if !on {
        let mut v = vis();
        v.live = false;
        v.them.clear();
        v.me.clear();
        return;
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("voice-tap".into())
        .spawn(|| {
            while CALL.load(Ordering::SeqCst) {
                let r = if std::env::var_os("SYNPHONE_DEMO_TAP").is_some() {
                    demo()
                } else if Path::new(TAP).exists() {
                    tap()
                } else {
                    mic()
                };
                if let Err(e) = r {
                    tracing::debug!("отвод звука разговора: {e}");
                }
                vis().live = false;
                std::thread::sleep(Duration::from_millis(500));
            }
            RUNNING.store(false, Ordering::SeqCst);
        })
        .ok();
}

fn tap() -> std::io::Result<()> {
    let s = std::os::unix::net::UnixStream::connect(TAP)?;
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut r = BufReader::new(s);
    let mut head = String::new();
    r.read_line(&mut head)?;
    let num = |key: &str| -> Option<u32> {
        let i = head.find(&format!("\"{key}\""))? + key.len() + 2;
        let rest = head[i..].trim_start_matches([':', ' ']);
        rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
    };
    let rate = num("rate").unwrap_or(16000).clamp(8000, 48000);
    let ch = num("channels").unwrap_or(2).clamp(1, 2) as usize;
    *RATE.lock().unwrap() = rate;
    vis().mono = ch == 1;
    let frames = rate as usize / LEVELS_PER_SEC;
    let mut buf = vec![0u8; frames * ch * 2];
    while CALL.load(Ordering::SeqCst) {
        r.read_exact(&mut buf)?;
        feed(&buf, ch);
    }
    Ok(())
}

/// Микрофон через PipeWire (способ alsa): сырой s16le моно из pw-record, пока идёт звонок.
fn mic() -> std::io::Result<()> {
    let mut child = Command::new("pw-record")
        .args(["--raw", "--format", "s16", "--channels", "1", "--rate", &MIC_RATE.to_string(), "--media-role", "Communication", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    *RATE.lock().unwrap() = MIC_RATE;
    vis().mono = true;
    let mut out = child.stdout.take().ok_or_else(|| std::io::Error::other("pw-record без stdout"))?;
    let frames = MIC_RATE as usize / LEVELS_PER_SEC;
    let mut buf = vec![0u8; frames * 2];
    let r = (|| {
        while CALL.load(Ordering::SeqCst) {
            out.read_exact(&mut buf)?;
            for s in buf.chunks_exact_mut(2) {
                let v = (i16::from_le_bytes([s[0], s[1]]) as i32 * MIC_GAIN).clamp(-32768, 32767) as i16;
                s.copy_from_slice(&v.to_le_bytes());
            }
            feed(&buf, 1);
        }
        Ok(())
    })();
    let _ = child.kill();
    let _ = child.wait();
    r
}

/// Отладка без модема: два «голоса» по очереди.
fn demo() -> std::io::Result<()> {
    let rate = 16000usize;
    *RATE.lock().unwrap() = rate as u32;
    let frames = rate / LEVELS_PER_SEC;
    let mut t = 0usize;
    while CALL.load(Ordering::SeqCst) {
        let mut buf = vec![0u8; frames * 4];
        for i in 0..frames {
            let n = (t + i) as f32;
            let sec = n / rate as f32;
            let talk = |phase: f32| ((sec * 0.35 + phase).sin() > 0.2) as u8 as f32 * (0.4 + 0.6 * ((sec * 7.0).sin() * 0.5 + 0.5));
            let me = (n * 0.05).sin() * 9000.0 * talk(0.0);
            let them = (n * 0.031).sin() * 11000.0 * talk(3.1);
            buf[i * 4..i * 4 + 2].copy_from_slice(&(me as i16).to_le_bytes());
            buf[i * 4 + 2..i * 4 + 4].copy_from_slice(&(them as i16).to_le_bytes());
        }
        t += frames;
        feed(&buf, 2);
        std::thread::sleep(Duration::from_millis(1000 / LEVELS_PER_SEC as u64));
    }
    Ok(())
}

/// Громкость RMS → 0..1 (−60…0 дБFS).
fn loud(sum2: f64, n: usize) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let rms = (sum2 / n as f64).sqrt() / 32768.0;
    let db = 20.0 * rms.max(1e-6).log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0) as f32
}

/// Кусок PCM: уровни и запись (моно — оба голоса).
fn feed(buf: &[u8], ch: usize) {
    let n = buf.len() / 2 / ch;
    let (mut s_me, mut s_them) = (0f64, 0f64);
    let mut mono = Vec::with_capacity(n * 2);
    for i in 0..n {
        let me = i16::from_le_bytes([buf[i * ch * 2], buf[i * ch * 2 + 1]]) as i32;
        let them = if ch == 2 { i16::from_le_bytes([buf[i * 4 + 2], buf[i * 4 + 3]]) as i32 } else { me };
        s_me += (me * me) as f64;
        s_them += (them * them) as f64;
        let m = if ch == 2 { (me + them).clamp(-32768, 32767) } else { me };
        mono.extend_from_slice(&(m as i16).to_le_bytes());
    }
    {
        let mut v = vis();
        v.me.push_back(loud(s_me, n));
        v.them.push_back(loud(s_them, if ch == 2 { n } else { 0 }));
        while v.me.len() > KEEP {
            v.me.pop_front();
        }
        while v.them.len() > KEEP {
            v.them.pop_front();
        }
        v.live = true;
        v.last = Some(Instant::now());
    }
    let mut rec = REC.lock().unwrap();
    if let Some(r) = rec.as_mut() {
        if let Some(w) = r.stdin.as_mut() {
            if w.write_all(&mono).is_err() {
                r.stdin = None;
            }
        }
    }
}

/// Идёт запись: путь.
pub fn recording() -> Option<PathBuf> {
    REC.lock().unwrap().as_ref().map(|r| r.path.clone())
}

/// Начать запись в `path` (Opus через ffmpeg).
pub fn record_start(path: &Path) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let rate = *RATE.lock().unwrap();
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "s16le", "-ar", &rate.to_string(), "-ac", "1", "-i", "-"])
        .args(["-c:a", "libopus", "-b:a", "32k", "-application", "voip", "-f", "ogg"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let stdin = child.stdin.take();
    let mut rec = REC.lock().unwrap();
    if let Some(old) = rec.take() {
        finish(old);
    }
    *rec = Some(Rec { child, stdin, path: path.to_path_buf() });
    Ok(())
}

fn finish(mut r: Rec) {
    drop(r.stdin.take());
    let _ = r.child.wait();
}

/// Остановить запись; путь файла (если он не пустой).
pub fn record_stop() -> Option<PathBuf> {
    let r = REC.lock().unwrap().take()?;
    let path = r.path.clone();
    finish(r);
    match std::fs::metadata(&path) {
        Ok(m) if m.len() > 200 => Some(path),
        _ => {
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(loud(0.0, 100), 0.0);
        let full = 32767f64 * 32767.0;
        assert!(loud(full, 1) > 0.99);
        // −20 дБ → 2/3 шкалы
        let a = 3277f64;
        assert!((loud(a * a, 1) - 0.667).abs() < 0.01);
    }
}
