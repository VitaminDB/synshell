//! Запись экрана устройства в файл: собранные кадры зрителя → `ffmpeg`
//! подпроцессом (сырой RGBA по каналу) → MP4/MKV в «Видео/Записи экрана».
//!
//! Трансляция шлёт только изменения — кадров нет, пока экран стоит, — поэтому
//! запись идёт с постоянной частотой: на каждый такт по часам уходит последний
//! кадр (повтор, если нового не было). Отстал кодер — такты догоняются
//! повторами, длительность видео совпадает с настоящей. Размер холста — первого
//! кадра; повёрнутый экран вписывается в него с полями.
//!
//! Звук — поток воспроизведения трансляции (`synlink-audio`, synlink
//! `audio.rs`) через `pw-record` без отката на микрофон; что не пришло вовремя
//! (звук выключен, поток застрял) — добивается тишиной по часам, так звук не
//! уезжает от видео. Звук включили посреди записи — подхватывается.
//!
//! «Исходное» — родное разрешение, частота обновления экрана устройства,
//! кодек трансляции, высокое качество. Кодер — аппаратный (NVENC, VAAPI), если
//! открывается, иначе программный.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use syngui::t;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

impl Codec {
    pub fn title(self) -> &'static str {
        match self {
            Codec::H264 => "H.264",
            Codec::Hevc => "HEVC",
            Codec::Av1 => "AV1",
        }
    }
}

/// Параметры записи; запоминаются между запусками.
#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    /// Исходные параметры трансляции (остальные поля — для своих).
    pub source: bool,
    /// Короткая сторона кадра, 0 — как у устройства.
    pub height: u32,
    pub fps: u32,
    pub codec: Codec,
    /// 0 — высокое, 1 — среднее, 2 — низкое.
    pub quality: u8,
    pub mkv: bool,
    /// Записывать звук устройства.
    pub sound: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { source: true, height: 0, fps: 30, codec: Codec::H264, quality: 1, mkv: false, sound: true }
    }
}

pub const HEIGHTS: [u32; 5] = [0, 1440, 1080, 720, 480];
pub const FPS: [u32; 4] = [60, 30, 24, 15];
pub const CODECS: [Codec; 3] = [Codec::H264, Codec::Hevc, Codec::Av1];

fn settings_file() -> PathBuf {
    synshell_common::paths::cache_home().join("synlink-view").join("record.json")
}

pub fn load_settings() -> Settings {
    std::fs::read(settings_file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_settings(s: &Settings) {
    let path = settings_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(s).unwrap_or_default());
}

/// Кодек трансляции (`None` — без потерь): «Исходное» пишет тем же.
static STREAM_CODEC: Mutex<Option<Codec>> = Mutex::new(None);

/// Чем идёт трансляция (поток приёма): `hevc`, `h264` или пусто — без потерь.
pub fn set_stream_codec(codec: &str) {
    let c = match codec {
        "hevc" => Some(Codec::Hevc),
        "h264" => Some(Codec::H264),
        _ => None,
    };
    *STREAM_CODEC.lock().unwrap() = c;
}

/// Итоговые параметры: что пишем на деле.
#[derive(Clone, Copy, Debug)]
struct Plan {
    height: u32,
    fps: u32,
    /// `None` — кодек трансляции (узнаётся к первому кадру).
    codec: Option<Codec>,
    quality: u8,
    mkv: bool,
    sound: bool,
}

/// `refresh` — частота обновления экрана устройства, к/с.
fn plan(s: &Settings, refresh: u32) -> Plan {
    if s.source {
        Plan { height: 0, fps: refresh.clamp(10, 120), codec: None, quality: 0, mkv: s.mkv, sound: s.sound }
    } else {
        Plan { height: s.height, fps: s.fps.clamp(1, 120), codec: Some(s.codec), quality: s.quality.min(2), mkv: s.mkv, sound: s.sound }
    }
}

fn source_codec() -> Codec {
    STREAM_CODEC.lock().unwrap().unwrap_or(Codec::H264)
}

/// Краткое описание исходных параметров для окна настроек.
pub fn source_summary(refresh: u32) -> String {
    let p = plan(&Settings::default(), refresh);
    t!("{fps} к/с · {codec} · высокое качество", fps = p.fps, codec = source_codec().title())
}

/// Что пишется на деле — известно с первого кадра.
#[derive(Clone, Debug, Serialize)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: &'static str,
    pub encoder: &'static str,
    pub sound: bool,
    pub path: PathBuf,
}

