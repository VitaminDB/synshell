//! Дата и время в оболочке: состояние часов системы (systemd-timedated),
//! служба «Часовой пояс автоматически» (`[time] auto_timezone`: пояс по
//! внешнему IP при старте, при появлении сети и раз в несколько часов) и
//! быстрые настройки во всплывающем окне часов.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synsystem::time::{self as systime, TimeStatus};

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi};

/// Проверять пояс так часто, когда он уже определён.
const RECHECK: Duration = Duration::from_secs(3 * 3600);
/// Повтор после неудачи (нет сети, геосервис не ответил).
const RETRY: Duration = Duration::from_secs(120);

static AUTO: AtomicBool = AtomicBool::new(false);

fn wake_slot() -> &'static Mutex<Option<Sender<()>>> {
    static W: OnceLock<Mutex<Option<Sender<()>>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(None))
}

/// Определить пояс сейчас (включили автоопределение, появилась сеть).
fn wake() {
    if let Some(tx) = wake_slot().lock().unwrap().as_ref() {
        let _ = tx.send(());
    }
}

thread_local! {
    static STATUS: Cell<Option<RwSignal<Option<TimeStatus>>>> = const { Cell::new(None) };
    static NOTE: Cell<Option<RwSignal<Option<String>>>> = const { Cell::new(None) };
}

/// Состояние часов системы; `None` — ещё не прочитано.
pub fn status_sig() -> RwSignal<Option<TimeStatus>> {
    STATUS.with(|s| match s.get() {
        Some(x) => x,
        None => {
            let x = use_signal(None);
            s.set(Some(x));
            refresh();
            x
        }
    })
}

/// Подсказка под быстрыми настройками (ход и ошибки действий).
fn note_sig() -> RwSignal<Option<String>> {
    NOTE.with(|s| match s.get() {
        Some(x) => x,
        None => {
            let x = use_signal(None);
            s.set(Some(x));
            x
        }
    })
}

/// Перечитать состояние в фоне; часы оболочки — сразу в новом поясе.
pub fn refresh() {
    let sig = status_sig();
    std::thread::spawn(move || {
        let st = systime::status();
        run_on_main_thread(move || {
            if sig.get_untracked().as_ref() != Some(&st) {
                sig.set(Some(st));
            }
            let ctx = ShellCtx::get();
            ctx.now.set(crate::clock::unix_now());
        });
    });
}

/// Действие с часами системы в фоне: подсказка «…», затем итог и перечитывание.
fn act(label: &'static str, f: impl FnOnce() -> std::result::Result<(), String> + Send + 'static) {
    let note = note_sig();
    note.set(Some(format!("{}…", syngui::i18n::t(label))));
    std::thread::spawn(move || {
        let r = f();
        run_on_main_thread(move || {
            note.set(match r {
                Ok(()) => None,
                Err(e) => Some(format!("{label}: {e}")),
            });
            refresh();
        });
    });
}

/// Запустить службу автоопределения пояса.
pub fn start(ctx: ShellCtx) {
    let (tx, rx) = channel::<()>();
    *wake_slot().lock().unwrap() = Some(tx);
    std::thread::Builder::new()
        .name("auto-timezone".into())
        .spawn(move || {
            let mut delay = RECHECK;
            loop {
                if let Err(RecvTimeoutError::Disconnected) = rx.recv_timeout(delay) {
                    return;
                }
                // Пачку пробуждений (сеть + настройка) — одной проверкой.
                while rx.try_recv().is_ok() {}
                if !AUTO.load(Ordering::Relaxed) {
                    delay = RECHECK;
                    continue;
                }
                delay = match systime::detect_timezone() {
                    Ok(tz) => {
                        if systime::status().timezone != tz {
                            match systime::set_timezone(&tz) {
                                Ok(()) => {
                                    log::info!("часовой пояс по сети: {tz}");
                                    run_on_main_thread(refresh);
                                }
                                Err(e) => log::warn!("часовой пояс {tz} не установлен: {e}"),
                            }
                        }
                        RECHECK
                    }
                    Err(e) => {
                        log::info!("часовой пояс по сети не определён: {e}");
                        RETRY
                    }
                };
            }
        })
        .ok();
    let online_before = Cell::new(false);
    create_effect(move || {
        let on = ctx.config.get().time.auto_timezone(ctx.form_factor);
        let online = ctx.network.get().online;
        let was = AUTO.swap(on, Ordering::Relaxed);
        let came_online = online && !online_before.replace(online);
        if on && (!was || came_online) {
            wake();
        }
    });
}

