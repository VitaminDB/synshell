//! Геймпады через evdev: оболочка сама читает `/dev/input/event*` (кнопки,
//! крестовина, левый стик) — для навигации по лаунчеру и кнопки Guide во
//! время игры. Чтение не монопольное: игра видит те же события. Пока открыто
//! быстрое меню, устройства захватываются (`EVIOCGRAB`), чтобы нажатия в
//! меню не доходили до игры.
//!
//! Подключение на лету: раз в пару секунд — поиск новых устройств.
//! Удержание направления повторяет шаг (как клавиша).

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Кнопка навигации — общая для геймпада и клавиатуры.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Up,
    Down,
    Left,
    Right,
    /// A / Cross / Enter — выбрать.
    Accept,
    /// B / Circle / Esc — назад.
    Back,
    /// X / Square — действия с игрой.
    Options,
    /// Y / Triangle — в избранное.
    Favorite,
    /// LB / RB — соседняя вкладка.
    PrevTab,
    NextTab,
    /// Start / Menu.
    Menu,
    /// Guide (PS/Xbox), Select+Start.
    Guide,
}

const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;

const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_MODE: u16 = 0x13c;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

const REPEAT_DELAY: Duration = Duration::from_millis(380);
const REPEAT_RATE: Duration = Duration::from_millis(110);

/// Подключено геймпадов (для строки состояния).
pub static CONNECTED: AtomicUsize = AtomicUsize::new(0);
/// Захватить устройства (открыто быстрое меню).
static GRAB: AtomicBool = AtomicBool::new(false);

pub fn set_grab(on: bool) {
    GRAB.store(on, Ordering::Relaxed);
}

fn ioc(dir: u64, nr: u64, size: u64) -> u64 {
    (dir << 30) | (size << 16) | ((b'E' as u64) << 8) | nr
}

/// `EVIOCGBIT(ev, len)`: битовая маска поддерживаемых кодов.
fn get_bits(fd: i32, ev: u16, buf: &mut [u8]) -> bool {
    let req = ioc(2, 0x20 + ev as u64, buf.len() as u64);
    // SAFETY: буфер длиной len, ядро пишет не больше len байт.
    unsafe { libc::ioctl(fd, req as _, buf.as_mut_ptr()) >= 0 }
}

fn has_bit(buf: &[u8], bit: u16) -> bool {
    let (i, b) = ((bit / 8) as usize, bit % 8);
    buf.get(i).is_some_and(|v| v & (1 << b) != 0)
}

/// `EVIOCGABS(abs)`: диапазон оси (min, max).
fn abs_range(fd: i32, abs: u16) -> Option<(i32, i32)> {
    let mut info = [0i32; 6];
    let req = ioc(2, 0x40 + abs as u64, 24);
    // SAFETY: struct input_absinfo — 6 × i32.
    let ok = unsafe { libc::ioctl(fd, req as _, info.as_mut_ptr()) >= 0 };
    ok.then_some((info[1], info[2]))
}

fn device_name(fd: i32) -> String {
    let mut buf = [0u8; 128];
    let req = ioc(2, 0x06, buf.len() as u64);
    // SAFETY: буфер на 128 байт.
    unsafe { libc::ioctl(fd, req as _, buf.as_mut_ptr()) };
    String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string()
}

fn set_device_grab(fd: i32, on: bool) {
    let req = ioc(1, 0x90, std::mem::size_of::<libc::c_int>() as u64);
    // SAFETY: EVIOCGRAB принимает целое по значению.
    unsafe { libc::ioctl(fd, req as _, if on { 1 } else { 0 } as libc::c_int) };
}

struct Pad {
    file: File,
    name: String,
    /// Диапазоны стика: центр и половина хода.
    stick: [(f32, f32); 2],
    /// Положение стика в долях −1..1.
    axis: [f32; 2],
    hat: [i32; 2],
    select: bool,
    start: bool,
    grabbed: bool,
}

