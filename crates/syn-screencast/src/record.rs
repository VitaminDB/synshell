use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use synshell_common::ipc::{Client, Request, Response};
use synshell_common::screenrec::{self, Status};
use synshell_common::t;
use wayland_client::protocol::{wl_buffer, wl_shm};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1;

use crate::enc::{self, Kind, Video, VideoOpts};
use crate::{memfd, Out, Session};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

#[derive(Clone, Debug)]
pub struct Args {
    pub path: PathBuf,
    pub mode: String,
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
}

impl Args {
    pub fn parse(args: &[String]) -> Args {
        let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
        let flag = |k: &str, d: bool| opt(k).map(|v| !matches!(v.as_str(), "0" | "off" | "no" | "false")).unwrap_or(d);
        Args {
            path: PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "screen.mp4".into())),
            mode: opt("--mode").unwrap_or_else(|| "screen".into()),
            output: opt("--output").filter(|s| !s.is_empty()),
            region: opt("--region").and_then(|s| {
                let v: Vec<f64> = s.split([',', 'x', ' ']).filter_map(|x| x.trim().parse().ok()).collect();
                (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
            }),
            window: opt("--window").and_then(|s| s.parse().ok()),
            fps: opt("--fps").and_then(|s| s.parse().ok()).unwrap_or(60).clamp(1, 120),
            audio: flag("--audio", true),
            mic: flag("--mic", false),
            cursor: flag("--cursor", true),
            codec: opt("--codec").unwrap_or_else(|| "h264".into()),
            encoder: opt("--encoder").unwrap_or_else(|| "auto".into()),
            quality: opt("--quality").unwrap_or_else(|| "high".into()),
            duration: opt("--duration").and_then(|s| s.parse().ok()).filter(|d: &f64| *d > 0.0),
        }
    }
}

const SLOTS: usize = 4;

pub struct SlotMem {
    pub fd: OwnedFd,
    pub map: *mut u8,
    pub size: usize,
    pub stride: u32,
    pub offset: u32,
    pub modifier: Option<u64>,
}

unsafe impl Send for SlotMem {}
unsafe impl Sync for SlotMem {}

impl Drop for SlotMem {
    fn drop(&mut self) {
        if !self.map.is_null() {
            unsafe { libc::munmap(self.map as *mut libc::c_void, self.size) };
        }
    }
}

#[derive(Default)]
struct HubSt {
    latest: Option<usize>,
    seq: u64,
    holds: [u32; SLOTS],
    error: Option<String>,
    captured: u64,
    stop: bool,
}

struct Hub {
    st: Mutex<HubSt>,
    cv: Condvar,
}

fn alloc_slots(s: &Session, w: u32, h: u32, stride: u32, dmabuf: bool) -> Result<Vec<Arc<SlotMem>>> {
    let mut v = Vec::new();
    let gbm = if dmabuf { Some(crate::gbm_device().context("GBM")?) } else { None };
    for _ in 0..SLOTS {
        let m = match &gbm {
            Some(g) => {
                let bo = g.create_buffer_object::<()>(w, h, gbm::Format::Xrgb8888, gbm::BufferObjectFlags::RENDERING | gbm::BufferObjectFlags::LINEAR)?;
                let fd = bo.fd_for_plane(0)?;
                let stride = bo.stride();
                let offset = bo.offset(0);
                let modifier = u64::from(bo.modifier());
                let size = unsafe { libc::lseek(fd.as_raw_fd(), 0, libc::SEEK_END) } as usize;
                let size = if size == 0 { (stride * h + offset) as usize } else { size };
                let map = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
                let map = if map == libc::MAP_FAILED { std::ptr::null_mut() } else { map as *mut u8 };
                SlotMem { fd, map, size, stride, offset, modifier: Some(modifier) }
            }
            None => {
                let size = (stride * h) as usize;
                let fd = memfd(size)?;
                let map = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
                if map == libc::MAP_FAILED {
                    bail!("mmap");
                }
                SlotMem { fd, map: map as *mut u8, size, stride, offset: 0, modifier: None }
            }
        };
        v.push(Arc::new(m));
    }
    let _ = s;
    Ok(v)
}

