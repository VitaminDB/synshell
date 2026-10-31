//! Тепло передней подсветки из двух каналов (белый + тёплый, E Ink):
//! `[brightness] warmth`. Каналы после загрузки восстанавливает
//! systemd-backlight (каждый сам по себе, с его нижней границей); при старте
//! сеанса и при правке конфига оболочка пересмешивает их: яркость — от
//! восстановленных, тепло — из конфига. Ползунок «Тепло» пишет каналы сразу,
//! а значение в конфиг — через 2 с после последнего сдвига.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ctx::ShellCtx;

/// Сохранить тепло, когда ползунок не трогали столько.
const SAVE_AFTER: Duration = Duration::from_secs(2);

/// Ещё не записанное тепло и время последнего сдвига.
static PENDING: Mutex<Option<(f32, Instant)>> = Mutex::new(None);
/// Поток записи жив (последнее значение побеждает).
static WRITER: Mutex<bool> = Mutex::new(false);

fn apply(w: f32) {
    let sys = synsystem::Sys::host();
    let Some(fl) = synsystem::backlight::front_light(&sys) else { return };
    let (b, cur) = fl.state(&sys);
    // каналы погашены — яркость не включаем, тепло применится со следующей
    if b <= 0.0 || (cur - w).abs() < 0.5 {
        return;
    }
    if let Err(e) = synsystem::backlight::set_front_light(&sys, b, w) {
        log::warn!("тепло подсветки: {e}");
    }
}

pub fn start(ctx: ShellCtx) {
    // последнее применённое значение конфига: перечитывание по другим поводам каналы не трогает
    let applied: Arc<Mutex<Option<f32>>> = Arc::new(Mutex::new(None));
    let on_cfg = move |w: Option<f32>| {
        let mut a = applied.lock().unwrap();
        if w.is_none() || *a == w {
            return;
        }
        *a = w;
        let w = w.unwrap().clamp(0.0, 100.0);
        std::thread::spawn(move || apply(w));
    };
    on_cfg(ctx.cfg().brightness.warmth);
    crate::on_reload(move |ctx, _| {
        // пока свой сдвиг не сохранён, конфиг ещё старый — не откатывать
        if PENDING.lock().unwrap().is_some() {
            return;
        }
        on_cfg(ctx.cfg().brightness.warmth)
    });
}

/// Сдвиг ползунка «Тепло»: каналы — сразу (в фоне), конфиг — позже.
pub fn set(w: f32) {
    *PENDING.lock().unwrap() = Some((w, Instant::now()));
    let mut running = WRITER.lock().unwrap();
    if *running {
        return;
    }
    *running = true;
    std::thread::spawn(|| {
        let sys = synsystem::Sys::host();
        let mut written: Option<f32> = None;
        loop {
            let Some((w, at)) = *PENDING.lock().unwrap() else { break };
            if written != Some(w) {
                if let Err(e) = synsystem::backlight::set_warmth(&sys, w) {
                    log::warn!("тепло подсветки: {e}");
                }
                written = Some(w);
            }
            if at.elapsed() >= SAVE_AFTER {
                let mut p = PENDING.lock().unwrap();
                if p.is_some_and(|(_, t)| t == at) {
                    *p = None;
                    *WRITER.lock().unwrap() = false;
                    drop(p);
                    let v = (w as f64).round();
                    syngui::async_runtime::run_on_main_thread(move || {
                        if let Err(e) = synshell_common::config_edit::set_value(&["brightness", "warmth"], toml_edit::Value::from(v)) {
                            log::error!("тепло подсветки: {e:#}");
                        }
                        crate::reload_after_write();
                    });
                    return;
                }
                continue;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        *WRITER.lock().unwrap() = false;
    });
}
