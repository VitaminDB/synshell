//! Звонок в оболочке, пока окна «Телефона» не видно: карточка входящего на экране блокировки (ответ без
//! разблокировки, как в Android/iOS), компактная карточка идущего разговора там же (таймер, динамик,
//! «Завершить») и зелёная плашка «идёт разговор» в панели (или, на телефоне без панели, — отдельной
//! поверхностью вверху экрана); нажатие на плашку поднимает «Телефон».
//!
//! Окна приложений поверх ext-session-lock не показать — поэтому на блокировке звонок рисует сама
//! оболочка. Состояние звонков — из `ctx.modem` (synmodemd), имена и фото — из
//! [`synshell_common::contacts`]. Отладка без модема: `SYNSHELL_DEMO_CALL=incoming|active|dialing`
//! подставляет выдуманный звонок (ответ и сброс меняют его на месте, в модем ничего не уходит).

use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use syngui::async_runtime::run_on_main_thread;
use syngui::prelude::*;
use syn_modem::api::{self, Call, CallState, Request, Status};
use synshell_common::contacts::{self, Contact};

use crate::ctx::ShellCtx;
use crate::ui::{icon, mi};

/// Трубка и «положить трубку» (Material Icons `call`, `call_end`).
const CALL: &str = "\u{E0B0}";
const CALL_END: &str = "\u{E0B1}";
const PERSON: &str = "\u{E7FD}";

/// Отладочный звонок (`SYNSHELL_DEMO_CALL`): запросы меняют `ctx.modem`, а не уходят в synmodemd.
static DEMO: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct Sig {
    /// Входящий (или второй входящий во время разговора), ещё не отвеченный.
    ringing: RwSignal<Option<Call>>,
    /// Идущий звонок: разговор, набор, гудки, удержание.
    ongoing: RwSignal<Option<Call>>,
    /// Окно «Телефона» в фокусе и не свёрнуто — плашка не нужна.
    phone_shown: RwSignal<bool>,
    /// Громкая связь.
    speaker: RwSignal<bool>,
}

thread_local! {
    static SIG: OnceCell<Sig> = const { OnceCell::new() };
    /// Контакты по номеру: файлы читаются один раз на номер (сбрасывается с концом звонков).
    static CONTACTS: RefCell<HashMap<String, Option<Contact>>> = RefCell::new(HashMap::new());
    static PILL: std::cell::Cell<Option<syngui_layer::SurfaceId>> = const { std::cell::Cell::new(None) };
    /// Входящий, ради которого уже включали экран.
    static WOKE_FOR: std::cell::Cell<Option<u8>> = const { std::cell::Cell::new(None) };
}

fn sig() -> Sig {
    SIG.with(|s| {
        *s.get_or_init(|| Sig { ringing: use_signal(None), ongoing: use_signal(None), phone_shown: use_signal(false), speaker: use_signal(false) })
    })
}

/// Звонящий входящий.
pub fn ringing(st: &Status) -> Option<Call> {
    st.calls.iter().find(|c| matches!(c.state, CallState::Incoming | CallState::Waiting)).cloned()
}

/// Идущий звонок: разговор, затем набор и гудки, затем удержание.
pub fn ongoing(st: &Status) -> Option<Call> {
    let pick = |s: &[CallState]| st.calls.iter().find(|c| s.contains(&c.state)).cloned();
    pick(&[CallState::Active]).or_else(|| pick(&[CallState::Dialing, CallState::Alerting])).or_else(|| pick(&[CallState::Held]))
}