fn capture(mut s: Session, out: Out, cursor: bool, period: Duration, fmt: wl_shm::Format, w: u32, h: u32, mems: Vec<Arc<SlotMem>>, hub: Arc<Hub>) {
    let r = (|| -> Result<()> {
        let mut bufs: Vec<wl_buffer::WlBuffer> = Vec::new();
        let mut pools = Vec::new();
        for m in &mems {
            match m.modifier {
                Some(modifier) => {
                    let dm = s.st.dmabuf.as_ref().context("linux-dmabuf")?;
                    let params = dm.create_params(&s.qh, ());
                    params.add(m.fd.as_fd(), 0, m.offset, m.stride, (modifier >> 32) as u32, modifier as u32);
                    let b = params.create_immed(w as i32, h as i32, crate::drm_format(fmt), zwp_linux_buffer_params_v1::Flags::empty(), &s.qh, ());
                    params.destroy();
                    bufs.push(b);
                }
                None => {
                    let pool = s.pool(m.fd.as_fd(), m.size as i32);
                    bufs.push(pool.create_buffer(0, w as i32, h as i32, m.stride as i32, fmt, &s.qh, ()));
                    pools.push(pool);
                }
            }
        }
        let mut last = Instant::now() - period;
        loop {
            let wait = period.saturating_sub(last.elapsed());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            let slot = {
                let mut st = hub.st.lock().unwrap();
                loop {
                    if st.stop {
                        return Ok(());
                    }
                    if let Some(i) = (0..SLOTS).find(|i| Some(*i) != st.latest && st.holds[*i] == 0) {
                        break i;
                    }
                    st = hub.cv.wait_timeout(st, Duration::from_millis(50)).unwrap().0;
                }
            };
            let first = hub.st.lock().unwrap().captured == 0;
            last = Instant::now();
            let (frame, (_, fw, fh, _)) = s.begin(&out, cursor)?;
            if (fw, fh) != (w, h) {
                frame.destroy();
                bail!("{}", t!("размер экрана изменился ({w}x{h}) — запись остановлена", w = fw, h = fh));
            }
            let ok = s.finish(&frame, &bufs[slot], !first)?;
            let mut st = hub.st.lock().unwrap();
            if ok {
                st.latest = Some(slot);
                st.seq += 1;
                st.captured += 1;
                hub.cv.notify_all();
            } else if st.captured == 0 {
                bail!("copy-failed");
            }
        }
    })();
    if let Err(e) = r {
        let mut st = hub.st.lock().unwrap();
        st.error = Some(format!("{e:#}"));
        hub.cv.notify_all();
    }
}

struct Geo {
    output: String,
    logical: [f64; 4],
    scale: f64,
}

fn ipc_outputs() -> Vec<Geo> {
    let Ok(mut c) = Client::connect() else { return Vec::new() };
    let _ = c.set_timeout(Duration::from_secs(2));
    match c.request(&Request::Outputs) {
        Ok(Response::Outputs { outputs }) => outputs
            .into_iter()
            .map(|o| Geo { output: o.name, logical: [o.geometry[0] as f64, o.geometry[1] as f64, o.geometry[2] as f64, o.geometry[3] as f64], scale: o.scale })
            .collect(),
        _ => Vec::new(),
    }
}

fn ipc_focused_output() -> Option<String> {
    let mut c = Client::connect().ok()?;
    let _ = c.set_timeout(Duration::from_secs(2));
    match c.request(&Request::Outputs) {
        Ok(Response::Outputs { outputs }) => outputs.into_iter().find(|o| o.focused).map(|o| o.name),
        _ => None,
    }
}

fn ipc_window(id: Option<u64>) -> Option<(u64, [f64; 4], Option<String>)> {
    let mut c = Client::connect().ok()?;
    let _ = c.set_timeout(Duration::from_secs(2));
    match c.request(&Request::Windows) {
        Ok(Response::Windows { windows }) => {
            let w = match id {
                Some(id) => windows.into_iter().find(|w| w.id == id),
                None => windows.into_iter().find(|w| w.focused && !w.minimized),
            }?;
            let g = w.geometry;
            Some((w.id, [g[0] as f64, g[1] as f64, g[2] as f64, g[3] as f64], w.output))
        }
        _ => None,
    }
}

fn contains(r: &[f64; 4], p: (f64, f64)) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

fn to_phys(r: [f64; 4], g: &Geo, full: (u32, u32)) -> [u32; 4] {
    let sx = full.0 as f64 / g.logical[2].max(1.0);
    let sy = full.1 as f64 / g.logical[3].max(1.0);
    let x0 = ((r[0] - g.logical[0]) * sx).round().clamp(0.0, full.0 as f64);
    let y0 = ((r[1] - g.logical[1]) * sy).round().clamp(0.0, full.1 as f64);
    let x1 = ((r[0] + r[2] - g.logical[0]) * sx).round().clamp(0.0, full.0 as f64);
    let y1 = ((r[1] + r[3] - g.logical[1]) * sy).round().clamp(0.0, full.1 as f64);
    [x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32]
}

