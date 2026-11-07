//! synphone — «Телефон»: набор номера, журнал звонков и экран разговора через демон модема `synmodemd`.
//! Есть звонок — окно целиком занимает экран разговора (оболочка поднимает окно на входящий).
//! `synphone --log` — открыть журнал, `synphone НОМЕР` — номер в набор; `--screenshot СНИМОК.png [--size ШxВ]
//! [--scale N] [--page dial|log|call]` — снимок окна с придуманными данными (без модема).

use std::time::Duration;

use syn_modem::api::{self, Call, CallKind, CallRecord, CallState, Event, Request, Status};
use syn_modem::time as tm;
use synshell_common::Config;
use syngui::async_runtime::run_on_main_thread;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::{GestureDetector, ShowIf};

type W = Box<dyn Widget>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Dial,
    Log,
}

#[derive(Clone, Copy)]
struct St {
    tab: RwSignal<Tab>,
    number: RwSignal<String>,
    log: RwSignal<Vec<CallRecord>>,
    status: RwSignal<Option<Status>>,
    toast: RwSignal<String>,
    /// Секунды UNIX, раз в секунду во время звонка (таймер разговора).
    tick: RwSignal<i64>,
    /// Клавиатура тонов на экране разговора.
    dtmf_open: RwSignal<bool>,
    /// Громкая связь и выключенный микрофон (голос разговора — palaudiod).
    speaker: RwSignal<bool>,
    muted: RwSignal<bool>,
    /// Индексы для ShowIf (Reactive отдаёт детям свободные ограничения — экраны во всю высоту через ShowIf):
    /// 0 — обычный экран, 1 — разговор; вкладка 0 — набор, 1 — журнал.
    mode: RwSignal<usize>,
    page: RwSignal<usize>,
    /// Состояние окна (фокус, развёрнуто) — для своих кнопок окна.
    window: RwSignal<syngui::window::WindowState>,
}

/// Телефон: окно на весь экран, без своего заголовка (рамку и кнопки окна не рисуем).
fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v.trim().eq_ignore_ascii_case("phone"))
}

fn main() {
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));
    let mut log = false;
    let mut number = String::new();
    let mut shot: Option<String> = None;
    let mut size = (430u32, 900u32);
    let mut scale = 2.0f64;
    let mut page = String::from("dial");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("synphone [NUMBER] [--log] [--screenshot FILE.png [--size WxH] [--scale N] [--page dial|log|call]]");
                return;
            }
            "--log" => log = true,
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            "--page" => page = args.next().unwrap_or(page),
            a if !a.starts_with("--") && number.is_empty() => number = a.to_string(),
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        // Снимок с придуманными данными (без модема): набор, журнал или разговор
        let r = syngui::shot::render_png(
            move || {
                let st = new_st(page == "log", number);
                demo(st, &page);
                create_effects(st);
                root(st)
            },
            &mss,
            size,
            scale,
            &out,
        );
        match r {
            Ok((w, h)) => println!("{out}: {w}x{h}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let win = use_signal(syngui::window::WindowState::default());
    App::new()
        .title(t!("Телефон"))
        .app_id("synphone")
        .size(420, 760)
        .min_size(320, 520)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_window_state(win)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss.clone(), theme))
        .run(move |_| {
            let st = new_st(log, number.clone());
            st.window.set(win.get_untracked());
            create_effect(move || st.window.set(win.get()));
            create_effects(st);
            watch(st);
            seen_effect(st);
            ticker(st);
            root(st)
        });
}

fn new_st(log: bool, number: String) -> St {
    St {
        tab: use_signal(if log { Tab::Log } else { Tab::Dial }),
        number: use_signal(number),
        log: use_signal(Vec::new()),
        status: use_signal(None),
        toast: use_signal(String::new()),
        tick: use_signal(tm::now()),
        dtmf_open: use_signal(false),
        speaker: use_signal(false),
        muted: use_signal(false),
        mode: use_signal(0),
        page: use_signal(usize::from(log)),
        window: use_signal(syngui::window::WindowState::default()),
    }
}

