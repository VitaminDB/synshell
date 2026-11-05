//! Устройства ядра через `/dev/uinput`: геймпад (как проводной Xbox 360 — SDL, Wine, Steam и
//! Android знают его раскладку по VID/PID) и мышь (её подхватывает композитор через libinput, как
//! настоящую). Клавиш у геймпада нет: устройство с клавишами композитор принял бы за клавиатуру.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

pub const REL_X: u16 = 0;
pub const REL_Y: u16 = 1;
pub const REL_WHEEL: u16 = 8;

pub const ABS_X: u16 = 0;
pub const ABS_Y: u16 = 1;
pub const ABS_Z: u16 = 2;
pub const ABS_RX: u16 = 3;
pub const ABS_RY: u16 = 4;
pub const ABS_RZ: u16 = 5;
pub const ABS_HAT0X: u16 = 0x10;
pub const ABS_HAT0Y: u16 = 0x11;

/// Кнопки геймпада, как их отдаёт драйвер xpad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadButton {
    A,
    B,
    X,
    Y,
    Lb,
    Rb,
    Back,
    Start,
    Guide,
    L3,
    R3,
}

impl PadButton {
    pub const ALL: [PadButton; 11] = [
        PadButton::A,
        PadButton::B,
        PadButton::X,
        PadButton::Y,
        PadButton::Lb,
        PadButton::Rb,
        PadButton::Back,
        PadButton::Start,
        PadButton::Guide,
        PadButton::L3,
        PadButton::R3,
    ];

    pub fn code(self) -> u16 {
        match self {
            PadButton::A => 0x130,
            PadButton::B => 0x131,
            PadButton::X => 0x133,
            PadButton::Y => 0x134,
            PadButton::Lb => 0x136,
            PadButton::Rb => 0x137,
            PadButton::Back => 0x13a,
            PadButton::Start => 0x13b,
            PadButton::Guide => 0x13c,
            PadButton::L3 => 0x13d,
            PadButton::R3 => 0x13e,
        }
    }
}

// linux/uinput.h
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405c_5503;
const UI_ABS_SETUP: libc::c_ulong = 0x401c_5504;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;
const UI_SET_ABSBIT: libc::c_ulong = 0x4004_5567;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
struct UinputAbsSetup {
    code: u16,
    absinfo: AbsInfo,
}

#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

pub struct Device {
    file: File,
}

impl Device {
    fn open() -> std::io::Result<File> {
        OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open("/dev/uinput")
    }

    fn ioctl(f: &File, req: libc::c_ulong, arg: libc::c_ulong) -> std::io::Result<()> {
        // SAFETY: запросы uinput с целым аргументом или указателем на структуру из linux/uinput.h.
        let r = unsafe { libc::ioctl(f.as_raw_fd(), req as _, arg) };
        if r < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn setup(f: &File, name: &str, vendor: u16, product: u16) -> std::io::Result<()> {
        let mut s = UinputSetup { id: InputId { bustype: 0x03, vendor, product, version: 0x110 }, name: [0; 80], ff_effects_max: 0 };
        let n = name.len().min(79);
        s.name[..n].copy_from_slice(&name.as_bytes()[..n]);
        Self::ioctl(f, UI_DEV_SETUP, &s as *const _ as libc::c_ulong)?;
        Self::ioctl(f, UI_DEV_CREATE, 0)
    }

    fn abs(f: &File, code: u16, min: i32, max: i32, fuzz: i32, flat: i32) -> std::io::Result<()> {
        Self::ioctl(f, UI_SET_ABSBIT, code as libc::c_ulong)?;
        let a = UinputAbsSetup { code, absinfo: AbsInfo { value: 0, minimum: min, maximum: max, fuzz, flat, resolution: 0 } };
        Self::ioctl(f, UI_ABS_SETUP, &a as *const _ as libc::c_ulong)
    }

    /// Геймпад «Microsoft X-Box 360 pad» (045e:028e): стики ±32767, курки 0–255, крестовина — HAT.
    pub fn gamepad() -> std::io::Result<Self> {
        let f = Self::open()?;
        Self::ioctl(&f, UI_SET_EVBIT, EV_KEY as _)?;
        for b in PadButton::ALL {
            Self::ioctl(&f, UI_SET_KEYBIT, b.code() as _)?;
        }
        Self::ioctl(&f, UI_SET_EVBIT, EV_ABS as _)?;
        for c in [ABS_X, ABS_Y, ABS_RX, ABS_RY] {
            Self::abs(&f, c, -32768, 32767, 16, 128)?;
        }
        for c in [ABS_Z, ABS_RZ] {
            Self::abs(&f, c, 0, 255, 0, 0)?;
        }
        for c in [ABS_HAT0X, ABS_HAT0Y] {
            Self::abs(&f, c, -1, 1, 0, 0)?;
        }
        Self::setup(&f, "Microsoft X-Box 360 pad", 0x045e, 0x028e)?;
        Ok(Self { file: f })
    }

    /// Мышь: движение, колесо, три кнопки.
    pub fn mouse() -> std::io::Result<Self> {
        let f = Self::open()?;
        Self::ioctl(&f, UI_SET_EVBIT, EV_KEY as _)?;
        for b in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE] {
            Self::ioctl(&f, UI_SET_KEYBIT, b as _)?;
        }
        Self::ioctl(&f, UI_SET_EVBIT, EV_REL as _)?;
        for r in [REL_X, REL_Y, REL_WHEEL] {
            Self::ioctl(&f, UI_SET_RELBIT, r as _)?;
        }
        Self::setup(&f, "syngamepad mouse", 0x1209, 0x5367)?;
        Ok(Self { file: f })
    }

    fn emit(&mut self, kind: u16, code: u16, value: i32) {
        let ev = InputEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind, code, value };
        // SAFETY: input_event — простая структура без указателей; пишем её байты как есть.
        let bytes = unsafe { std::slice::from_raw_parts(&ev as *const _ as *const u8, std::mem::size_of::<InputEvent>()) };
        if let Err(e) = self.file.write_all(bytes) {
            log::debug!("uinput: {e}");
        }
    }

    pub fn key(&mut self, code: u16, pressed: bool) {
        self.emit(EV_KEY, code, pressed as i32);
    }

    pub fn abs_value(&mut self, code: u16, value: i32) {
        self.emit(EV_ABS, code, value);
    }

    pub fn rel(&mut self, code: u16, value: i32) {
        if value != 0 {
            self.emit(EV_REL, code, value);
        }
    }

    pub fn sync(&mut self) {
        self.emit(EV_SYN, SYN_REPORT, 0);
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = Self::ioctl(&self.file, UI_DEV_DESTROY, 0);
    }
}