fn even(c: [u32; 4]) -> [u32; 4] {
    [c[0], c[1], (c[2] & !1).max(2), (c[3] & !1).max(2)]
}

fn place(origin: (u32, u32), size: (u32, u32), full: (u32, u32)) -> [u32; 4] {
    let x = origin.0.min(full.0.saturating_sub(size.0));
    let y = origin.1.min(full.1.saturating_sub(size.1));
    [x, y, size.0, size.1]
}

fn notify(cmd: String) {
    let _ = synshell_common::ipc::send_action(synshell_common::Action::Shell(cmd));
}

pub fn run(a: Args) -> Result<()> {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        ffmpeg_next::ffi::av_log_set_level(ffmpeg_next::ffi::AV_LOG_ERROR as i32);
    }
    let mut st = Status {
        state: "starting".into(),
        pid: std::process::id() as i32,
        path: a.path.display().to_string(),
        mode: a.mode.clone(),
        fps: a.fps,
        codec: a.codec.clone(),
        audio: a.audio,
        mic: a.mic,
        cursor: a.cursor,
        ..Default::default()
    };
    screenrec::write(&st);
    let r = record(&a, &mut st);
    match &r {
        Ok(()) => {
            st.state = "done".into();
            screenrec::write(&st);
            notify(format!("screenrec-saved {}", st.path));
        }
        Err(e) => {
            st.state = "error".into();
            st.error = format!("{e:#}");
            screenrec::write(&st);
            notify(format!("screenrec-failed {}", st.error));
        }
    }
    eprintln!("syn-screencast: {}", serde_json::to_string(&st).unwrap_or_default());
    r
}

