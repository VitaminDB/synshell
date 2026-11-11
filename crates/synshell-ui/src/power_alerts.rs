//! Оповещения о питании (`[power_alerts]`, «Параметры → Питание → Оповещения»): устройство не должно пропадать
//! молча.
//!
//! - Разряд: «Низкий заряд» (`low`), срочное «Критический заряд» (`critical`, видно и в «Не беспокоить»),
//!   у самого нуля (`action_at`) — отсчёт `action_delay` с кнопками «Отменить» / «Сейчас» и чистое выключение
//!   (или сон): так не портятся файлы и понятно, почему устройство погасло. Экран при этом включается.
//!   Зарядка снимает всё.
//! - Сон по простою (`[idle] suspend_after`): предупреждение за `sleep_warning` с, кнопка «Не засыпать».
//! - Неожиданное выключение: при входе после загрузки, если прошлый запуск системы кончился без записи
//!   `shutdown` в wtmp (разряд в ноль, зависание, сбой питания, удержание кнопки), — когда это было и какой был
//!   заряд (отметку оболочка пишет в `$XDG_STATE_HOME/synshell/power-last`).
//! - Сон телефона: служба платформы (synmobile `syn-sleepd`) во сне заряд проверяет сама — порог и действие
//!   оболочка пишет ей в `/run/syn-sleep/battery-floor`; выключение из сна она отмечает в
//!   `/var/lib/syn-sleep/last-poweroff`, оболочка скажет о нём после загрузки.
//!
//! Уведомления оболочки synlink пересылает соединённым устройствам — компьютер видит «Низкий заряд» телефона.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use syngui::prelude::create_effect;
use syngui::t;

use crate::ctx::ShellCtx;
use crate::notifications;

const KEY_BAT: &str = "power-battery";
const KEY_ACT: &str = "power-action";
const KEY_SLEEP: &str = "power-sleep";
const KEY_LAST: &str = "power-last-boot";

const FLOOR_FILE: &str = "/run/syn-sleep/battery-floor";
const SLEEP_POWEROFF: &str = "/var/lib/syn-sleep/last-poweroff";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Level {
    None,
    Low,
    Critical,
    Action,
}

thread_local! {
    static LEVEL: Cell<Level> = const { Cell::new(Level::None) };
    /// Таймер отсчёта перед выключением.
    static COUNTDOWN: Cell<Option<u64>> = const { Cell::new(None) };
    static LAST_SAVED: Cell<Option<(u32, bool)>> = const { Cell::new(None) };
}

/// Для потока простоя: (suspend_after, sleep_warning); 0 — не следить.
static IDLE_CFG: Mutex<(u32, u32)> = Mutex::new((0, 0));

