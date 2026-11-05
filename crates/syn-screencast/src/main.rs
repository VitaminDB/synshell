//! syn-screencast — демонстрация экрана для портала synshell (бэкенд
//! `org.freedesktop.impl.portal.ScreenCast` в оболочке, `syn_system::portal_screencast`).
//!
//! Кадры берутся у synwm по wlr-screencopy прямо в память буферов PipeWire: буферы — memfd,
//! которые выделяет PipeWire, из того же fd делается пул wl_shm, и композитор рисует кадр
//! сразу туда, без лишних копий. `copy_with_damage` ждёт изменений на экране, так что
//! неподвижный экран ничего не стоит. Поток — видеоисточник PipeWire, ведущий (driver):
//! кадр готов — буфер в очередь и `trigger_process`.
//!
//!   syn-screencast --list                         выводы (JSON)
//!   syn-screencast --stream ВЫВОД [--cursor 0|1] [--fps N]
//!                                                 поток; в stdout строка JSON с node id;
//!                                                 конец — закрытие stdin или SIGTERM
//!   syn-screencast --screenshot ФАЙЛ.png [--output ВЫВОД] [--cursor 0|1]

use std::io::{BufRead, Write};
use std::os::fd::{AsFd, BorrowedFd, RawFd};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use pipewire as pw;
use pw::spa;
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

type Mgr = zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
type Frame = zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;

// ─── Wayland ────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
struct Out {
    wl: Option<wl_output::WlOutput>,
    name: String,
    desc: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scale: i32,
}

#[derive(Default)]
struct FrameState {
    /// Формат буфера shm: (формат, ширина, высота, шаг строки).
    shm: Option<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    ready: bool,
    failed: bool,
    y_invert: bool,
}

