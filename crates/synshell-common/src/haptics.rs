//! Виброотклик через вибромотор (input-устройство с force feedback, на
//! телефоне — `qcom-hv-haptics`). Любой процесс сеанса зовёт [`play`] с
//! видом отклика; разрешено ли и с какой силой — из `[haptics]` config.toml
//! (процесс, следящий за конфигом, передаёт свежие значения в
//! [`set_config`]; иначе модуль читает их сам и перечитывает, когда
//! config.toml меняется).
//!
//! Работа — в отдельном потоке: он держит устройство открытым (эффекты в
//! ядре живут, пока открыт дескриптор) и проигрывает узоры «вкл/пауза».
//! Устройства нет (десктоп) — отклики молча пропадают. На десктопе
//! чужое FF-устройство (геймпад) не трогаем: подходит только вибромотор по
//! имени или любое FF-устройство на телефоне.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::config::Haptics;

/// Вид отклика: от него зависят узор и какой переключатель `[haptics]` его
/// разрешает.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feedback {
    /// Клавиша экранной клавиатуры.
    Key,
    /// Жест от края экрана сработал.
    Gesture,
    /// Жест с удержанием («Недавние»).
    GestureHold,
    /// Удержание пальцем в приложении (выбор, меню).
    LongPress,
    /// Лёгкий щелчок (переключатель).
    Tick,
    /// Пришло уведомление.
    Notification,
}

impl Feedback {
    fn allowed(self, h: &Haptics) -> bool {
        h.enabled
            && match self {
                Self::Key => h.keyboard,
                Self::Gesture | Self::GestureHold => h.gestures,
                Self::LongPress | Self::Tick => h.touch,
                Self::Notification => h.notifications,
            }
    }

    /// Узор: (вибрация, пауза) в мс.
    fn pattern(self) -> &'static [(u16, u16)] {
        match self {
            Self::Key => &[(10, 0)],
            Self::Tick => &[(12, 0)],
            Self::Gesture => &[(18, 0)],
            Self::LongPress => &[(26, 0)],
            Self::GestureHold => &[(34, 0)],
            Self::Notification => &[(70, 110), (70, 0)],
        }
    }
}

/// Настройки и откуда они: переданы процессом (он сам следит за конфигом)
/// или прочитаны здесь — тогда перечитываются, когда config.toml меняется
/// (проверка не чаще раза в 2 с).
struct State {
    haptics: Haptics,
    explicit: bool,
    mtime: Option<std::time::SystemTime>,
    checked: Instant,
}

static CONFIG: Mutex<Option<State>> = Mutex::new(None);

/// Свежие настройки (после чтения или перечитывания config.toml).
pub fn set_config(h: &Haptics) {
    if let Ok(mut c) = CONFIG.lock() {
        *c = Some(State { haptics: h.clone(), explicit: true, mtime: None, checked: Instant::now() });
    }
}

fn config() -> Haptics {
    let mut c = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let mtime = || std::fs::metadata(crate::paths::config_file()).and_then(|m| m.modified()).ok();
    let stale = match c.as_mut() {
        None => true,
        Some(s) if s.explicit || s.checked.elapsed() < Duration::from_secs(2) => false,
        Some(s) => {
            s.checked = Instant::now();
            mtime() != s.mtime
        }
    };
    if stale {
        let m = mtime();
        *c = Some(State { haptics: crate::config::Config::load().0.haptics, explicit: false, mtime: m, checked: Instant::now() });
    }
    c.as_ref().map(|s| s.haptics.clone()).unwrap_or_default()
}

/// Отклик, если он разрешён настройками.
pub fn play(f: Feedback) {
    let h = config();
    if f.allowed(&h) {
        send(f.pattern().to_vec(), h.strength);
    }
}

/// Проба силы (страница настроек): без оглядки на переключатели.
pub fn test(strength: u32) {
    send(vec![(40, 0)], strength);
}

struct Job {
    pattern: Vec<(u16, u16)>,
    strength: u32,
}

fn send(pattern: Vec<(u16, u16)>, strength: u32) {
    static TX: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("haptics".into())
            .spawn(move || {
                let mut dev: Option<Device> = None;
                let mut last_probe: Option<Instant> = None;
                while let Ok(job) = rx.recv() {
                    if dev.is_none() && last_probe.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
                        last_probe = Some(Instant::now());
                        dev = Device::find();
                    }
                    let Some(d) = dev.as_mut() else { continue };
                    if d.play(&job).is_err() {
                        // Устройство пропало (модуль выгрузили) — поищем снова.
                        dev = None;
                        last_probe = None;
                    }
                }
            })
            .ok();
        Mutex::new(tx)
    });
    if let Ok(tx) = tx.lock() {
        let _ = tx.send(Job { pattern, strength: strength.min(100) });
    }
}

// ─── evdev force feedback ───────────────────────────────────────────────────