/// Следить за звонками и окном «Телефона»; плашка на телефоне без панели.
pub fn start(ctx: ShellCtx) {
    let s = sig();
    create_effect(move || {
        let st = ctx.modem.get();
        let r = st.as_ref().and_then(ringing);
        let o = st.as_ref().and_then(ongoing);
        if s.ringing.get_untracked() != r {
            s.ringing.set(r.clone());
        }
        // Новый входящий — включить экран, как кнопка питания (погашен простоем, DPMS или датчиком
        // приближения); заблокирован — сразу видна карточка звонка. Гасится он потом по обычному
        // таймеру простоя (synwm считает `screen-on` активностью).
        let new_ring = r.as_ref().is_some_and(|c| Some(c.id) != WOKE_FOR.with(|w| w.get()));
        WOKE_FOR.with(|w| w.set(r.as_ref().map(|c| c.id)));
        if new_ring {
            log::info!("входящий вызов: включаю экран (screen-on)");
            crate::actions::run(synshell_common::action::Action::ScreenOn);
        }
        let new_call = o.as_ref().map(|c| c.id) != s.ongoing.get_untracked().map(|c| c.id);
        if s.ongoing.get_untracked() != o {
            s.ongoing.set(o.clone());
        }
        if new_call && o.is_some() {
            refresh_speaker();
        }
        if r.is_none() && o.is_none() {
            CONTACTS.with(|c| c.borrow_mut().clear());
        }
    });
    create_effect(move || {
        let shown = ctx.windows.get().iter().any(|w| w.app_id == "synphone" && w.focused && !w.minimized);
        if s.phone_shown.get_untracked() != shown {
            s.phone_shown.set(shown);
        }
    });
    if ctx.is_phone() {
        create_effect(move || {
            let _ = ctx.generation.get();
            let want = pill_wanted() && !syngui_layer::session_locked().get() && !has_panel(&ctx);
            let open = PILL.with(|p| p.get());
            match (want, open) {
                (true, None) => open_pill_surface(&ctx),
                (false, Some(id)) => {
                    PILL.with(|p| p.set(None));
                    syngui_layer::close_surface(id);
                }
                _ => {}
            }
        });
    }
}

/// Горизонтальная панель на этом форм-факторе есть — плашка встанет в неё.
fn has_panel(ctx: &ShellCtx) -> bool {
    use synshell_common::config::Edge;
    ctx.cfg().panels.iter().any(|p| p.shows_on(ctx.form_factor) && matches!(p.edge, Edge::Top | Edge::Bottom))
}

/// Плашка нужна: есть звонок, а окна «Телефона» не видно (реактивно).
fn pill_wanted() -> bool {
    let s = sig();
    (s.ongoing.get().is_some() || s.ringing.get().is_some()) && !s.phone_shown.get()
}

fn open_pill_surface(ctx: &ShellCtx) {
    use syngui_layer::{Anchor, KeyboardInteractivity, Layer, SurfaceSpec};
    let id = syngui_layer::create_surface(
        SurfaceSpec {
            namespace: "syndesktop-call-pill".into(),
            layer: Layer::Top,
            anchor: Anchor::TOP,
            size: (0, 0),
            auto_size: true,
            margin: [6, 0, 0, 0],
            exclusive_zone: -1,
            keyboard: KeyboardInteractivity::None,
            output: crate::manager::focused_output(ctx),
            clear_color: [0.0; 4],
        },
        || Box::new(DecoratedBox::new().child(pill(false)).class("call-pill-float")),
    );
    PILL.with(|p| p.set(Some(id)));
}

// ─── Запросы ─────────────────────────────────────────────────────────────────

fn modem_request(req: Request) {
    if DEMO.load(Ordering::Relaxed) {
        let ctx = ShellCtx::get();
        let now = crate::clock::unix_now();
        ctx.modem.update(|m| {
            let Some(m) = m else { return };
            match req {
                Request::Answer { id } => {
                    for c in m.calls.iter_mut() {
                        if c.id == id {
                            c.state = CallState::Active;
                            c.answered_at = Some(now);
                        } else if c.state == CallState::Active {
                            c.state = CallState::Held;
                        }
                    }
                }
                Request::Hangup { id } => m.calls.retain(|c| c.id != id),
                _ => {}
            }
        });
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = api::request(&req) {
            log::warn!("звонок: {req:?}: {e:#}");
        }
    });
}

pub fn answer(id: u8) {
    modem_request(Request::Answer { id });
}

pub fn hangup(id: u8) {
    modem_request(Request::Hangup { id });
}