fn is_gamepad(fd: i32) -> bool {
    let mut keys = [0u8; 96];
    if !get_bits(fd, EV_KEY, &mut keys) {
        return false;
    }
    has_bit(&keys, BTN_SOUTH) || has_bit(&keys, BTN_MODE) || has_bit(&keys, BTN_DPAD_UP)
}

fn open_pad(path: &PathBuf) -> Option<Pad> {
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(path).ok()?;
    let fd = file.as_raw_fd();
    if !is_gamepad(fd) {
        return None;
    }
    let name = device_name(fd);
    // Свой экранный контроллер syngamepad — тоже геймпад, но навигации им не нужно.
    if name.to_lowercase().contains("syngamepad") {
        return None;
    }
    let range = |abs| {
        abs_range(fd, abs).filter(|(lo, hi)| hi > lo).map(|(lo, hi)| ((lo + hi) as f32 / 2.0, (hi - lo) as f32 / 2.0)).unwrap_or((0.0, 32767.0))
    };
    log::info!("геймпад: {name} ({})", path.display());
    Some(Pad { file, name, stick: [range(ABS_X), range(ABS_Y)], axis: [0.0; 2], hat: [0; 2], select: false, start: false, grabbed: false })
}

/// Направление, которое сейчас удерживается (крестовина или стик).
fn held_dir(p: &Pad) -> Option<Nav> {
    const DEAD: f32 = 0.6;
    if p.hat[1] < 0 || p.axis[1] < -DEAD {
        Some(Nav::Up)
    } else if p.hat[1] > 0 || p.axis[1] > DEAD {
        Some(Nav::Down)
    } else if p.hat[0] < 0 || p.axis[0] < -DEAD {
        Some(Nav::Left)
    } else if p.hat[0] > 0 || p.axis[0] > DEAD {
        Some(Nav::Right)
    } else {
        None
    }
}

/// Запустить поток чтения; `emit` вызывается в главном потоке syngui.
pub fn start(emit: fn(Nav)) {
    std::thread::Builder::new()
        .name("gamepad".into())
        .spawn(move || run(emit))
        .expect("поток геймпада");
}

fn run(emit: fn(Nav)) {
    let send = move |n: Nav| syngui::async_runtime::run_on_main_thread(move || emit(n));
    let mut pads: HashMap<PathBuf, Pad> = HashMap::new();
    let mut last_scan = Instant::now() - Duration::from_secs(10);
    // Повтор направления: что держим, когда сработает следующий шаг.
    let mut repeat: Option<(PathBuf, Nav, Instant)> = None;
    let mut buf = [0u8; 24 * 64];
    loop {
        if last_scan.elapsed() > Duration::from_secs(2) {
            last_scan = Instant::now();
            if let Ok(rd) = std::fs::read_dir("/dev/input") {
                for e in rd.flatten() {
                    let path = e.path();
                    let is_event = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event"));
                    if is_event && !pads.contains_key(&path) {
                        if let Some(p) = open_pad(&path) {
                            pads.insert(path, p);
                        }
                    }
                }
            }
            CONNECTED.store(pads.len(), Ordering::Relaxed);
        }
        let grab = GRAB.load(Ordering::Relaxed);
        let mut fds: Vec<libc::pollfd> = Vec::new();
        let keys: Vec<PathBuf> = pads.keys().cloned().collect();
        for k in &keys {
            let p = pads.get_mut(k).unwrap();
            if p.grabbed != grab {
                set_device_grab(p.file.as_raw_fd(), grab);
                p.grabbed = grab;
            }
            fds.push(libc::pollfd { fd: p.file.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        let timeout = match &repeat {
            Some((_, _, at)) => at.saturating_duration_since(Instant::now()).as_millis().min(500) as i32,
            None => 500,
        };
        if fds.is_empty() {
            std::thread::sleep(Duration::from_millis(500));
        } else {
            // SAFETY: массив pollfd живёт до конца вызова.
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
        }
        let mut dead = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            if fds[i].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                dead.push(k.clone());
                continue;
            }
            if fds[i].revents & libc::POLLIN == 0 {
                continue;
            }
            let p = pads.get_mut(k).unwrap();
            let before = held_dir(p);
            loop {
                match p.file.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for ev in buf[..n].chunks_exact(24) {
                            let ty = u16::from_ne_bytes([ev[16], ev[17]]);
                            let code = u16::from_ne_bytes([ev[18], ev[19]]);
                            let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
                            handle_event(p, ty, code, value, &send);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        dead.push(k.clone());
                        break;
                    }
                }
            }
            let after = held_dir(p);
            if after != before {
                match after {
                    Some(d) => {
                        send(d);
                        repeat = Some((k.clone(), d, Instant::now() + REPEAT_DELAY));
                    }
                    None => {
                        if repeat.as_ref().is_some_and(|(rk, _, _)| rk == k) {
                            repeat = None;
                        }
                    }
                }
            }
        }
        for k in dead {
            if let Some(p) = pads.remove(&k) {
                log::info!("геймпад отключён: {}", p.name);
            }
            if repeat.as_ref().is_some_and(|(rk, _, _)| *rk == k) {
                repeat = None;
            }
            CONNECTED.store(pads.len(), Ordering::Relaxed);
        }
        if let Some((k, d, at)) = &mut repeat {
            if Instant::now() >= *at {
                if pads.get(k).and_then(held_dir) == Some(*d) {
                    send(*d);
                    *at = Instant::now() + REPEAT_RATE;
                } else {
                    repeat = None;
                }
            }
        }
    }
}