fn create_effects(st: St) {
    create_effect(move || {
        let calls = st.status.get().is_some_and(|s| !s.calls.is_empty());
        st.mode.set(usize::from(calls));
        // Звонок кончился — palaudiod сбрасывает громкую связь и микрофон
        if !calls {
            st.speaker.set(false);
            st.muted.set(false);
        }
    });
    create_effect(move || st.page.set(if st.tab.get() == Tab::Log { 1 } else { 0 }));
}

/// Придуманные данные для снимков (`--screenshot`): сеть, журнал, разговор.
fn demo(st: St, page: &str) {
    let now = tm::now();
    let mut status = Status { present: true, radio: true, operator: "Demo Mobile".into(), plmn: "001/01".into(), technology: "LTE".into(), bars: Some(4), ..Default::default() };
    status.registration = api::Registration::Home;
    if page == "call" {
        status.calls = vec![Call { id: 1, number: "+1 555 0142".into(), state: CallState::Active, incoming: true, answered_at: Some(now - 83), started_at: now - 90 }];
    }
    st.status.set(Some(status));
    if page == "dial" && st.number.get_untracked().is_empty() {
        st.number.set("+1 555 0198".into());
    }
    let rec = |id: u64, number: &str, kind: CallKind, ago: i64, duration: u32, new: bool| CallRecord { id, number: number.into(), kind, time: now - ago, duration, new };
    st.log.set(vec![
        rec(1, "+1 555 0142", CallKind::Incoming, 25 * 60, 312, false),
        rec(2, "+1 555 0198", CallKind::Missed, 2 * 3600, 0, page != "log"),
        rec(3, "+1 555 0117", CallKind::Outgoing, 5 * 3600, 74, false),
        rec(4, "", CallKind::Rejected, 26 * 3600, 0, false),
        rec(5, "+1 555 0163", CallKind::Outgoing, 28 * 3600, 0, false),
        rec(6, "+1 555 0142", CallKind::Incoming, 3 * 86400, 1265, false),
        rec(7, "+1 555 0129", CallKind::Outgoing, 4 * 86400, 45, false),
        rec(8, "+1 555 0181", CallKind::Missed, 6 * 86400, 0, false),
    ]);
}

fn theme(cfg: &Config) -> String {
    synshell_common::style::AppStyle::new(cfg, "synphone").base(include_str!("../styles/synphone.mss")).build()
}

// ─── Данные ─────────────────────────────────────────────────────────────────

fn reload_log(st: St) {
    std::thread::spawn(move || {
        if let Ok(l) = api::call_log() {
            run_on_main_thread(move || st.log.set(l));
        }
    });
}

fn watch(st: St) {
    std::thread::spawn(move || loop {
        let r = api::subscribe(|ev| {
            match ev {
                Event::Status { status } => run_on_main_thread(move || {
                    let first = st.status.get_untracked().is_none();
                    // Звонок закончился — закрыть клавиатуру тонов
                    if status.calls.is_empty() {
                        st.dtmf_open.set(false);
                    }
                    st.status.set(Some(status));
                    if first {
                        reload_log(st);
                    }
                }),
                Event::CallLog => run_on_main_thread(move || reload_log(st)),
                _ => {}
            }
            true
        });
        let msg = match r {
            Ok(()) => t!("synmodemd остановлен").to_string(),
            Err(e) => format!("{e:#}"),
        };
        run_on_main_thread(move || {
            st.status.set(None);
            st.toast.set(t!("Модем недоступен: {msg}", msg = msg));
        });
        std::thread::sleep(Duration::from_secs(5));
    });
}

/// Журнал открыт — пропущенные просмотрены.
fn seen_effect(st: St) {
    create_effect(move || {
        if st.tab.get() == Tab::Log && st.log.get().iter().any(|c| c.new) {
            std::thread::spawn(|| {
                let _ = api::request(&Request::CallLogSeen);
            });
        }
    });
}

fn ticker(st: St) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        run_on_main_thread(move || {
            if st.status.get_untracked().is_some_and(|s| !s.calls.is_empty()) {
                st.tick.set(tm::now());
            }
        });
    });
}

fn act(st: St, req: Request) {
    std::thread::spawn(move || {
        if let Err(e) = api::request(&req) {
            let m = format!("{e:#}");
            run_on_main_thread(move || st.toast.set(m));
        }
    });
}

fn dial(st: St, number: String) {
    let number = number.trim().to_string();
    if number.is_empty() {
        return;
    }
    act(st, Request::Dial { number });
}

