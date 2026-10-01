//! Модем телефона в оболочке: состояние сети из synmodemd (`ctx.modem`), уведомления о новых SMS и
//! пропущенных звонках, вызов «Телефона» на входящий звонок, полоски сигнала.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use synmodem::api::{self, CallKind, CallState, Event, Registration, SimState, SmsStatus, Status};

use crate::ctx::ShellCtx;

static STARTED: AtomicBool = AtomicBool::new(false);

/// Подписаться на synmodemd (один раз на процесс). Без доступа к событиям (не в группе wheel/network) —
/// только опрос состояния.
pub fn start(ctx: ShellCtx) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = ctx;
    std::thread::Builder::new()
        .name("shell-modem".into())
        .spawn(move || {
            // Входящие, на которые уже позвали «Телефон», и пропущенные, о которых уже сказали
            let mut rung: HashSet<u8> = HashSet::new();
            let mut told_missed: HashSet<u64> = api::call_log().map(|l| l.iter().map(|c| c.id).collect()).unwrap_or_default();
            loop {
                if !std::path::Path::new(api::SOCKET).exists() {
                    std::thread::sleep(Duration::from_secs(10));
                    continue;
                }
                let r = api::subscribe(|ev| {
                    on_event(ev, &mut rung, &mut told_missed);
                    true
                });
                if r.is_err() {
                    // Нет доступа к событиям — состояние сети опросом
                    while let Ok(st) = api::status() {
                        run_on_main_thread(move || ShellCtx::get().modem.set(Some(st)));
                        std::thread::sleep(Duration::from_secs(5));
                    }
                }
                run_on_main_thread(|| ShellCtx::get().modem.set(None));
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .ok();
}

fn on_event(ev: Event, rung: &mut HashSet<u8>, told_missed: &mut HashSet<u64>) {
    match ev {
        Event::Status { status } => {
            for c in &status.calls {
                if matches!(c.state, CallState::Incoming | CallState::Waiting) && rung.insert(c.id) {
                    run_on_main_thread(open_phone);
                }
            }
            rung.retain(|id| status.calls.iter().any(|c| c.id == *id));
            run_on_main_thread(move || ShellCtx::get().modem.set(Some(status)));
        }
        Event::Sms { message } if message.incoming && message.status == SmsStatus::Received && !message.read => {
            run_on_main_thread(move || {
                let cmd = format!("synsms --chat '{}'", message.number.replace('\'', ""));
                crate::notifications::local_command(
                    ShellCtx::get(),
                    &format!("sms:{}", message.number),
                    &message.number,
                    &message.text,
                    "mail-message-new",
                    cmd,
                );
            });
        }
        Event::CallLog => {
            let Ok(log) = api::call_log() else { return };
            for c in log.into_iter().filter(|c| c.kind == CallKind::Missed && c.new) {
                if told_missed.insert(c.id) {
                    run_on_main_thread(move || {
                        crate::notifications::local_command(
                            ShellCtx::get(),
                            "missed-call",
                            "Пропущенный звонок",
                            &c.number,
                            "call-missed",
                            "synphone --log".into(),
                        );
                    });
                }
            }
        }
        _ => {}
    }
}

/// Входящий звонок: окно «Телефона» — поднять, иначе запустить (экран звонка он покажет сам).
fn open_phone() {
    let ctx = ShellCtx::get();
    if let Some(w) = ctx.windows.get_untracked().iter().find(|w| w.app_id == "synphone") {
        crate::actions::window_op(w.id, synshell_common::ipc::WindowOp::Activate);
    } else {
        crate::actions::spawn("synphone");
    }
}

/// Строка состояния мобильной связи: (заголовок, подробности).
pub fn summary(s: &Status) -> (String, String) {
    if !s.radio {
        return ("Режим полёта".into(), "Мобильная связь выключена".into());
    }
    match s.sim.state {
        SimState::Absent => return ("Нет SIM".into(), String::new()),
        SimState::PinRequired => return ("SIM заблокирована".into(), "Нужен PIN-код".into()),
        SimState::PukRequired | SimState::Blocked => return ("SIM заблокирована".into(), "Нужен PUK-код".into()),
        SimState::Initializing | SimState::Unknown => return ("SIM…".into(), "Подготовка карты".into()),
        SimState::Error => return ("Ошибка SIM".into(), String::new()),
        SimState::Ready => {}
    }
    let op = if s.operator.is_empty() { s.plmn.clone() } else { s.operator.clone() };
    match s.registration {
        Registration::Home | Registration::Roaming => {
            let mut sub = s.technology.clone();
            if s.roaming {
                sub = if sub.is_empty() { "роуминг".into() } else { format!("{sub} · роуминг") };
            }
            (op, sub)
        }
        Registration::Limited => ("Только экстренные".into(), op),
        Registration::Denied => ("Сеть отказала".into(), op),
        Registration::Searching | Registration::NotRegistered | Registration::Unknown => ("Поиск сети…".into(), String::new()),
    }
}

/// Полоски сигнала: 4 столбика растущей высоты, закрашены по уровню (`None` — все пустые).
pub fn signal_bars(bars: Option<u8>) -> Row {
    let n = bars.unwrap_or(0);
    let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::End).class("sigbars");
    for k in 1..=4u8 {
        let on = bars.is_some() && k <= n;
        row = row.child(DecoratedBox::new().class(&format!("sigbar sigbar-{k} {}", if on { "sigbar-on" } else { "sigbar-off" })));
    }
    row
}
