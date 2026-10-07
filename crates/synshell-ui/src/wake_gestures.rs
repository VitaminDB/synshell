//! Пробуждение жестами (телефон): двойной стук по погашенному экрану и
//! «поднять, чтобы разбудить» (`[mobile] double_tap_wake`, `raise_to_wake`).
//!
//! Двойной тап по экрану распознаёт сам тачскрин при погашенной панели и
//! присылает клавишу KEY_WAKEUP (композитор будит экран любым вводом) —
//! режим включает программа платформы `/usr/lib/syn-sensors/touch-gesture`
//! (тачскрины Xiaomi). Без неё — алгоритм `dbtap` DSP датчиков (стук по
//! корпусу). «Поднять» — алгоритм `pickup` SLPI: события строками от
//! `/usr/lib/syn-sensors/ssc-events` (arch-mobile-port, docs/15), только пока
//! экран погашен. Клиент SSC будящий — жест поднимает телефон из сна, а
//! `--inhibit` запрещает сон на пару секунд, пока экран включается.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use syngui::prelude::create_effect;
use synshell_common::ipc::{Client, Request};
use synshell_common::Action;

use crate::ctx::ShellCtx;

const PROGRAM: &str = "/usr/lib/syn-sensors/ssc-events";
const TOUCH: &str = "/usr/lib/syn-sensors/touch-gesture";

/// Последнее, что передано тачскрину (двойной тап вкл/выкл).
static TOUCH_STATE: Mutex<Option<bool>> = Mutex::new(None);

/// Запущенная программа и её набор жестов.
static RUNNING: Mutex<Option<(Vec<&'static str>, Child)>> = Mutex::new(None);

pub fn start(ctx: ShellCtx) {
    let touch = std::path::Path::new(TOUCH).exists();
    if !touch && !std::path::Path::new(PROGRAM).exists() {
        return;
    }
    create_effect(move || {
        let off = !ctx.screen_on.get();
        let cfg = ctx.config.get();
        let m = &cfg.mobile;
        if touch {
            set_touch(m.double_tap_wake);
        }
        let mut types = Vec::new();
        if m.double_tap_wake && !touch {
            types.push("dbtap");
        }
        if m.raise_to_wake {
            types.push("pickup");
        }
        set(if off { types } else { Vec::new() });
    });
}

/// Двойной тап тачскрина: включить или выключить (только при смене).
fn set_touch(on: bool) {
    let mut last = TOUCH_STATE.lock().unwrap();
    if *last == Some(on) {
        return;
    }
    match Command::new(TOUCH).args(["doubletap", if on { "1" } else { "0" }]).status() {
        Ok(st) if st.success() => {
            log::info!("двойной тап тачскрина: {}", if on { "включён" } else { "выключен" });
            *last = Some(on);
        }
        Ok(st) => log::warn!("{TOUCH}: {st}"),
        Err(e) => log::warn!("{TOUCH}: {e}"),
    }
}

fn set(types: Vec<&'static str>) {
    let mut running = RUNNING.lock().unwrap();
    if running.as_ref().map(|(t, _)| t) == Some(&types) {
        return;
    }
    // Закрытый stdin — программа сама закрывает датчики в SSC и выходит.
    if let Some((_, mut child)) = running.take() {
        drop(child.stdin.take());
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    if types.is_empty() {
        return;
    }
    let child = Command::new(PROGRAM)
        .args(["--inhibit", "wake-gesture"])
        .args(&types)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            log::warn!("жесты пробуждения: {PROGRAM}: {e}");
            return;
        }
    };
    if let Some(out) = child.stdout.take() {
        std::thread::Builder::new()
            .name("wake-gestures".into())
            .spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let kind = line.split_whitespace().next().unwrap_or_default();
                    if matches!(kind, "dbtap" | "pickup") {
                        log::info!("жест пробуждения: {line}");
                        // Прямо в композитор: главный поток оболочки мог ещё не проснуться.
                        if let Err(e) = Client::connect().and_then(|mut c| c.request(&Request::Action { action: Action::ScreenOn })) {
                            log::warn!("жест пробуждения: screen-on: {e}");
                        }
                    }
                }
            })
            .ok();
    }
    log::info!("жесты пробуждения: {types:?}");
    *running = Some((types, child));
}
