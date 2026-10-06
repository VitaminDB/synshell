//! synlink-view — экран другого устройства synshell: трансляция и
//! управление (демон synlink, `synshell_common::link`).
//!
//! Кадры приходят только изменившимися прямоугольниками (zstd) и, с видео,
//! пакетами аппаратного кодера той стороны (`video.rs`): здесь сначала
//! накладываются изменившиеся места декодированного видеокадра, потом куски
//! без потерь, и буфер уходит в `LiveView`. Поворот экрана устройства —
//! окно меняет ширину и высоту местами. Ввод: на телефон —
//! мышь как палец (правая кнопка — «назад», средняя — «домой»), на
//! компьютер — мышь и касания как мышь; клавиши и текст — как есть.
//!
//! «Звук» — звук устройства играет здесь, микрофон отсюда уходит туда
//! виртуальным источником (`Request::Audio` с `hold`: держим соединение с
//! демоном, пока звук включён). «Во весь экран» (и F11) — окно на весь экран
//! без панели; выход — кнопка в углу (видна несколько секунд после касания или
//! движения мыши), Esc или системное «назад». Режим запоминается.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use syngui::async_runtime::run_on_main_thread;
use syngui::input::{Key, MouseButton};
use syngui::prelude::*;
use syngui::widgets::{EventHook, LiveFrame, LiveInput, LiveView, Reactive};
use syngui::window::WindowState;
use syngui::GestureDetector;
use std::result::Result;
use synshell_common::ipc::{self, InputEvent};
use synshell_common::link::{self, DeviceKind, PeerInfo, Request, Response, ScreenAck, ScreenHeader};

const STYLES: &str = include_str!("../styles/view.mss");

mod video;

mod gl {
    pub const BACK: &str = "\u{E5C4}";
    pub const HOME: &str = "\u{E88A}";
    pub const RECENTS: &str = "\u{E53B}";
    pub const SHADE: &str = "\u{E5CF}";
    pub const SHOT: &str = "\u{E3B0}";
    pub const FOLDER: &str = "\u{E2C7}";
    pub const KEYBOARD: &str = "\u{E312}";
    pub const LOCK: &str = "\u{E897}";
    pub const USB: &str = "\u{E1E0}";
    pub const WIFI: &str = "\u{E63E}";
    pub const PHONE: &str = "\u{E32C}";
    pub const COMPUTER: &str = "\u{E30A}";
    pub const LAPTOP: &str = "\u{E31E}";
    pub const SYNC: &str = "\u{E627}";
    pub const VOLUME_UP: &str = "\u{E050}";
    pub const VOLUME_OFF: &str = "\u{E04F}";
    pub const FULLSCREEN: &str = "\u{E5D0}";
    pub const FULLSCREEN_EXIT: &str = "\u{E5D1}";
}

/// Звук трансляции.
#[derive(Clone, Copy, PartialEq)]
enum Sound {
    Off,
    Starting,
    On,
}

#[derive(Clone, PartialEq)]
struct Stats {
    fps: f32,
    kbps: f32,
    size: (u32, u32),
    state: String,
    /// Как идут кадры: `HEVC`, `H264` или «без потерь».
    codec: String,
}

#[derive(Clone, Copy)]
struct Ctx {
    device: &'static str,
    peer: &'static PeerInfo,
    frame_rev: RwSignal<u64>,
    stats: RwSignal<Stats>,
    /// Состояние окна (во весь экран — без панели).
    win: RwSignal<WindowState>,
    sound: RwSignal<Sound>,
    /// Сообщение в строке состояния на несколько секунд (ошибка звука).
    note: RwSignal<String>,
    /// Кнопка выхода из полноэкранного режима видна.
    hud: RwSignal<bool>,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,wgpu_core=warn,wgpu_hal=warn,naga=warn")).init();
    let Some(device) = std::env::args().nth(1).filter(|a| !a.starts_with('-')) else {
        eprintln!("synlink-view УСТРОЙСТВО — экран связанного устройства (имя, id или phone)");
        std::process::exit(2);
    };
    let peer = match find_peer(&device) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("synlink-view: {e}");
            std::process::exit(1);
        }
    };
    // Окно под пропорции экрана устройства.
    let (w, h) = output_size(&peer.id).unwrap_or(if peer.kind.is_touch() { (1080, 2400) } else { (1920, 1080) });
    let (win_w, win_h) = window_size(w, h);

    let cfg = synshell_common::Config::load().0;
    let a = &cfg.appearance;
    let mut mss = a.mss_variables();
    mss.push_str(&a.theme_mss_variables());
    mss.push_str(STYLES);

    let device: &'static str = Box::leak(peer.id.clone().into_boxed_str());
    let peer: &'static PeerInfo = Box::leak(Box::new(peer));
    let frame = LiveFrame::new();
    let title = format!("{} — экран", peer.name);
    let win = use_signal(WindowState::default());
    App::new()
        .title(&title)
        .app_id("synlink-view")
        .size(win_w, win_h)
        .min_size(240, 240)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .with_window_state(win)
        .run(move |_| {
            let ctx = Ctx {
                device,
                peer,
                frame_rev: use_signal(0u64),
                stats: use_signal(Stats { fps: 0.0, kbps: 0.0, size: (0, 0), state: "Соединение…".into(), codec: String::new() }),
                win,
                sound: use_signal(Sound::Off),
                note: use_signal(String::new()),
                hud: use_signal(false),
            };
            watch_fullscreen(ctx);
            if remembered_fullscreen() {
                // Окна ещё нет — после его создания.
                run_on_main_thread(|| syngui::signal::set_fullscreen(true));
            }
            start_stream(ctx, frame.clone());
            let input = start_input(device);
            Box::new(root(ctx, frame.clone(), input))
        });
}