#[derive(Default)]
struct Wl {
    outputs: Vec<Out>,
    shm: Option<wl_shm::WlShm>,
    mgr: Option<Mgr>,
    dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    frame: FrameState,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Wl {
    fn event(st: &mut Self, reg: &wl_registry::WlRegistry, ev: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = ev {
            match interface.as_str() {
                "wl_output" => {
                    let o: wl_output::WlOutput = reg.bind(name, version.min(4), qh, st.outputs.len());
                    st.outputs.push(Out { wl: Some(o), scale: 1, ..Default::default() });
                }
                "wl_shm" => st.shm = Some(reg.bind(name, 1, qh, ())),
                "zwlr_screencopy_manager_v1" => st.mgr = Some(reg.bind(name, version.min(3), qh, ())),
                "zwp_linux_dmabuf_v1" if version >= 3 => st.dmabuf = Some(reg.bind(name, 3, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, usize> for Wl {
    fn event(st: &mut Self, _: &wl_output::WlOutput, ev: wl_output::Event, i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        let Some(o) = st.outputs.get_mut(*i) else { return };
        match ev {
            wl_output::Event::Geometry { x, y, .. } => {
                o.x = x;
                o.y = y;
            }
            wl_output::Event::Mode { flags, width, height, .. } => {
                if matches!(flags, WEnum::Value(f) if f.contains(wl_output::Mode::Current)) {
                    o.w = width;
                    o.h = height;
                }
            }
            wl_output::Event::Scale { factor } => o.scale = factor,
            wl_output::Event::Name { name } => o.name = name,
            wl_output::Event::Description { description } => o.desc = description,
            _ => {}
        }
    }
}

impl Dispatch<Frame, ()> for Wl {
    fn event(st: &mut Self, _: &Frame, ev: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let f = &mut st.frame;
        match ev {
            zwlr_screencopy_frame_v1::Event::Buffer { format: WEnum::Value(format), width, height, stride } => f.shm = Some((format, width, height, stride)),
            zwlr_screencopy_frame_v1::Event::BufferDone => f.buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Flags { flags: WEnum::Value(fl) } => f.y_invert = fl.contains(zwlr_screencopy_frame_v1::Flags::YInvert),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => f.ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => f.failed = true,
            _ => {}
        }
    }
}

delegate_noop!(Wl: ignore wl_shm::WlShm);
delegate_noop!(Wl: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Wl: ignore wl_buffer::WlBuffer);
delegate_noop!(Wl: ignore Mgr);
delegate_noop!(Wl: ignore zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1);
delegate_noop!(Wl: ignore zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1);

struct Session {
    _conn: Connection,
    q: EventQueue<Wl>,
    qh: QueueHandle<Wl>,
    st: Wl,
}

impl Session {
    fn connect() -> Result<Session> {
        let conn = Connection::connect_to_env().context("нет WAYLAND_DISPLAY — нужен сеанс synwm")?;
        let mut q = conn.new_event_queue();
        let qh = q.handle();
        conn.display().get_registry(&qh, ());
        let mut st = Wl::default();
        q.roundtrip(&mut st)?;
        q.roundtrip(&mut st)?;
        if st.mgr.is_none() {
            bail!("композитор не умеет wlr-screencopy");
        }
        Ok(Session { _conn: conn, q, qh, st })
    }

    fn output(&self, name: &str) -> Result<Out> {
        if name.is_empty() {
            return self.st.outputs.first().cloned().context("нет выводов");
        }
        self.st.outputs.iter().find(|o| o.name == name).cloned().with_context(|| format!("нет вывода {name}"))
    }

    /// Начать захват: дождаться описания буфера.
    fn begin(&mut self, out: &Out, cursor: bool) -> Result<(Frame, (wl_shm::Format, u32, u32, u32))> {
        self.st.frame = FrameState::default();
        let mgr = self.st.mgr.as_ref().unwrap();
        let frame = mgr.capture_output(cursor as i32, out.wl.as_ref().unwrap(), &self.qh, ());
        while !self.st.frame.buffer_done && !self.st.frame.failed {
            self.q.blocking_dispatch(&mut self.st)?;
        }
        let fmt = self.st.frame.shm.filter(|_| !self.st.frame.failed).context("композитор не дал буфер shm")?;
        Ok((frame, fmt))
    }

    /// Скопировать кадр в буфер и дождаться готовности.
    fn finish(&mut self, frame: &Frame, buf: &wl_buffer::WlBuffer, damage: bool) -> Result<bool> {
        if damage {
            frame.copy_with_damage(buf);
        } else {
            frame.copy(buf);
        }
        while !self.st.frame.ready && !self.st.frame.failed {
            self.q.blocking_dispatch(&mut self.st)?;
        }
        frame.destroy();
        Ok(self.st.frame.ready)
    }

    fn pool(&self, fd: BorrowedFd<'_>, size: i32) -> wl_shm_pool::WlShmPool {
        self.st.shm.as_ref().unwrap().create_pool(fd, size, &self.qh, ())
    }
}

fn memfd(size: usize) -> Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::memfd_create(c"syn-screencast".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        bail!("memfd_create: {}", std::io::Error::last_os_error());
    }
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::ftruncate(fd.as_fd().as_raw_fd_compat(), size as libc::off_t) } != 0 {
        bail!("ftruncate: {}", std::io::Error::last_os_error());
    }
    Ok(fd)
}

trait RawCompat {
    fn as_raw_fd_compat(&self) -> RawFd;
}
impl RawCompat for BorrowedFd<'_> {
    fn as_raw_fd_compat(&self) -> RawFd {
        use std::os::fd::AsRawFd;
        self.as_raw_fd()
    }
}

// ─── Список и снимок ────────────────────────────────────────────────────

fn list() -> Result<()> {
    let s = Session::connect()?;
    let v: Vec<serde_json::Value> = s
        .st
        .outputs
        .iter()
        .map(|o| serde_json::json!({"name": o.name, "description": o.desc, "x": o.x, "y": o.y, "width": o.w, "height": o.h, "scale": o.scale}))
        .collect();
    println!("{}", serde_json::Value::Array(v));
    Ok(())
}

fn screenshot(path: &str, output: &str, cursor: bool) -> Result<()> {
    let mut s = Session::connect()?;
    let out = s.output(output)?;
    let (frame, (fmt, w, h, stride)) = s.begin(&out, cursor)?;
    let size = (stride * h) as usize;
    let fd = memfd(size)?;
    let pool = s.pool(fd.as_fd(), size as i32);
    let buf = pool.create_buffer(0, w as i32, h as i32, stride as i32, fmt, &s.qh, ());
    if !s.finish(&frame, &buf, false)? {
        bail!("композитор не отдал кадр");
    }
    let map = unsafe { libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, fd.as_fd().as_raw_fd_compat(), 0) };
    if map == libc::MAP_FAILED {
        bail!("mmap");
    }
    let src = unsafe { std::slice::from_raw_parts(map as *const u8, size) };
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let row = if s.st.frame.y_invert { h - 1 - y } else { y };
        for x in 0..w {
            let p = &src[(row * stride + x * 4) as usize..][..4];
            // xrgb/argb8888 в памяти — B, G, R, A; xbgr/abgr — R, G, B, A.
            let (r, g, b) = match fmt {
                wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => (p[0], p[1], p[2]),
                _ => (p[2], p[1], p[0]),
            };
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    unsafe { libc::munmap(map, size) };
    buf.destroy();
    pool.destroy();
    image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8)?;
    println!("{path}: {w}x{h}");
    Ok(())
}

// ─── Поток ──────────────────────────────────────────────────────────────

/// Задание захватчику: заполнить буфер PipeWire.
struct Job {
    buf: usize,
    fd: RawFd,
    offset: u32,
    stride: u32,
    maxsize: u32,
    /// DMA-BUF: модификатор раскладки; `None` — shm (memfd).
    dmabuf: Option<u64>,
}

/// Ответ захватчика в цикл PipeWire.
enum Done {
    Frame { buf: usize, ok: bool },
    Error(String),
}

fn spa_format(f: wl_shm::Format) -> Option<spa::param::video::VideoFormat> {
    use spa::param::video::VideoFormat as V;
    Some(match f {
        wl_shm::Format::Xrgb8888 => V::BGRx,
        wl_shm::Format::Argb8888 => V::BGRA,
        wl_shm::Format::Xbgr8888 => V::RGBx,
        wl_shm::Format::Abgr8888 => V::RGBA,
        _ => return None,
    })
}

/// Код DRM того же формата (для DMA-BUF).
fn drm_format(f: wl_shm::Format) -> u32 {
    let c = |s: &[u8; 4]| u32::from_le_bytes(*s);
    match f {
        wl_shm::Format::Argb8888 => c(b"AR24"),
        wl_shm::Format::Xbgr8888 => c(b"XB24"),
        wl_shm::Format::Abgr8888 => c(b"AB24"),
        _ => c(b"XR24"),
    }
}

fn pod(obj: spa::pod::Object) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &spa::pod::Value::Object(obj)).unwrap().0.into_inner()
}

const MOD_LINEAR: u64 = 0;

/// Предложение формата: с модификатором — DMA-BUF (LINEAR), без — shm.
fn format_pod(vfmt: spa::param::video::VideoFormat, w: u32, h: u32, max_fps: u32, dmabuf: bool) -> Vec<u8> {
    use spa::param::format::{FormatProperties as F, MediaSubtype, MediaType};
    let mut obj = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(F::MediaType, Id, MediaType::Video),
        spa::pod::property!(F::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(F::VideoFormat, Id, vfmt),
        spa::pod::property!(F::VideoSize, Rectangle, spa::utils::Rectangle { width: w, height: h }),
        spa::pod::property!(F::VideoFramerate, Fraction, spa::utils::Fraction { num: 0, denom: 1 }),
        spa::pod::property!(
            F::VideoMaxFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction { num: max_fps, denom: 1 },
            spa::utils::Fraction { num: 1, denom: 1 },
            spa::utils::Fraction { num: max_fps, denom: 1 }
        ),
    );
    if dmabuf {
        obj.properties.push(spa::pod::Property {
            key: F::VideoModifier.as_raw(),
            flags: spa::pod::PropertyFlags::MANDATORY,
            value: spa::pod::Value::Choice(spa::pod::ChoiceValue::Long(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Enum { default: MOD_LINEAR as i64, alternatives: vec![MOD_LINEAR as i64] },
            ))),
        });
    }
    pod(obj)
}