/// Команда palaudiod (`/run/palaudio/control`, строка на команду); ответ — одна строка.
fn palaudio(cmd: &str) -> std::io::Result<String> {
    use std::io::{BufRead, BufReader, Write};
    let mut s = std::os::unix::net::UnixStream::connect("/run/palaudio/control")?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    s.write_all(format!("{cmd}\n").as_bytes())?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line)?;
    Ok(line)
}

/// Громкая связь сейчас — по маршруту голоса palaudiod.
fn refresh_speaker() {
    if DEMO.load(Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        let Ok(line) = palaudio("status") else { return };
        let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
        let on = v.pointer("/voice/output").and_then(|o| o.as_str()) == Some("speaker");
        run_on_main_thread(move || sig().speaker.set(on));
    });
}

fn toggle_speaker() {
    let s = sig();
    let on = !s.speaker.get_untracked();
    s.speaker.set(on);
    if DEMO.load(Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(move || {
        let r = palaudio(&format!("voice-route {}", if on { "speaker" } else { "auto" }));
        let bad = match r {
            Ok(l) if l.contains("\"error\"") => Some(l),
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(e) = bad {
            log::warn!("громкая связь: {}", e.trim());
            run_on_main_thread(refresh_speaker);
        }
    });
}

// ─── Кто звонит ──────────────────────────────────────────────────────────────

fn contact(number: &str) -> Option<Contact> {
    if number.trim().is_empty() {
        return None;
    }
    CONTACTS.with(|c| c.borrow_mut().entry(number.to_string()).or_insert_with(|| contacts::find_by_number(number)).clone())
}

/// Имя контакта, номер или «Скрытый номер».
pub fn caller(number: &str) -> String {
    match contact(number) {
        Some(c) => c.title(),
        None if number.trim().is_empty() => t!("Скрытый номер").into(),
        None => number.to_string(),
    }
}

/// Фото кругом; без фото — кружок с первой буквой имени или значком.
fn avatar(number: &str, class: &str) -> Box<dyn Widget> {
    let c = contact(number);
    if let Some(p) = c.as_ref().and_then(|c| c.photo()) {
        return Box::new(Image::new(p.to_string_lossy()).fit(ImageFit::Cover).class(format!("{class} call-avatar-photo")));
    }
    let letter = c.map(|c| c.initial()).unwrap_or_default();
    let inner: Box<dyn Widget> =
        if letter.is_empty() { Box::new(icon(PERSON).class("call-avatar-icon")) } else { Box::new(Text::new(letter).class("call-avatar-letter")) };
    Box::new(DecoratedBox::new().child(crate::ui::vcenter(inner)).class(class.to_string()))
}

/// Длительность разговора «4:07» / «1:02:09».
pub fn duration(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Состояние звонка словами: таймер разговора, «Вызов…», «Удержание».
fn state_text(c: &Call, now: i64) -> String {
    match c.state {
        CallState::Active => duration(now - c.answered_at.unwrap_or(now)),
        CallState::Dialing | CallState::Alerting => t!("Вызов…").into(),
        CallState::Held => t!("Удержание").into(),
        CallState::Incoming | CallState::Waiting => t!("Входящий вызов").into(),
        CallState::Ended => String::new(),
    }
}

/// Текст, перерисовываемый раз в секунду (таймер), — отдельным узлом: карточка с кнопками не пересоздаётся.
fn live_state(call: Call, class: &'static str) -> impl Widget {
    let ctx = ShellCtx::get();
    crate::ui::rx(move || {
        let now = ctx.now.get();
        syngui::i18n::subscribe();
        Box::new(Text::new(state_text(&call, now)).max_lines(1).class(class))
    })
}

// ─── Экран блокировки ────────────────────────────────────────────────────────

/// Большая круглая кнопка с подписью под ней.
fn big_button(glyph: &str, kind: &str, label: String, f: impl FnMut() + Send + 'static) -> impl Widget {
    syngui::GestureDetector::new().on_click(f).child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().child(crate::ui::vcenter(icon(glyph).class("call-btn-icon"))).class(format!("call-btn call-btn-{kind}")))
            .child(Text::new(label).class("call-btn-label")),
    )
}

/// Маленькая круглая кнопка карточки разговора.
fn mini_button(glyph: &str, class: String, f: impl FnMut() + Send + 'static) -> impl Widget {
    syngui::GestureDetector::new()
        .on_click(f)
        .child(DecoratedBox::new().child(crate::ui::vcenter(icon(glyph).class("call-mini-icon"))).class(class))
}

/// Входящий звонок поверх экрана блокировки: фото, имя, «Отклонить» и «Ответить» (без разблокировки).
/// Пока звонка нет — пусто; касания под карточку не проходят.
pub fn lock_overlay() -> impl Widget {
    let s = sig();
    crate::ui::rx(move || {
        let Some(call) = s.ringing.get() else { return Box::new(Column::new()) };
        syngui::i18n::subscribe();
        let name = caller(&call.number);
        // Номер под именем — когда звонит известный контакт.
        let sub = if contact(&call.number).is_some() { call.number.clone() } else { String::new() };
        let label = if call.state == CallState::Waiting { t!("Второй вызов") } else { t!("Входящий вызов") };
        let head = Column::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon(CALL).class("call-lock-label-icon"))
                    .child(Text::new(label).class("call-lock-label")),
            )
            .child(DecoratedBox::new().class("call-lock-gap"))
            .child(avatar(&call.number, "call-avatar call-lock-avatar"))
            .child(Text::new(name).max_lines(2).class("call-lock-name"))
            .child(Text::new(sub).class("call-lock-number"));
        let id = call.id;
        let buttons = Row::new()
            .gap(96.0)
            .child(big_button(CALL_END, "red", t!("Отклонить").into(), move || hangup(id)))
            .child(big_button(CALL, "green", t!("Ответить").into(), move || answer(id)));
        // Размер — во весь экран явно: внутри Reactive ограничения свободные.
        let vp = syngui::viewport::viewport_size().get();
        Box::new(
            Stack::new()
                .fit(StackFit::Expand)
                .child(DecoratedBox::new().class("call-lock-scrim"))
                .child(
                    syngui::GestureDetector::new()
                        .on_click(|| {})
                        .on_swipe(|_, _| {})
                        .child(
                            Column::new()
                                .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(head)
                                .child(buttons)
                                .class("call-lock"),
                        ),
                )
                .style("width", vp.width)
                .style("height", vp.height),
        )
    })
}

