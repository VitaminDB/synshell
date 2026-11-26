use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Recording;
use crate::{paths, t};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Status {
    pub state: String,
    pub pid: i32,
    pub path: String,
    pub started_ms: u64,
    pub output: String,
    pub mode: String,
    pub region: Option<[i32; 4]>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub encoder: String,
    pub audio: bool,
    pub mic: bool,
    pub cursor: bool,
    pub seconds: f64,
    pub frames: u64,
    pub captured: u64,
    pub dropped: u64,
    pub bytes: u64,
    pub audio_seconds: f64,
    pub error: String,
}

impl Status {
    pub fn active(&self) -> bool {
        matches!(self.state.as_str(), "starting" | "recording" | "saving") && alive(self.pid)
    }

    pub fn elapsed(&self) -> f64 {
        if self.started_ms == 0 {
            return 0.0;
        }
        (now_ms().saturating_sub(self.started_ms)) as f64 / 1000.0
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn alive(pid: i32) -> bool {
    pid > 0 && std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|c| String::from_utf8_lossy(&c).contains("--record"))
}

pub fn state_file() -> PathBuf {
    let sock = paths::socket_path();
    let name = sock.file_name().and_then(|n| n.to_str()).unwrap_or("synwm.sock").to_string();
    let key = name.trim_start_matches("synwm.").trim_end_matches(".sock").to_string();
    paths::runtime_dir().join("synshell-screenrec").join(format!("{key}.json"))
}

pub fn log_file() -> PathBuf {
    state_file().with_extension("log")
}

pub fn read() -> Option<Status> {
    serde_json::from_slice(&std::fs::read(state_file()).ok()?).ok()
}

pub fn current() -> Option<Status> {
    read().filter(Status::active)
}

pub fn write(st: &Status) {
    let p = state_file();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, serde_json::to_vec_pretty(st).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Screen,
    Area,
    Window,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s.trim() {
            "screen" | "output" | "full" => Mode::Screen,
            "area" | "region" => Mode::Area,
            "window" => Mode::Window,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Screen => "screen",
            Mode::Area => "area",
            Mode::Window => "window",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Opts {
    pub mode: Mode,
    pub output: Option<String>,
    pub region: Option<[f64; 4]>,
    pub window: Option<u64>,
    pub fps: u32,
    pub audio: bool,
    pub mic: bool,
    pub cursor: bool,
    pub codec: String,
    pub encoder: String,
    pub quality: String,
    pub duration: Option<f64>,
    pub path: Option<PathBuf>,
}

impl Opts {
    pub fn from_config(c: &Recording) -> Opts {
        Opts {
            mode: Mode::parse(&c.mode).unwrap_or(Mode::Screen),
            output: None,
            region: None,
            window: None,
            fps: c.fps.clamp(1, 120),
            audio: c.audio,
            mic: c.mic,
            cursor: c.cursor,
            codec: c.codec.clone(),
            encoder: c.encoder.clone(),
            quality: c.quality.clone(),
            duration: None,
            path: None,
        }
    }
}

pub fn dir(c: &Recording) -> PathBuf {
    let d = c.dir.trim();
    if d.is_empty() {
        paths::user_dir_or_default("VIDEOS").join("Записи экрана")
    } else {
        paths::expand_tilde(d)
    }
}

pub fn timestamp() -> String {
    let mut t: libc::time_t = 0;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::time(&mut t);
        libc::localtime_r(&t, &mut tm);
    }
    format!("{:04}-{:02}-{:02}_{:02}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

pub fn new_path(dir: &Path) -> PathBuf {
    let stem = format!("Запись_{}", timestamp());
    let mut p = dir.join(format!("{stem}.mp4"));
    let mut n = 2;
    while p.exists() {
        p = dir.join(format!("{stem}_{n}.mp4"));
        n += 1;
    }
    p
}

fn engine() -> PathBuf {
    if let Some(p) = std::env::var_os("SYN_SCREENCAST") {
        return PathBuf::from(p);
    }
    if let Ok(me) = std::env::current_exe() {
        if let Some(p) = me.parent().map(|d| d.join("syn-screencast")).filter(|p| p.exists()) {
            return p;
        }
    }
    PathBuf::from("syn-screencast")
}

pub fn start(o: &Opts, cfg: &Recording) -> Result<Status> {
    if let Some(s) = current() {
        bail!("{}", t!("уже идёт запись в {path}", path = s.path));
    }
    let path = match &o.path {
        Some(p) => p.clone(),
        None => {
            let d = dir(cfg);
            std::fs::create_dir_all(&d).with_context(|| d.display().to_string())?;
            new_path(&d)
        }
    };
    if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d).with_context(|| d.display().to_string())?;
    }
    let _ = std::fs::remove_file(state_file());
    let mut c = Command::new(engine());
    c.arg("--record").arg(&path);
    c.args(["--mode", o.mode.as_str()]);
    if let Some(out) = &o.output {
        c.args(["--output", out]);
    }
    if let Some(r) = o.region {
        c.args(["--region", &format!("{},{},{},{}", r[0], r[1], r[2], r[3])]);
    }
    if let Some(w) = o.window {
        c.args(["--window", &w.to_string()]);
    }
    c.args(["--fps", &o.fps.to_string()]);
    c.args(["--audio", if o.audio { "1" } else { "0" }]);
    c.args(["--mic", if o.mic { "1" } else { "0" }]);
    c.args(["--cursor", if o.cursor { "1" } else { "0" }]);
    c.args(["--codec", &o.codec, "--encoder", &o.encoder, "--quality", &o.quality]);
    if let Some(d) = o.duration {
        c.args(["--duration", &d.to_string()]);
    }
    let log = log_file();
    if let Some(d) = log.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(std::fs::File::create(&log)?);
    use std::os::unix::process::CommandExt;
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = c.spawn().with_context(|| t!("не запустилась syn-screencast"))?;
    let t0 = Instant::now();
    loop {
        if let Ok(Some(code)) = child.try_wait() {
            let st = read().unwrap_or_default();
            let why = if !st.error.is_empty() { st.error } else { last_log_line() };
            bail!("{}", t!("запись не началась ({code}): {why}", code = code, why = why));
        }
        if let Some(s) = read().filter(|s| s.pid == child.id() as i32) {
            match s.state.as_str() {
                "recording" => {
                    std::thread::spawn(move || child.wait());
                    return Ok(s);
                }
                "error" => {
                    let _ = child.wait();
                    bail!("{}", s.error);
                }
                _ => {}
            }
        }
        if t0.elapsed() > Duration::from_secs(20) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{}", t!("запись не началась за 20 с: {why}", why = last_log_line()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn last_log_line() -> String {
    std::fs::read_to_string(log_file()).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).last().unwrap_or("").trim().to_string()
}

pub fn stop() -> Result<Status> {
    let Some(s) = current() else { bail!("{}", t!("запись не идёт")) };
    unsafe { libc::kill(s.pid, libc::SIGTERM) };
    wait_done(s.pid, Duration::from_secs(60))
}

pub fn wait() -> Result<Status> {
    let Some(s) = current() else { return read().context(t!("записи не было")) };
    wait_done(s.pid, Duration::from_secs(u64::MAX / 4))
}

fn wait_done(pid: i32, limit: Duration) -> Result<Status> {
    let t0 = Instant::now();
    while alive(pid) {
        if t0.elapsed() > limit {
            bail!("{}", t!("запись не завершилась (pid {pid})", pid = pid));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    read().context(t!("нет сведений о записи"))
}