// ─── Интерфейс ──────────────────────────────────────────────────────────────

fn root(st: St) -> W {
    let body = Stack::new()
        .fit(StackFit::Expand)
        .child(ShowIf::new(0, st.mode).child(main_view(st)))
        .child(ShowIf::new(1, st.mode).child(in_call(st)));
    let toast = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let t = st.toast.get();
            if t.is_empty() {
                return vec![];
            }
            vec![Box::new(DecoratedBox::new().child(Text::new(t).max_lines(3).class("toast-text")).class("toast"))]
        }))
        .class("toast-place");
    create_effect(move || {
        let t = st.toast.get();
        if !t.is_empty() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(4));
                run_on_main_thread(move || {
                    if st.toast.get_untracked() == t {
                        st.toast.set(String::new());
                    }
                });
            });
        }
    });
    // Своя рамка окна: заголовок с кнопками окна; на телефоне — только содержимое во весь экран
    let chrome = !is_phone();
    let content = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().child(body).class("root")).child(toast);
    let inner: W = if chrome {
        Box::new(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(titlebar(st)).child(DecoratedBox::new().child(content).class("grow")))
    } else {
        Box::new(content)
    };
    let frame = DecoratedBox::new().class(if chrome { "window-frame" } else { "window-frame flat" }).clip(true).child(inner);
    Box::new(
        EventHook::new()
            .on_key_down(move |k, _| {
                use syngui::input::Key;
                // Набор с клавиатуры компьютера
                let in_call = st.status.get_untracked().is_some_and(|s| !s.calls.is_empty());
                let digit = match k {
                    Key::Num0 => Some('0'),
                    Key::Num1 => Some('1'),
                    Key::Num2 => Some('2'),
                    Key::Num3 => Some('3'),
                    Key::Num4 => Some('4'),
                    Key::Num5 => Some('5'),
                    Key::Num6 => Some('6'),
                    Key::Num7 => Some('7'),
                    Key::Num8 => Some('8'),
                    Key::Num9 => Some('9'),
                    _ => None,
                };
                if let Some(c) = digit {
                    if in_call {
                        dtmf(st, c);
                    } else {
                        st.tab.set(Tab::Dial);
                        st.number.update(|n| n.push(c));
                    }
                    return KeyReply::Handled;
                }
                match k {
                    Key::Backspace if !in_call => {
                        st.number.update(|n| {
                            n.pop();
                        });
                        KeyReply::Handled
                    }
                    Key::Enter if !in_call => {
                        dial(st, st.number.get_untracked());
                        KeyReply::Handled
                    }
                    _ => KeyReply::Ignore,
                }
            })
            .child(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(chrome).child(frame)),
    )
}

/// Заголовок окна: знак и название слева, сеть оператора посередине (за неё окно
/// перетаскивается), кнопки окна из настроек рабочего стола справа. Фон — как у экрана,
/// заголовок продолжает его, а не лежит отдельной полосой.
fn titlebar(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![Box::new(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let net = Reactive::new(move || -> Vec<W> {
        let (text, ok) = net_text(st);
        vec![Box::new(Text::new(text).max_lines(1).elide(Elide::End).class(if ok { "net-ok" } else { "net-bad" }))]
    });
    let brand = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new("\u{E0B0}").class("brand-icon")).class("brand"))
        .child(Text::new(t!("Телефон")).max_lines(1).class("brand-title"));
    Box::new(
        DecoratedBox::new()
            .child(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(WindowDragRegion::new().child(DecoratedBox::new().child(brand).class("tb-left")))
                    .child(DecoratedBox::new().class("grow tb-drag").child(WindowDragRegion::new().child(DecoratedBox::new().child(net).class("tb-drag-space"))))
                    .child(DecoratedBox::new().child(controls).class("window-controls")),
            )
            .class("titlebar"),
    )
}