pub fn start(ctx: ShellCtx) {
    std::thread::Builder::new().name("power-last-boot".into()).spawn(report_last_boot).ok();
    // Порог для службы сна и настройки потока простоя — при каждой смене конфига.
    let floor = Cell::new(String::new());
    create_effect(move || {
        let cfg = ctx.config.get();
        let a = &cfg.power_alerts;
        let line = format!("{} {}\n", if a.action == "none" { 0 } else { a.action_at }, a.action);
        if floor.replace(line.clone()) != line {
            write_floor(&line);
        }
        let suspend = if ctx.is_phone() { 0 } else { cfg.idle.suspend_after };
        *IDLE_CFG.lock().unwrap() = (suspend, a.sleep_warning);
    });
    // Отладка: `SYNSHELL_DEMO_BATTERY=ФАЙЛ` — заряд из файла «процент [charging]» (раз в 2 с), а не от драйвера.
    let demo = std::env::var_os("SYNSHELL_DEMO_BATTERY").map(PathBuf::from);
    let fake: syngui::prelude::RwSignal<Option<(u32, bool)>> = syngui::prelude::use_signal(None);
    if let Some(path) = demo.clone() {
        syngui_layer::add_timer(Duration::from_millis(500), move || {
            let v = std::fs::read_to_string(&path).ok().and_then(|s| {
                let mut it = s.split_whitespace();
                Some((it.next()?.parse().ok()?, it.next() == Some("charging")))
            });
            if fake.get_untracked() != v {
                fake.set(v);
            }
            Some(Duration::from_secs(2))
        });
    }
    let is_demo = demo.is_some();
    create_effect(move || {
        let (b, ext) = if is_demo {
            let f = fake.get();
            (f.map(|(percent, charging)| crate::system::Battery { percent, charging, full: false, minutes: Some(percent * 6), power_w: None }), f.map(|f| f.1))
        } else {
            (ctx.battery.get(), ctx.external_power.get())
        };
        let cfg = ctx.config.get();
        let Some(b) = b else { return };
        if LAST_SAVED.get() != Some((b.percent, b.charging)) {
            LAST_SAVED.set(Some((b.percent, b.charging)));
            save_state(Some(&b), "");
        }
        let a = &cfg.power_alerts;
        let on_battery = !b.charging && !b.full && ext != Some(true);
        if !on_battery {
            if LEVEL.replace(Level::None) != Level::None {
                cancel_countdown();
                notifications::close_local(ctx, KEY_BAT);
                notifications::close_local(ctx, KEY_ACT);
            }
            return;
        }
        let p = b.percent;
        let level = if a.action_at > 0 && p <= a.action_at && a.action != "none" {
            Level::Action
        } else if a.critical > 0 && p <= a.critical {
            Level::Critical
        } else if a.low > 0 && p <= a.low {
            Level::Low
        } else {
            Level::None
        };
        // Только вниз: дрожание датчика (20 → 21 → 20) не повторяет оповещение; сброс — зарядкой.
        if level > LEVEL.get() {
            LEVEL.set(level);
            alert(ctx, level, &b, a);
        }
    });
    // Отметка «жив» для отчёта о неожиданном выключении.
    syngui_layer::add_timer(Duration::from_secs(120), move || {
        save_state(ctx.battery.get_untracked().as_ref(), "");
        Some(Duration::from_secs(120))
    });
    std::thread::Builder::new().name("power-idle".into()).spawn(idle_watch).ok();
}

fn remaining(b: &crate::system::Battery) -> String {
    match b.minutes {
        Some(m) if m >= 60 => t!("Осталось около {h} ч {m} мин.", h = m / 60, m = m % 60),
        Some(m) if m > 0 => t!("Осталось около {m} мин.", m = m),
        _ => String::new(),
    }
}

fn alert(ctx: ShellCtx, level: Level, b: &crate::system::Battery, a: &synshell_common::config::PowerAlerts) {
    let p = b.percent;
    let left = remaining(b);
    log::info!("питание: заряд {p} %, оповещение {level:?}");
    match level {
        Level::None => {}
        Level::Low => {
            let body = format!("{left} {}", t!("Подключите зарядку.")).trim().to_string();
            notifications::local_alert(ctx, KEY_BAT, &t!("Низкий заряд: {p} %", p = p), &body, "battery-low", 1, &[], |_| {});
        }
        Level::Critical => {
            let then = match a.action.as_str() {
                "poweroff" if a.action_at > 0 => t!("При {n} % устройство выключится само.", n = a.action_at),
                "suspend" if a.action_at > 0 => t!("При {n} % устройство уснёт.", n = a.action_at),
                _ => String::new(),
            };
            let body = format!("{left} {} {then}", t!("Подключите зарядку.")).split_whitespace().collect::<Vec<_>>().join(" ");
            notifications::local_alert(ctx, KEY_BAT, &t!("Критический заряд: {p} %", p = p), &body, "battery-caution", 2, &[], |_| {});
            crate::sounds::play(crate::sounds::Ev::BatteryLow);
        }
        Level::Action => countdown(ctx, p, a),
    }
}

