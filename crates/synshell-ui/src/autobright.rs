//! Автояркость (`[brightness] auto`, как адаптивная яркость Android).
//!
//! Освещённость даёт iio-sensor-proxy (D-Bus `net.hadess.SensorProxy`,
//! `ClaimLight`, свойство `LightLevel` в люксах). Кривая «люксы → проценты» —
//! в логарифмической шкале; сдвиг ползунка при включённой автояркости
//! запоминается как поправка к кривой (`[brightness] offset`, пишется в
//! конфиг через 2 с после последнего сдвига). Переход
//! плавный: светлее — за `smooth_ms`, темнее — вдвое медленнее и только если
//! стало заметно темнее и держится 1,5 с (мигание тени не гасит экран).

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use synshell_common::config::Brightness;

use crate::ctx::ShellCtx;

/// Опорные точки кривой: люксы → доля диапазона `min_pct..max_pct`.
/// Внизу круче, чем у Android: в комнатном полумраке (50–150 лк) экран заметно темнее, чем при дневном свете.
const CURVE: [(f32, f32); 9] = [
    (0.0, 0.0),
    (3.0, 0.02),
    (15.0, 0.06),
    (60.0, 0.13),
    (150.0, 0.22),
    (400.0, 0.38),
    (1000.0, 0.56),
    (3000.0, 0.80),
    (10000.0, 1.0),
];
/// Шаг анимации.
const TICK: Duration = Duration::from_millis(40);
/// Меньшие расхождения с целью не трогают яркость (проценты).
const DEADBAND: f32 = 2.0;
/// Темнее — только если цель ниже текущей на столько процентов и держится столько.
const DIM_HYST: f32 = 5.0;
const DIM_HOLD: Duration = Duration::from_millis(1500);
/// Поправку сохранить, когда ползунок не трогали столько.
const SAVE_AFTER: Duration = Duration::from_secs(2);

fn curve(lux: f32) -> f32 {
    let x = (lux.max(0.0) + 1.0).log10();
    let pts = CURVE.map(|(l, v)| ((l + 1.0).log10(), v));
    if x <= pts[0].0 {
        return pts[0].1;
    }
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    1.0
}

/// Включить или выключить автояркость (плитка шторки, кнопка «А»).
pub fn set_auto(on: bool) {
    if let Err(e) = synshell_common::config_edit::set_value(&["brightness", "auto"], toml_edit::Value::from(on)) {
        log::error!("автояркость: {e:#}");
    }
    crate::reload_after_write();
}

fn target(cfg: &Brightness, lux: f32, offset: f32) -> f32 {
    let (lo, hi) = (cfg.min_pct.clamp(1, 100) as f32, cfg.max_pct.clamp(1, 100) as f32);
    (lo + (hi - lo).max(0.0) * curve(lux) + offset).clamp(lo.min(hi), hi.max(lo))
}

enum Msg {
    Lux(f32),
}

pub fn start(ctx: ShellCtx) {
    let cfg = Arc::new(Mutex::new(ctx.cfg().brightness.clone()));
    let c2 = cfg.clone();
    crate::on_reload(move |ctx, _| *c2.lock().unwrap() = ctx.cfg().brightness.clone());
    let (tx, rx) = channel();
    let c3 = cfg.clone();
    std::thread::Builder::new()
        .name("autobright".into())
        .spawn(move || {
            let mut logged = false;
            loop {
                // Датчик занимаем, только пока автояркость включена.
                while !c3.lock().unwrap().auto {
                    std::thread::sleep(Duration::from_secs(2));
                }
                if let Err(e) = listen(&tx, &c3) {
                    if !logged {
                        log::info!("автояркость: датчика освещённости нет ({e})");
                        logged = true;
                    }
                    std::thread::sleep(Duration::from_secs(30));
                }
            }
        })
        .ok();
    std::thread::Builder::new().name("autobright-apply".into()).spawn(move || apply_loop(rx, cfg)).ok();
}

fn listen(tx: &std::sync::mpsc::Sender<Msg>, cfg: &Arc<Mutex<Brightness>>) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::system()?;
    let proxy = zbus::blocking::Proxy::new(&conn, "net.hadess.SensorProxy", "/net/hadess/SensorProxy", "net.hadess.SensorProxy")?;
    if !proxy.get_property::<bool>("HasAmbientLight")? {
        return Err(zbus::Error::Failure("нет датчика освещённости".into()));
    }
    proxy.call_method("ClaimLight", &())?;
    log::info!("автояркость: датчик освещённости подключён");
    let changes = proxy.receive_property_changed::<f64>("LightLevel");
    let _ = tx.send(Msg::Lux(proxy.get_property::<f64>("LightLevel")? as f32));
    for c in changes {
        if !cfg.lock().unwrap().auto {
            // выключили — отдать датчик (выход из потока свойств закроет соединение)
            let _ = proxy.call_method("ReleaseLight", &());
            return Ok(());
        }
        if let Ok(v) = c.get() {
            let _ = tx.send(Msg::Lux(v as f32));
        }
    }
    Err(zbus::Error::Failure("поток свойств закрыт".into()))
}

