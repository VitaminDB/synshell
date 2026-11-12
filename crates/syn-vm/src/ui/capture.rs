//! Захват ввода для игр (машины с 3D-видеокартой syn-gpu): указатель заперт и скрыт, сырые
//! смещения мыши (relative-pointer, без ускорения) и кнопки уходят в относительную мышь
//! гостя (`usb-mouse`, активная на время захвата), сочетания (Alt+Tab, Win) — гостю.
//! Выход — Ctrl+Alt; при потере фокуса окна syngui снимает захват сам.
//!
//! Второй вид — «своя мышь гостя» (DOS: PS/2 или vmmouse драйвера SYNMOUSE.COM в относительном
//! режиме): отдельная мышь не подключается, смещения идут активной мыши. Нужен ли он, решает
//! [`watch_mice`]: активная мышь QEMU (`query-mice`, `current`) стала относительной — подсказка
//! и захват по щелчку, снова абсолютной — захват снимается сам.

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
    vm: Vm,
    st: crate::state::St,
    tx: Sender<Ev>,
    /// Блокировка указателя уже подтверждена (на Wayland — не сразу после запроса).
    locked_seen: bool,
    /// Своя мышь гостя (без `usb-mouse`): на выходе планшет не возвращается.
    own: bool,
}

/// Машины, у которых активная мышь QEMU сейчас относительная (по `query-mice`).
static REL: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Гостю нужна относительная мышь (захват по щелчку).
pub fn rel_wanted(id: &str) -> bool {
    REL.lock().unwrap().iter().any(|x| x == id)
}

/// Следить за активной мышью машины без планшета (DOS), пока открыт её экран (`alive`): относительная
/// — подсказать и захватывать по щелчку, абсолютная — снять свой захват. Опрос `query-mice` раз в 0,4 с.
pub fn watch_mice(st: crate::state::St, vm: Vm, alive: impl Fn() -> bool + Send + 'static) {
    static WATCHING: Mutex<Vec<String>> = Mutex::new(Vec::new());
    {
        let mut w = WATCHING.lock().unwrap();
        if w.contains(&vm.id) {
            return;
        }
        w.push(vm.id.clone());
    }
    std::thread::Builder::new()
        .name("syn-vm-mice".into())
        .spawn(move || {
            let mut last: Option<bool> = None;
            while alive() && crate::qemu::pid(&vm).is_some() {
                let rel = crate::qmp::call(&vm, "query-mice", serde_json::Value::Null)
                    .ok()
                    .and_then(|v| v.as_array().and_then(|a| a.iter().find(|m| m["current"] == true).map(|m| m["absolute"] == false)));
                if let Some(rel) = rel.filter(|r| last != Some(*r)) {
                    last = Some(rel);
                    {
                        let mut g = REL.lock().unwrap();
                        g.retain(|x| x != &vm.id);
                        if rel {
                            g.push(vm.id.clone());
                        }
                    }
                    log(&vm, if rel { "мышь гостя относительная — нужен захват" } else { "мышь гостя абсолютная" });
                    let id = vm.id.clone();
                    run_on_main_thread(move || {
                        if rel {
                            if !active(&id) {
                                crate::state::toast(st, t!("Игра просит захват мыши — щёлкните в окне (Ctrl+Alt — освободить)"));
                            }
                        } else if CAP.lock().unwrap().as_ref().is_some_and(|c| c.id == id && c.own) {
                            stop("мышь гостя снова абсолютная");
                            crate::state::toast(st, t!("Мышь снова обычная — захват снят"));
                        }
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            REL.lock().unwrap().retain(|x| x != &vm.id);
            WATCHING.lock().unwrap().retain(|x| x != &vm.id);
        })
        .ok();
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
        stop("указатель отпущен (окно потеряло фокус?)");
        return false;
    }
    true
}

/// Включить захват (в фоне: относительная мышь подключается около секунды): мышь гостя —
/// относительной, указатель — запереть. Итог и ошибки — уведомлением и в журнал.
pub fn start(st: crate::state::St, vm: &Vm) {
    begin(st, vm, false)
}

/// Захват на своей относительной мыши гостя (DOS: PS/2, vmmouse в относительном режиме).
pub fn start_own(st: crate::state::St, vm: &Vm) {
    begin(st, vm, true)
}

fn begin(st: crate::state::St, vm: &Vm, own: bool) {
    if CAP.lock().unwrap().as_ref().is_some_and(|c| c.id == vm.id) {
        log(vm, "уже включён — повторный вход пропущен");
        return;
    }
    stop("вход у другой машины");
    let v = vm.clone();
    if own {
        log(vm, "своя относительная мышь гостя, запираю указатель");
        lock(st, v, true);
        return;
    }
    std::thread::spawn(move || {
        log(&v, "подключаю относительную мышь");
        if let Err(e) = crate::qmp::rel_mouse_on(&v) {
            log(&v, &format!("мышь не подключилась: {e}"));
            let msg = t!("Не удалось подключить мышь для игр: {e}", e = e);
            run_on_main_thread(move || crate::state::toast_err(st, msg));
            return;
        }
        log(&v, "мышь активна, запираю указатель");
        run_on_main_thread(move || lock(st, v, false));
    });
}

fn lock(st: crate::state::St, vm: Vm, own: bool) {
    if let Err(e) = syngui::signal::set_pointer_lock(true) {
        log(&vm, &format!("указатель не заперся: {e}"));
        if !own {
            std::thread::spawn(move || crate::qmp::rel_mouse_off(&vm));
        }
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
            if own {
                log(&v, "выход");
            } else {
                log(&v, "выход, указатель — планшету");
                crate::qmp::rel_mouse_off(&v);
            }
        })
        .ok();
    let mtx = tx.clone();
    syngui::signal::on_raw_motion(Some(Box::new(move |dx, dy| {
        let _ = mtx.send(Ev::Motion(dx, dy));
    })));
    *CAP.lock().unwrap() = Some(Cap { id: vm.id.clone(), vm: vm.clone(), st, tx, locked_seen: false, own });
    log(&vm, "включён");
    // Панель над экраном — скрыть (перерисовка).
    st.frame_tick.update(|t| *t = t.wrapping_add(1));
    crate::state::toast(st, t!("Мышь захвачена игрой — Ctrl+Alt, чтобы освободить"));
}

/// Выключить захват: указатель и сочетания — системе, мышь гостя — снова планшет.
pub fn stop(reason: &str) {
    let Some(c) = CAP.lock().unwrap().take() else { return };
    log(&c.vm, &format!("выход: {reason}"));
    let st = c.st;
    run_on_main_thread(move || st.frame_tick.update(|t| *t = t.wrapping_add(1)));
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