fn main_view(st: St) -> W {
    let content = Stack::new()
        .fit(StackFit::Expand)
        .child(ShowIf::new(0, st.page).child(dial_view(st)))
        .child(ShowIf::new(1, st.page).child(log_view(st)));
    let navbar = Reactive::new(move || -> Vec<W> {
        let cur = st.tab.get();
        let missed = st.log.get().iter().filter(|c| c.new).count();
        let mut row = Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround);
        for (t, glyph, label) in [(Tab::Dial, "\u{E0BC}", t!("Набор")), (Tab::Log, "\u{E889}", t!("Журнал"))] {
            let mut item = Column::new()
                .gap(2.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(Icon::new(glyph).class("nb-icon")).class(if cur == t { "nb-pill nb-pill-on" } else { "nb-pill" }))
                .child(Text::new(label).class("nb-label"));
            if t == Tab::Log && missed > 0 {
                item = item.child(DecoratedBox::new().class("nb-dot"));
            }
            row = row.child(GestureDetector::new().on_click(move || st.tab.set(t)).child(DecoratedBox::new().child(item).class("nb-item")));
        }
        vec![Box::new(row.class("navbar"))]
    });
    // Reactive отдаёт детям свободные ограничения — полную высоту даёт Stack снаружи
    Box::new(
        Column::new()
            .child(network_note(st))
            .child(DecoratedBox::new().child(content).class("grow"))
            .child(navbar),
    )
}

/// Сеть: оператор (и роуминг) или причина, почему её нет; `true` — сеть есть.
fn net_text(st: St) -> (String, bool) {
    match st.status.get() {
        None => (t!("Модем недоступен").to_string(), false),
        Some(s) if !s.present => (t!("Модем не запущен").into(), false),
        Some(s) if !s.radio => (t!("Режим полёта").into(), false),
        Some(s) if !s.registration.registered() => (t!("Нет сети").into(), false),
        Some(s) => {
            let op = if s.operator.is_empty() { s.plmn.clone() } else { s.operator.clone() };
            (if s.roaming { t!("{op} · роуминг", op = op) } else { op }, true)
        }
    }
}

/// Строка сети над экраном набора — на телефоне; в окне с заголовком она в заголовке.
fn network_note(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        if !is_phone() {
            return vec![];
        }
        let (text, ok) = net_text(st);
        vec![Box::new(Row::new().gap(6.0).main_axis_alignment(MainAxisAlignment::Center).child(Text::new(text).class(if ok { "net-ok" } else { "net-bad" })).class("net"))]
    })
}

const KEYS: [(char, &str); 12] = [
    ('1', ""),
    ('2', "ABC"),
    ('3', "DEF"),
    ('4', "GHI"),
    ('5', "JKL"),
    ('6', "MNO"),
    ('7', "PQRS"),
    ('8', "TUV"),
    ('9', "WXYZ"),
    ('*', ""),
    ('0', "+"),
    ('#', ""),
];

fn keypad(on_key: impl Fn(char) + Send + Sync + Clone + 'static, plus: bool) -> impl Widget {
    let mut grid = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for row in KEYS.chunks(3) {
        let mut r = Row::new().gap(18.0);
        for &(c, letters) in row {
            let f = on_key.clone();
            let f2 = on_key.clone();
            let mut g = GestureDetector::new().on_click(move || f(c));
            if c == '0' && plus {
                g = g.on_long_press(move |_| f2('+'));
            }
            r = r.child(
                g.child(
                    DecoratedBox::new()
                        .child(
                            Column::new()
                                .main_axis_alignment(MainAxisAlignment::Center)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(Text::new(c.to_string()).class("key-digit"))
                                .child(Text::new(letters).class("key-letters"))
                                .class("grow"),
                        )
                        .class("key"),
                ),
            );
        }
        grid = grid.child(r);
    }
    grid
}

fn dial_view(st: St) -> W {
    let display = Reactive::new(move || -> Vec<W> {
        let n = st.number.get();
        let mut row = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).main_axis_alignment(MainAxisAlignment::Center);
        row = row.child(DecoratedBox::new().class("display-side"));
        row = row.child(Text::new(if n.is_empty() { " ".into() } else { n.clone() }).max_lines(1).class("display"));
        if n.is_empty() {
            row = row.child(DecoratedBox::new().class("display-side"));
        } else {
            row = row.child(
                GestureDetector::new()
                    .on_click(move || {
                        st.number.update(|n| {
                            n.pop();
                        })
                    })
                    .on_long_press(move |_| st.number.set(String::new()))
                    .child(DecoratedBox::new().child(Icon::new("\u{E14A}").class("bs-icon")).class("display-side")),
            );
        }
        vec![Box::new(row.class("display-row"))]
    });
    let call_btn = GestureDetector::new()
        .on_click(move || dial(st, st.number.get_untracked()))
        .child(DecoratedBox::new().child(Icon::new("\u{E0B0}").class("round-icon")).class("round round-green"));
    Box::new(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(display)
            .child(keypad(move |c| st.number.update(|n| n.push(c)), true))
            .child(DecoratedBox::new().child(call_btn).class("call-row"))
            .class("grow dial"),
    )
}