/// Идущий разговор на экране блокировки: имя, таймер, «Динамик» и «Завершить». Нет разговора (или
/// звонит входящий — его рисует [`lock_overlay`]) — пусто.
pub fn lock_banner() -> impl Widget {
    let s = sig();
    crate::ui::rx(move || {
        let Some(call) = s.ongoing.get() else { return Box::new(Column::new()) };
        if s.ringing.get().is_some() {
            return Box::new(Column::new());
        }
        syngui::i18n::subscribe();
        let w = (syngui::viewport::viewport_size().get().width - 48.0).clamp(0.0, 460.0);
        let id = call.id;
        let speaker = crate::ui::rx(move || {
            let on = s.speaker.get();
            Box::new(mini_button(mi::VOLUME_UP, format!("call-mini {}", if on { "call-mini-on" } else { "call-mini-soft" }), toggle_speaker))
        });
        Box::new(
            DecoratedBox::new()
                .child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(avatar(&call.number, "call-avatar call-banner-avatar"))
                        .child(
                            Column::new()
                                .gap(2.0)
                                .child(Text::new(caller(&call.number)).max_lines(1).class("call-banner-name"))
                                .child(live_state(call.clone(), "call-banner-time"))
                                .class("grow"),
                        )
                        .child(speaker)
                        .child(mini_button(CALL_END, "call-mini call-mini-red".into(), move || hangup(id))),
                )
                .class("call-banner")
                .style("width", w),
        )
    })
}

// ─── Плашка в панели ─────────────────────────────────────────────────────────