/// Размер окна под кадр устройства `w`×`h`: высота 860 у вертикального
/// экрана, ширина 1280 у горизонтального, плюс строка заголовка.
fn window_size(w: u32, h: u32) -> (u32, u32) {
    if h > w {
        let hh = 860u32;
        ((hh as f32 * w as f32 / h.max(1) as f32) as u32 + 2, hh + 56)
    } else {
        let ww = 1280u32;
        (ww, (ww as f32 * h as f32 / w.max(1) as f32) as u32 + 56)
    }
}

fn find_peer(device: &str) -> Result<PeerInfo, String> {
    match link::request(&Request::Status) {
        Ok(Response::Status { status }) => {
            let d = device.to_lowercase();
            status
                .peers
                .into_iter()
                .filter(|p| p.paired)
                .find(|p| {
                    p.id == device
                        || p.id.starts_with(device)
                        || p.name.to_lowercase() == d
                        || p.ssh_host.as_deref() == Some(device)
                        || (d == "phone" && p.kind.is_touch())
                        || (d == "desktop" && !p.kind.is_touch())
                })
                .ok_or_else(|| format!("нет спаренного устройства «{device}»"))
        }
        Ok(Response::Error { message }) => Err(message),
        Ok(_) => Err("неожиданный ответ synlink".into()),
        Err(e) => Err(format!("synlink не запущен: {e}")),
    }
}

/// Размер кадра первого вывода устройства (пиксели, с поворотом).
fn output_size(id: &str) -> Option<(u32, u32)> {
    let Ok(Response::Wm { wm: ipc::Response::Outputs { outputs } }) =
        link::request(&Request::Wm { device: id.into(), wm: ipc::Request::Outputs })
    else {
        return None;
    };
    let o = outputs.first()?;
    let m = o.current_mode.and_then(|i| o.modes.get(i))?;
    let rot = o.transform.contains("90") || o.transform.contains("270");
    Some(if rot { (m.height as u32, m.width as u32) } else { (m.width as u32, m.height as u32) })
}

// ─── кадры ───────────────────────────────────────────────────────────────────

fn start_stream(ctx: Ctx, frame: Arc<LiveFrame>) {
    let (device, rev, stats) = (ctx.device.to_string(), ctx.frame_rev, ctx.stats);
    let cursor = !ctx.peer.kind.is_touch();
    std::thread::Builder::new()
        .name("view-stream".into())
        .spawn(move || loop {
            let err = stream_once(&device, cursor, &frame, rev, stats);
            let msg = match err {
                Ok(()) => "Трансляция остановлена — переподключение…".to_string(),
                Err(e) => format!("{e} — переподключение…"),
            };
            run_on_main_thread(move || {
                let mut s = stats.get_untracked();
                s.state = msg;
                s.fps = 0.0;
                stats.set(s);
            });
            std::thread::sleep(Duration::from_secs(2));
        })
        .ok();
}

