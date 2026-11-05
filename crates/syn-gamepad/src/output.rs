//! Куда уходят нажатия: геймпад и мышь (uinput), клавиши (zwp_virtual_keyboard_v1). Счётчики —
//! одну кнопку геймпада или клавишу могут держать два элемента сразу (стик WASD и кнопка W).

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use crate::layout::{Axis, Bind};
use crate::uinput::{self, Device, PadButton};

/// Порог отклонения стика для клавиш и крестовины.
const DIR_THRESHOLD: f32 = 0.4;
/// Стик-мышь: пикселей в секунду при полном отклонении (умножается на чувствительность).
const MOUSE_SPEED: f32 = 900.0;
const MOUSE_TICK: Duration = Duration::from_millis(16);

#[derive(Default)]
struct Output {
    pad: Option<Device>,
    mouse: Option<Device>,
    /// Устройства не создались (нет доступа к /dev/uinput) — не пробовать на каждое нажатие.
    failed: bool,
    buttons: HashMap<PadButton, u32>,
    triggers: [u32; 2],
    keys: HashMap<u32, u32>,
    mods: u32,
    mouse_buttons: HashMap<u16, u32>,
    /// Клавиши направлений, которые держит каждый стик/крестовина (по номеру элемента).
    dir_keys: HashMap<usize, Vec<u32>>,
    /// Направление крестовины-HAT каждого элемента: суммируются.
    hats: HashMap<usize, (i32, i32)>,
    /// Стики-мыши: скорость каждого.
    mouse_vel: HashMap<usize, (f32, f32)>,
    mouse_frac: (f32, f32),
    mouse_timer: Option<u64>,
}

thread_local! {
    static OUT: RefCell<Output> = RefCell::new(Output::default());
}

fn with<R>(f: impl FnOnce(&mut Output) -> R) -> R {
    OUT.with(|o| f(&mut o.borrow_mut()))
}

/// Создать устройства (контроллер показан): игры видят подключённый геймпад.
pub fn connect() {
    with(|o| {
        if o.pad.is_some() || o.failed {
            return;
        }
        match (Device::gamepad(), Device::mouse()) {
            (Ok(p), Ok(m)) => {
                o.pad = Some(p);
                o.mouse = Some(m);
                log::info!("геймпад и мышь uinput созданы");
            }
            (p, m) => {
                o.failed = true;
                let e = p.err().or(m.err()).map(|e| e.to_string()).unwrap_or_default();
                log::error!("/dev/uinput: {e} — кнопки геймпада и мыши не работают, клавиши — да");
            }
        }
    });
}

/// Отпустить всё и убрать устройства (контроллер спрятан или выключен).
pub fn disconnect() {
    release_all();
    with(|o| {
        o.pad = None;
        o.mouse = None;
        o.failed = false;
    });
}

/// Отпустить всё нажатое (палец ушёл вместе с поверхностью).
pub fn release_all() {
    let keys: Vec<u32> = with(|o| o.keys.keys().copied().collect());
    for k in keys {
        syngui_layer::virtual_keyboard_key(k, false);
    }
    with(|o| {
        o.keys.clear();
        if o.mods != 0 {
            o.mods = 0;
            syngui_layer::virtual_keyboard_modifiers(0, 0, 0);
        }
        o.dir_keys.clear();
        o.mouse_vel.clear();
        if let Some(t) = o.mouse_timer.take() {
            syngui_layer::cancel_timer(t);
        }
        if let Some(p) = o.pad.as_mut() {
            for b in o.buttons.keys() {
                p.key(b.code(), false);
            }
            for c in [uinput::ABS_X, uinput::ABS_Y, uinput::ABS_RX, uinput::ABS_RY, uinput::ABS_Z, uinput::ABS_RZ, uinput::ABS_HAT0X, uinput::ABS_HAT0Y] {
                p.abs_value(c, 0);
            }
            p.sync();
        }
        if let Some(m) = o.mouse.as_mut() {
            for b in o.mouse_buttons.keys() {
                m.key(*b, false);
            }
            m.sync();
        }
        o.buttons.clear();
        o.triggers = [0, 0];
        o.mouse_buttons.clear();
        o.hats.clear();
    });
}