fn kind_icon(k: CallKind) -> (&'static str, &'static str) {
    match k {
        CallKind::Incoming => ("\u{E0B5}", "log-icon"),
        CallKind::Outgoing => ("\u{E0B2}", "log-icon"),
        CallKind::Missed => ("\u{E0B4}", "log-icon log-missed"),
        CallKind::Rejected => ("\u{E0B1}", "log-icon log-missed"),
    }
}

fn log_view(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let mut log = st.log.get();
        log.sort_by(|a, b| b.time.cmp(&a.time));
        if log.is_empty() {
            return vec![Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new("\u{E889}").class("empty-icon"))
                    .child(Text::new(t!("Звонков пока не было")).class("muted"))
                    .class("empty"),
            )];
        }
        log.into_iter()
            .map(|c| -> W {
                let (glyph, class) = kind_icon(c.kind);
                let sub = match c.kind {
                    CallKind::Missed => t!("пропущенный").to_string(),
                    CallKind::Rejected => t!("отклонён").to_string(),
                    _ if c.duration > 0 => tm::duration(c.duration),
                    CallKind::Outgoing => t!("не ответили").to_string(),
                    CallKind::Incoming => String::new(),
                };
                let n1 = c.number.clone();
                let n2 = c.number.clone();
                let n3 = c.number.clone();
                Box::new(
                    GestureDetector::new()
                        .on_click(move || {
                            st.number.set(n1.clone());
                            st.tab.set(Tab::Dial);
                        })
                        .child(
                            DecoratedBox::new()
                                .child(
                                    Row::new()
                                        .gap(12.0)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .child(Icon::new(glyph).class(class))
                                        .child(
                                            Column::new()
                                                .gap(2.0)
                                                .child(Text::new(if c.number.is_empty() { t!("Скрытый номер").into() } else { c.number.clone() }).max_lines(1).class(if c.kind == CallKind::Missed { "log-name log-name-missed" } else { "log-name" }))
                                                .child(Text::new(format!("{}{}", tm::short(c.time), if sub.is_empty() { String::new() } else { format!(" · {sub}") })).class("log-sub"))
                                                .class("grow"),
                                        )
                                        .child(GestureDetector::new().on_click(move || {
                                            let n = n2.replace('\'', "");
                                            let _ = std::process::Command::new("synsms").args(["--chat", &n]).spawn();
                                        }).child(DecoratedBox::new().child(Icon::new("\u{E0B7}").class("row-btn-icon")).class("row-btn")))
                                        .child(GestureDetector::new().on_click(move || dial(st, n3.clone())).child(DecoratedBox::new().child(Icon::new("\u{E0B0}").class("row-btn-icon")).class("row-btn"))),
                                )
                                .class("log-row"),
                        ),
                )
            })
            .collect()
    });
    Box::new(
        Column::new()
            .child(Text::new(t!("Журнал")).class("title"))
            .child(ScrollView::new().vertical().child(Column::new().gap(2.0).child(list).class("list")).class("grow")),
    )
}

fn dtmf(st: St, c: char) {
    let Some(id) = st.status.get_untracked().and_then(|s| s.calls.iter().find(|c| c.state == CallState::Active).map(|c| c.id)) else {
        return;
    };
    act(st, Request::Dtmf { id, digit: c });
}

/// Команда голосу разговора в palaudiod (сокет управления, группа audio); в фоне.
fn voice_cmd(st: St, cmd: String) {
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};
        let r = (|| -> std::io::Result<String> {
            let mut s = std::os::unix::net::UnixStream::connect("/run/palaudio/control")?;
            s.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            s.write_all(format!("{cmd}\n").as_bytes())?;
            let mut line = String::new();
            BufReader::new(s).read_line(&mut line)?;
            Ok(line)
        })();
        let err = match r {
            Ok(l) if l.contains("\"error\"") => Some(l),
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(e) = err {
            syngui::async_runtime::run_on_main_thread(move || st.toast.set(t!("Звук разговора: {error}", error = e.trim())));
        }
    });
}