fn stream_once(device: &str, cursor: bool, frame: &LiveFrame, rev: RwSignal<u64>, stats: RwSignal<Stats>) -> Result<(), String> {
    let mut c = link::Client::connect().map_err(|e| format!("synlink: {e}"))?;
    let video_ok = video::available();
    match c.request(&Request::Screen { device: device.into(), output: None, cursor, video: video_ok }).map_err(|e| e.to_string())? {
        Response::Ok => {}
        Response::Error { message } => return Err(message),
        other => return Err(format!("неожиданный ответ: {other:?}")),
    }
    let (mut rd, mut wr) = c.into_parts();
    let mut buf: Vec<u8> = Vec::new();
    let (mut fw, mut fh) = (0u32, 0u32);
    let mut count = 0u32;
    let mut bytes = 0usize;
    let mut window = Instant::now();
    let mut dec: Option<video::Decoder> = None;
    // Ждём ключевой кадр (декодер новый или ошибся) — просим его в подтверждении.
    let mut want_key = false;
    let mut codec = String::new();
    // Показ отстаёт от приёма — перестраивать не чаще, чем главный поток успевает.
    let pending = Arc::new(AtomicU64::new(0));
    while let Some(pkt) = link::read_frame(&mut rd).map_err(|e| e.to_string())? {
        bytes += pkt.len();
        if pkt.len() < 4 {
            continue;
        }
        let hlen = u32::from_be_bytes([pkt[0], pkt[1], pkt[2], pkt[3]]) as usize;
        let Ok(h) = serde_json::from_slice::<ScreenHeader>(&pkt[4..4 + hlen.min(pkt.len() - 4)]) else { continue };
        if (h.width, h.height) != (fw, fh) {
            // Поворот экрана устройства (ширина и высота поменялись) — и окно.
            let rotated = fw > 0 && (fw > fh) != (h.width > h.height);
            fw = h.width;
            fh = h.height;
            buf = vec![0u8; fw as usize * fh as usize * 4];
            if rotated {
                let (ww, wh) = window_size(fw, fh);
                syngui::window::request_size(ww, wh);
            }
        }
        let mut off = 4 + hlen;
        let lossless: Vec<([u32; 4], std::ops::Range<usize>)> = h
            .rects
            .iter()
            .map(|&[x, y, w, hh, len]| {
                let r = off..(off + len as usize).min(pkt.len());
                off += len as usize;
                ([x, y, w, hh], r)
            })
            .collect();
        let mut ask_key = false;
        if let Some(v) = &h.video {
            codec = v.codec.clone();
            let data = &pkt[off.min(pkt.len())..(off + v.len as usize).min(pkt.len())];
            if dec.as_ref().is_none_or(|d| d.codec != v.codec) {
                match video::Decoder::new(&v.codec) {
                    Ok(d) => {
                        dec = Some(d);
                        want_key = true;
                    }
                    Err(e) => log::warn!("{e}"),
                }
            }
            if let Some(d) = dec.as_mut() {
                if want_key && !v.key {
                    ask_key = true;
                } else {
                    want_key = false;
                    let t0 = Instant::now();
                    let res = d.decode(data, &v.rects, &mut buf, fw, fh);
                    if std::env::var_os("SYNLINK_VIEW_TIMING").is_some() {
                        eprintln!("декод+цвет {:.1} мс, {} байт", t0.elapsed().as_secs_f64() * 1e3, data.len());
                    }
                    match res {
                        Ok(_) => {}
                        Err(e) => {
                            log::warn!("видео: {e}");
                            dec = None;
                            ask_key = true;
                        }
                    }
                }
            }
        }
        for ([x, y, w, hh], r) in lossless {
            let Ok(raw) = zstd::bulk::decompress(&pkt[r], (w * hh * 4) as usize) else { continue };
            let row = w as usize * 4;
            for i in 0..hh as usize {
                let dst = ((y as usize + i) * fw as usize + x as usize) * 4;
                if dst + row <= buf.len() && (i + 1) * row <= raw.len() {
                    buf[dst..dst + row].copy_from_slice(&raw[i * row..(i + 1) * row]);
                }
            }
        }
        let t1 = Instant::now();
        frame.set(fw, fh, Arc::from(buf.as_slice()));
        if std::env::var_os("SYNLINK_VIEW_TIMING").is_some() {
            eprintln!("кадр в окно {:.1} мс", t1.elapsed().as_secs_f64() * 1e3);
        }
        let ack = serde_json::to_vec(&ScreenAck { seq: h.seq, key: ask_key }).unwrap_or_default();
        link::write_frame(&mut wr, &ack).map_err(|e| e.to_string())?;
        count += 1;
        if pending.fetch_add(1, Ordering::AcqRel) == 0 {
            let p = pending.clone();
            run_on_main_thread(move || {
                p.store(0, Ordering::Release);
                rev.set(rev.get_untracked() + 1);
            });
        }
        let el = window.elapsed();
        if el >= Duration::from_millis(1000) || count == 1 {
            let fps = count as f32 / el.as_secs_f32().max(0.001);
            let kbps = bytes as f32 / 1024.0 / el.as_secs_f32().max(0.001);
            let size = (fw, fh);
            let label = if codec.is_empty() { "без потерь".to_string() } else { codec.to_uppercase() };
            run_on_main_thread(move || stats.set(Stats { fps, kbps, size, state: String::new(), codec: label }));
            if el >= Duration::from_millis(1000) {
                count = 0;
                bytes = 0;
                window = Instant::now();
                codec.clear();
            }
        }
    }
    Ok(())
}