/// Отсчёт перед выключением (сном) у нуля.
fn countdown(ctx: ShellCtx, p: u32, a: &synshell_common::config::PowerAlerts) {
    cancel_countdown();
    // Отсчёт заменяет «Критический заряд» — одна карточка, а не две.
    notifications::close_local(ctx, KEY_BAT);
    let off = a.action != "suspend";
    let delay = a.action_delay.max(5);
    let summary = if off { t!("Заряд {p} % — выключение через {s} с", p = p, s = delay) } else { t!("Заряд {p} % — сон через {s} с", p = p, s = delay) };
    let now_label = if off { t!("Выключить сейчас") } else { t!("Уснуть сейчас") };
    let cancel_label = t!("Отменить");
    notifications::local_alert(
        ctx,
        KEY_ACT,
        &summary,
        &t!("Подключите зарядку — отсчёт остановится."),
        "battery-empty",
        2,
        &[("cancel", cancel_label.as_str()), ("now", now_label.as_str())],
        move |k| match k {
            "now" => act(off, p),
            "cancel" => {
                log::info!("питание: выключение при разряде отменено");
                cancel_countdown();
            }
            _ => {}
        },
    );
    crate::sounds::play(crate::sounds::Ev::BatteryLow);
    // Увидеть отсчёт (и успеть отменить) можно только на горящем экране.
    if !ctx.screen_on.get_untracked() {
        send_action(synshell_common::Action::ScreenOn);
    }
    let id = syngui_layer::add_timer(Duration::from_secs(delay as u64), move || {
        COUNTDOWN.set(None);
        let ctx = ShellCtx::get();
        // Пока шёл отсчёт, подключили зарядку — эффект уже всё снял, но проверить ещё раз.
        let demo = std::env::var_os("SYNSHELL_DEMO_BATTERY").is_some();
        let charging = !demo && (ctx.battery.get_untracked().is_none_or(|b| b.charging || b.full) || ctx.external_power.get_untracked() == Some(true));
        if !charging {
            act(off, p);
        }
        None
    });
    COUNTDOWN.set(Some(id));
}

fn cancel_countdown() {
    if let Some(id) = COUNTDOWN.take() {
        syngui_layer::cancel_timer(id);
    }
}

fn act(off: bool, p: u32) {
    cancel_countdown();
    log::warn!("питание: заряд {p} % — {}", if off { "выключаю" } else { "усыпляю" });
    if std::env::var_os("SYNSHELL_DEMO_BATTERY").is_some() {
        log::warn!("питание: демо-заряд — действие не выполняется");
        notifications::local_alert(ShellCtx::get(), KEY_ACT, "demo: poweroff", "", "battery-empty", 1, &[], |_| {});
        return;
    }
    if off {
        save_state(ShellCtx::get().battery.get_untracked().as_ref(), "battery");
        send_action(synshell_common::Action::PowerOff);
    } else {
        send_action(synshell_common::Action::Suspend);
    }
}

fn send_action(action: synshell_common::Action) {
    std::thread::spawn(move || {
        use synshell_common::ipc::{Client, Request};
        if let Err(e) = Client::connect().and_then(|mut c| c.request(&Request::Action { action })) {
            log::warn!("питание: действие: {e}");
        }
    });
}

fn write_floor(line: &str) {
    let dir = std::path::Path::new(FLOOR_FILE).parent().unwrap();
    if !dir.is_dir() {
        return;
    }
    let tmp = format!("{FLOOR_FILE}.{}", std::process::id());
    if let Err(e) = std::fs::write(&tmp, line).and_then(|_| std::fs::rename(&tmp, FLOOR_FILE)) {
        log::debug!("питание: {FLOOR_FILE}: {e}");
        let _ = std::fs::remove_file(&tmp);
    }
}

// ─── Сон по простою ─────────────────────────────────────────────────────────

fn idle_watch() {
    use synshell_common::ipc::{Client, Request, Response};
    let mut shown = false;
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let (suspend, warn) = *IDLE_CFG.lock().unwrap();
        if suspend == 0 || warn == 0 {
            if shown {
                shown = false;
                syngui::async_runtime::run_on_main_thread(|| notifications::close_local(ShellCtx::get(), KEY_SLEEP));
            }
            continue;
        }
        let Ok(Response::Idle { idle }) = Client::connect().and_then(|mut c| c.request(&Request::Idle)) else { continue };
        let left = suspend as i64 - idle.seconds as i64;
        let due = !idle.inhibited && left > 0 && left <= warn as i64;
        if due && !shown {
            shown = true;
            syngui::async_runtime::run_on_main_thread(move || {
                let stay = t!("Не засыпать");
                notifications::local_alert(
                    ShellCtx::get(),
                    KEY_SLEEP,
                    &t!("Сон через {s} с", s = left),
                    &t!("Нет действий — устройство уснёт и пропадёт из сети."),
                    "system-suspend",
                    1,
                    &[("stay", stay.as_str())],
                    |k| {
                        if k == "stay" {
                            send_action(synshell_common::Action::ScreenOn);
                        }
                    },
                );
            });
        } else if !due && shown {
            shown = false;
            syngui::async_runtime::run_on_main_thread(|| notifications::close_local(ShellCtx::get(), KEY_SLEEP));
        }
    }
}

