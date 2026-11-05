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
}

static CAP: Mutex<Option<Cap>> = Mutex::new(None);

/// Захват включён у какой-нибудь машины.
pub fn any() -> bool {
    CAP.lock().unwrap().is_some()
}

/// Захват включён у этой машины (и указатель всё ещё заперт).
pub fn active(id: &str) -> bool {
    let on = CAP.lock().unwrap().as_ref().is_some_and(|c| c.id == id);
    if on && !syngui::signal::pointer_locked() {
        // Окно потеряло фокус — syngui уже отпустил указатель; привести своё в порядок.
        stop();
        return false;
    }
    on
}

/// Включить захват: относительная мышь гостя — активной, указатель — запереть.
pub fn start(vm: &Vm) -> std::result::Result<(), String> {
    stop();
    // Относительная мышь — на ходу (около секунды, пока гость её поднимет).
    if let Err(e) = crate::qmp::rel_mouse_on(vm) {
        return Err(t!("Не удалось подключить мышь для игр: {e}", e = e));
    }
    if let Err(e) = syngui::signal::set_pointer_lock(true) {
        crate::qmp::rel_mouse_off(vm);
        return Err(e);
    }
    let _ = syngui::signal::inhibit_shortcuts(true);
    let (tx, rx) = channel::<Ev>();
    let v = vm.clone();
    std::thread::Builder::new()
        .name("syn-vm-capture".into())
        .spawn(move || {
            let Ok(mut inp) = crate::qmp::RelInput::open(&v) else { return };
            // Дробные смещения копятся: на HiDPI мышь даёт доли пикселя.
            let (mut fx, mut fy) = (0.0f64, 0.0f64);
            while let Ok(ev) = rx.recv() {
                match ev {
                    Ev::Motion(dx, dy) => {
                        fx += dx;
                        fy += dy;
                        let (ix, iy) = (fx.trunc(), fy.trunc());
                        fx -= ix;
                        fy -= iy;
                        let _ = inp.motion(ix as i64, iy as i64);
                    }
                    Ev::Button(b, down) => {
                        let _ = inp.button(b, down);
                    }
                    Ev::Stop => break,
                }
            }
            crate::qmp::rel_mouse_off(&v);
        })
        .map_err(|e| e.to_string())?;
    let mtx = tx.clone();
    syngui::signal::on_raw_motion(Some(Box::new(move |dx, dy| {
        let _ = mtx.send(Ev::Motion(dx, dy));
    })));
    *CAP.lock().unwrap() = Some(Cap { id: vm.id.clone(), tx });
    Ok(())
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