/// Скопировать прямоугольник `r` из кадра `src` (ширина `sw`) в `dst` (ширина `dw`).
pub(crate) fn blit(dst: &mut [u8], dw: u32, src: &[u8], sw: u32, [x, y, w, h]: [u32; 4]) {
    let row = w as usize * 4;
    for i in 0..h as usize {
        let d = ((y as usize + i) * dw as usize + x as usize) * 4;
        let s = ((y as usize + i) * sw as usize + x as usize) * 4;
        if d + row <= dst.len() && s + row <= src.len() {
            dst[d..d + row].copy_from_slice(&src[s..s + row]);
            // Альфа в RGBA от swscale — 255.
        }
    }
}

// ─── ввод ────────────────────────────────────────────────────────────────────

/// Поток ввода: события копятся и уходят пачками (движения мыши
/// схлопываются — важно последнее положение).
fn start_input(device: &'static str) -> mpsc::Sender<InputEvent> {
    let (tx, rx) = mpsc::channel::<InputEvent>();
    std::thread::Builder::new()
        .name("view-input".into())
        .spawn(move || {
            let mut client: Option<link::Client> = None;
            while let Ok(first) = rx.recv() {
                let mut batch = vec![first];
                std::thread::sleep(Duration::from_millis(4));
                while let Ok(e) = rx.try_recv() {
                    // Подряд идущие движения — только последнее.
                    if let (Some(InputEvent::Motion { .. }), InputEvent::Motion { .. }) = (batch.last(), &e) {
                        batch.pop();
                    }
                    batch.push(e);
                }
                let req = Request::Input { device: device.into(), output: None, events: batch };
                let mut res = client.as_mut().map(|c| c.request(&req));
                if !matches!(res, Some(Ok(_))) {
                    client = link::Client::connect().ok();
                    res = client.as_mut().map(|c| c.request(&req));
                }
                if std::env::var_os("SYNLINK_VIEW_DEBUG").is_some() {
                    eprintln!("ввод → {:?}", res.map(|r| r.map_err(|e| e.to_string())));
                }
            }
        })
        .ok();
    tx
}

/// Клавиша syngui → код evdev (`linux/input-event-codes.h`).
fn evdev(k: Key) -> Option<u32> {
    use Key::*;
    Some(match k {
        A => 30, B => 48, C => 46, D => 32, E => 18, F => 33, G => 34, H => 35, I => 23, J => 36, K => 37, L => 38, M => 50,
        N => 49, O => 24, P => 25, Q => 16, R => 19, S => 31, T => 20, U => 22, V => 47, W => 17, X => 45, Y => 21, Z => 44,
        Num1 => 2, Num2 => 3, Num3 => 4, Num4 => 5, Num5 => 6, Num6 => 7, Num7 => 8, Num8 => 9, Num9 => 10, Num0 => 11,
        F1 => 59, F2 => 60, F3 => 61, F4 => 62, F5 => 63, F6 => 64, F7 => 65, F8 => 66, F9 => 67, F10 => 68, F11 => 87, F12 => 88,
        Escape => 1, Enter => 28, Tab => 15, Backspace => 14, Delete => 111, Insert => 110, Home => 102, End => 107,
        PageUp => 104, PageDown => 109, Left => 105, Right => 106, Up => 103, Down => 108,
        Shift => 42, Ctrl => 29, Alt => 56, Meta => 125, Space => 57, ContextMenu => 127,
        MediaPlayPause => 164, MediaStop => 166, MediaNext => 163, MediaPrevious => 165,
        _ => return None,
    })
}

struct InputState {
    touch: bool,
    pressed: bool,
    /// Удерживаемые модификаторы (Ctrl, Alt, Meta): буквы с ними — клавишами.
    mods: u8,
    /// Esc вывел из полноэкранного режима — его отпускание тоже не отправлять.
    esc: bool,
}