fn apply_loop(rx: Receiver<Msg>, cfg: Arc<Mutex<Brightness>>) {
    let sys = synsystem::Sys::host();
    let mut lux: Option<f32> = None;
    let mut offset = cfg.lock().unwrap().offset;
    // Поправка из конфига, которую видели последней (сменилась в файле — принять), и несохранённая своя.
    let mut cfg_offset = offset;
    let mut unsaved: Option<Instant> = None;
    let mut was_auto = false;
    // Последний код, который записали сами: другое значение в sysfs — пользователь подвинул ползунок.
    let mut written: Option<u32> = None;
    let mut cur: Option<f32> = None;
    let mut dim_since: Option<Instant> = None;
    loop {
        match rx.recv_timeout(TICK) {
            Ok(Msg::Lux(l)) => lux = Some(l),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let c = cfg.lock().unwrap().clone();
        if (c.offset - cfg_offset).abs() > f32::EPSILON {
            (offset, cfg_offset) = (c.offset, c.offset);
        }
        if unsaved.is_some_and(|t| t.elapsed() >= SAVE_AFTER) {
            unsaved = None;
            let v = (offset * 10.0).round() / 10.0;
            cfg_offset = v;
            syngui::async_runtime::run_on_main_thread(move || {
                if let Err(e) = synshell_common::config_edit::set_value(&["brightness", "offset"], toml_edit::Value::from(v as f64)) {
                    log::error!("автояркость: {e:#}");
                }
                crate::reload_after_write();
            });
        }
        if !c.auto {
            if was_auto {
                (written, cur, lux, was_auto) = (None, None, None, false);
            }
            continue;
        }
        was_auto = true;
        let (Some(l), Some(bl)) = (lux, synsystem::backlight::primary(&sys)) else { continue };
        let now_pct = bl.percent();
        if written.is_some_and(|w| w.abs_diff(bl.brightness) > 1) {
            // ручная правка: запомнить поправку к кривой
            offset = (now_pct - target(&c, l, 0.0)).clamp(-50.0, 50.0);
            unsaved = Some(Instant::now());
            log::debug!("автояркость: поправка {offset:+.0}% при {l:.0} лк");
            written = Some(bl.brightness);
            cur = Some(now_pct);
            continue;
        }
        let goal = target(&c, l, offset);
        let pos = cur.unwrap_or(now_pct);
        let diff = goal - pos;
        if diff.abs() < DEADBAND && cur.is_none_or(|p| (p - goal).abs() < 0.5) {
            dim_since = None;
            continue;
        }
        if diff < 0.0 {
            // темнее — с гистерезисом и выдержкой, если переход ещё не начат
            if cur.is_none_or(|p| (p - now_pct).abs() < 0.5) && -diff < DIM_HYST {
                continue;
            }
            let since = *dim_since.get_or_insert_with(Instant::now);
            if since.elapsed() < DIM_HOLD {
                continue;
            }
        } else {
            dim_since = None;
        }
        let dur = c.smooth_ms.max(1) as f32 * if diff < 0.0 { 2.0 } else { 1.0 };
        // экспоненциальное приближение: за `dur` проходит ~95 % пути
        let k = 1.0 - (-3.0 * TICK.as_millis() as f32 / dur).exp();
        let mut next = pos + diff * k;
        if (goal - next).abs() < 0.3 {
            next = goal;
            dim_since = None;
        }
        match synsystem::backlight::set_percent(&sys, next) {
            Ok(b) => {
                written = Some(b.brightness);
                cur = Some(next);
                syngui::async_runtime::run_on_main_thread(move || ShellCtx::get().brightness.set(Some(next)));
            }
            Err(e) => {
                log::warn!("автояркость: {e}");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_monotonic() {
        let mut prev = -1.0;
        for l in [0.0, 1.0, 10.0, 50.0, 200.0, 542.0, 2000.0, 9000.0, 50000.0] {
            let v = curve(l);
            assert!(v >= prev && (0.0..=1.0).contains(&v), "{l} → {v}");
            prev = v;
        }
        let c = Brightness::default();
        assert!((target(&c, 0.0, 0.0) - 2.0).abs() < 0.01);
        assert!((target(&c, 100000.0, 0.0) - 100.0).abs() < 0.01);
    }
}
