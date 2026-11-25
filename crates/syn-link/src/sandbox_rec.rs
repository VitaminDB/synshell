//! Запись экрана песочницы (`synlink sandbox-record`): у песочницы нет демона
//! synlink и трансляции, поэтому кадры берутся прямо у её synwm (IPC
//! `FrameStream`, как у демона) и с постоянной частотой отдаются в `ffmpeg`.
//! Строки JSON на stdout — те же, что у `synlink-view --record`
//! (`started` / `done` / `error`); SIGTERM — закончить запись.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::json;
use synshell_common::ipc::{FrameInfo, Request, Response};
use synshell_tr::t;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    STOP.store(true, Ordering::Release);
}

fn emit(v: serde_json::Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

struct Wm {
    rd: BufReader<UnixStream>,
    wr: UnixStream,
}

impl Wm {
    fn send(&mut self, req: &Request) -> Result<()> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.wr.write_all(line.as_bytes())?;
        Ok(())
    }
    fn wait(&mut self) -> Result<Option<FrameInfo>> {
        let mut line = String::new();
        loop {
            if STOP.load(Ordering::Acquire) {
                return Ok(None);
            }
            match self.rd.read_line(&mut line) {
                Ok(0) => bail!("{}", t!("композитор закрыл соединение")),
                Ok(_) => break,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        match serde_json::from_str::<Response>(&line)? {
            Response::Frame { frame } => Ok(Some(frame)),
            Response::Error { message } => bail!("{message}"),
            other => bail!("{}", t!("неожиданный ответ: {other}", other = format!("{other:?}"))),
        }
    }
}

fn read_frame(info: &FrameInfo, buf: &mut Vec<u8>) -> Result<()> {
    let n = info.width as usize * info.height as usize * 4;
    let mut f = std::fs::File::open(&info.path).with_context(|| t!("файл кадра"))?;
    buf.resize(n, 0);
    f.read_exact(buf)?;
    Ok(())
}

pub struct Opts {
    pub name: String,
    pub out: Option<PathBuf>,
    pub fps: u32,
    pub height: Option<u32>,
    pub duration: Option<f64>,
    pub mkv: bool,
}

pub fn run(o: Opts) -> Result<()> {
    match record(o) {
        Ok(()) => Ok(()),
        Err(e) => {
            emit(json!({"event": "error", "message": format!("{e:#}")}));
            Err(e)
        }
    }
}

fn record(o: Opts) -> Result<()> {
    unsafe {
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_term as *const () as libc::sighandler_t);
    }
    let sb = crate::sandbox::load(&o.name).with_context(|| t!("песочницы {name} нет", name = o.name))?;
    let s = UnixStream::connect(&sb.socket).with_context(|| t!("нет связи с композитором песочницы"))?;
    s.set_read_timeout(Some(Duration::from_millis(200)))?;
    let mut wm = Wm { wr: s.try_clone()?, rd: BufReader::new(s) };
    wm.send(&Request::FrameStream { output: None, cursor: true, video: None })?;
    let first = wm.wait()?.context(t!("нет первого кадра"))?;
    let (w, h) = (first.width, first.height);
    let latest = Arc::new(Mutex::new(Vec::new()));
    read_frame(&first, &mut latest.lock().unwrap())?;

    let fps = o.fps.clamp(1, 120);
    let path = o.out.clone().unwrap_or_else(|| {
        let stamp = chrono_like_stamp();
        synshell_common::paths::user_dir_or_default("VIDEOS").join(t!("Записи экрана")).join(format!("synlink-sandbox-{}-{stamp}.{}", o.name, if o.mkv { "mkv" } else { "mp4" }))
    });
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    // чётный размер для yuv420p; высота — короткая сторона по запросу
    let scale = match o.height.filter(|h| *h > 0) {
        Some(t) => format!("scale=-2:{}", t & !1),
        None => "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
    };
    let mut ff = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &fps.to_string(), "-i", "-"])
        .args(["-vf", &scale, "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p"])
        .args(if o.mkv { vec![] } else { vec!["-movflags", "+faststart"] })
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| t!("нет ffmpeg"))?;
    let mut stdin = ff.stdin.take().context("stdin ffmpeg")?;
    let info = json!({"width": w, "height": h, "fps": fps, "codec": "h264", "encoder": "libx264", "sound": false, "path": path.display().to_string()});
    let mut started = info.clone();
    started["event"] = json!("started");
    emit(started);

    // кадры композитора — в своём потоке (FrameNext ждёт изменения экрана)
    let reader = {
        let latest = latest.clone();
        std::thread::spawn(move || -> Result<()> {
            let mut buf = Vec::new();
            loop {
                wm.send(&Request::FrameNext { key: false })?;
                match wm.wait()? {
                    Some(fi) if fi.width == w && fi.height == h => {
                        read_frame(&fi, &mut buf)?;
                        std::mem::swap(&mut *latest.lock().unwrap(), &mut buf);
                    }
                    Some(_) => bail!("{}", t!("размер экрана песочницы изменился")),
                    None => return Ok(()),
                }
            }
        })
    };

    let t0 = Instant::now();
    let dt = Duration::from_secs_f64(1.0 / fps as f64);
    let mut n: u64 = 0;
    let mut err = None;
    while !STOP.load(Ordering::Acquire) {
        if o.duration.is_some_and(|d| t0.elapsed().as_secs_f64() >= d) {
            break;
        }
        if reader.is_finished() {
            break;
        }
        let frame = latest.lock().unwrap().clone();
        if let Err(e) = stdin.write_all(&frame) {
            err = Some(anyhow::anyhow!("ffmpeg: {e}"));
            break;
        }
        n += 1;
        // постоянная частота по часам: отставание догоняется повтором кадра
        let next = t0 + dt * n as u32;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    STOP.store(true, Ordering::Release);
    drop(stdin);
    let st = ff.wait()?;
    if let Ok(Err(e)) = reader.join() {
        err = err.or(Some(e));
    }
    if let Some(e) = err {
        bail!("{e:#}");
    }
    if !st.success() {
        bail!("{}", t!("ffmpeg завершился с ошибкой ({st})", st = st));
    }
    let mut done = info;
    done["event"] = json!("done");
    done["seconds"] = json!(n as f64 / fps as f64);
    emit(done);
    Ok(())
}

/// Метка времени для имени файла: ГГГГММДД-ЧЧММСС (местное время).
fn chrono_like_stamp() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}