fn state_text(c: &Call, now: i64) -> String {
    match c.state {
        CallState::Dialing => t!("Вызов…").into(),
        CallState::Alerting => t!("Гудки…").into(),
        CallState::Incoming => t!("Входящий вызов").into(),
        CallState::Waiting => t!("Второй вызов").into(),
        CallState::Active => tm::duration(c.answered_at.map(|a| (now - a).max(0) as u32).unwrap_or(0)),
        CallState::Held => t!("На удержании").into(),
        CallState::Ended => t!("Завершён").into(),
    }
}

fn round(glyph: &str, class: &str, label: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    Column::new()
        .gap(6.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Icon::new(glyph).class("round-icon")).class(format!("round {class}"))))
        .child(Text::new(syngui::i18n::t(label)).class("round-label"))
}

fn in_call(st: St) -> W {
    let info = Reactive::new(move || -> Vec<W> {
        let now = st.tick.get();
        let Some(s) = st.status.get() else { return vec![] };
        // Главный — входящий/ожидающий, иначе первый
        let Some(c) = s.calls.iter().find(|c| matches!(c.state, CallState::Incoming | CallState::Waiting)).or(s.calls.first()).cloned() else {
            return vec![];
        };
        vec![Box::new(
            Column::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new("\u{E7FD}").class("big-avatar-icon")).class("grow")).class("big-avatar"))
                .child(Text::new(if c.number.is_empty() { t!("Скрытый номер").into() } else { c.number.clone() }).max_lines(1).class("call-number"))
                .child(Text::new(state_text(&c, now)).class("call-state"))
                .class("call-info"),
        )]
    });
    let controls = Reactive::new(move || -> Vec<W> {
        let Some(s) = st.status.get() else { return vec![] };
        let ringing = s.calls.iter().find(|c| c.state == CallState::Incoming).map(|c| c.id);
        let active: Vec<u8> = s.calls.iter().map(|c| c.id).collect();
        if let Some(id) = ringing {
            return vec![Box::new(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::SpaceAround)
                    .child(round("\u{E0B1}", "round-red", n_!("Отклонить"), move || act(st, Request::Hangup { id })))
                    .child(round("\u{E0B0}", "round-green", n_!("Ответить"), move || act(st, Request::Answer { id })))
                    .class("call-buttons"),
            )];
        }
        let open = st.dtmf_open.get();
        let speaker = st.speaker.get();
        let muted = st.muted.get();
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if open {
            col = col.child(keypad(move |c| dtmf(st, c), false));
        }
        let soft = |on: bool| if on { "round-soft round-soft-on" } else { "round-soft" };
        col = col.child(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::SpaceAround)
                .child(round(if muted { "\u{E02B}" } else { "\u{E029}" }, soft(muted), if muted { n_!("Микрофон выкл.") } else { n_!("Микрофон") }, move || {
                    let on = !st.muted.get_untracked();
                    st.muted.set(on);
                    voice_cmd(st, format!("voice-mute {}", if on { "on" } else { "off" }));
                }))
                .child(round("\u{E050}", soft(speaker), n_!("Динамик"), move || {
                    let on = !st.speaker.get_untracked();
                    st.speaker.set(on);
                    voice_cmd(st, format!("voice-route {}", if on { "speaker" } else { "auto" }));
                }))
                .child(round("\u{E0BC}", soft(open), n_!("Клавиши"), move || st.dtmf_open.set(!st.dtmf_open.get_untracked())))
                .class("call-buttons"),
        );
        col = col.child(
            Row::new()
                .main_axis_alignment(MainAxisAlignment::SpaceAround)
                .child(round("\u{E0B1}", "round-red", n_!("Завершить"), move || {
                    for id in active.clone() {
                        act(st, Request::Hangup { id });
                    }
                }))
                .class("call-buttons"),
        );
        vec![Box::new(col)]
    });
    Box::new(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            // Reactive отдаёт детям свободные ограничения — центрирует строка снаружи
            .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(info))
            .child(controls)
            .class("grow in-call"),
    )
}