fn on_input(st: &mut InputState, tx: &mpsc::Sender<InputEvent>, e: LiveInput) {
    if std::env::var_os("SYNLINK_VIEW_DEBUG").is_some() {
        eprintln!("ввод: {e:?}");
    }
    let cl = |v: f32| (v as f64).clamp(0.0, 1.0);
    let send = |ev: InputEvent| {
        let _ = tx.send(ev);
    };
    match e {
        // Телефон: мышь как палец.
        LiveInput::Down { button: MouseButton::Left, x, y } if st.touch => {
            st.pressed = true;
            send(InputEvent::TouchDown { id: 0, x: cl(x), y: cl(y) });
            send(InputEvent::TouchFrame);
        }
        LiveInput::Move { x, y } if st.touch => {
            if st.pressed {
                send(InputEvent::TouchMotion { id: 0, x: cl(x), y: cl(y) });
                send(InputEvent::TouchFrame);
            }
        }
        LiveInput::Up { button: MouseButton::Left, .. } if st.touch => {
            if st.pressed {
                st.pressed = false;
                send(InputEvent::TouchUp { id: 0 });
                send(InputEvent::TouchFrame);
            }
        }
        LiveInput::Down { button: MouseButton::Right, .. } if st.touch => action("back"),
        LiveInput::Down { button: MouseButton::Middle, .. } if st.touch => action("shell home"),
        LiveInput::Up { .. } if st.touch => {}
        LiveInput::Down { .. } if st.touch => {}
        // Компьютер: мышь.
        LiveInput::Move { x, y } => send(InputEvent::Motion { x: cl(x), y: cl(y) }),
        LiveInput::Down { button, x, y } | LiveInput::Up { button, x, y } => {
            let pressed = matches!(e, LiveInput::Down { .. });
            let code = match button {
                MouseButton::Left => 0x110,
                MouseButton::Right => 0x111,
                MouseButton::Middle => 0x112,
                _ => return,
            };
            send(InputEvent::Motion { x: cl(x), y: cl(y) });
            send(InputEvent::Button { button: code, pressed });
        }
        LiveInput::Wheel { dx, dy, x, y } => {
            send(InputEvent::Motion { x: cl(x), y: cl(y) });
            send(InputEvent::Axis { dx: dx as f64 * 15.0, dy: dy as f64 * 15.0, discrete: true });
        }
        // Касания окна: на телефон — касаниями, на компьютер — мышью.
        LiveInput::TouchStart { id, x, y } => {
            if st.touch {
                send(InputEvent::TouchDown { id: id as u32, x: cl(x), y: cl(y) });
                send(InputEvent::TouchFrame);
            } else {
                send(InputEvent::Motion { x: cl(x), y: cl(y) });
                send(InputEvent::Button { button: 0x110, pressed: true });
            }
        }
        LiveInput::TouchMove { id, x, y } => {
            if st.touch {
                send(InputEvent::TouchMotion { id: id as u32, x: cl(x), y: cl(y) });
                send(InputEvent::TouchFrame);
            } else {
                send(InputEvent::Motion { x: cl(x), y: cl(y) });
            }
        }
        LiveInput::TouchEnd { id, .. } => {
            if st.touch {
                send(InputEvent::TouchUp { id: id as u32 });
                send(InputEvent::TouchFrame);
            } else {
                send(InputEvent::Button { button: 0x110, pressed: false });
            }
        }
        LiveInput::Key { key, pressed } => {
            let bit = match key {
                Key::Ctrl => 1,
                Key::Alt => 2,
                Key::Meta => 4,
                _ => 0,
            };
            if bit != 0 {
                if pressed {
                    st.mods |= bit;
                } else {
                    st.mods &= !bit;
                }
            }
            // Печатные без Ctrl/Alt/Meta придут символом (раскладка той стороны).
            let printable = matches!(key, Key::Space) || matches!(key, Key::A | Key::B | Key::C | Key::D | Key::E | Key::F | Key::G | Key::H | Key::I | Key::J | Key::K | Key::L | Key::M | Key::N | Key::O | Key::P | Key::Q | Key::R | Key::S | Key::T | Key::U | Key::V | Key::W | Key::X | Key::Y | Key::Z | Key::Num0 | Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9);
            if printable && st.mods == 0 {
                return;
            }
            if let Some(code) = evdev(key) {
                send(InputEvent::Key { code, pressed });
            }
        }
        LiveInput::Char(c) => {
            if st.mods == 0 && !c.is_control() {
                send(InputEvent::Text { text: c.to_string() });
            }
        }
    }
}