// ─── Отметка и отчёт о неожиданном выключении ───────────────────────────────

#[derive(Default, Debug)]
struct Mark {
    boot: String,
    time: i64,
    percent: Option<u32>,
    charging: bool,
    reason: String,
}

fn state_file() -> PathBuf {
    synshell_common::paths::state_home().join("synshell/power-last")
}

fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default().trim().to_string()
}

fn load_mark() -> Option<Mark> {
    let s = std::fs::read_to_string(state_file()).ok()?;
    let mut m = Mark::default();
    for l in s.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        match k {
            "boot" => m.boot = v.into(),
            "time" => m.time = v.parse().unwrap_or(0),
            "percent" => m.percent = v.parse().ok(),
            "charging" => m.charging = v == "1",
            "reason" => m.reason = v.into(),
            _ => {}
        }
    }
    Some(m)
}

fn save_state(b: Option<&crate::system::Battery>, reason: &str) {
    let path = state_file();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let mut s = format!("boot={}\ntime={}\n", boot_id(), crate::clock::unix_now());
    if let Some(b) = b {
        s += &format!("percent={}\ncharging={}\n", b.percent, if b.charging || b.full { 1 } else { 0 });
    }
    if !reason.is_empty() {
        s += &format!("reason={reason}\n");
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, s).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Прошлый запуск системы по wtmp: (время загрузки, кончился ли записью `shutdown`). `None` — не узнать.
fn wtmp_previous() -> Option<(i64, bool)> {
    wtmp_parse(&std::fs::read("/var/log/wtmp").ok()?)
}

/// Записи `struct utmp` glibc (384 байта): тип, пользователь (`reboot`, `shutdown`), время.
fn wtmp_parse(d: &[u8]) -> Option<(i64, bool)> {
    const REC: usize = 384;
    let user = |r: &[u8]| -> String {
        let u = &r[44..76];
        String::from_utf8_lossy(&u[..u.iter().position(|&c| c == 0).unwrap_or(u.len())]).into_owned()
    };
    // (тип, пользователь, время)
    let recs: Vec<(i16, String, i64)> =
        d.chunks_exact(REC).map(|r| (i16::from_ne_bytes([r[0], r[1]]), user(r), i32::from_ne_bytes([r[340], r[341], r[342], r[343]]) as i64)).collect();
    const RUN_LVL: i16 = 1;
    const BOOT_TIME: i16 = 2;
    let boots: Vec<usize> = recs.iter().enumerate().filter(|(_, r)| r.0 == BOOT_TIME).map(|(i, _)| i).collect();
    let (&prev, &cur) = (boots.get(boots.len().checked_sub(2)?)?, boots.last()?);
    let clean = recs[prev..cur].iter().any(|r| r.0 == RUN_LVL && r.1 == "shutdown");
    Some((recs[prev].2, clean))
}

fn when(ts: i64) -> String {
    let now = crate::clock::unix_now();
    if crate::clock::format(ts, "%F") == crate::clock::format(now, "%F") {
        crate::clock::format(ts, "%H:%M")
    } else {
        crate::clock::format(ts, "%d %B, %H:%M")
    }
}

/// После загрузки: почему прошлый раз устройство погасло (один раз на загрузку).
fn report_last_boot() {
    let boot = boot_id();
    let mark = load_mark();
    if mark.as_ref().is_some_and(|m| m.boot == boot) {
        return; // перезапуск оболочки в той же загрузке
    }
    // Отметки ещё не было (первый запуск с оповещениями) — о давних выключениях не говорить.
    if mark.is_none() {
        save_state(syn_system::battery::read(&syn_system::Sys::host()).as_ref(), "");
        return;
    }
    let prev = wtmp_previous();
    // Отметка из прошлой загрузки (не из позапрошлой, если оболочка тогда не запускалась).
    let mark = mark.filter(|m| prev.is_none_or(|(t, _)| m.time >= t));
    let sleep_off = std::fs::read_to_string(SLEEP_POWEROFF).ok().and_then(|s| {
        let mut it = s.split_whitespace();
        (it.next()? == "battery").then_some(())?;
        let p: u32 = it.next()?.parse().ok()?;
        let t: i64 = it.next()?.parse().ok()?;
        prev.is_none_or(|(b, _)| t >= b).then_some((p, t))
    });
    let msg: Option<(String, String)> = if let Some(m) = mark.as_ref().filter(|m| m.reason == "battery") {
        Some((
            t!("Выключено из-за разряда"),
            t!("Заряд {p} % ({time}) — устройство выключилось само, чтобы не потерять данные.", time = when(m.time), p = m.percent.unwrap_or(0)),
        ))
    } else if let Some((p, t)) = sleep_off {
        Some((t!("Выключено из-за разряда"), t!("Во сне заряд упал до {p} % ({time}) — устройство выключилось само.", time = when(t), p = p)))
    } else if prev.is_some_and(|(_, clean)| !clean) {
        let body = match mark.as_ref() {
            Some(Mark { time, percent: Some(p), charging: false, .. }) if *p <= 5 => {
                t!("Последняя отметка — {time}, заряд {p} %: аккумулятор разрядился до нуля.", time = when(*time), p = p)
            }
            Some(Mark { time, percent: Some(p), .. }) => t!(
                "Последняя отметка — {time}, заряд {p} %. Причина — зависание, сбой питания или долгое нажатие кнопки питания.",
                time = when(*time),
                p = p
            ),
            Some(m) => t!("Последняя отметка — {time}. Причина — разряд, зависание, сбой питания или долгое нажатие кнопки питания.", time = when(m.time)),
            None => t!("Система не завершила работу. Причина — разряд, зависание, сбой питания или долгое нажатие кнопки питания."),
        };
        Some((t!("Устройство выключилось неожиданно"), body))
    } else {
        None
    };
    // Новая загрузка — своя отметка (повторный запуск оболочки отчёт не повторит).
    let b = syn_system::battery::read(&syn_system::Sys::host());
    save_state(b.as_ref(), "");
    let Some((summary, body)) = msg else { return };
    log::warn!("питание: {summary}: {body}");
    // Подождать, пока поднимутся уведомления и synlink (тогда отчёт уйдёт и на соединённые устройства).
    std::thread::sleep(Duration::from_secs(15));
    syngui::async_runtime::run_on_main_thread(move || {
        let ctx = ShellCtx::get();
        if ctx.cfg().power_alerts.unexpected {
            notifications::local_alert(ctx, KEY_LAST, &summary, &body, "dialog-warning", 1, &[], |_| {});
        }
    });
}

#[cfg(test)]
mod tests {
    fn rec(kind: i16, user: &str, t: i32) -> Vec<u8> {
        let mut r = vec![0u8; 384];
        r[..2].copy_from_slice(&kind.to_ne_bytes());
        r[44..44 + user.len()].copy_from_slice(user.as_bytes());
        r[340..344].copy_from_slice(&t.to_ne_bytes());
        r
    }

    #[test]
    fn wtmp_clean_and_unclean() {
        let clean = [rec(2, "reboot", 100), rec(7, "master", 110), rec(1, "shutdown", 200), rec(2, "reboot", 300)].concat();
        assert_eq!(super::wtmp_parse(&clean), Some((100, true)));
        let unclean = [rec(2, "reboot", 100), rec(1, "shutdown", 150), rec(2, "reboot", 300), rec(7, "master", 310), rec(2, "reboot", 500)].concat();
        assert_eq!(super::wtmp_parse(&unclean), Some((300, false)));
        assert_eq!(super::wtmp_parse(&rec(2, "reboot", 1)), None);
    }

    #[test]
    fn wtmp_this_machine() {
        // Настоящий wtmp, если есть: разбирается без паники.
        if let Ok(d) = std::fs::read("/var/log/wtmp") {
            eprintln!("wtmp: {:?}", super::wtmp_parse(&d));
        }
    }

    #[test]
    fn levels_order() {
        use super::Level::*;
        assert!(Action > Critical && Critical > Low && Low > None);
    }
}