const EV_FF: u16 = 0x15;
const FF_RUMBLE: u16 = 0x50;
const FF_CONSTANT: u16 = 0x52;
const FF_GAIN: u16 = 0x60;

/// `struct ff_effect` (64-битные ABI): объединение с указателем выровнено
/// на 8 — отсюда поле-заполнитель.
#[repr(C)]
struct FfEffect {
    kind: u16,
    id: i16,
    direction: u16,
    trigger: [u16; 2],
    replay_length: u16,
    replay_delay: u16,
    _pad: u16,
    u: [u64; 4],
}

#[repr(C)]
struct InputEvent {
    time: [libc::c_long; 2],
    kind: u16,
    code: u16,
    value: i32,
}

const fn ioc(dir: u64, nr: u64, size: u64) -> u64 {
    (dir << 30) | (size << 16) | ((b'E' as u64) << 8) | nr
}

fn eviocgbit(ev: u16, len: usize) -> u64 {
    ioc(2, 0x20 + ev as u64, len as u64)
}

const EVIOCSFF: u64 = ioc(1, 0x80, std::mem::size_of::<FfEffect>() as u64);

fn bit(bits: &[u8], n: u16) -> bool {
    bits.get(n as usize / 8).is_some_and(|b| b & (1 << (n % 8)) != 0)
}

struct Device {
    file: File,
    kind: u16,
    /// Загруженный эффект: дальше он только обновляется.
    id: i16,
}

impl Device {
    fn find() -> Option<Self> {
        let phone = std::env::var("SYNSHELL_FORM_FACTOR").as_deref() == Ok("phone");
        let mut paths: Vec<_> = std::fs::read_dir("/dev/input").ok()?.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")) {
                continue;
            }
            let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&p) else { continue };
            let fd = file.as_raw_fd();
            let mut ffb = [0u8; 16];
            if unsafe { libc::ioctl(fd, eviocgbit(EV_FF, ffb.len()) as _, ffb.as_mut_ptr()) } < 0 {
                continue;
            }
            let kind = if bit(&ffb, FF_CONSTANT) {
                FF_CONSTANT
            } else if bit(&ffb, FF_RUMBLE) {
                FF_RUMBLE
            } else {
                continue;
            };
            let mut name = [0u8; 128];
            unsafe { libc::ioctl(fd, ioc(2, 0x06, name.len() as u64) as _, name.as_mut_ptr()) };
            let name = String::from_utf8_lossy(name.split(|b| *b == 0).next().unwrap_or(&[])).to_lowercase();
            if !(phone || name.contains("hapt") || name.contains("vib")) {
                continue;
            }
            tracing::info!(device = %p.display(), name, "вибромотор");
            return Some(Self { file, kind, id: -1 });
        }
        None
    }

    fn write_event(&self, kind: u16, code: u16, value: i32) -> std::io::Result<()> {
        let ev = InputEvent { time: [0, 0], kind, code, value };
        let n = unsafe {
            libc::write(self.file.as_raw_fd(), &ev as *const InputEvent as *const libc::c_void, std::mem::size_of::<InputEvent>())
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn play(&mut self, job: &Job) -> std::io::Result<()> {
        let level = (job.strength as f32 / 100.0).clamp(0.0, 1.0);
        if level <= 0.0 {
            return Ok(());
        }
        // Сила — усилением (если драйвер умеет) и уровнем эффекта.
        let _ = self.write_event(EV_FF, FF_GAIN, (level * 0xffff as f32) as i32);
        for &(on, off) in &job.pattern {
            let mut e = FfEffect { kind: self.kind, id: self.id, direction: 0, trigger: [0; 2], replay_length: on, replay_delay: 0, _pad: 0, u: [0; 4] };
            let bytes = unsafe { std::slice::from_raw_parts_mut(e.u.as_mut_ptr() as *mut u8, 32) };
            if self.kind == FF_CONSTANT {
                bytes[0..2].copy_from_slice(&((level * 0x7fff as f32) as i16).to_ne_bytes());
            } else {
                bytes[0..2].copy_from_slice(&((level * 0xffff as f32) as u16).to_ne_bytes());
            }
            if unsafe { libc::ioctl(self.file.as_raw_fd(), EVIOCSFF as _, &mut e as *mut FfEffect) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            self.id = e.id;
            self.write_event(EV_FF, self.id as u16, 1)?;
            std::thread::sleep(Duration::from_millis(on as u64 + off as u64));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_sizes() {
        assert_eq!(std::mem::size_of::<FfEffect>(), 48);
        assert_eq!(std::mem::size_of::<InputEvent>(), 24);
        assert_eq!(EVIOCSFF, 0x40304580);
    }

    #[test]
    fn switches() {
        let mut h = Haptics::default();
        assert!(Feedback::Key.allowed(&h));
        h.keyboard = false;
        assert!(!Feedback::Key.allowed(&h));
        assert!(Feedback::Gesture.allowed(&h));
        h.enabled = false;
        assert!(!Feedback::Gesture.allowed(&h));
    }
}
