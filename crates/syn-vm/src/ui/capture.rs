//! Захват ввода для игр (машины с 3D-видеокартой syn-gpu): указатель заперт и скрыт, сырые
//! смещения мыши (relative-pointer, без ускорения) и кнопки уходят в относительную мышь
//! гостя (`usb-mouse`, активная на время захвата), сочетания (Alt+Tab, Win) — гостю.
//! Выход — Ctrl+Alt; при потере фокуса окна syngui снимает захват сам.

use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

use syngui::prelude::*;

use crate::model::Vm;

enum Ev {
    Motion(f64, f64),
    Button(&'static str, bool),
    Stop,
}

struct Cap {
    id: String,
    tx: Sender<Ev>,
    /// Блокировка указателя уже подтверждена (на Wayland — не сразу после запроса).
    locked_seen: bool,
}

static CAP: Mutex<Option<Cap>> = Mutex::new(None);

/// Ход захвата и ошибки — в журнал машины (`syn-vm.log` в её папке).
pub fn log(vm: &Vm, msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(vm.dir().join("syn-vm.log")) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{t} захват: {msg}");
    }
}

/// Захват включён у какой-нибудь машины.
pub fn any() -> bool {
    CAP.lock().unwrap().is_some()
}

/// Захват включён у этой машины.
pub fn active(id: &str) -> bool {
    let mut g = CAP.lock().unwrap();
    let Some(c) = g.as_mut().filter(|c| c.id == id) else { return false };
    let locked = syngui::signal::pointer_locked();
    if locked {
        c.locked_seen = true;
        return true;
    }
    if c.locked_seen {
        // Окно потеряло фокус — syngui уже отпустил указатель; привести своё в порядок.
        drop(g);
        stop();
        return false;
    }
    true
}

/// Включить захват (в фоне: относительная мышь подключается около секунды): мышь гостя —
/// относительной, указатель — запереть. Итог и ошибки — уведомлением и в журнал.
pub fn start(st: crate::state::St, vm: &Vm) {
    stop();
    let v = vm.clone();
    std::thread::spawn(move || {
        log(&v, "подключаю относительную мышь");
        if let Err(e) = crate::qmp::rel_mouse_on(&v) {
            log(&v, &format!("мышь не подключилась: {e}"));
            let msg = t!("Не удалось подключить мышь для игр: {e}", e = e);
            run_on_main_thread(move || crate::state::toast_err(st, msg));
            return;
        }
        log(&v, "мышь активна, запираю указатель");
        run_on_main_thread(move || lock(st, v));
    });
}

fn lock(st: crate::state::St, vm: Vm) {
    if let Err(e) = syngui::signal::set_pointer_lock(true) {
        log(&vm, &format!("указатель не заперся: {e}"));
        std::thread::spawn(move || crate::qmp::rel_mouse_off(&vm));
        crate::state::toast_err(st, e);
        return;
    }
    if let Err(e) = syngui::signal::inhibit_shortcuts(true) {
        log(&vm, &format!("сочетания не перехвачены: {e}"));
    }
    let (tx, rx) = channel::<Ev>();
    let v = vm.clone();
    std::thread::Builder::new()
        .name("syn-vm-capture".into())
        .spawn(move || {
            let mut inp = match crate::qmp::RelInput::open(&v) {
                Ok(i) => i,
                Err(e) => {
                    log(&v, &format!("QMP: {e}"));
                    return;
                }
            };
            // Дробные смещения копятся: на HiDPI мышь даёт доли пикселя.
            let (mut fx, mut fy) = (0.0f64, 0.0f64);
            let mut errs = 0;
            let mut moves = 0u64;
            while let Ok(ev) = rx.recv() {
                let r = match ev {
                    Ev::Motion(dx, dy) => {
                        moves += 1;
                        if moves == 1 || moves % 500 == 0 {
                            log(&v, &format!("смещений мыши: {moves}"));
                        }
                        fx += dx;
                        fy += dy;
                        let (ix, iy) = (fx.trunc(), fy.trunc());
                        fx -= ix;
                        fy -= iy;
                        inp.motion(ix as i64, iy as i64)
                    }
                    Ev::Button(b, down) => {
                        log(&v, &format!("кнопка {b} {}", if down { "нажата" } else { "отпущена" }));
                        inp.button(b, down)
                    }
                    Ev::Stop => break,
                };
                if let Err(e) = r {
                    errs += 1;
                    if errs <= 3 {
                        log(&v, &format!("QMP: {e}"));
                    }
                }
            }
            // Соединение ввода — закрыть до запросов к основному монитору.
            drop(inp);
            log(&v, "выход, указатель — планшету");
            crate::qmp::rel_mouse_off(&v);
        })
        .ok();
    let mtx = tx.clone();
    syngui::signal::on_raw_motion(Some(Box::new(move |dx, dy| {
        let _ = mtx.send(Ev::Motion(dx, dy));
    })));
    *CAP.lock().unwrap() = Some(Cap { id: vm.id.clone(), tx, locked_seen: false });
    log(&vm, "включён");
    crate::state::toast(st, t!("Мышь захвачена игрой — Ctrl+Alt, чтобы освободить"));
}

/// Выключить захват: указатель и сочетания — системе, мышь гостя — снова планшет.
pub fn stop() {
    let Some(c) = CAP.lock().unwrap().take() else { return };
    syngui::signal::on_raw_motion(None);
    let _ = syngui::signal::set_pointer_lock(false);
    let _ = syngui::signal::inhibit_shortcuts(false);
    let _ = c.tx.send(Ev::Stop);
}

/// Кнопка мыши в захвате: 0 — левая, 1 — средняя, 2 — правая.
pub fn button(idx: u8, down: bool) {
    let b = match idx {
        0 => "left",
        1 => "middle",
        2 => "right",
        _ => return,
    };
    if let Some(c) = CAP.lock().unwrap().as_ref() {
        let _ = c.tx.send(Ev::Button(b, down));
    }
}

/// Колесо в захвате: шаги (> 0 — вниз).
pub fn wheel(steps: i32) {
    let b = if steps > 0 { "wheel-down" } else { "wheel-up" };
    if let Some(c) = CAP.lock().unwrap().as_ref() {
        for _ in 0..steps.unsigned_abs() {
            let _ = c.tx.send(Ev::Button(b, true));
            let _ = c.tx.send(Ev::Button(b, false));
        }
    }
}