fn record(a: &Args, st: &mut Status) -> Result<()> {
    let geos = ipc_outputs();
    let mut follow: Option<u64> = None;
    let mut want: Option<[f64; 4]> = None;
    let mut out_name = a.output.clone();
    match a.mode.as_str() {
        "area" => {
            let r = a.region.context(t!("для записи области нужна --region X,Y,Ш,В"))?;
            want = Some(r);
            if out_name.is_none() {
                let c = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                out_name = geos.iter().find(|g| contains(&g.logical, c)).map(|g| g.output.clone());
            }
        }
        "window" => {
            let (id, r, o) = ipc_window(a.window).with_context(|| match a.window {
                Some(id) => t!("нет окна {id}", id = id),
                None => t!("нет активного окна"),
            })?;
            follow = Some(id);
            want = Some(r);
            if out_name.is_none() {
                let c = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
                out_name = geos.iter().find(|g| contains(&g.logical, c)).map(|g| g.output.clone()).or(o);
            }
        }
        _ => {
            if out_name.is_none() {
                out_name = ipc_focused_output();
            }
        }
    }
    let period = Duration::from_secs_f64(1.0 / a.fps as f64);

    let mut s = Session::connect()?;
    let out = s.output(out_name.as_deref().unwrap_or(""))?;
    let (probe, (fmt, w, h, stride)) = s.begin(&out, a.cursor)?;
    probe.destroy();
    let geo = geos.iter().find(|g| g.output == out.name).map(|g| Geo { output: g.output.clone(), logical: g.logical, scale: g.scale }).unwrap_or(Geo {
        output: out.name.clone(),
        logical: [out.x as f64, out.y as f64, w as f64 / out.scale.max(1) as f64, h as f64 / out.scale.max(1) as f64],
        scale: out.scale.max(1) as f64,
    });
    let full = (w, h);
    let crop0 = match want {
        Some(r) => even(to_phys(r, &geo, full)),
        None => even([0, 0, w, h]),
    };
    if crop0[2] < 16 || crop0[3] < 16 {
        bail!("{}", t!("слишком маленькая область"));
    }
    let size = (crop0[2], crop0[3]);
    let crop = Arc::new(Mutex::new(crop0));

    let want_dma = s.st.dmabuf.is_some() && crate::gbm_device().is_some() && std::env::var("SYN_SCREENREC_SHM").is_err();
    let mut attempt_dma = want_dma;
    let (hub, mems) = loop {
        let mems = alloc_slots(&s, w, h, stride, attempt_dma)?;
        let hub = Arc::new(Hub { st: Mutex::new(HubSt::default()), cv: Condvar::new() });
        let sess = std::mem::replace(&mut s, Session::connect()?);
        let (h2, m2, o2) = (hub.clone(), mems.clone(), sess.output(&out.name)?);
        let cursor = a.cursor;
        std::thread::Builder::new().name("screenrec-capture".into()).spawn(move || capture(sess, o2, cursor, period, fmt, w, h, m2, h2))?;
        let t0 = Instant::now();
        let mut g = hub.st.lock().unwrap();
        while g.captured == 0 && g.error.is_none() && t0.elapsed() < Duration::from_secs(5) && !STOP.load(Ordering::SeqCst) {
            g = hub.cv.wait_timeout(g, Duration::from_millis(50)).unwrap().0;
        }
        if g.captured > 0 {
            drop(g);
            break (hub, mems);
        }
        let err = g.error.clone().unwrap_or_else(|| t!("композитор не отдал кадр за 5 с"));
        g.stop = true;
        drop(g);
        if attempt_dma && !STOP.load(Ordering::SeqCst) {
            eprintln!("syn-screencast: DMA-BUF не подошёл ({err}) — запись через shm");
            attempt_dma = false;
            continue;
        }
        bail!("{err}");
    };
    let dmabuf = mems[0].modifier.is_some();
    let drm_node = std::env::var("SYN_SCREENCAST_DRM").unwrap_or_else(|_| "/dev/dri/renderD128".into());

    let hevc = a.codec == "hevc" || a.codec == "h265";
    let vo = VideoOpts { w: size.0, h: size.1, full, fps: a.fps, hevc, quality: &a.quality, dmabuf, drm_node: &drm_node };
    let mut errs = Vec::new();
    let mut video = None;
    for k in Kind::order(&a.encoder) {
        if k == Kind::Nvenc && mems.iter().any(|m| m.map.is_null()) {
            errs.push("NVENC: нет отображения буфера".to_string());
            continue;
        }
        if k == Kind::X264 && mems.iter().any(|m| m.map.is_null()) {
            continue;
        }
        match Video::open(k, &vo) {
            Ok(v) => {
                video = Some(v);
                break;
            }
            Err(e) => errs.push(format!("{e:#}")),
        }
    }
    let Some(mut video) = video else { bail!("{}", t!("нет видеокодера: {errs}", errs = errs.join("; "))) };
    if !errs.is_empty() {
        eprintln!("syn-screencast: не подошли: {}", errs.join("; "));
    }

    let capture_audio = if a.audio || a.mic {
        match crate::audio::Capture::start(a.audio, a.mic) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("syn-screencast: звук: {e:#}");
                None
            }
        }
    } else {
        None
    };
    let mut aenc = match &capture_audio {
        Some(_) => Some(enc::Audio::open()?),
        None => None,
    };
    let mut mux = enc::Mux::new(&a.path, &video, aenc.as_ref(), hevc)?;

    if let Some(id) = follow {
        let crop = crop.clone();
        let geo2 = Geo { output: geo.output.clone(), logical: geo.logical, scale: geo.scale };
        std::thread::Builder::new().name("screenrec-follow".into()).spawn(move || {
            while !STOP.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
                if let Some((_, r, _)) = ipc_window(Some(id)) {
                    let p = to_phys(r, &geo2, full);
                    let c = place((p[0], p[1]), size, full);
                    *crop.lock().unwrap() = c;
                }
            }
        })?;
    }

    st.state = "recording".into();
    st.output = out.name.clone();
    st.width = size.0;
    st.height = size.1;
    st.region = want.map(|_| [crop0[0] as i32, crop0[1] as i32, crop0[2] as i32, crop0[3] as i32]);
    st.encoder = video.name.clone();
    st.audio = capture_audio.as_ref().is_some_and(|c| c.system.is_some());
    st.mic = capture_audio.as_ref().is_some_and(|c| c.mic.is_some());
    let t0 = Instant::now();
    st.started_ms = screenrec::now_ms();
    if let Some(c) = &capture_audio {
        c.set_start(t0);
    }
    screenrec::write(st);
    notify("screenrec-started".into());
    eprintln!("syn-screencast: запись {}: {}x{} {} к/с, {}, {}", a.path.display(), size.0, size.1, a.fps, video.name, if dmabuf { "DMA-BUF" } else { "shm" });

    let fps = a.fps as u64;
    let lag = enc::RATE / 5;
    let mut n: u64 = 0;
    let mut dropped: u64 = 0;
    let mut written_audio: i64 = 0;
    let mut last_save = Instant::now();
    let mut err: Option<anyhow::Error> = None;
    let end_n = a.duration.map(|d| (d * a.fps as f64).round() as u64);
    loop {
        if STOP.load(Ordering::SeqCst) || end_n.is_some_and(|e| n >= e) {
            break;
        }
        if let Some(e) = hub.st.lock().unwrap().error.clone() {
            err = Some(anyhow::anyhow!(e));
            break;
        }
        let due = (t0.elapsed().as_secs_f64() * a.fps as f64).floor() as u64 + 1;
        let due = end_n.map_or(due, |e| due.min(e));
        if due > n + fps / 2 + 1 {
            dropped += due - 1 - n;
            n = due - 1;
        }
        while n < due {
            let slot = {
                let mut g = hub.st.lock().unwrap();
                let i = g.latest.unwrap_or(0);
                g.holds[i] += 1;
                i
            };
            let m = &mems[slot];
            let hub2 = hub.clone();
            let src = enc::Src {
                fd: m.fd.as_raw_fd(),
                size: m.size,
                modifier: m.modifier,
                map: m.map,
                stride: m.stride,
                offset: m.offset,
                full,
                crop: *crop.lock().unwrap(),
                release: Box::new(move || {
                    let mut g = hub2.st.lock().unwrap();
                    g.holds[slot] = g.holds[slot].saturating_sub(1);
                    hub2.cv.notify_all();
                }),
            };
            let r = video.encode(src, n as i64, &mut |p| mux.write(p, true));
            if let Err(e) = r {
                err = Some(e);
                break;
            }
            n += 1;
        }
        if err.is_some() {
            break;
        }
        if let (Some(c), Some(ae)) = (&capture_audio, aenc.as_mut()) {
            let target = ((t0.elapsed().as_secs_f64() * enc::RATE as f64) as i64 - lag).min((n as f64 / a.fps as f64 * enc::RATE as f64) as i64);
            if target > written_audio {
                let frames = (target - written_audio) as usize;
                let pcm = c.mix(written_audio, frames);
                written_audio = target;
                if let Err(e) = ae.push(&pcm, &mut |p| mux.write(p, false)) {
                    err = Some(e);
                    break;
                }
            }
        }
        if last_save.elapsed() > Duration::from_millis(1000) {
            last_save = Instant::now();
            st.frames = n;
            st.dropped = dropped;
            st.captured = hub.st.lock().unwrap().captured;
            st.bytes = mux.bytes;
            st.seconds = n as f64 / a.fps as f64;
            screenrec::write(st);
        }
        let next = t0 + Duration::from_secs_f64(n as f64 / a.fps as f64);
        if let Some(d) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(d.min(Duration::from_millis(20)));
        }
    }
    hub.st.lock().unwrap().stop = true;
    hub.cv.notify_all();
    st.state = "saving".into();
    screenrec::write(st);
    if let Err(e) = video.finish(&mut |p| mux.write(p, true)) {
        err.get_or_insert(e);
    }
    if let (Some(c), Some(ae)) = (&capture_audio, aenc.as_mut()) {
        let end = (n as f64 / a.fps as f64 * enc::RATE as f64) as i64;
        let settle = Instant::now();
        while settle.elapsed() < Duration::from_millis(250) {
            std::thread::sleep(Duration::from_millis(20));
        }
        if end > written_audio {
            let pcm = c.mix(written_audio, (end - written_audio) as usize);
            written_audio = end;
            if let Err(e) = ae.push(&pcm, &mut |p| mux.write(p, false)) {
                err.get_or_insert(e);
            }
        }
        if let Err(e) = ae.finish(&mut |p| mux.write(p, false)) {
            err.get_or_insert(e);
        }
        st.audio_seconds = written_audio as f64 / enc::RATE as f64;
        eprintln!("syn-screencast: звук: {} пакетов PipeWire", c.chunks());
    }
    drop(capture_audio);
    if let Err(e) = mux.finish() {
        err.get_or_insert(e);
    }
    drop(mux);
    st.frames = n;
    st.dropped = dropped;
    st.captured = hub.st.lock().unwrap().captured;
    st.seconds = n as f64 / a.fps as f64;
    st.bytes = std::fs::metadata(&a.path).map(|m| m.len()).unwrap_or(0);
    drop(video);
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