fn buffers_pod(size: u32, stride: u32, dmabuf: bool) -> Vec<u8> {
    let int = |k: u32, v: i32| spa::pod::Property { key: k, flags: spa::pod::PropertyFlags::empty(), value: spa::pod::Value::Int(v) };
    pod(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamBuffers.as_raw(),
        id: spa::param::ParamType::Buffers.as_raw(),
        properties: vec![
            spa::pod::Property {
                key: spa::sys::SPA_PARAM_BUFFERS_buffers,
                flags: spa::pod::PropertyFlags::empty(),
                value: spa::pod::Value::Choice(spa::pod::ChoiceValue::Int(spa::utils::Choice(
                    spa::utils::ChoiceFlags::empty(),
                    spa::utils::ChoiceEnum::Range { default: 4, min: 2, max: 8 },
                ))),
            },
            int(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(spa::sys::SPA_PARAM_BUFFERS_size, size as i32),
            int(spa::sys::SPA_PARAM_BUFFERS_stride, stride as i32),
            int(spa::sys::SPA_PARAM_BUFFERS_dataType, 1 << if dmabuf { spa::sys::SPA_DATA_DmaBuf } else { spa::sys::SPA_DATA_MemFd }),
        ],
    })
}

/// В согласованном формате есть модификатор — значит, потребитель берёт DMA-BUF.
fn has_modifier(param: &spa::pod::Pod) -> bool {
    let Ok((_, v)) = spa::pod::deserialize::PodDeserializer::deserialize_any_from(param.as_bytes()) else { return false };
    matches!(v, spa::pod::Value::Object(o) if o.properties.iter().any(|p| p.key == spa::param::format::FormatProperties::VideoModifier.as_raw()))
}

