//! Звук диктофона: микрофон (`syngui::audio::AudioRecorder`, потоковый режим, узел PipeWire), поток кадров
//! в своём потоке → кодировщик (ffmpeg: Opus/FLAC; WAV — `WavStreamWriter`) и пики по 50 мс для волны.
//!
//! Микрофон может быть открыт и без записи — «монитор»: волна и уровень живые, файл не пишется. Запись
//! («дубль», [`Take`]) идёт во временный `….part` и переименовывается при остановке, рядом пишутся сведения
//! (длительность, огибающая). Всё блокирующее (открытие устройства, ожидание ffmpeg) — не в потоке интерфейса.

use std::collections::VecDeque;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use syngui::audio::{AudioRecorder, RecorderOptions, WavStreamWriter};
use syngui::t;

use crate::store::{self, Format, Meta, Settings};

/// Длительность одного столбика волны.
pub const BIN_SECS: f64 = 0.05;
/// Столбиков в живой ленте монитора (30 с).
const LIVE_CAP: usize = 600;

/// Состояние для рисования: читает холст, пишет поток кадров.
pub struct Vis {
    /// Пики последних столбиков (монитор и запись).
    pub live: VecDeque<f32>,
    /// Пики записанной части текущего дубля.
    pub hist: Vec<f32>,
    /// Пик последнего столбика.
    pub level: f32,
    /// Удерживаемый пик для индикатора.
    pub hold: f32,
    pub hold_at: Instant,
    /// Последняя перегрузка (сэмпл у предела).
    pub clip_at: Option<Instant>,
    /// Когда пришёл последний столбик — плавная прокрутка между ними.
    pub last_bin: Instant,
    /// Микрофон открыт.
    pub open: bool,
    /// Идёт дубль (пишется файл).
    pub recording: bool,
    pub paused: bool,
    /// Записано сэмплов в текущем дубле.
    pub samples: u64,
    pub rate: u32,
}

impl Vis {
    fn new() -> Self {
        Vis {
            live: VecDeque::with_capacity(LIVE_CAP),
            hist: Vec::new(),
            level: 0.0,
            hold: 0.0,
            hold_at: Instant::now(),
            clip_at: None,
            last_bin: Instant::now(),
            open: false,
            recording: false,
            paused: false,
            samples: 0,
            rate: 48_000,
        }
    }

    /// Длительность дубля, с.
    pub fn elapsed(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.samples as f64 / self.rate as f64
        }
    }
}

pub fn vis() -> MutexGuard<'static, Vis> {
    static V: OnceLock<Mutex<Vis>> = OnceLock::new();
    V.get_or_init(|| Mutex::new(Vis::new())).lock().unwrap_or_else(|e| e.into_inner())
}

// ─── кодировщик ────────────────────────────────────────────────────────────

enum Sink {
    Ffmpeg { child: Child, stdin: Option<BufWriter<ChildStdin>> },
    Wav(WavStreamWriter),
}