// ─── кадры ───────────────────────────────────────────────────────────────────

type Frame = (u32, u32, Arc<[u8]>);

/// Последний кадр, пока идёт запись.
static LATEST: Mutex<Option<Frame>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Звук трансляции сейчас включён (решает зритель).
pub static SOUND_ON: AtomicBool = AtomicBool::new(false);

/// Новый кадр трансляции (поток приёма). Без записи — ничего.
pub fn push(w: u32, h: u32, data: &Arc<[u8]>) {
    if ACTIVE.load(Ordering::Relaxed) {
        *LATEST.lock().unwrap() = Some((w, h, data.clone()));
    }
}

// ─── запись ──────────────────────────────────────────────────────────────────

pub struct Recording {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Result<PathBuf, String>>,
    pub started: Instant,
    /// Параметры файла, когда запись пошла (первый кадр, кодер открылся).
    pub info: Arc<Mutex<Option<Info>>>,
}

impl Recording {
    /// Начать запись в фоне; кадры — через `push`. Файл — `out` или
    /// «Видео/Записи экрана/synlink-<name>-<время>».
    pub fn start(name: &str, s: &Settings, refresh: u32, out: Option<PathBuf>) -> Self {
        let p = plan(s, refresh);
        let stop = Arc::new(AtomicBool::new(false));
        let info = Arc::new(Mutex::new(None));
        *LATEST.lock().unwrap() = None;
        ACTIVE.store(true, Ordering::Relaxed);
        let path = out.unwrap_or_else(|| output_path(name, p.mkv));
        let (st, inf) = (stop.clone(), info.clone());
        let thread = std::thread::Builder::new()
            .name("view-record".into())
            .spawn(move || {
                let r = run(p, &path, &st, &inf);
                ACTIVE.store(false, Ordering::Relaxed);
                *LATEST.lock().unwrap() = None;
                if r.is_err() {
                    let _ = std::fs::remove_file(&path);
                }
                r.map(|()| path)
            })
            .expect("поток записи");
        Self { stop, thread, started: Instant::now(), info }
    }

    /// Запись уже кончилась сама (ошибка кодера).
    pub fn finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Остановить и дождаться файла.
    pub fn finish(self) -> Result<PathBuf, String> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap_or_else(|_| Err(t!("поток записи упал").into()))
    }
}

fn output_path(device_name: &str, mkv: bool) -> PathBuf {
    let dir = synshell_common::paths::user_dir_or_default("VIDEOS").join(t!("Записи экрана"));
    let _ = std::fs::create_dir_all(&dir);
    let name: String = device_name.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    dir.join(format!("synlink-{name}-{}.{}", stamp(), if mkv { "mkv" } else { "mp4" }))
}

/// Местное время `ГГГГ-ММ-ДД_ЧЧ-ММ-СС`.
fn stamp() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

/// Кадр трансляции до начала записи — ждём не дольше этого.
const FIRST_FRAME: Duration = Duration::from_secs(10);