/// Устройство GBM на GPU композитора (`SYN_SCREENCAST_DRM`, иначе renderD128).
fn gbm_device() -> Option<gbm::Device<std::fs::File>> {
    let node = std::env::var("SYN_SCREENCAST_DRM").unwrap_or_else(|_| "/dev/dri/renderD128".into());
    let f = std::fs::OpenOptions::new().read(true).write(true).open(&node).ok()?;
    gbm::Device::new(f).ok()
}

/// Память буфера PipeWire, выделенная нами (флаг ALLOC_BUFFERS).
enum Mem {
    Shm(std::os::fd::OwnedFd),
    Dma(gbm::BufferObject<()>, std::os::fd::OwnedFd),
}

fn stream(output: &str, cursor: bool, max_fps: u32) -> Result<()> {
    // Описание буфера — пробным захватом (размер и формат вывода).
    let mut s = Session::connect()?;
    let out = s.output(output)?;
    let (probe, (fmt, w, h, stride)) = s.begin(&out, cursor)?;
    probe.destroy();
    let vfmt = spa_format(fmt).with_context(|| format!("формат {fmt:?} не поддержан"))?;
    let gbm = if s.st.dmabuf.is_some() { gbm_device() } else { None };

    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let stream = pw::stream::StreamRc::new(
        core,
        "syn-screencast",
        pw::properties::properties! {
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::NODE_NAME => "syn-screencast",
            *pw::keys::NODE_DESCRIPTION => format!("Экран {}", out.name),
        },
    )?;

    // Захватчик — свой поток с Wayland: очередь заданий отсюда, ответы — в цикл PipeWire.
    let (job_tx, job_rx) = mpsc::channel::<Option<Job>>();
    let (done_tx, done_rx) = pw::channel::channel::<Done>();
    let out2 = out.clone();
    let period = Duration::from_secs_f64(1.0 / max_fps.max(1) as f64);
    let drm_fmt = drm_format(fmt);
    std::thread::spawn(move || {
        let mut bufs: std::collections::HashMap<usize, (Option<wl_shm_pool::WlShmPool>, wl_buffer::WlBuffer)> = Default::default();
        let mut last = Instant::now() - period;
        let mut first = true;
        while let Ok(Some(job)) = job_rx.recv() {
            let r = (|| -> Result<bool> {
                let wait = period.saturating_sub(last.elapsed());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
                let (frame, (_, fw, fh, _)) = s.begin(&out2, cursor)?;
                if (fw, fh) != (w, h) {
                    frame.destroy();
                    bail!("размер вывода изменился ({fw}x{fh}) — начните показ заново");
                }
                if !bufs.contains_key(&job.buf) {
                    let fd = unsafe { BorrowedFd::borrow_raw(job.fd) };
                    let entry = match job.dmabuf {
                        Some(modifier) => {
                            let dm = s.st.dmabuf.as_ref().context("нет linux-dmabuf")?;
                            let params = dm.create_params(&s.qh, ());
                            params.add(fd, 0, job.offset, job.stride, (modifier >> 32) as u32, modifier as u32);
                            let b = params.create_immed(w as i32, h as i32, drm_fmt, zwp_linux_buffer_params_v1::Flags::empty(), &s.qh, ());
                            params.destroy();
                            (None, b)
                        }
                        None => {
                            if job.maxsize < stride * h {
                                bail!("буфер PipeWire меньше кадра");
                            }
                            let pool = s.pool(fd, (job.offset + job.maxsize) as i32);
                            let b = pool.create_buffer(job.offset as i32, w as i32, h as i32, stride as i32, fmt, &s.qh, ());
                            (Some(pool), b)
                        }
                    };
                    bufs.insert(job.buf, entry);
                }
                let buf = bufs[&job.buf].1.clone();
                // Первый кадр — сразу, дальше — по изменениям на экране.
                let ok = s.finish(&frame, &buf, !first)?;
                first = false;
                last = Instant::now();
                Ok(ok)
            })();
            let msg = match r {
                Ok(ok) => Done::Frame { buf: job.buf, ok },
                Err(e) => Done::Error(e.to_string()),
            };
            let fatal = matches!(msg, Done::Error(_));
            if done_tx.send(msg).is_err() || fatal {
                break;
            }
        }
    });

    struct Loop {
        busy: bool,
        announced: bool,
        /// Потребитель взял DMA-BUF (по согласованному формату).
        dmabuf: bool,
        mem: std::collections::HashMap<usize, (Mem, u32)>,
    }
    let state = std::rc::Rc::new(std::cell::RefCell::new(Loop { busy: false, announced: false, dmabuf: false, mem: Default::default() }));
    let job_tx = std::rc::Rc::new(job_tx);

    // Взять свободный буфер и отдать захватчику.
    let request = {
        let stream = stream.clone();
        let state = state.clone();
        let job_tx = job_tx.clone();
        move || {
            if state.borrow().busy {
                return;
            }
            let raw = unsafe { stream.dequeue_raw_buffer() };
            if raw.is_null() {
                return;
            }
            let st = state.borrow();
            let Some((mem, stride)) = st.mem.get(&(raw as usize)) else {
                drop(st);
                unsafe { stream.queue_raw_buffer(raw) };
                return;
            };
            use std::os::fd::AsRawFd;
            let job = match mem {
                Mem::Shm(fd) => Job { buf: raw as usize, fd: fd.as_raw_fd(), offset: 0, stride: *stride, maxsize: stride * h, dmabuf: None },
                Mem::Dma(bo, fd) => Job { buf: raw as usize, fd: fd.as_raw_fd(), offset: bo.offset(0), stride: *stride, maxsize: stride * h, dmabuf: Some(u64::from(bo.modifier())) },
            };
            drop(st);
            state.borrow_mut().busy = true;
            let _ = job_tx.send(Some(job));
        }
    };
    let request = std::rc::Rc::new(request);

    let ml = mainloop.clone();
    let _done = done_rx.attach(mainloop.loop_(), {
        let stream = stream.clone();
        let state = state.clone();
        let request = request.clone();
        move |msg| match msg {
            Done::Frame { buf, ok } => {
                let raw = buf as *mut pw::sys::pw_buffer;
                let stride = state.borrow().mem.get(&buf).map(|m| m.1).unwrap_or(stride);
                unsafe {
                    let d = &mut *(*(*raw).buffer).datas;
                    let c = &mut *d.chunk;
                    c.offset = 0;
                    c.stride = stride as i32;
                    c.size = if ok { stride * h } else { 0 };
                    c.flags = if ok { 0 } else { spa::sys::SPA_CHUNK_FLAG_CORRUPTED as i32 };
                    stream.queue_raw_buffer(raw);
                }
                let _ = stream.trigger_process();
                state.borrow_mut().busy = false;
                request();
            }
            Done::Error(e) => {
                eprintln!("syn-screencast: {e}");
                ml.quit();
            }
        }
    });

    let ml2 = mainloop.clone();
    let gbm = std::rc::Rc::new(gbm);
    let _listener = stream
        .add_local_listener::<()>()
        .state_changed({
            let state = state.clone();
            let request = request.clone();
            move |s, _, _old, new| match new {
                pw::stream::StreamState::Paused => {
                    if !state.borrow().announced {
                        state.borrow_mut().announced = true;
                        let line = serde_json::json!({"node": s.node_id(), "width": w, "height": h});
                        println!("{line}");
                        let _ = std::io::stdout().flush();
                    }
                }
                pw::stream::StreamState::Streaming => request(),
                pw::stream::StreamState::Error(e) => {
                    eprintln!("syn-screencast: поток: {e}");
                    ml2.quit();
                }
                _ => {}
            }
        })
        .param_changed({
            let state = state.clone();
            let gbm = gbm.clone();
            move |s, _, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let dma = has_modifier(param) && gbm.is_some();
                state.borrow_mut().dmabuf = dma;
                // Шаг строки DMA-BUF узнаётся только у выделенного буфера — пробный.
                let dstride = if dma {
                    gbm.as_ref().as_ref().and_then(|g| g.create_buffer_object::<()>(w, h, gbm::Format::Xrgb8888, gbm::BufferObjectFlags::RENDERING | gbm::BufferObjectFlags::LINEAR).ok()).map(|b| b.stride()).unwrap_or(stride)
                } else {
                    stride
                };
                let b = buffers_pod(dstride * h, dstride, dma);
                let mut params = [spa::pod::Pod::from_bytes(&b).unwrap()];
                let _ = s.update_params(&mut params);
                eprintln!("syn-screencast: {}", if dma { "DMA-BUF (LINEAR)" } else { "shm (memfd)" });
            }
        })
        .add_buffer({
            let state = state.clone();
            let gbm = gbm.clone();
            move |_, _, raw| {
                let dma = state.borrow().dmabuf;
                let r = (|| -> Result<(Mem, u32)> {
                    if dma {
                        let g = gbm.as_ref().as_ref().context("нет GBM")?;
                        let bo = g.create_buffer_object::<()>(w, h, gbm::Format::Xrgb8888, gbm::BufferObjectFlags::RENDERING | gbm::BufferObjectFlags::LINEAR)?;
                        let fd = bo.fd_for_plane(0)?;
                        let st = bo.stride();
                        Ok((Mem::Dma(bo, fd), st))
                    } else {
                        Ok((Mem::Shm(memfd((stride * h) as usize)?), stride))
                    }
                })();
                let Ok((mem, st)) = r.map_err(|e| eprintln!("syn-screencast: буфер: {e}")) else { return };
                use std::os::fd::AsRawFd;
                unsafe {
                    let d = &mut *(*(*raw).buffer).datas;
                    match &mem {
                        Mem::Shm(fd) => {
                            d.type_ = spa::sys::SPA_DATA_MemFd;
                            d.fd = fd.as_raw_fd() as i64;
                            d.mapoffset = 0;
                        }
                        Mem::Dma(bo, fd) => {
                            d.type_ = spa::sys::SPA_DATA_DmaBuf;
                            d.fd = fd.as_raw_fd() as i64;
                            d.mapoffset = bo.offset(0);
                        }
                    }
                    d.flags = spa::sys::SPA_DATA_FLAG_READWRITE;
                    d.maxsize = st * h;
                    d.data = std::ptr::null_mut();
                    let c = &mut *d.chunk;
                    c.offset = 0;
                    c.stride = st as i32;
                    c.size = st * h;
                }
                state.borrow_mut().mem.insert(raw as usize, (mem, st));
            }
        })
        .remove_buffer({
            let state = state.clone();
            move |_, _, raw| {
                state.borrow_mut().mem.remove(&(raw as usize));
            }
        })
        .process({
            let request = request.clone();
            move |_, _| request()
        })
        .register()?;

    // Сначала DMA-BUF (если есть GBM), запасной — shm.
    let mut fmts: Vec<Vec<u8>> = Vec::new();
    if gbm.is_some() {
        fmts.push(format_pod(vfmt, w, h, max_fps, true));
    }
    fmts.push(format_pod(vfmt, w, h, max_fps, false));
    let mut params: Vec<&spa::pod::Pod> = fmts.iter().map(|b| spa::pod::Pod::from_bytes(b).unwrap()).collect();
    stream.connect(spa::utils::Direction::Output, None, pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::ALLOC_BUFFERS | pw::stream::StreamFlags::AUTOCONNECT, &mut params)?;

    // Конец — закрытие stdin (портал закрыл сеанс).
    let ml3 = mainloop.clone();
    let _stdin = mainloop.loop_().add_io(std::io::stdin(), spa::support::system::IoFlags::IN | spa::support::system::IoFlags::HUP, move |inp| {
        let mut line = String::new();
        if inp.lock().read_line(&mut line).map(|n| n == 0).unwrap_or(true) {
            ml3.quit();
        }
    });
    mainloop.run();
    let _ = job_tx.send(None);
    Ok(())
}