fn handle_event(p: &mut Pad, ty: u16, code: u16, value: i32, send: &impl Fn(Nav)) {
    match ty {
        EV_KEY => {
            let down = value == 1;
            match code {
                BTN_SELECT => p.select = value != 0,
                BTN_START => p.start = value != 0,
                _ => {}
            }
            if value == 0 {
                // Отпускание кнопок крестовины.
                match code {
                    BTN_DPAD_UP | BTN_DPAD_DOWN => p.hat[1] = 0,
                    BTN_DPAD_LEFT | BTN_DPAD_RIGHT => p.hat[0] = 0,
                    _ => {}
                }
            }
            if !down {
                return;
            }
            // Select+Start — Guide для геймпадов без неё.
            if (code == BTN_START && p.select) || (code == BTN_SELECT && p.start) {
                send(Nav::Guide);
                return;
            }
            let nav = match code {
                BTN_SOUTH => Nav::Accept,
                BTN_EAST => Nav::Back,
                BTN_WEST => Nav::Options,
                BTN_NORTH => Nav::Favorite,
                BTN_TL => Nav::PrevTab,
                BTN_TR => Nav::NextTab,
                BTN_START => Nav::Menu,
                BTN_MODE => Nav::Guide,
                // Крестовина кнопками — как шляпка.
                BTN_DPAD_UP => return set_hat(p, 1, -1),
                BTN_DPAD_DOWN => return set_hat(p, 1, 1),
                BTN_DPAD_LEFT => return set_hat(p, 0, -1),
                BTN_DPAD_RIGHT => return set_hat(p, 0, 1),
                _ => return,
            };
            send(nav);
        }
        EV_ABS => match code {
            ABS_HAT0X => p.hat[0] = value.signum(),
            ABS_HAT0Y => p.hat[1] = value.signum(),
            ABS_X | ABS_Y => {
                let i = code as usize;
                let (c, half) = p.stick[i];
                p.axis[i] = ((value as f32 - c) / half.max(1.0)).clamp(-1.0, 1.0);
            }
            _ => {}
        },
        _ => {}
    }
}

fn set_hat(p: &mut Pad, axis: usize, v: i32) {
    p.hat[axis] = v;
}
