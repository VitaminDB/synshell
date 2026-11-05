//! Запись экрана гостя в видеофайл — без рабочего стола компьютера вокруг: кадры те же,
//! что у встроенного экрана (VNC на сокете машины, родное разрешение гостя), постоянная
//! частота кадров (последний кадр повторяется, пропусков нет), кодирование — ffmpeg
//! (NVENC, VAAPI, иначе x264). Звук — поток QEMU в PipeWire: появляется, только когда гость
//! что-то играет, поэтому пишется непрерывно — в паузах тишина, звук не уезжает.
//!
//! Запись живёт отдельным процессом (`syn-vm --record-run`), её состояние —
//! `$XDG_RUNTIME_DIR/syn-vm/<id>/record.json`; остановка — SIGTERM (`--record-stop`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::Vm;
use crate::vnc::Session;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Info {
    pub pid: i32,
    pub path: String,
    /// Unix-время начала.
    pub started: u64,
    pub fps: u32,
    pub audio: bool,
    pub encoder: String,
    pub size: (u32, u32),
    /// Итог (после остановки).
    pub done: Option<Stats>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Stats {
    pub frames: u64,
    pub seconds: f64,
    /// Тики, когда кодировщик не успевал и кадр пришлось повторить для ровного времени.
    pub late: u64,
    /// Сколько разных кадров пришло от гостя.
    pub updates: u64,
    pub bytes: u64,
    pub audio_seconds: f64,
    pub error: String,
}

fn info_path(vm: &Vm) -> PathBuf {
    vm.run_dir().join("record.json")
}

/// Идёт ли запись (и её сведения).
pub fn status(vm: &Vm) -> Option<Info> {
    let i: Info = serde_json::from_str(&std::fs::read_to_string(info_path(vm)).ok()?).ok()?;
    alive(i.pid).then_some(i)
}

fn alive(pid: i32) -> bool {
    pid > 0 && std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| String::from_utf8_lossy(&c).contains("--record-run"))
}

/// Параметры записи.
#[derive(Clone, Debug)]
pub struct Opts {
    pub out: PathBuf,
    pub fps: u32,
    pub audio: bool,
    pub duration: Option<Duration>,
}

/// Путь по умолчанию: «Видео/Записи экрана/<машина> <время>.mp4».
pub fn default_path(vm: &Vm) -> PathBuf {
    let dir = synshell_common::paths::user_dir_or_default("VIDEOS").join("Записи экрана");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("{} {}.mp4", vm.name.replace('/', "-"), crate::state::chrono_like(crate::model::now())))
}