fn run(p: Plan, path: &Path, stop: &AtomicBool, info: &Mutex<Option<Info>>) -> Result<(), String> {
    // Холст — по первому кадру.
    let t0 = Instant::now();
    let first = loop {
        if let Some(f) = LATEST.lock().unwrap().clone() {
            break f;
        }
        if stop.load(Ordering::Relaxed) || t0.elapsed() > FIRST_FRAME {
            return Err(t!("нет кадров трансляции").into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (cw, ch) = (first.0, first.1);
    let (ow, oh) = out_size(cw, ch, p.height);
    let codec = p.codec.unwrap_or_else(source_codec);
    let enc = pick_encoder(codec).ok_or_else(|| t!("нет кодера {codec} в ffmpeg", codec = codec.title()))?;
    *info.lock().unwrap() =
        Some(Info { width: ow, height: oh, fps: p.fps, codec: codec.title(), encoder: enc.name, sound: p.sound, path: path.to_path_buf() });

    // Канал звука: ffmpeg читает его как fd 3.
    let audio = if p.sound { Some(pipe()?) } else { None };
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
    if enc.vaapi {
        cmd.args(["-vaapi_device", VAAPI_DEVICE]);
    }
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-video_size", &format!("{cw}x{ch}"), "-framerate", &p.fps.to_string(), "-i", "pipe:0"]);
    if audio.is_some() {
        cmd.args(["-f", "s16le", "-ar", "48000", "-ac", "2", "-i", "pipe:3"]);
    }
    let mut vf = format!("scale={ow}:{oh}:flags=lanczos:out_color_matrix=bt709:out_range=tv");
    vf.push_str(if enc.vaapi { ",format=nv12,hwupload" } else { ",format=yuv420p" });
    cmd.args(["-vf", &vf]);
    cmd.args(["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"]);
    cmd.args(["-c:v", enc.name, "-g", &(p.fps * 2).to_string()]);
    cmd.args(enc.quality_args(p.quality));
    if audio.is_some() {
        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    }
    if !p.mkv {
        cmd.args(["-movflags", "+faststart"]);
    }
    if codec == Codec::Hevc && !p.mkv {
        // Иначе QuickTime и браузеры не узнают HEVC в MP4.
        cmd.args(["-tag:v", "hvc1"]);
    }
    cmd.arg(path);
    cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    // Своя группа: Ctrl+C в терминале — нам, ffmpeg доводит файл по концу кадров.
    cmd.process_group(0);
    if let Some((r, _)) = &audio {
        let fd = r.as_raw_fd();
        // SAFETY: в потомке до exec — только dup2.
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(fd, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut ff = cmd.spawn().map_err(|e| t!("ffmpeg не запускается: {e}", e = e))?;
    let mut stdin = ff.stdin.take().ok_or("stdin ffmpeg")?;
    let mut stderr = ff.stderr.take().ok_or("stderr ffmpeg")?;
    let err_text = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let audio_done = Arc::new(AtomicBool::new(false));
    let audio_thread = audio.map(|(r, w)| {
        // Свой конец чтения ffmpeg уже унаследовал.
        drop(r);
        let done = audio_done.clone();
        std::thread::Builder::new().name("view-record-audio".into()).spawn(move || audio_loop(w, &done)).ok()
    });

    // Видео: такт за тактом по часам, последний кадр.
    let start = Instant::now();
    let tick = Duration::from_secs_f64(1.0 / p.fps as f64);
    let mut canvas: Vec<u8> = Vec::new();
    let mut n: u64 = 0;
    let mut res = Ok(());
    while !stop.load(Ordering::Relaxed) {
        let due = start + tick * n as u32;
        let now = Instant::now();
        if due > now {
            std::thread::sleep(due - now);
        }
        let Some((w, h, data)) = LATEST.lock().unwrap().clone() else { break };
        let bytes: &[u8] = if (w, h) == (cw, ch) {
            &data
        } else {
            fit(&data, w, h, &mut canvas, cw, ch);
            &canvas
        };
        if let Err(e) = stdin.write_all(bytes) {
            res = Err(t!("ffmpeg прервался: {e}", e = e));
            break;
        }
        n += 1;
    }
    audio_done.store(true, Ordering::Relaxed);
    drop(stdin);
    if let Some(Some(t)) = audio_thread {
        let _ = t.join();
    }
    let status = ff.wait().map_err(|e| e.to_string())?;
    let text = err_text.join().unwrap_or_default();
    if !status.success() {
        let last = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
        log::warn!("ffmpeg: {text}");
        return Err(t!("ffmpeg: {msg}", msg = if last.is_empty() { status.to_string() } else { last }));
    }
    res
}

/// Размер файла: короткая сторона `height` (0 — как есть, больше исходной не
/// растягиваем), обе стороны чётные.
fn out_size(w: u32, h: u32, height: u32) -> (u32, u32) {
    let short = w.min(h);
    let k = if height == 0 || height >= short { 1.0 } else { height as f64 / short as f64 };
    let even = |v: f64| ((v / 2.0).round() as u32 * 2).max(2);
    (even(w as f64 * k), even(h as f64 * k))
}

/// Вписать кадр `w`×`h` в холст `cw`×`ch` с чёрными полями (поворот экрана).
fn fit(src: &[u8], w: u32, h: u32, dst: &mut Vec<u8>, cw: u32, ch: u32) {
    dst.clear();
    dst.resize(cw as usize * ch as usize * 4, 0);
    for px in dst.chunks_exact_mut(4) {
        px[3] = 255;
    }
    if w == 0 || h == 0 {
        return;
    }
    let k = (cw as f64 / w as f64).min(ch as f64 / h as f64);
    let (fw, fh) = (((w as f64 * k) as u32).max(1), ((h as f64 * k) as u32).max(1));
    let (ox, oy) = ((cw - fw.min(cw)) / 2, (ch - fh.min(ch)) / 2);
    for y in 0..fh.min(ch) {
        let sy = ((y as f64 / k) as u32).min(h - 1) as usize;
        let row = &mut dst[((oy + y) as usize * cw as usize + ox as usize) * 4..];
        for x in 0..fw.min(cw) as usize {
            let sx = ((x as f64 / k) as u32).min(w - 1) as usize;
            let s = (sy * w as usize + sx) * 4;
            if s + 4 <= src.len() {
                row[x * 4..x * 4 + 4].copy_from_slice(&src[s..s + 4]);
            }
        }
    }
}

fn pipe() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [0 as libc::c_int; 2];
    // Конец чтения без CLOEXEC не нужен: dup2 в fd 3 снимает флаг у копии.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

// ─── звук ────────────────────────────────────────────────────────────────────

const RATE: u64 = 48_000;
const FRAME_BYTES: u64 = 4;
/// Отстали больше этого — добить тишиной до `KEEP` позади часов.
const LAG: Duration = Duration::from_millis(120);
const KEEP: Duration = Duration::from_millis(40);
/// Поток трансляции в PipeWire (synlink `audio.rs`).
const AUDIO_NODE: &str = "synlink-audio";

/// Звук в ffmpeg: что дал `pw-record`, плюс тишина там, где он молчал, — по часам.
fn audio_loop(w: OwnedFd, done: &AtomicBool) {
    let mut out = std::fs::File::from(w);
    let start = Instant::now();
    let mut written: u64 = 0;
    let mut rec: Option<(Child, mpsc::Receiver<Vec<u8>>)> = None;
    let mut last_try: Option<Instant> = None;
    let bytes_at = |d: Duration| (d.as_secs_f64() * RATE as f64) as u64 * FRAME_BYTES;
    while !done.load(Ordering::Relaxed) {
        let want_sound = SOUND_ON.load(Ordering::Relaxed);
        // Звук включили — подключиться (не чаще раза в секунду); выключили — отпустить.
        if want_sound && rec.is_none() && last_try.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            last_try = Some(Instant::now());
            rec = spawn_pw_record();
        }
        if !want_sound {
            if let Some((mut c, _)) = rec.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        let mut alive = true;
        if let Some((_, rx)) = &rec {
            loop {
                match rx.try_recv() {
                    Ok(chunk) => {
                        // Не убегать вперёд часов дальше задержки канала.
                        if written + (chunk.len() as u64) <= bytes_at(start.elapsed() + LAG) {
                            if out.write_all(&chunk).is_err() {
                                return;
                            }
                            written += chunk.len() as u64;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        alive = false;
                        break;
                    }
                }
            }
        }
        if !alive {
            if let Some((mut c, _)) = rec.take() {
                let _ = c.wait();
            }
        }
        let el = start.elapsed();
        if el > LAG && written < bytes_at(el - LAG) {
            let fill = bytes_at(el - KEEP) - written;
            if out.write_all(&vec![0u8; fill as usize]).is_err() {
                return;
            }
            written += fill;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if let Some((mut c, _)) = rec {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn spawn_pw_record() -> Option<(Child, mpsc::Receiver<Vec<u8>>)> {
    let mut c = Command::new("pw-record")
        .args(["--raw", "--format", "s16", "--rate", "48000", "--channels", "2", "--latency", "20ms", "--target", AUDIO_NODE])
        // Нет потока трансляции — не цепляться к микрофону по умолчанию.
        .args(["-P", "{ node.name = \"synlink-view-record\", node.dont-fallback = true, node.dont-reconnect = true, node.passive = true }", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| log::warn!("pw-record: {e}"))
        .ok()?;
    let mut so = c.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("view-record-pw".into())
        .spawn(move || {
            let mut buf = vec![0u8; 3840];
            while let Ok(n) = so.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some((c, rx))
}

// ─── кодеры ──────────────────────────────────────────────────────────────────

const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

#[derive(Clone, Copy)]
struct Encoder {
    name: &'static str,
    vaapi: bool,
}

impl Encoder {
    /// Качество: высокое, среднее, низкое.
    fn quality_args(self, q: u8) -> Vec<String> {
        let q = q.min(2) as usize;
        let pick = |v: [u32; 3]| v[q].to_string();
        let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let mut a = match self.name {
            "h264_nvenc" => s(&["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-b:v", "0", "-cq", &pick([19, 25, 31])]),
            "hevc_nvenc" => s(&["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-b:v", "0", "-cq", &pick([21, 27, 32])]),
            "av1_nvenc" => s(&["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-b:v", "0", "-cq", &pick([24, 32, 40])]),
            "h264_vaapi" | "hevc_vaapi" | "av1_vaapi" => s(&["-rc_mode", "CQP", "-qp", &pick([20, 26, 32])]),
            "libx264" => s(&["-preset", "veryfast", "-crf", &pick([18, 23, 28])]),
            "libx265" => s(&["-preset", "superfast", "-crf", &pick([20, 26, 31])]),
            "libsvtav1" => s(&["-preset", "10", "-crf", &pick([26, 34, 42])]),
            _ => Vec::new(),
        };
        if self.name == "libsvtav1" {
            a.extend(s(&["-svtav1-params", "lp=4"]));
        }
        a
    }
}

fn candidates(c: Codec) -> [Encoder; 3] {
    let e = |name, vaapi| Encoder { name, vaapi };
    match c {
        Codec::H264 => [e("h264_nvenc", false), e("h264_vaapi", true), e("libx264", false)],
        Codec::Hevc => [e("hevc_nvenc", false), e("hevc_vaapi", true), e("libx265", false)],
        Codec::Av1 => [e("av1_nvenc", false), e("av1_vaapi", true), e("libsvtav1", false)],
    }
}

/// Первый кодер, который на деле открывается (пробный кадр), — запоминается.
fn pick_encoder(c: Codec) -> Option<Encoder> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    candidates(c).into_iter().find(|e| {
        if let Some(&ok) = cache.lock().unwrap().get(e.name) {
            return ok;
        }
        let ok = probe(*e);
        cache.lock().unwrap().insert(e.name, ok);
        ok
    })
}

fn probe(e: Encoder) -> bool {
    let mut c = Command::new("ffmpeg");
    c.args(["-hide_banner", "-loglevel", "quiet", "-nostdin"]);
    if e.vaapi {
        if !Path::new(VAAPI_DEVICE).exists() {
            return false;
        }
        c.args(["-vaapi_device", VAAPI_DEVICE]);
    }
    c.args(["-f", "lavfi", "-i", "color=s=256x256:r=30", "-frames:v", "2"]);
    c.args(["-vf", if e.vaapi { "format=nv12,hwupload" } else { "format=yuv420p" }]);
    c.args(["-c:v", e.name, "-f", "null", "-"]);
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c.status().is_ok_and(|s| s.success())
}

/// ffmpeg установлен.
pub fn available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    })
}