fn action(a: &str) {
    let a = a.to_string();
    std::thread::spawn(move || {
        let Ok(action) = a.parse() else { return };
        let device = DEVICE.get().cloned().unwrap_or_default();
        let _ = link::request(&Request::Wm { device, wm: ipc::Request::Action { action } });
    });
}

static DEVICE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

// ─── интерфейс ───────────────────────────────────────────────────────────────

fn icon(glyph: &str) -> syngui::widget::StyledWidget<syngui::widgets::Icon> {
    syngui::widgets::Icon::new(glyph).class("icon")
}

fn tool(glyph: &'static str, tip: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    let _ = tip;
    GestureDetector::new().on_click(f).child(DecoratedBox::new().child(icon(glyph).class("tool-icon")).class("tool"))
}

fn kind_glyph(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Phone | DeviceKind::Tablet => gl::PHONE,
        DeviceKind::Laptop => gl::LAPTOP,
        DeviceKind::Desktop => gl::COMPUTER,
    }
}

fn root(ctx: Ctx, frame: Arc<LiveFrame>, input: mpsc::Sender<InputEvent>) -> impl Widget {
    let _ = DEVICE.set(ctx.device.to_string());
    let touch = ctx.peer.kind.is_touch();
    let stats = ctx.stats;

    let screen = {
        let frame = frame.clone();
        let input = Arc::new(Mutex::new((input, InputState { touch, pressed: false, mods: 0, esc: false })));
        Reactive::new(move || {
            let _ = ctx.frame_rev.get();
            let input = input.clone();
            vec![Box::new(LiveView::new(frame.clone()).class("screen").on_input(move |e| {
                let fs = ctx.win.get_untracked().fullscreen;
                if fs && matches!(e, LiveInput::Move { .. } | LiveInput::Down { .. } | LiveInput::TouchStart { .. }) {
                    poke_hud(ctx);
                }
                let mut g = input.lock().unwrap();
                let (tx, st) = &mut *g;
                // Esc во весь экран — выход, той стороне не уходит.
                if let LiveInput::Key { key: Key::Escape, pressed } = e {
                    if pressed && fs {
                        st.esc = true;
                        leave_fullscreen();
                        return;
                    }
                    if !pressed && std::mem::take(&mut st.esc) {
                        return;
                    }
                }
                on_input(st, tx, e);
            })) as Box<dyn Widget>]
        })
    };
    let overlay = Reactive::new(move || {
        let s = stats.get();
        if s.size.0 > 0 && s.state.is_empty() {
            return vec![Box::new(DecoratedBox::new()) as Box<dyn Widget>];
        }
        vec![Box::new(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .gap(10.0)
                .child(icon(gl::SYNC).class("wait-icon"))
                .child(Text::new(if s.state.is_empty() { "Соединение…".to_string() } else { s.state }).max_lines(3).class("wait-text")),
        ) as Box<dyn Widget>]
    });
    // Во весь экран: кнопка выхода в углу, пока недавно трогали.
    let hud = Reactive::new(move || {
        // Читать оба сигнала всегда: подписка — на прочитанные.
        let (fs, shown) = (ctx.win.get().fullscreen, ctx.hud.get());
        if !(fs && shown) {
            return Vec::new();
        }
        vec![Box::new(
            Column::new().cross_axis_alignment(CrossAxisAlignment::Start).child(
                DecoratedBox::new()
                    .child(
                        GestureDetector::new()
                            .on_click(leave_fullscreen)
                            .child(DecoratedBox::new().child(icon(gl::FULLSCREEN_EXIT).class("fs-exit-icon")).class("fs-exit")),
                    )
                    .class("fs-corner"),
            ),
        ) as Box<dyn Widget>]
    });
    let bar = Reactive::new(move || {
        if ctx.win.get().fullscreen {
            return Vec::new();
        }
        vec![Box::new(bar(ctx)) as Box<dyn Widget>]
    });

    EventHook::new()
        // Системное «назад» телефона — из полноэкранного режима.
        .on_back(move || {
            if ctx.win.get_untracked().fullscreen {
                leave_fullscreen();
                true
            } else {
                false
            }
        })
        .child(
            Column::new()
                .gap(0.0)
                .child(bar)
                .child(
                    DecoratedBox::new()
                        .child(Stack::new().fit(StackFit::Expand).child(screen).child(overlay).child(hud))
                        .class("stage grow"),
                )
                .class("root"),
        )
}