/// Самопроверка: подключиться к узлу потребителем, считать кадры `secs` секунд, первый
/// кадр сохранить в PNG.
fn probe(node: u32, secs: f64, png: Option<String>) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let stream = pw::stream::StreamRc::new(core, "syn-screencast-probe", pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Video", *pw::keys::MEDIA_CATEGORY => "Capture", *pw::keys::MEDIA_ROLE => "Screen",
    })?;
    #[derive(Default)]
    struct P {
        info: spa::param::video::VideoInfoRaw,
        frames: u64,
        empty: u64,
        saved: bool,
        first: Option<Instant>,
    }
    let data = std::rc::Rc::new(std::cell::RefCell::new(P::default()));
    let d2 = data.clone();
    let d3 = data.clone();
    let _l = stream
        .add_local_listener::<()>()
        .param_changed(move |_, _, id, param| {
            if let (Some(p), true) = (param, id == spa::param::ParamType::Format.as_raw()) {
                let _ = d2.borrow_mut().info.parse(p);
            }
        })
        .process(move |st, _| {
            let Some(mut b) = st.dequeue_buffer() else { return };
            let mut p = d3.borrow_mut();
            let (w, h) = (p.info.size().width, p.info.size().height);
            let datas = b.datas_mut();
            let size = datas[0].chunk().size();
            if size == 0 {
                p.empty += 1;
                return;
            }
            p.frames += 1;
            p.first.get_or_insert_with(Instant::now);
            if !p.saved {
                if let (Some(path), Some(bytes)) = (png.as_ref(), datas[0].data()) {
                    let stride = (w * 4) as usize;
                    let mut rgba = Vec::with_capacity(stride * h as usize);
                    let bgr = matches!(p.info.format(), spa::param::video::VideoFormat::BGRx | spa::param::video::VideoFormat::BGRA);
                    for px in bytes[..stride * h as usize].chunks_exact(4) {
                        if bgr {
                            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                        } else {
                            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                        }
                    }
                    let _ = image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8);
                    p.saved = true;
                }
            }
        })
        .register()?;
    use spa::param::format::{FormatProperties as F, MediaSubtype, MediaType};
    use spa::param::video::VideoFormat as V;
    let format = pod(spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(F::MediaType, Id, MediaType::Video),
        spa::pod::property!(F::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(F::VideoFormat, Choice, Enum, Id, V::BGRx, V::BGRx, V::BGRA, V::RGBx, V::RGBA),
        spa::pod::property!(F::VideoSize, Choice, Range, Rectangle, spa::utils::Rectangle { width: 1920, height: 1080 }, spa::utils::Rectangle { width: 1, height: 1 }, spa::utils::Rectangle { width: 8192, height: 8192 }),
        spa::pod::property!(F::VideoFramerate, Choice, Range, Fraction, spa::utils::Fraction { num: 0, denom: 1 }, spa::utils::Fraction { num: 0, denom: 1 }, spa::utils::Fraction { num: 1000, denom: 1 }),
    ));
    let mut params = [spa::pod::Pod::from_bytes(&format).unwrap()];
    stream.connect(spa::utils::Direction::Input, Some(node), pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS, &mut params)?;
    let ml = mainloop.clone();
    let t = mainloop.loop_().add_timer(move |_| ml.quit());
    t.update_timer(Some(Duration::from_secs_f64(secs)), None).into_result()?;
    mainloop.run();
    let p = data.borrow();
    let rate = p.first.map(|f| p.frames.saturating_sub(1) as f64 / f.elapsed().as_secs_f64().max(0.001)).unwrap_or(0.0);
    println!("{}", serde_json::json!({"frames": p.frames, "empty": p.empty, "fps": (rate * 10.0).round() / 10.0, "width": p.info.size().width, "height": p.info.size().height, "format": format!("{:?}", p.info.format())}));
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let cursor = opt("--cursor").map(|v| v != "0").unwrap_or(true);
    let r = match args.first().map(String::as_str) {
        Some("--list") => list(),
        Some("--stream") => stream(&args.get(1).cloned().unwrap_or_default(), cursor, opt("--fps").and_then(|s| s.parse().ok()).unwrap_or(60)),
        Some("--probe") => probe(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0), opt("--secs").and_then(|s| s.parse().ok()).unwrap_or(3.0), opt("--png")),
        Some("--screenshot") => screenshot(args.get(1).map(String::as_str).unwrap_or("screenshot.png"), &opt("--output").unwrap_or_default(), cursor),
        _ => {
            eprintln!("syn-screencast --list | --stream ВЫВОД [--cursor 0|1] [--fps N] | --screenshot ФАЙЛ.png [--output ВЫВОД] [--cursor 0|1]");
            std::process::exit(2);
        }
    };
    if let Err(e) = r {
        eprintln!("syn-screencast: {e:#}");
        std::process::exit(1);
    }
}
