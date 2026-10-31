//! Запомненная яркость подсветки: `[brightness] level` и тепло передней
//! подсветки из двух каналов (белый + тёплый, E Ink) `[brightness] warmth`.
//!
//! systemd-backlight восстанавливать подсветку не всегда успевает: сохраняет
//! при остановке уже погашенное, после `reboot -f` не сохраняет вовсе, а
//! записанное при старом максимуме не принимает (EINVAL) — каналы остаются
//! на нуле. Поэтому оболочка помнит сама: последнюю ненулевую яркость,
//! выставленную пользователем (ползунок, клавиши, жесты), и тепло; при старте
//! сеанса возвращает их из конфига, при включении экрана — если подсветка
//! погашена. Подсветку пишут сразу, значения в конфиг — через 2 с после
//! последнего сдвига.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use syngui::prelude::create_effect;

use crate::ctx::ShellCtx;

/// Сохранить значение, когда его не трогали столько.
const SAVE_AFTER: Duration = Duration::from_secs(2);

/// Отложенная запись одного ключа `[brightness]`: последнее значение побеждает.
struct Delayed {
    key: &'static str,
    /// Ещё не записанное значение и время последнего сдвига.
    pending: Mutex<Option<(f64, Instant)>>,
    /// Поток записи жив.
    writer: Mutex<bool>,
}

static WARMTH: Delayed = Delayed { key: "warmth", pending: Mutex::new(None), writer: Mutex::new(false) };
static LEVEL: Delayed = Delayed { key: "level", pending: Mutex::new(None), writer: Mutex::new(false) };

impl Delayed {
    fn busy(&self) -> bool {
        self.pending.lock().unwrap().is_some()
    }

    fn save_later(&'static self, v: f64) {
        *self.pending.lock().unwrap() = Some((v, Instant::now()));
        let mut running = self.writer.lock().unwrap();
        if *running {
            return;
        }
        *running = true;
        std::thread::spawn(move || loop {
            let Some((v, at)) = *self.pending.lock().unwrap() else {
                *self.writer.lock().unwrap() = false;
                return;
            };
            let left = SAVE_AFTER.saturating_sub(at.elapsed());
            if !left.is_zero() {
                std::thread::sleep(left);
                continue;
            }
            let mut p = self.pending.lock().unwrap();
            if p.is_some_and(|(_, t)| t == at) {
                *p = None;
                *self.writer.lock().unwrap() = false;
                drop(p);
                let key = self.key;
                syngui::async_runtime::run_on_main_thread(move || {
                    if let Err(e) = synshell_common::config_edit::set_value(&["brightness", key], toml_edit::Value::from(v)) {
                        log::error!("подсветка: [brightness] {key}: {e:#}");
                    }
                    crate::reload_after_write();
                });
                return;
            }
        });
    }
}

/// Вернуть подсветке запомненное (в фоне). `only_if_dark` — только погашенную.
fn restore(level: Option<f32>, warmth: Option<f32>, only_if_dark: bool) {
    std::thread::spawn(move || match synsystem::backlight::restore(&synsystem::Sys::host(), level, warmth, only_if_dark) {
        Ok(true) => log::info!("подсветка: восстановлена яркость {level:?}, тепло {warmth:?}"),
        Ok(false) => {}
        Err(e) => log::warn!("подсветка: {e}"),
    });
}

pub fn start(ctx: ShellCtx) {
    let cfg = ctx.cfg();
    let b = &cfg.brightness;
    restore(b.level, b.warmth, false);
    // последнее применённое тепло конфига: перечитывание по другим поводам каналы не трогает
    let applied: Arc<Mutex<Option<f32>>> = Arc::new(Mutex::new(b.warmth));
    crate::on_reload(move |ctx, _| {
        // пока свой сдвиг не сохранён, конфиг ещё старый — не откатывать
        if WARMTH.busy() {
            return;
        }
        let w = ctx.cfg().brightness.warmth;
        let mut a = applied.lock().unwrap();
        if w.is_none() || *a == w {
            return;
        }
        *a = w;
        // тепло, сменённое в Параметрах, — при прежней яркости
        let w = w.unwrap().clamp(0.0, 100.0);
        std::thread::spawn(move || {
            let sys = synsystem::Sys::host();
            let Some((b, cur)) = synsystem::backlight::front_light_state(&sys) else { return };
            // каналы погашены — яркость не включаем, тепло применится со следующей
            if b <= 0.0 || (cur - w).abs() < 0.5 {
                return;
            }
            if let Err(e) = synsystem::backlight::set_front_light(&sys, b, w) {
                log::warn!("тепло подсветки: {e}");
            }
        });
    });
    // Экран включили, а подсветка погашена (её погасили вместе с панелью или во сне) — вернуть запомненное.
    let was_on = std::cell::Cell::new(true);
    create_effect(move || {
        let on = ctx.screen_on.get();
        let was = was_on.replace(on);
        if on && !was {
            let cfg = ctx.cfg();
            restore(cfg.brightness.level, cfg.brightness.warmth, true);
        }
    });
}

/// Пользователь выставил яркость (проценты): запомнить в `[brightness] level`.
/// Ноль не запоминается — это «погасить», а не уровень.
pub fn remember_level(pct: f32) {
    if pct > 0.0 {
        LEVEL.save_later((pct.min(100.0) as f64 * 10.0).round() / 10.0);
    }
}

/// Пишет каналы тепла в фоне: последнее значение побеждает.
static WARMTH_SYSFS: Mutex<Option<f32>> = Mutex::new(None);

/// Сдвиг ползунка «Тепло»: каналы — сразу (в фоне), конфиг — позже.
pub fn set_warmth(w: f32) {
    WARMTH.save_later((w as f64).round());
    if WARMTH_SYSFS.lock().unwrap().replace(w).is_some() {
        return;
    }
    std::thread::spawn(|| {
        let sys = synsystem::Sys::host();
        loop {
            // значение снимается под замком после записи: новый сдвиг за это время подхватит этот же поток
            let Some(w) = *WARMTH_SYSFS.lock().unwrap() else { return };
            if let Err(e) = synsystem::backlight::set_warmth(&sys, w) {
                log::warn!("тепло подсветки: {e}");
            }
            let mut p = WARMTH_SYSFS.lock().unwrap();
            if *p == Some(w) {
                *p = None;
                return;
            }
        }
    });
}