/// Панель: устройство, состояние, кнопки.
fn bar(ctx: Ctx) -> impl Widget {
    let touch = ctx.peer.kind.is_touch();
    let peer = ctx.peer;
    let stats = ctx.stats;
    let note = ctx.note;
    let transport = peer.transport;

    let header = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(icon(kind_glyph(peer.kind)).class("dev-icon")).class("dev-badge"))
        .child(
            Column::new()
                .gap(0.0)
                .child(Text::new(peer.name.clone()).max_lines(1).class("dev-name"))
                .child(Reactive::new(move || {
                    let s = stats.get();
                    let n = note.get();
                    let line = if !n.is_empty() {
                        n
                    } else if !s.state.is_empty() {
                        s.state.clone()
                    } else {
                        format!("{} × {} · {:.0} к/с · {:.0} КБ/с · {}", s.size.0, s.size.1, s.fps, s.kbps, s.codec)
                    };
                    vec![Box::new(Text::new(line).max_lines(1).class("dev-state")) as Box<dyn Widget>]
                }))
                .class("grow"),
        )
        .child(match transport {
            Some(t) => Box::new(
                DecoratedBox::new()
                    .child(
                        Row::new()
                            .gap(4.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(icon(if t == link::Transport::Usb { gl::USB } else { gl::WIFI }).class("chip-icon"))
                            .child(Text::new(t.title()).class("chip-text")),
                    )
                    .class(if t == link::Transport::Usb { "chip chip-usb" } else { "chip chip-wifi" }),
            ) as Box<dyn Widget>,
            None => Box::new(DecoratedBox::new()),
        });

    let dev = ctx.device;
    let mut tools = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if touch {
        tools = tools
            .child(tool(gl::BACK, "Назад", || action("back")))
            .child(tool(gl::HOME, "Домой", || action("shell home")))
            .child(tool(gl::RECENTS, "Недавние", || action("shell recents")))
            .child(tool(gl::SHADE, "Шторка", || action("shell shade")))
            .child(tool(gl::KEYBOARD, "Клавиатура", || action("spawn synkeyboard toggle")));
    }
    let sound = Reactive::new(move || {
        let s = ctx.sound.get();
        let (glyph, class) = match s {
            Sound::On => (gl::VOLUME_UP, "tool tool-on"),
            Sound::Starting => (gl::VOLUME_UP, "tool tool-wait"),
            Sound::Off => (gl::VOLUME_OFF, "tool"),
        };
        vec![Box::new(
            GestureDetector::new()
                .on_click(move || toggle_sound(ctx))
                .child(DecoratedBox::new().child(icon(glyph).class("tool-icon")).class(class)),
        ) as Box<dyn Widget>]
    });
    tools = tools
        .child(DecoratedBox::new().class("grow"))
        .child(sound)
        .child(tool(gl::FULLSCREEN, "Во весь экран", || syngui::signal::set_fullscreen(true)))
        .child(tool(gl::LOCK, "Блокировка", || action("lock")))
        .child(tool(gl::SHOT, "Снимок", move || screenshot(dev)))
        .child(tool(gl::FOLDER, "Файлы", move || open_files(dev)));

    DecoratedBox::new().child(Column::new().gap(8.0).child(header).child(tools)).class("bar")
}

fn screenshot(device: &'static str) {
    std::thread::spawn(move || {
        let dir = synshell_common::paths::user_dir_or_default("PICTURES").join("Снимки экрана");
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = dir.join(format!("synlink-{device}-{stamp}.png"));
        let _ = link::request(&Request::Screenshot {
            device: device.into(),
            output: None,
            path: Some(path.to_string_lossy().into_owned()),
            max_size: None,
        });
    });
}

fn open_files(device: &'static str) {
    std::thread::spawn(move || {
        let path = match link::request(&Request::Status) {
            Ok(Response::Status { status }) => status.peers.into_iter().find(|p| p.id == device).and_then(|p| p.files_path()),
            _ => None,
        };
        let path = match path {
            Some(p) => Some(p),
            None => match link::request(&Request::Mount { device: device.into(), mount: true }) {
                Ok(Response::Mount { path }) => path,
                _ => None,
            },
        };
        if let Some(p) = path {
            let _ = std::process::Command::new("synfiles").arg(p).spawn();
        }
    });
}

// ─── звук ────────────────────────────────────────────────────────────────────

/// Соединение с демоном, держащее звук (номер включения, сокет).
static SOUND_CONN: Mutex<Option<(u64, std::os::unix::net::UnixStream)>> = Mutex::new(None);
static SOUND_GEN: AtomicU64 = AtomicU64::new(0);