impl Sink {
    fn open(fmt: Format, kbps: u32, rate: u32, part: &Path, title: &str) -> Result<Sink, String> {
        if fmt == Format::Wav {
            return WavStreamWriter::open(part, rate, 1).map(Sink::Wav).map_err(|e| e.to_string());
        }
        let mut c = Command::new("ffmpeg");
        c.args(["-hide_banner", "-loglevel", "error", "-f", "f32le", "-ar", &rate.to_string(), "-ac", "1", "-i", "-"]);
        match fmt {
            Format::Opus => {
                c.args(["-c:a", "libopus", "-b:a", &format!("{}k", kbps.clamp(6, 510)), "-f", "ogg"]);
            }
            _ => {
                c.args(["-c:a", "flac", "-sample_fmt", "s16", "-f", "flac"]);
            }
        }
        c.args(["-metadata", &format!("title={title}"), "-y"]).arg(part);
        c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = c.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { t!("Не найден ffmpeg — запись в Opus и FLAC невозможна; выберите WAV в настройках") } else { format!("ffmpeg: {e}") })?;
        let stdin = child.stdin.take().map(|s| BufWriter::with_capacity(64 * 1024, s));
        Ok(Sink::Ffmpeg { child, stdin })
    }

    fn write(&mut self, s: &[f32]) -> Result<(), String> {
        match self {
            Sink::Wav(w) => w.write_chunk(s).map_err(|e| e.to_string()),
            Sink::Ffmpeg { stdin, .. } => {
                let Some(w) = stdin.as_mut() else { return Ok(()) };
                let mut buf = Vec::with_capacity(s.len() * 4);
                for v in s {
                    buf.extend_from_slice(&v.clamp(-1.0, 1.0).to_le_bytes());
                }
                w.write_all(&buf).map_err(|e| format!("ffmpeg: {e}"))
            }
        }
    }

    /// Дописать и закрыть (ждёт ffmpeg).
    fn finish(self) -> Result<(), String> {
        match self {
            Sink::Wav(mut w) => w.finalize().map_err(|e| e.to_string()),
            Sink::Ffmpeg { mut child, stdin } => {
                if let Some(mut w) = stdin {
                    let _ = w.flush();
                    drop(w);
                }
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                let st = child.wait().map_err(|e| e.to_string())?;
                if st.success() {
                    Ok(())
                } else {
                    Err(format!("ffmpeg: {}", err.trim().lines().last().map(str::to_string).unwrap_or_else(|| syngui::t!("ошибка"))))
                }
            }
        }
    }

    fn kill(self) {
        match self {
            Sink::Wav(_) => {}
            Sink::Ffmpeg { mut child, stdin } => {
                drop(stdin);
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Текущий дубль.
struct Take {
    sink: Sink,
    part: PathBuf,
    dest: PathBuf,
    title: String,
    started: i64,
    mic: String,
    error: Option<String>,
}

fn take() -> MutexGuard<'static, Option<Take>> {
    static T: Mutex<Option<Take>> = Mutex::new(None);
    T.lock().unwrap_or_else(|e| e.into_inner())
}

// ─── микрофон ──────────────────────────────────────────────────────────────

struct Mic {
    _rec: AudioRecorder,
    node: Option<String>,
}

fn mic() -> MutexGuard<'static, Option<Mic>> {
    static M: Mutex<Option<Mic>> = Mutex::new(None);
    M.lock().unwrap_or_else(|e| e.into_inner())
}

/// Поколение микрофона: поток кадров старого не трогает состояние нового.
static GEN: AtomicU64 = AtomicU64::new(0);

/// Открыт ли микрофон и какой узел.
pub fn mic_open() -> Option<Option<String>> {
    mic().as_ref().map(|m| m.node.clone())
}

/// Открыть микрофон `node` (блокирует до 3 с). Открытый другой — закрывается (если не идёт запись).
pub fn open_mic(node: Option<String>) -> Result<(), String> {
    if let Some(cur) = mic_open() {
        if cur == node {
            return Ok(());
        }
        if take().is_some() {
            return Err(t!("Микрофон нельзя сменить во время записи"));
        }
        close_mic();
    }
    let rec = AudioRecorder::start_with(RecorderOptions { pipewire_node: node.clone(), ..RecorderOptions::streaming() }).map_err(|e| e.to_string())?;
    let rate = rec.sample_rate().max(8000);
    let rx = rec.open_stream().and_then(|s| s.take_receiver()).ok_or_else(|| t!("Поток микрофона недоступен"))?;
    let gen = GEN.fetch_add(1, Ordering::AcqRel) + 1;
    {
        let mut v = vis();
        v.open = true;
        v.rate = rate;
        v.live.clear();
        v.level = 0.0;
        v.hold = 0.0;
    }
    tracing::info!("микрофон {:?}: {} Гц ({})", node, rate, rec.actual_device_name());
    std::thread::Builder::new()
        .name("sar-frames".into())
        .spawn(move || frames(rx, rate, gen))
        .map_err(|e| e.to_string())?;
    *mic() = Some(Mic { _rec: rec, node });
    Ok(())
}

/// Закрыть микрофон (запись, если шла, останавливается без сохранения — вызывать после `stop`).
pub fn close_mic() {
    let m = mic().take();
    GEN.fetch_add(1, Ordering::AcqRel);
    {
        let mut v = vis();
        v.open = false;
        v.level = 0.0;
    }
    if let Some(m) = m {
        drop(m);
    }
}

/// Поток кадров: запись в дубль и пики по 50 мс.
fn frames(rx: std::sync::mpsc::Receiver<Vec<f32>>, rate: u32, gen: u64) {
    let bin_len = ((rate as f64 * BIN_SECS) as usize).max(1);
    let mut acc = 0usize;
    let mut peak = 0.0f32;
    while let Ok(chunk) = rx.recv() {
        if GEN.load(Ordering::Acquire) != gen {
            break;
        }
        let paused = vis().paused;
        let mut writing = false;
        {
            let mut t = take();
            if let Some(tk) = t.as_mut() {
                if !paused && tk.error.is_none() {
                    writing = true;
                    if let Err(e) = tk.sink.write(&chunk) {
                        tracing::warn!("{e}");
                        tk.error = Some(e);
                    }
                }
            }
        }
        let mut bins: Vec<f32> = Vec::new();
        let mut clip = false;
        for &s in &chunk {
            let a = s.abs();
            if a > peak {
                peak = a;
            }
            if a >= 0.995 {
                clip = true;
            }
            acc += 1;
            if acc >= bin_len {
                bins.push(peak);
                acc = 0;
                peak = 0.0;
            }
        }
        let mut v = vis();
        if writing {
            v.samples += chunk.len() as u64;
        }
        if clip {
            v.clip_at = Some(Instant::now());
        }
        for b in bins {
            if v.live.len() >= LIVE_CAP {
                v.live.pop_front();
            }
            v.live.push_back(b);
            if writing {
                v.hist.push(b);
            }
            v.level = b;
            if b >= v.hold || v.hold_at.elapsed().as_secs_f32() > 1.2 {
                v.hold = b;
                v.hold_at = Instant::now();
            }
            if writing || !v.paused {
                v.last_bin = Instant::now();
            }
        }
    }
}

// ─── дубль ─────────────────────────────────────────────────────────────────

/// Начать запись в папку из настроек (микрофон должен быть открыт). Возвращает конечный путь.
pub fn start(s: &Settings, mic_label: &str) -> Result<PathBuf, String> {
    if take().is_some() {
        return Err(t!("Запись уже идёт"));
    }
    let dir = s.dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let started = store::now();
    let title = store::default_title(started);
    let dest = store::unique_path(&dir, &store::file_stem_of(&title), s.format.ext(), None);
    let part = part_of(&dest);
    let rate = vis().rate;
    let sink = Sink::open(s.format, s.opus_kbps, rate, &part, &title)?;
    {
        let mut v = vis();
        v.hist.clear();
        v.samples = 0;
        v.paused = false;
        v.recording = true;
        v.clip_at = None;
    }
    *take() = Some(Take { sink, part, dest: dest.clone(), title, started, mic: mic_label.to_string(), error: None });
    Ok(dest)
}

pub fn part_of(dest: &Path) -> PathBuf {
    let mut n = dest.file_name().unwrap_or_default().to_os_string();
    n.push(".part");
    dest.with_file_name(n)
}

pub fn set_paused(p: bool) {
    vis().paused = p;
}

/// Ошибка кодировщика текущего дубля.
pub fn take_error() -> Option<String> {
    take().as_ref().and_then(|t| t.error.clone())
}

/// Остановить и сохранить (блокирует до завершения ffmpeg — вызывать в фоне). Слишком короткая
/// (без звука) запись удаляется.
pub fn stop() -> Result<Option<PathBuf>, String> {
    let Some(tk) = take().take() else { return Ok(None) };
    let (hist, samples, rate) = {
        let mut v = vis();
        v.recording = false;
        v.paused = false;
        (std::mem::take(&mut v.hist), v.samples, v.rate)
    };
    let err = tk.error.clone();
    let r = tk.sink.finish();
    if samples == 0 {
        let _ = std::fs::remove_file(&tk.part);
        return Ok(None);
    }
    if let Some(e) = err.or(r.err()) {
        // что успело записаться — оставить
        if tk.part.exists() {
            let _ = std::fs::rename(&tk.part, &tk.dest);
        }
        return Err(e);
    }
    std::fs::rename(&tk.part, &tk.dest).map_err(|e| format!("{}: {e}", tk.dest.display()))?;
    let duration = samples as f64 / rate.max(1) as f64;
    store::write_meta(&tk.dest, &Meta { title: tk.title, duration, peaks: store::envelope(&hist, store::PEAKS), created: tk.started, mic: tk.mic });
    Ok(Some(tk.dest))
}

/// Отменить дубль: файл удаляется.
pub fn cancel() {
    let Some(tk) = take().take() else { return };
    {
        let mut v = vis();
        v.recording = false;
        v.paused = false;
        v.hist.clear();
        v.samples = 0;
    }
    tk.sink.kill();
    let _ = std::fs::remove_file(&tk.part);
}

// ─── разбор чужих файлов ──────────────────────────────────────────────────

/// Длительность и огибающая файла без сведений (декодирование ffmpeg в 8 кГц моно).
pub fn analyze(path: &Path) -> Option<Meta> {
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let bin = (8000.0 * BIN_SECS) as usize;
    let mut bins = Vec::new();
    let (mut acc, mut peak, mut total) = (0usize, 0.0f32, 0u64);
    let mut buf = vec![0u8; 64 * 1024];
    let mut rest: Vec<u8> = Vec::new();
    loop {
        let n = match out.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        rest.extend_from_slice(&buf[..n]);
        let whole = rest.len() / 4 * 4;
        for c in rest[..whole].chunks_exact(4) {
            let a = f32::from_le_bytes([c[0], c[1], c[2], c[3]]).abs();
            peak = peak.max(a);
            acc += 1;
            total += 1;
            if acc == bin {
                bins.push(peak);
                acc = 0;
                peak = 0.0;
            }
        }
        rest.drain(..whole);
    }
    let _ = child.wait();
    if total == 0 {
        return None;
    }
    if acc > 0 {
        bins.push(peak);
    }
    Some(Meta { duration: total as f64 / 8000.0, peaks: store::envelope(&bins, store::PEAKS), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_ffmpeg() -> bool {
        Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    /// Кодировщики пишут файл верной длительности, разбор его читает.
    #[test]
    fn sinks_and_analyze() {
        let d = std::env::temp_dir().join(format!("sar-eng-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let rate = 44_100;
        // 2 с: 1 с тишины и 1 с синуса 440 Гц
        let tone: Vec<f32> = (0..rate * 2).map(|i| if i < rate { 0.0 } else { 0.5 * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() }).collect();
        let formats: &[Format] = if have_ffmpeg() { &[Format::Wav, Format::Opus, Format::Flac] } else { &[Format::Wav] };
        for &f in formats {
            let p = d.join(format!("t.{}", f.ext()));
            let mut s = Sink::open(f, 64, rate as u32, &p, "тест").unwrap();
            for c in tone.chunks(1024) {
                s.write(c).unwrap();
            }
            s.finish().unwrap();
            assert!(p.metadata().unwrap().len() > 1000, "{f:?}");
            if have_ffmpeg() {
                let m = analyze(&p).unwrap();
                assert!((m.duration - 2.0).abs() < 0.08, "{f:?}: {}", m.duration);
                assert_eq!(m.peaks.len(), 40);
                assert!(m.peaks[5] < 30 && m.peaks[35] > 200, "{f:?}: {:?}", m.peaks);
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn part_names() {
        assert_eq!(part_of(Path::new("/a/Запись 1.ogg")), PathBuf::from("/a/Запись 1.ogg.part"));
    }
}