fn count<K: std::hash::Hash + Eq + Copy>(map: &mut HashMap<K, u32>, k: K, pressed: bool) -> bool {
    let n = map.entry(k).or_insert(0);
    let was = *n;
    if pressed {
        *n += 1;
    } else {
        *n = n.saturating_sub(1);
    }
    let now = *n;
    if now == 0 {
        map.remove(&k);
    }
    (was == 0) != (now == 0)
}

fn key(o: &mut Output, code: u32, pressed: bool) {
    if !count(&mut o.keys, code, pressed) {
        return;
    }
    let mask = crate::keys::modifier_mask(code);
    if mask != 0 {
        // Маска — до нажатия обычной клавиши: композитор сам её клиентам не считает.
        o.mods = o.keys.keys().fold(0, |m, k| m | crate::keys::modifier_mask(*k));
        syngui_layer::virtual_keyboard_modifiers(o.mods, 0, 0);
    }
    syngui_layer::virtual_keyboard_key(code, pressed);
}

/// Кнопка нажата/отпущена.
pub fn button(bind: &Bind, pressed: bool) {
    with(|o| match bind {
        Bind::None => {}
        Bind::Pad(b) => {
            if count(&mut o.buttons, *b, pressed) {
                if let Some(p) = o.pad.as_mut() {
                    p.key(b.code(), pressed);
                    p.sync();
                }
            }
        }
        Bind::Trigger(right) => {
            let i = *right as usize;
            let was = o.triggers[i] > 0;
            o.triggers[i] = if pressed { o.triggers[i] + 1 } else { o.triggers[i].saturating_sub(1) };
            if was != (o.triggers[i] > 0) {
                if let Some(p) = o.pad.as_mut() {
                    p.abs_value(if *right { uinput::ABS_RZ } else { uinput::ABS_Z }, if pressed { 255 } else { 0 });
                    p.sync();
                }
            }
        }
        Bind::Keys(codes) => {
            if pressed {
                for c in codes {
                    key(o, *c, true);
                }
            } else {
                for c in codes.iter().rev() {
                    key(o, *c, false);
                }
            }
        }
        Bind::Mouse(b) => {
            if count(&mut o.mouse_buttons, *b, pressed) {
                if let Some(m) = o.mouse.as_mut() {
                    m.key(*b, pressed);
                    m.sync();
                }
            }
        }
        Bind::WheelUp | Bind::WheelDown => {
            if pressed {
                if let Some(m) = o.mouse.as_mut() {
                    m.rel(uinput::REL_WHEEL, if *bind == Bind::WheelUp { 1 } else { -1 });
                    m.sync();
                }
            }
        }
    });
}