/// Начать запись в отдельном процессе; вернуться, когда пошли кадры.
pub fn start(vm: &Vm, o: &Opts) -> Result<Info> {
    if crate::qemu::pid(vm).is_none() {
        bail!("«{}» не запущена", vm.name);
    }
    if let Some(i) = status(vm) {
        bail!("уже идёт запись в {} (syn-vm --record-stop {})", i.path, vm.id);
    }
    let _ = std::fs::remove_file(info_path(vm));
    let exe = std::env::current_exe()?;
    let mut c = Command::new(exe);
    c.arg("--record-run").arg(&vm.id).arg(&o.out).arg(o.fps.to_string()).arg(if o.audio { "1" } else { "0" }).arg(o.duration.map(|d| d.as_secs_f64()).unwrap_or(0.0).to_string());
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(std::fs::File::create(vm.run_dir().join("record.log"))?);
    use std::os::unix::process::CommandExt;
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = c.spawn().context("запуск записи")?;
    let t0 = Instant::now();
    loop {
        if let Ok(Some(st)) = child.try_wait() {
            let log = std::fs::read_to_string(vm.run_dir().join("record.log")).unwrap_or_default();
            bail!("запись не началась ({st}): {}", log.lines().filter(|l| !l.contains(" INFO ")).last().unwrap_or("").trim());
        }
        if let Some(i) = status(vm).filter(|i| i.size.0 > 0) {
            std::thread::spawn(move || child.wait());
            return Ok(i);
        }
        if t0.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            bail!("запись не началась за 15 с — нет кадра от гостя");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Остановить и дождаться готового файла.
pub fn stop(vm: &Vm) -> Result<Info> {
    let Some(i) = status(vm) else { bail!("у «{}» запись не идёт", vm.name) };
    unsafe { libc::kill(i.pid, libc::SIGTERM) };
    let t0 = Instant::now();
    while alive(i.pid) {
        if t0.elapsed() > Duration::from_secs(60) {
            bail!("запись не завершилась за 60 с (pid {})", i.pid);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let done: Info = serde_json::from_str(&std::fs::read_to_string(info_path(vm))?)?;
    Ok(done)
}

/// Дождаться конца записи (по длительности или остановке).
pub fn wait(vm: &Vm) -> Result<Info> {
    while status(vm).is_some() {
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(serde_json::from_str(&std::fs::read_to_string(info_path(vm))?)?)
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// Подходящий кодировщик: (имя, аргументы до входов, аргументы кодирования).
fn encoder(fps: u32) -> (String, Vec<String>, Vec<String>) {
    let list = Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let gop = (fps * 2).to_string();
    if list.contains("h264_nvenc") && Path::new("/dev/nvidia0").exists() && nvenc_works() {
        return ("h264_nvenc".into(), vec![], [s(&["-c:v", "h264_nvenc", "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", "21", "-b:v", "0", "-pix_fmt", "yuv420p", "-g"]), vec![gop]].concat());
    }
    if list.contains("h264_vaapi") && Path::new("/dev/dri/renderD128").exists() {
        return ("h264_vaapi".into(), s(&["-vaapi_device", "/dev/dri/renderD128"]), [s(&["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi", "-qp", "20", "-g"]), vec![gop]].concat());
    }
    ("libx264".into(), vec![], [s(&["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", "-g"]), vec![gop]].concat())
}

/// NVENC может быть недоступен (драйвер, занятые сеансы) — проверить коротким пробным кодированием.
fn nvenc_works() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=black:s=256x256:d=0.1", "-c:v", "h264_nvenc", "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Поток вывода звука QEMU в PipeWire (object.serial) — есть, пока гость играет.
fn qemu_audio_stream(qemu_pid: i32) -> Option<String> {
    let o = Command::new("pw-dump").stderr(Stdio::null()).output().ok()?;
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).ok()?;
    let clients: Vec<i64> = v
        .as_array()?
        .iter()
        .filter(|x| x.get("type").and_then(|t| t.as_str()).is_some_and(|t| t.ends_with("Client")) && x.pointer("/info/props/application.process.id").map(|p| p.to_string().trim_matches('"') == qemu_pid.to_string()).unwrap_or(false))
        .filter_map(|x| x.get("id")?.as_i64())
        .collect();
    v.as_array()?.iter().find_map(|x| {
        let p = x.pointer("/info/props")?;
        let class = p.get("media.class")?.as_str()?;
        let cid = p.get("client.id").and_then(|c| c.as_i64().or_else(|| c.as_str()?.parse().ok()));
        let mine = cid.is_some_and(|c| clients.contains(&c)) || p.get("application.process.id").map(|a| a.to_string().trim_matches('"') == qemu_pid.to_string()).unwrap_or(false);
        (class == "Stream/Output/Audio" && mine).then(|| p.get("object.serial").map(|s| s.to_string().trim_matches('"').to_string()))?
    })
}

const RATE: usize = 48000;

/// Звук в FIFO для ffmpeg: ровно в реальном времени, из потока QEMU, когда он есть, иначе тишина.
fn audio_pump(fifo: PathBuf, qemu_pid: i32, written: Arc<AtomicU64>) {
    let buf: Arc<Mutex<std::collections::VecDeque<u8>>> = Arc::new(Mutex::new(Default::default()));
    // Поиск потока и захват (перезапуск, если гость закрыл и снова открыл звук).
    let b2 = buf.clone();
    std::thread::spawn(move || {
        let mut child: Option<Child> = None;
        let mut cur = String::new();
        while !STOP.load(Ordering::SeqCst) {
            let serial = qemu_audio_stream(qemu_pid).unwrap_or_default();
            let dead = child.as_mut().is_none_or(|c| c.try_wait().ok().flatten().is_some());
            if serial != cur || (dead && !serial.is_empty()) {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                cur = serial.clone();
                if !serial.is_empty() {
                    let c = Command::new("pw-record")
                        .args(["--target", &serial, "--rate", "48000", "--channels", "2", "--format", "s16", "-P", "stream.capture.sink=false", "-"])
                        .stdin(Stdio::null())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .spawn();
                    if let Ok(mut c) = c {
                        let mut out = c.stdout.take().unwrap();
                        let b3 = b2.clone();
                        std::thread::spawn(move || {
                            let mut chunk = vec![0u8; 19200];
                            while let Ok(n) = out.read(&mut chunk) {
                                if n == 0 {
                                    break;
                                }
                                let mut q = b3.lock().unwrap();
                                q.extend(&chunk[..n]);
                                // Не копить больше полсекунды — запаздывание хуже пропуска.
                                let max = RATE * 4 / 2;
                                if q.len() > max {
                                    let extra = q.len() - max;
                                    q.drain(..extra - extra % 4);
                                }
                            }
                        });
                        child = Some(c);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if let Some(mut c) = child {
            let _ = c.kill();
            let _ = c.wait();
        }
    });
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&fifo) else { return };
    let t0 = Instant::now();
    let mut frames_out: u64 = 0;
    let step = Duration::from_millis(10);
    while !STOP.load(Ordering::SeqCst) {
        let due = (t0.elapsed().as_secs_f64() * RATE as f64) as u64;
        if due > frames_out {
            let n = (due - frames_out) as usize;
            let mut out = vec![0u8; n * 4];
            {
                let mut q = buf.lock().unwrap();
                let take = (q.len() / 4).min(n) * 4;
                for (i, b) in q.drain(..take).enumerate() {
                    out[i] = b;
                }
            }
            if f.write_all(&out).is_err() {
                break;
            }
            frames_out += n as u64;
            written.store(frames_out, Ordering::Relaxed);
        }
        std::thread::sleep(step);
    }
}

/// Сама запись (процесс `--record-run`).
pub fn run(vm: &Vm, o: Opts) -> Result<()> {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as libc::sighandler_t);
    }
    let qpid = crate::qemu::pid(vm).context("машина не запущена")?;
    let fps = o.fps.clamp(1, 120);
    let mut info = Info { pid: std::process::id() as i32, path: o.out.display().to_string(), started: crate::model::now(), fps, audio: o.audio, ..Default::default() };
    let save = |i: &Info| {
        let p = info_path(vm);
        let tmp = p.with_extension("tmp");
        let _ = std::fs::write(&tmp, serde_json::to_string(i).unwrap_or_default());
        let _ = std::fs::rename(tmp, p);
    };
    save(&info);

    // Кадры — свой сеанс VNC (общий доступ, окну не мешает).
    let session = Session::open(vm.vnc_path(), Duration::from_secs(5), Arc::new(|| {}), Arc::new(|_| {}));
    let t0 = Instant::now();
    while session.frame.size().0 == 0 {
        if !session.alive() || t0.elapsed() > Duration::from_secs(10) || STOP.load(Ordering::SeqCst) {
            bail!("нет кадра от гостя");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (fw, fh) = session.frame.size();
    // yuv420 — чётные стороны.
    let (w, h) = (fw & !1, fh & !1);
    let (enc, pre, args) = encoder(fps);

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "warning", "-y"]).args(&pre);
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-framerate", &fps.to_string(), "-thread_queue_size", "512", "-i", "-"]);
    let fifo = vm.run_dir().join("record-audio.fifo");
    let audio_written = Arc::new(AtomicU64::new(0));
    if o.audio {
        let _ = std::fs::remove_file(&fifo);
        let c = std::ffi::CString::new(fifo.to_string_lossy().as_bytes())?;
        if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
            bail!("mkfifo: {}", std::io::Error::last_os_error());
        }
        cmd.args(["-f", "s16le", "-ar", "48000", "-ac", "2", "-thread_queue_size", "1024", "-i"]).arg(&fifo);
    }
    cmd.args(&args);
    if o.audio {
        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    }
    let ext = o.out.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_lowercase();
    if ext == "mp4" || ext == "mov" {
        cmd.args(["-movflags", "+faststart"]);
    }
    cmd.arg(&o.out).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut ff = cmd.spawn().context("ffmpeg не найден (пакет ffmpeg)")?;
    let mut stdin = ff.stdin.take().unwrap();
    let mut fferr = ff.stderr.take().unwrap();
    let errlog = Arc::new(Mutex::new(String::new()));
    let e2 = errlog.clone();
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = fferr.read_to_string(&mut s);
        *e2.lock().unwrap() = s;
    });
    if o.audio {
        let (f2, aw) = (fifo.clone(), audio_written.clone());
        std::thread::spawn(move || audio_pump(f2, qpid, aw));
    }

    info.encoder = enc;
    info.size = (w, h);
    save(&info);
    tracing::info!("запись {}: {w}x{h} {fps} к/с, {}", o.out.display(), info.encoder);

    let period = Duration::from_secs_f64(1.0 / fps as f64);
    let mut stats = Stats::default();
    let mut last_ver = 0u64;
    let mut frame: Vec<u8> = vec![0; (w * h * 4) as usize];
    let mut write_err = String::new();
    // Первый кадр — до отсчёта: ffmpeg и кодировщик открываются не мгновенно.
    {
        let (sw, sh, rgba, ver) = session.frame.latest();
        last_ver = last_ver.max(ver);
        fit_into(&rgba, sw, sh, &mut frame, w, h);
        stdin.write_all(&frame).context("ffmpeg")?;
        stats.frames = 1;
        stats.updates = 1;
    }
    // Запись в ffmpeg — своим потоком через очередь: короткие подпоры кодировщика
    // не сбивают такт; переполнение очереди — настоящее отставание.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Arc<Vec<u8>>>(fps as usize);
    let werr = Arc::new(Mutex::new(String::new()));
    let werr2 = werr.clone();
    let writer = std::thread::spawn(move || {
        for f in rx {
            if let Err(e) = stdin.write_all(&f) {
                *werr2.lock().unwrap() = e.to_string();
                break;
            }
        }
        drop(stdin);
    });
    let mut cur = Arc::new(frame.clone());
    let start = Instant::now() - period;
    loop {
        if STOP.load(Ordering::SeqCst) || o.duration.is_some_and(|d| start.elapsed() >= d) {
            break;
        }
        if !session.alive() {
            stats.error = "экран гостя отключился (машина выключена?)".into();
            break;
        }
        let (sw, sh, rgba, ver) = session.frame.latest();
        if ver != last_ver && sw > 0 {
            last_ver = ver;
            stats.updates += 1;
            fit_into(&rgba, sw, sh, &mut frame, w, h);
            cur = Arc::new(frame.clone());
        }
        // Сколько кадров уже положено по времени — столько и записать (ровно, без пропусков).
        let due = (start.elapsed().as_secs_f64() * fps as f64).floor() as u64 + 1;
        let n = due.saturating_sub(stats.frames);
        if n > 1 {
            stats.late += n - 1;
        }
        for _ in 0..n {
            match tx.try_send(cur.clone()) {
                Ok(()) => {}
                Err(std::sync::mpsc::TrySendError::Full(f)) => {
                    // Кодировщик отстал на секунду — ждать его (время в файле остаётся ровным).
                    stats.late += 1;
                    if tx.send(f).is_err() {
                        write_err = werr.lock().unwrap().clone();
                        break;
                    }
                }
                Err(_) => {
                    write_err = werr.lock().unwrap().clone();
                    break;
                }
            }
            stats.frames += 1;
        }
        if !write_err.is_empty() {
            break;
        }
        let next = start + period.mul_f64(stats.frames as f64);
        if let Some(d) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
    STOP.store(true, Ordering::SeqCst);
    session.close();
    drop(tx);
    let _ = writer.join();
    let st = ff.wait()?;
    let _ = std::fs::remove_file(&fifo);
    stats.seconds = stats.frames as f64 / fps as f64;
    stats.audio_seconds = audio_written.load(Ordering::Relaxed) as f64 / RATE as f64;
    stats.bytes = std::fs::metadata(&o.out).map(|m| m.len()).unwrap_or(0);
    if !st.success() {
        stats.error = format!("ffmpeg: {}", errlog.lock().unwrap().trim());
    } else if !write_err.is_empty() && stats.error.is_empty() {
        stats.error = format!("ffmpeg: {write_err}");
    }
    info.done = Some(stats);
    save(&info);
    Ok(())
}

/// Кадр гостя в кадр записи: тот же размер — копия, иначе вписать с полями (гость сменил разрешение).
fn fit_into(src: &[u8], sw: u32, sh: u32, dst: &mut [u8], w: u32, h: u32) {
    // Тот же размер (или на пиксель больше — нечётная сторона): построчная копия.
    if sw >= w && sh >= h && sw - w <= 1 && sh - h <= 1 {
        let row = (w * 4) as usize;
        for y in 0..h as usize {
            let s = y * sw as usize * 4;
            dst[y * row..(y + 1) * row].copy_from_slice(&src[s..s + row]);
        }
        return;
    }
    let scale = (w as f64 / sw as f64).min(h as f64 / sh as f64);
    let (tw, th) = ((sw as f64 * scale) as u32, (sh as f64 * scale) as u32);
    let (ox, oy) = ((w - tw.min(w)) / 2, (h - th.min(h)) / 2);
    dst.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&[0, 0, 0, 255]));
    for y in 0..th.min(h) {
        let sy = ((y as f64 + 0.5) / scale) as u32;
        let sy = sy.min(sh - 1) as usize;
        for x in 0..tw.min(w) {
            let sx = (((x as f64 + 0.5) / scale) as u32).min(sw - 1) as usize;
            let si = (sy * sw as usize + sx) * 4;
            let di = (((y + oy) * w + x + ox) * 4) as usize;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit() {
        let src = vec![255u8; 4 * 4 * 4];
        let mut dst = vec![0u8; 8 * 4 * 4];
        fit_into(&src, 4, 4, &mut dst, 8, 4);
        // Поля слева чёрные, середина белая.
        assert_eq!(&dst[0..4], &[0, 0, 0, 255]);
        let mid = ((1 * 8 + 4) * 4) as usize;
        assert_eq!(&dst[mid..mid + 4], &[255, 255, 255, 255]);
        let mut same = vec![0u8; 4 * 4 * 4];
        fit_into(&src, 4, 4, &mut same, 4, 4);
        assert_eq!(same, src);
    }
}