/// Включить или выключить автоопределение пояса (`[time] auto_timezone`).
fn set_auto_timezone(on: bool) {
    if let Err(e) = synshell_common::config_edit::set_value(&["time", "auto_timezone"], toml_edit::Value::from(on)) {
        log::error!("[time] auto_timezone: {e:#}");
    }
    crate::reload_after_write();
}

/// Смещение пояса сейчас (с летним временем) — `UTC+05:00`.
pub fn current_offset(now: i64) -> String {
    systime::format_offset(crate::clock::utc_offset(now))
}

fn switch_row(glyph: &str, title: &str, hint: String, on: bool, enabled: bool, f: impl FnMut(bool) + Send + 'static) -> impl Widget {
    Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon(glyph).class("net-icon"))
        .child(
            Column::new()
                .gap(1.0)
                .child(Text::new(title.to_string()).class("net-ssid"))
                .child(Text::new(hint).max_lines(2).class("net-hint"))
                .class("grow"),
        )
        .child(Toggle::with_state(on).disabled(!enabled).on_change(f))
        .class("net-row net-row-static")
}

/// Быстрые настройки часов: пояс, «время автоматически» (NTP),
/// «часовой пояс автоматически», переход в «Параметры → Дата и время».
pub fn quick(ctx: ShellCtx) -> impl Widget {
    let st = status_sig();
    let note = note_sig();
    refresh();
    Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(DecoratedBox::new().class("menu-sep"))
        .child(crate::ui::rx(move || {
            let Some(s) = st.get() else {
                return Box::new(Text::new(t!("Чтение настроек часов…")).class("net-hint"));
            };
            let now = ctx.now.get_untracked();
            let auto_tz = ctx.config.get().time.auto_timezone(ctx.form_factor);
            let zone = if s.timezone.is_empty() { t!("не задан (UTC)").to_string() } else { s.timezone.clone() };
            let mut col = Column::new().gap(2.0).child(
                Row::new()
                    .gap(10.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(mi::PUBLIC).class("net-icon"))
                    .child(
                        Column::new()
                            .gap(1.0)
                            .child(Text::new(t!("Часовой пояс")).class("net-ssid"))
                            .child(Text::new(format!("{zone} · {}", current_offset(now))).max_lines(1).class("net-hint"))
                            .class("grow"),
                    )
                    .class("net-row net-row-static"),
            );
            if !s.service {
                return Box::new(col.child(Text::new(t!("Служба systemd-timedated недоступна — часы не настроить")).max_lines(2).class("net-note")));
            }
            let ntp_hint = match (s.can_ntp, s.ntp, s.synced) {
                (false, _, _) => t!("Служба синхронизации не установлена").to_string(),
                (true, true, true) => t!("По сети · сверено").to_string(),
                (true, true, false) => t!("По сети · ещё не сверено").to_string(),
                (true, false, _) => t!("Вручную").to_string(),
            };
            col = col
                .child(switch_row(mi::SCHEDULE, &t!("Время автоматически"), ntp_hint, s.ntp, s.can_ntp, |on| {
                    act(if on { n_!("Включение синхронизации") } else { n_!("Выключение синхронизации") }, move || systime::set_ntp(on))
                }))
                .child(switch_row(
                    mi::PUBLIC,
                    &t!("Часовой пояс автоматически"),
                    if auto_tz { t!("По местоположению в сети").into() } else { t!("Вручную").into() },
                    auto_tz,
                    true,
                    set_auto_timezone,
                ));
            Box::new(col)
        }))
        .child(move || match note.get() {
            Some(t) => Text::new(t).max_lines(3).class("net-toast"),
            None => Text::new(String::new()).class("net-toast-none"),
        })
        .child(crate::popup::menu_item(mi::SETTINGS, t!("Параметры даты и времени…"), || crate::actions::spawn("synsettings datetime")))
}