/// Стик или крестовина элемента `slot` отклонён на (x, y), каждая ось −1…1, y вниз.
pub fn axis(slot: usize, axis: Axis, x: f32, y: f32) {
    match axis {
        Axis::LeftStick | Axis::RightStick => with(|o| {
            let (cx, cy) = if axis == Axis::LeftStick { (uinput::ABS_X, uinput::ABS_Y) } else { (uinput::ABS_RX, uinput::ABS_RY) };
            if let Some(p) = o.pad.as_mut() {
                p.abs_value(cx, (x.clamp(-1.0, 1.0) * 32767.0) as i32);
                p.abs_value(cy, (y.clamp(-1.0, 1.0) * 32767.0) as i32);
                p.sync();
            }
        }),
        Axis::Hat => with(|o| {
            let d = (dir(x), dir(y));
            if d == (0, 0) {
                o.hats.remove(&slot);
            } else {
                o.hats.insert(slot, d);
            }
            let (hx, hy) = o.hats.values().fold((0, 0), |a, d| ((a.0 + d.0).clamp(-1, 1), (a.1 + d.1).clamp(-1, 1)));
            if let Some(p) = o.pad.as_mut() {
                p.abs_value(uinput::ABS_HAT0X, hx);
                p.abs_value(uinput::ABS_HAT0Y, hy);
                p.sync();
            }
        }),
        Axis::Wasd | Axis::Arrows => {
            let names = if axis == Axis::Wasd { ["w", "a", "s", "d"] } else { ["up", "left", "down", "right"] };
            let code = |n: &str| crate::keys::code(n).unwrap_or(0);
            let mut want = Vec::new();
            match dir(y) {
                -1 => want.push(code(names[0])),
                1 => want.push(code(names[2])),
                _ => {}
            }
            match dir(x) {
                -1 => want.push(code(names[1])),
                1 => want.push(code(names[3])),
                _ => {}
            }
            with(|o| {
                let held = o.dir_keys.remove(&slot).unwrap_or_default();
                for k in held.iter().filter(|k| !want.contains(k)) {
                    key(o, *k, false);
                }
                for k in want.iter().filter(|k| !held.contains(k)) {
                    key(o, *k, true);
                }
                if !want.is_empty() {
                    o.dir_keys.insert(slot, want);
                }
            });
        }
        Axis::Mouse => {
            let start = with(|o| {
                if x == 0.0 && y == 0.0 {
                    o.mouse_vel.remove(&slot);
                } else {
                    o.mouse_vel.insert(slot, (x, y));
                }
                !o.mouse_vel.is_empty() && o.mouse_timer.is_none()
            });
            if start {
                let t = syngui_layer::add_timer(MOUSE_TICK, || {
                    let go = with(|o| {
                        let (vx, vy) = o.mouse_vel.values().fold((0.0, 0.0), |a, v| (a.0 + v.0, a.1 + v.1));
                        let k = MOUSE_SPEED * MOUSE_TICK.as_secs_f32();
                        // Мягкий старт: мелкое отклонение — точное движение.
                        let curve = |v: f32| v * v.abs();
                        o.mouse_frac.0 += curve(vx) * k;
                        o.mouse_frac.1 += curve(vy) * k;
                        let (dx, dy) = (o.mouse_frac.0.trunc(), o.mouse_frac.1.trunc());
                        o.mouse_frac.0 -= dx;
                        o.mouse_frac.1 -= dy;
                        if let Some(m) = o.mouse.as_mut() {
                            m.rel(uinput::REL_X, dx as i32);
                            m.rel(uinput::REL_Y, dy as i32);
                            m.sync();
                        }
                        if o.mouse_vel.is_empty() {
                            o.mouse_timer = None;
                            false
                        } else {
                            true
                        }
                    });
                    go.then_some(MOUSE_TICK)
                });
                with(|o| o.mouse_timer = Some(t));
            }
        }
    }
}

/// Стик-мышь со своей чувствительностью: скорость масштабируется заранее.
pub fn mouse_axis(slot: usize, x: f32, y: f32, sensitivity: f32) {
    axis(slot, Axis::Mouse, x * sensitivity, y * sensitivity);
}

/// Тачпад: сдвиг мыши на (dx, dy).
pub fn mouse_move(dx: f32, dy: f32) {
    with(|o| {
        o.mouse_frac.0 += dx;
        o.mouse_frac.1 += dy;
        let (ix, iy) = (o.mouse_frac.0.trunc(), o.mouse_frac.1.trunc());
        o.mouse_frac.0 -= ix;
        o.mouse_frac.1 -= iy;
        if let Some(m) = o.mouse.as_mut() {
            m.rel(uinput::REL_X, ix as i32);
            m.rel(uinput::REL_Y, iy as i32);
            m.sync();
        }
    });
}

fn dir(v: f32) -> i32 {
    if v <= -DIR_THRESHOLD {
        -1
    } else if v >= DIR_THRESHOLD {
        1
    } else {
        0
    }
}
