//! Пробуждение жестами (телефон): двойной стук по погашенному экрану и
//! «поднять, чтобы разбудить» (`[mobile] double_tap_wake`, `raise_to_wake`).
//!
//! Жесты распознаёт DSP датчиков (SLPI, алгоритмы `dbtap` и `pickup`);
//! события приходят строками от `/usr/lib/syn-sensors/ssc-events`
//! (arch-mobile-port, docs/15). Программа работает только пока экран
//! погашен: датчики опрашиваются по необходимости, а стук по включённому
//! экрану ничего не делает. Клиент SSC будящий — жест поднимает телефон из
//! сна, а `--inhibit` запрещает сон на пару секунд, пока экран включается.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use syngui::prelude::create_effect;
use synshell_common::ipc::{Client, Request};
use synshell_common::Action;

use crate::ctx::ShellCtx;

const PROGRAM: &str = "/usr/lib/syn-sensors/ssc-events";

/// Запущенная программа и её набор жестов.
static RUNNING: Mutex<Option<(Vec<&'static str>, Child)>> = Mutex::new(None);

pub fn start(ctx: ShellCtx) {
    if !std::path::Path::new(PROGRAM).exists() {
        return;
    }
    create_effect(move || {
        let off = !ctx.screen_on.get();
        let cfg = ctx.config.get();
        let m = &cfg.mobile;
        let mut types = Vec::new();
        if m.double_tap_wake {
            types.push("dbtap");
        }
        if m.raise_to_wake {
            types.push("pickup");
        }
        set(if off { types } else { Vec::new() });
    });
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