fn toggle_sound(ctx: Ctx) {
    match ctx.sound.get_untracked() {
        Sound::Starting => {}
        Sound::On => {
            // Закрыли соединение — демон выключает звук.
            if let Some((_, s)) = SOUND_CONN.lock().unwrap().take() {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            ctx.sound.set(Sound::Off);
        }
        Sound::Off => {
            ctx.sound.set(Sound::Starting);
            let (device, sound, note) = (ctx.device, ctx.sound, ctx.note);
            let gen = SOUND_GEN.fetch_add(1, Ordering::Relaxed) + 1;
            std::thread::Builder::new()
                .name("view-sound".into())
                .spawn(move || {
                    let started = (|| -> Result<link::Client, String> {
                        let mut c = link::Client::connect().map_err(|e| format!("synlink: {e}"))?;
                        let req = Request::Audio { device: device.into(), on: true, mic: true, hold: true };
                        match c.request(&req).map_err(|e| e.to_string())? {
                            Response::Ok => Ok(c),
                            Response::Error { message } => Err(message),
                            other => Err(format!("неожиданный ответ: {other:?}")),
                        }
                    })();
                    let c = match started {
                        Ok(c) => c,
                        Err(e) => {
                            run_on_main_thread(move || {
                                sound.set(Sound::Off);
                                show_note(note, format!("Звук: {e}"));
                            });
                            return;
                        }
                    };
                    let (mut rd, wr) = c.into_parts();
                    *SOUND_CONN.lock().unwrap() = Some((gen, wr));
                    run_on_main_thread(move || sound.set(Sound::On));
                    // Ждать: демон скажет, если звук оборвался; закрыли сами — конец потока.
                    let mut line = String::new();
                    let _ = rd.read_line(&mut line);
                    let mine = {
                        let mut g = SOUND_CONN.lock().unwrap();
                        if g.as_ref().is_some_and(|(n, _)| *n == gen) {
                            g.take();
                            true
                        } else {
                            false
                        }
                    };
                    if !mine {
                        return;
                    }
                    let msg = match serde_json::from_str::<Response>(&line) {
                        Ok(Response::Error { message }) => message,
                        _ => "соединение прервалось".to_string(),
                    };
                    run_on_main_thread(move || {
                        sound.set(Sound::Off);
                        show_note(note, format!("Звук выключен: {msg}"));
                    });
                })
                .ok();
        }
    }
}

/// Показать сообщение в строке состояния на 6 с.
fn show_note(note: RwSignal<String>, text: String) {
    note.set(text.clone());
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(6));
        run_on_main_thread(move || {
            if note.get_untracked() == text {
                note.set(String::new());
            }
        });
    });
}

// ─── во весь экран ───────────────────────────────────────────────────────────

/// Сколько видна кнопка выхода после касания или движения мыши.
const HUD_TIME: Duration = Duration::from_secs(3);

fn fullscreen_file() -> std::path::PathBuf {
    synshell_common::paths::cache_home().join("synlink-view").join("fullscreen")
}

fn remembered_fullscreen() -> bool {
    std::fs::read_to_string(fullscreen_file()).is_ok_and(|s| s.trim() == "1")
}

/// Запомнить режим и показать кнопку выхода при входе (F11, кнопка, запуск).
fn watch_fullscreen(ctx: Ctx) {
    let last = Arc::new(AtomicBool::new(ctx.win.get_untracked().fullscreen));
    let first = Arc::new(AtomicBool::new(true));
    syngui::signal::create_effect(move || {
        let fs = ctx.win.get().fullscreen;
        // Первый вызов — состояние по умолчанию до окна, не решение пользователя.
        if first.swap(false, Ordering::Relaxed) {
            return;
        }
        if last.swap(fs, Ordering::Relaxed) == fs {
            return;
        }
        let path = fullscreen_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, if fs { "1" } else { "0" });
        if fs {
            poke_hud(ctx);
        } else {
            ctx.hud.set(false);
        }
    });
}

static HUD_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

/// Показать кнопку выхода на `HUD_TIME` (главный поток).
fn poke_hud(ctx: Ctx) {
    if !ctx.win.get_untracked().fullscreen {
        return;
    }
    *HUD_UNTIL.lock().unwrap() = Some(Instant::now() + HUD_TIME);
    if ctx.hud.get_untracked() {
        return;
    }
    ctx.hud.set(true);
    let hud = ctx.hud;
    std::thread::spawn(move || loop {
        let until = HUD_UNTIL.lock().unwrap().unwrap_or_else(Instant::now);
        let now = Instant::now();
        if now >= until {
            run_on_main_thread(move || hud.set(false));
            return;
        }
        std::thread::sleep(until - now);
    });
}

fn leave_fullscreen() {
    syngui::signal::set_fullscreen(false);
}