/// Зелёная плашка с трубкой и таймером, пока идёт звонок, а окна «Телефона» не видно; нажатие —
/// поднять «Телефон». `vertical` — только значок (боковая панель).
pub fn pill(vertical: bool) -> impl Widget {
    let s = sig();
    crate::ui::rx(move || {
        if !pill_wanted() {
            return Box::new(Column::new());
        }
        let Some(call) = s.ringing.get().or_else(|| s.ongoing.get()) else { return Box::new(Column::new()) };
        let ringing = matches!(call.state, CallState::Incoming | CallState::Waiting);
        let mut row = Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(CALL).class("call-pill-icon"));
        if !vertical {
            if ringing {
                syngui::i18n::subscribe();
                row = row.child(Text::new(caller(&call.number)).max_lines(1).class("call-pill-text"));
            } else {
                row = row.child(live_state(call.clone(), "call-pill-text"));
            }
        }
        let class = if ringing { "call-pill call-pill-ringing" } else { "call-pill" };
        Box::new(
            syngui::GestureDetector::new()
                .on_click(crate::modem::open_phone)
                .child(crate::ui::vcenter(DecoratedBox::new().child(row).class(class))),
        )
    })
}

/// Плашка в редакторе панели (звонка нет — показать, где она встанет).
pub fn pill_placeholder() -> impl Widget {
    crate::ui::vcenter(DecoratedBox::new().child(icon(CALL).class("call-pill-icon")).class("call-pill call-pill-placeholder"))
}

// ─── Отладка ─────────────────────────────────────────────────────────────────

/// `SYNSHELL_DEMO_CALL=incoming|waiting|active|dialing|held` — выдуманный звонок вместо модема.
/// Номер — `SYNSHELL_DEMO_CALL_NUMBER` (пусто — скрытый); `SYNSHELL_DEMO_CALL_DELAY` — через сколько
/// секунд после запуска звонок появится (проверка пробуждения погашенного экрана).
pub fn demo_status() -> Option<Status> {
    let kind = std::env::var("SYNSHELL_DEMO_CALL").ok().filter(|v| !v.is_empty())?;
    let state = match kind.as_str() {
        "incoming" => CallState::Incoming,
        "waiting" => CallState::Waiting,
        "dialing" => CallState::Alerting,
        "held" => CallState::Held,
        _ => CallState::Active,
    };
    DEMO.store(true, Ordering::Relaxed);
    let now = crate::clock::unix_now();
    let number = std::env::var("SYNSHELL_DEMO_CALL_NUMBER").unwrap_or_else(|_| "+7 701 555 12 34".into());
    let mut calls = Vec::new();
    if state == CallState::Waiting {
        calls.push(Call { id: 1, number: "+7 777 000 00 01".into(), state: CallState::Active, incoming: false, answered_at: Some(now - 95), started_at: now - 100 });
    }
    calls.push(Call {
        id: 2,
        number,
        state,
        incoming: matches!(state, CallState::Incoming | CallState::Waiting),
        answered_at: matches!(state, CallState::Active | CallState::Held).then_some(now - 187),
        started_at: now - 190,
    });
    Some(Status { present: true, radio: true, bars: Some(3), calls, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(247), "4:07");
        assert_eq!(duration(3729), "1:02:09");
        assert_eq!(duration(-5), "0:00");
    }

    #[test]
    fn picks_calls() {
        let call = |id, state| Call { id, number: String::new(), state, incoming: true, answered_at: None, started_at: 0 };
        let st = Status { calls: vec![call(1, CallState::Held), call(2, CallState::Active), call(3, CallState::Waiting)], ..Default::default() };
        assert_eq!(ringing(&st).map(|c| c.id), Some(3));
        assert_eq!(ongoing(&st).map(|c| c.id), Some(2));
        let st = Status { calls: vec![call(1, CallState::Held), call(4, CallState::Ended)], ..Default::default() };
        assert_eq!(ringing(&st), None);
        assert_eq!(ongoing(&st).map(|c| c.id), Some(1));
    }
}
