//! synsms — «Сообщения»: переписка по SMS через демон модема `synmodemd`. Телефон — стек «переписки →
//! чат», рабочий стол — две колонки. `synsms --chat НОМЕР` открывает переписку, `--new` — новое сообщение;
//! `--screenshot СНИМОК.png [--size ШxВ] [--scale N]` — снимок окна с придуманной перепиской (без модема).

use std::time::Duration;

use syn_modem::api::{self, Event, Request, Sms, SmsStatus, Status};
use syn_modem::time as tm;
use synshell_common::Config;
use syngui::async_runtime::run_on_main_thread;
use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::GestureDetector;

type W = Box<dyn Widget>;

#[derive(Clone, Copy)]
struct St {
    msgs: RwSignal<Vec<Sms>>,
    /// Открытая переписка (номер как в последнем сообщении).
    chat: RwSignal<Option<String>>,
    composing: RwSignal<bool>,
    draft: RwSignal<String>,
    new_number: RwSignal<String>,
    toast: RwSignal<String>,
    status: RwSignal<Option<Status>>,
    /// Ждёт подтверждения удаления переписки.
    confirm_delete: RwSignal<bool>,
    /// Демон недоступен (причина).
    offline: RwSignal<Option<String>>,
    /// Состояние окна (фокус, развёрнуто) — для своих кнопок окна.
    window: RwSignal<syngui::window::WindowState>,
}

/// Телефон: окно на весь экран, без своих кнопок окна и рамки.
fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v.trim().eq_ignore_ascii_case("phone"))
}

/// Ключ переписки: последние 10 цифр номера («+7701…» и «8701…» — одна переписка), буквенный — как есть.
fn key(number: &str) -> String {
    let digits: String = number.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 10 {
        digits[digits.len() - 10..].to_string()
    } else if digits.is_empty() {
        number.to_lowercase()
    } else {
        digits
    }
}

struct Conv {
    number: String,
    last: Sms,
    unread: usize,
}

fn conversations(msgs: &[Sms]) -> Vec<Conv> {
    let mut out: Vec<Conv> = Vec::new();
    for m in msgs {
        let k = key(&m.number);
        match out.iter_mut().find(|c| key(&c.number) == k) {
            Some(c) => {
                if m.time >= c.last.time {
                    c.last = m.clone();
                    c.number = m.number.clone();
                }
                c.unread += usize::from(m.incoming && !m.read);
            }
            None => out.push(Conv { number: m.number.clone(), last: m.clone(), unread: usize::from(m.incoming && !m.read) }),
        }
    }
    out.sort_by(|a, b| b.last.time.cmp(&a.last.time));
    out
}

fn main() {
    synshell_common::i18n::init(&synshell_common::catalogs!("../i18n"));
    let mut chat: Option<String> = None;
    let mut new = false;
    let mut shot: Option<String> = None;
    let mut size = (430u32, 900u32);
    let mut scale = 2.0f64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("synsms [--chat NUMBER | --new] [--screenshot FILE.png [--size WxH] [--scale N]]");
                return;
            }
            "--chat" => chat = args.next(),
            "--new" => new = true,
            "--screenshot" => shot = args.next(),
            "--size" => {
                if let Some((w, h)) = args.next().as_deref().and_then(|s| s.split_once('x')) {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
            }
            "--scale" => scale = args.next().and_then(|s| s.parse().ok()).unwrap_or(scale),
            _ => {}
        }
    }
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    if let Some(out) = shot {
        // Снимок с придуманной перепиской (без модема); `--chat` — открыть переписку с этим номером
        let r = syngui::shot::render_png(
            move || {
                let st = new_st(chat, new);
                demo(st);
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
        .title(t!("Сообщения"))
        .app_id("synsms")
        .size(960, 680)
        .min_size(340, 480)
        .frameless()
        .transparent(true)
        .background(Color::from_srgb(0, 0, 0, 0.0))
        .with_window_state(win)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_live_styles(synshell_common::live::styles(mss, theme))
        .run(move |_| {
            let st = new_st(chat.clone(), new);
            st.window.set(win.get_untracked());
            create_effect(move || st.window.set(win.get()));
            watch(st);
            mark_read_effect(st);
            root(st)
        });
}

fn new_st(chat: Option<String>, new: bool) -> St {
    St {
        msgs: use_signal(Vec::new()),
        chat: use_signal(chat),
        composing: use_signal(new),
        draft: use_signal(String::new()),
        new_number: use_signal(String::new()),
        toast: use_signal(String::new()),
        status: use_signal(None),
        confirm_delete: use_signal(false),
        offline: use_signal(None),
        window: use_signal(syngui::window::WindowState::default()),
    }
}

/// Придуманная переписка для снимков (`--screenshot`).
fn demo(st: St) {
    let now = tm::now();
    let mut status = Status { present: true, radio: true, operator: "Demo Mobile".into(), ..Default::default() };
    status.registration = syn_modem::api::Registration::Home;
    st.status.set(Some(status));
    let ru = syngui::i18n::language().base() == "ru";
    let tx = |en: &'static str, r: &'static str| if ru { r } else { en };
    let mut id = 0u64;
    let mut m = |number: &str, text: &str, ago: i64, incoming: bool, read: bool| {
        id += 1;
        Sms { id, number: number.into(), text: text.into(), time: now - ago, incoming, read, status: if incoming { SmsStatus::Received } else { SmsStatus::Sent } }
    };
    let alex = "+1 555 0142";
    let msgs = vec![
        m(alex, tx("Hi! Are we still on for the hike on Saturday?", "Привет! В субботу идём в поход?"), 2 * 86400 + 3600, true, true),
        m(alex, tx("Yes, meet at the station at 8:00.", "Да, встречаемся на вокзале в 8:00."), 2 * 86400 + 3000, false, true),
        m(alex, tx("Great. I'll bring the tent.", "Отлично. Палатку возьму я."), 2 * 86400 + 2900, true, true),
        m(alex, tx("The forecast says sunny all day", "Обещают солнце весь день"), 50 * 60, true, true),
        m(alex, tx("Perfect, don't forget water and snacks", "Отлично, не забудь воду и перекус"), 45 * 60, false, true),
        m(alex, tx("Already packed. See you tomorrow!", "Уже собрал. До завтра!"), 12 * 60, true, false),
        m("+1 555 0198", tx("Your table for two is booked for 19:30.", "Столик на двоих забронирован на 19:30."), 3 * 3600, true, false),
        m("+1 555 0117", tx("Thanks, got the files.", "Спасибо, файлы получил."), 27 * 3600, true, true),
        m("+1 555 0117", tx("Sending the draft tonight.", "Черновик пришлю вечером."), 28 * 3600, false, true),
        m("DemoBank", tx("Code 4821 to confirm your sign-in. Do not share it.", "Код 4821 для входа. Никому не сообщайте."), 3 * 86400, true, true),
        m("+1 555 0163", tx("Happy birthday! 🎉", "С днём рождения! 🎉"), 5 * 86400, true, true),
        m("+1 555 0129", tx("Running 10 minutes late", "Опаздываю на 10 минут"), 9 * 86400, false, true),
    ];
    // Открытая переписка прочитана (как после SmsRead у демона)
    let open = st.chat.get_untracked().map(|c| key(&c));
    let msgs = msgs.into_iter().map(|mut m| {
        if open.as_deref() == Some(key(&m.number).as_str()) {
            m.read = true;
        }
        m
    });
    st.msgs.set(msgs.collect());
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let p = a.palette();
    let dark = a.is_dark();
    synshell_common::style::AppStyle::new(cfg, "synsms")
        .vars(&[
            ("input-bg", if dark { p.bg.mix(p.surface, 0.35) } else { p.surface }.hex()),
            ("bubble-in", if dark { p.surface.mix(p.fg, 0.08) } else { p.surface.mix(p.fg, 0.06) }.hex()),
        ])
        .base(include_str!("../styles/synsms.mss"))
        .build()
}

// ─── Данные ─────────────────────────────────────────────────────────────────

fn reload(st: St) {
    std::thread::spawn(move || {
        let r = api::sms_list();
        run_on_main_thread(move || match r {
            Ok(list) => {
                st.msgs.set(list);
                st.offline.set(None);
            }
            Err(e) => st.offline.set(Some(format!("{e:#}"))),
        });
    });
}

/// Поток событий демона; демон перезапустился — переподключиться.
fn watch(st: St) {
    std::thread::spawn(move || loop {
        let r = api::subscribe(|ev| {
            match ev {
                Event::Status { status } => {
                    // Первое событие подписки — заодно перечитать список (демон мог смениться)
                    let first = status.present;
                    run_on_main_thread(move || {
                        let fresh = st.status.get_untracked().is_none();
                        st.status.set(Some(status));
                        if fresh || !first {
                            reload(st);
                        }
                    });
                }
                Event::Sms { message } => run_on_main_thread(move || {
                    st.msgs.update(|l| match l.iter_mut().find(|m| m.id == message.id) {
                        Some(m) => *m = message,
                        None => l.push(message),
                    });
                }),
                Event::SmsChanged => run_on_main_thread(move || reload(st)),
                Event::CallLog | Event::Ussd { .. } | Event::EsimProgress { .. } => {}
            }
            true
        });
        let msg = match r {
            Ok(()) => t!("synmodemd остановлен").to_string(),
            Err(e) => format!("{e:#}"),
        };
        run_on_main_thread(move || {
            st.offline.set(Some(msg));
            st.status.set(None);
        });
        std::thread::sleep(Duration::from_secs(3));
    });
}

/// Открытая переписка с непрочитанными — отметить прочитанной.
fn mark_read_effect(st: St) {
    create_effect(move || {
        let Some(chat) = st.chat.get() else { return };
        let k = key(&chat);
        let numbers: Vec<String> = st
            .msgs
            .get()
            .iter()
            .filter(|m| m.incoming && !m.read && key(&m.number) == k)
            .map(|m| m.number.clone())
            .collect();
        if numbers.is_empty() {
            return;
        }
        std::thread::spawn(move || {
            let mut seen = std::collections::HashSet::new();
            for n in numbers {
                if seen.insert(n.clone()) {
                    let _ = api::request(&Request::SmsRead { number: n });
                }
            }
        });
    });
}

fn send(st: St, number: String) {
    let text = st.draft.get_untracked().trim().to_string();
    let number = number.trim().to_string();
    if text.is_empty() {
        return;
    }
    if number.is_empty() {
        st.toast.set(t!("Укажите номер").into());
        return;
    }
    st.draft.set(String::new());
    if st.composing.get_untracked() {
        st.composing.set(false);
        st.new_number.set(String::new());
        st.chat.set(Some(number.clone()));
    }
    std::thread::spawn(move || {
        if let Err(e) = api::request(&Request::SmsSend { number, text }) {
            let m = t!("Не отправлено: {error}", error = format!("{e:#}"));
            run_on_main_thread(move || st.toast.set(m));
        }
    });
}

fn delete_chat(st: St, number: &str) {
    let k = key(number);
    let ids: Vec<u64> = st.msgs.get_untracked().iter().filter(|m| key(&m.number) == k).map(|m| m.id).collect();
    st.confirm_delete.set(false);
    st.chat.set(None);
    std::thread::spawn(move || {
        if let Err(e) = api::request(&Request::SmsDelete { ids }) {
            let m = t!("Не удалено: {error}", error = format!("{e:#}"));
            run_on_main_thread(move || st.toast.set(m));
        }
    });
}

// ─── Интерфейс ──────────────────────────────────────────────────────────────

fn go_back(st: St) -> bool {
    if st.confirm_delete.get_untracked() {
        st.confirm_delete.set(false);
        true
    } else if st.composing.get_untracked() {
        st.composing.set(false);
        true
    } else if st.chat.get_untracked().is_some() {
        st.chat.set(None);
        true
    } else {
        false
    }
}

fn root(st: St) -> W {
    let narrow = syngui::viewport::viewport_below(720.0);
    let body = Reactive::new(move || -> Vec<W> { vec![if narrow.get() { phone(st) } else { desktop(st) }] });
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
    // Своя рамка окна; кнопки окна — в верхних панелях экранов (см. `bar_end`)
    let chrome = !is_phone();
    let frame = DecoratedBox::new()
        .class(if chrome { "window-frame" } else { "window-frame flat" })
        .clip(true)
        .child(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().child(body).class("root")).child(toast));
    Box::new(
        EventHook::new()
            .on_key_down(move |k, _| {
                if matches!(k, syngui::input::Key::Escape) && go_back(st) {
                    KeyReply::Handled
                } else {
                    KeyReply::Ignore
                }
            })
            .child(
                GestureDetector::new()
                    .on_back(move || go_back(st))
                    .child(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(chrome).child(frame)),
            ),
    )
}

/// Кнопки окна в конце верхней панели (на телефоне их нет).
fn bar_end(st: St, row: Row) -> Row {
    if is_phone() {
        return row;
    }
    row.child(Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![Box::new(DecoratedBox::new().child(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused)).class("window-controls"))]
    }))
}

/// Заголовок панели, за который перетаскивается окно (на телефоне — просто текст).
fn drag_title(w: impl Widget + 'static) -> W {
    if is_phone() {
        return Box::new(DecoratedBox::new().child(w).class("grow"));
    }
    Box::new(DecoratedBox::new().class("grow drag-box").child(WindowDragRegion::new().child(DecoratedBox::new().child(w).class("drag-fill"))))
}

fn phone(st: St) -> W {
    Box::new(Reactive::new(move || -> Vec<W> {
        let screen = if st.composing.get() {
            3u64
        } else if st.chat.get().is_some() {
            2
        } else {
            1
        };
        vec![Box::new(
            AnimatedSwitcher::new(screen, move || -> W {
                if st.composing.get_untracked() {
                    new_view(st, true)
                } else if let Some(n) = st.chat.get_untracked() {
                    chat_view(st, n, true)
                } else {
                    list_view(st, true)
                }
            })
            .directional(true)
            .slide(48.0, 0.0)
            .duration_ms(220)
            .animate_size(false)
            .class("grow"),
        )]
    }))
}

fn desktop(st: St) -> W {
    let right = Reactive::new(move || -> Vec<W> {
        let v: W = if st.composing.get() {
            new_view(st, false)
        } else if let Some(n) = st.chat.get() {
            chat_view(st, n, false)
        } else {
            // Пустая правая колонка: панель только с кнопками окна, чтобы окно было за что двигать
            let mut stack = Stack::new().fit(StackFit::Expand).child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .gap(8.0)
                    .child(Icon::new("\u{E0B7}").class("empty-icon"))
                    .child(Text::new(t!("Выберите переписку")).class("muted")),
            );
            if !is_phone() {
                stack = stack.child(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(bar_end(st, Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(drag_title(DecoratedBox::new()))).class("bar")));
            }
            Box::new(stack)
        };
        vec![v]
    });
    Box::new(
        Row::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().child(list_view(st, false)).class("side"))
            .child(DecoratedBox::new().child(right).class("grow main")),
    )
}

fn icon_btn(glyph: &str, f: impl Fn() + Send + Sync + 'static) -> impl Widget {
    GestureDetector::new().on_click(f).child(DecoratedBox::new().child(Icon::new(glyph).class("btn-icon")).class("icon-btn"))
}

/// Строка состояния сети под заголовком (нет сети / нет демона), иначе ничего.
fn network_note(st: St) -> impl Widget {
    Reactive::new(move || -> Vec<W> {
        let text = if let Some(e) = st.offline.get() {
            Some(t!("Модем недоступен: {error}", error = e))
        } else {
            st.status.get().and_then(|s| {
                if !s.present {
                    Some(t!("Модем не запущен").into())
                } else if !s.radio {
                    Some(t!("Режим полёта: отправка недоступна").into())
                } else if !s.registration.registered() {
                    Some(t!("Нет сети: сообщения уйдут, когда она появится").into())
                } else {
                    None
                }
            })
        };
        match text {
            Some(t) => vec![Box::new(DecoratedBox::new().child(Text::new(t).max_lines(2).class("note-text")).class("note"))],
            None => vec![],
        }
    })
}

fn avatar(number: &str) -> impl Widget {
    // Буквенный отправитель — первая буква, номер — значок человека
    let first = number.chars().find(|c| c.is_alphabetic()).map(|c| c.to_uppercase().to_string());
    let inner: W = match first {
        Some(c) => Box::new(Text::new(c).class("avatar-text")),
        None => Box::new(Icon::new("\u{E7FD}").class("avatar-icon")),
    };
    DecoratedBox::new()
        .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(inner).class("grow"))
        .class("avatar")
}

/// Список переписок; `alone` — список занимает окно один (узкое окно): в его панели кнопки окна.
fn list_view(st: St, alone: bool) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let convs = conversations(&st.msgs.get());
        let open = st.chat.get().map(|c| key(&c));
        if convs.is_empty() {
            return vec![Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new("\u{E0B7}").class("empty-icon"))
                    .child(Text::new(t!("Сообщений пока нет")).class("muted"))
                    .class("empty"),
            )];
        }
        convs
            .into_iter()
            .map(|c| -> W {
                let number = c.number.clone();
                let on = open.as_deref() == Some(key(&c.number).as_str());
                let last = c.last.text.replace('\n', " ");
                let last = if c.last.incoming { last } else { t!("Вы: {text}", text = last) };
                let mut right = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::End).child(Text::new(tm::short(c.last.time)).class("conv-time"));
                if c.unread > 0 {
                    right = right.child(DecoratedBox::new().child(Text::new(c.unread.to_string()).class("badge-text")).class("badge"));
                }
                Box::new(
                    GestureDetector::new()
                        .on_click(move || {
                            st.composing.set(false);
                            st.confirm_delete.set(false);
                            st.chat.set(Some(number.clone()));
                        })
                        .child(
                            DecoratedBox::new()
                                .child(
                                    Row::new()
                                        .gap(12.0)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .child(avatar(&c.number))
                                        .child(
                                            Column::new()
                                                .gap(2.0)
                                                .child(Text::new(c.number.clone()).max_lines(1).class(if c.unread > 0 { "conv-name conv-unread" } else { "conv-name" }))
                                                .child(Text::new(last).max_lines(1).class("conv-last"))
                                                .class("grow"),
                                        )
                                        .child(right),
                                )
                                .class(if on { "conv conv-on" } else { "conv" }),
                        ),
                )
            })
            .collect()
    });
    let fab = GestureDetector::new()
        .on_click(move || {
            st.chat.set(None);
            st.composing.set(true);
        })
        .child(DecoratedBox::new().child(Icon::new("\u{E0C9}").class("fab-icon")).class("fab"));
    let fab_place = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::End)
        .child(fab)
        .class("fab-place");
    Box::new(
        Stack::new()
            .fit(StackFit::Expand)
            .child(
                Column::new()
                    .child({
                        let row = Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(drag_title(Text::new(t!("Сообщения")).max_lines(1).class("title")));
                        if alone { bar_end(st, row) } else { row }.class("bar")
                    })
                    .child(network_note(st))
                    .child(ScrollView::new().vertical().child(Column::new().gap(2.0).child(list).class("list")).class("grow")),
            )
            .child(fab_place),
    )
}

fn composer(st: St, number: impl Fn() -> String + Send + Sync + Clone + 'static) -> impl Widget {
    // Строка целиком в Reactive: поле — прямой flex-ребёнок Row (у Reactive с .grow поле
    // оставалось натуральной ширины, кнопка «Отправить» прилипала к нему посередине строки)
    Reactive::new(move || -> Vec<W> {
        let n1 = number.clone();
        let n2 = number.clone();
        let draft = st.draft.get();
        let can = !draft.trim().is_empty();
        vec![Box::new(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(
                    TextField::new()
                        .text(draft)
                        .placeholder(t!("Сообщение"))
                        .on_change(move |t| st.draft.set(t.to_string()))
                        .on_submit(move |_| send(st, n1()))
                        .class("input grow"),
                )
                .child(
                    GestureDetector::new()
                        .on_click(move || send(st, n2()))
                        .child(DecoratedBox::new().child(Icon::new("\u{E163}").class("send-icon")).class(if can { "send send-on" } else { "send" })),
                )
                .class("composer"),
        )]
    })
}

fn chat_view(st: St, number: String, phone: bool) -> W {
    let k = key(&number);
    let title = number.clone();
    let n_del = number.clone();
    let confirm_bar = Reactive::new(move || -> Vec<W> {
        if st.confirm_delete.get() {
            let n = n_del.clone();
            let row = Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(drag_title(Text::new(t!("Удалить переписку?")).max_lines(1).class("bar-title")))
                .child(GestureDetector::new().on_click(move || st.confirm_delete.set(false)).child(DecoratedBox::new().child(Text::new(t!("Отмена")).class("btn-text")).class("btn")))
                .child(GestureDetector::new().on_click(move || delete_chat(st, &n)).child(DecoratedBox::new().child(Text::new(t!("Удалить")).class("btn-text btn-danger-text")).class("btn btn-danger")));
            return vec![Box::new(bar_end(st, row).class("bar"))];
        }
        vec![]
    });
    let normal_bar = Reactive::new({
        let title = title.clone();
        move || -> Vec<W> {
            if st.confirm_delete.get() {
                return vec![];
            }
            let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if phone {
                row = row.child(icon_btn("\u{E5C4}", move || st.chat.set(None)));
            }
            let row = row
                .child(avatar(&title))
                .child(drag_title(Text::new(title.clone()).max_lines(1).class("bar-title")))
                .child(icon_btn("\u{E872}", move || st.confirm_delete.set(true)));
            vec![Box::new(bar_end(st, row).class("bar"))]
        }
    });
    let msgs = Reactive::new(move || -> Vec<W> {
        let mut list: Vec<Sms> = st.msgs.get().into_iter().filter(|m| key(&m.number) == k).collect();
        list.sort_by_key(|m| (m.time, m.id));
        let mut out: Vec<W> = Vec::new();
        let mut day = i64::MIN;
        for m in list {
            let d = tm::local(m.time).days;
            if d != day {
                day = d;
                out.push(Box::new(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(Text::new(tm::day_title(m.time)).class("day")).class("day-row")));
            }
            out.push(bubble(&m));
        }
        out
    });
    let num_for_send = number.clone();
    Box::new(
        Column::new()
            .child(confirm_bar)
            .child(normal_bar)
            .child(network_note(st))
            .child(ScrollView::new().vertical().follow_end(true).child(Column::new().gap(6.0).child(msgs).class("thread")).class("grow"))
            .child(composer(st, move || num_for_send.clone())),
    )
}

fn bubble(m: &Sms) -> W {
    let status = match m.status {
        SmsStatus::Sending => format!(" · {}", t!("отправка…")),
        SmsStatus::Failed => format!(" · {}", t!("не отправлено")),
        _ => String::new(),
    };
    let body = Column::new()
        .gap(4.0)
        .child(Text::new(m.text.clone()).class(if m.incoming { "bubble-text" } else { "bubble-text bubble-text-out" }))
        .child(Text::new(format!("{}{status}", tm::hm(m.time))).class(if m.incoming { "bubble-meta" } else { "bubble-meta bubble-meta-out" }));
    let class = match (m.incoming, m.status) {
        (true, _) => "bubble bubble-in",
        (false, SmsStatus::Failed) => "bubble bubble-out bubble-failed",
        (false, _) => "bubble bubble-out",
    };
    Box::new(
        Row::new()
            .main_axis_alignment(if m.incoming { MainAxisAlignment::Start } else { MainAxisAlignment::End })
            .child(DecoratedBox::new().child(body).class(class)),
    )
}

fn new_view(st: St, phone: bool) -> W {
    let mut bar = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if phone {
        bar = bar.child(icon_btn("\u{E5C4}", move || st.composing.set(false)));
    }
    let bar = bar_end(st, bar.child(drag_title(Text::new(t!("Новое сообщение")).max_lines(1).class("bar-title")))).class("bar");
    let number = Reactive::new(move || -> Vec<W> {
        vec![Box::new(
            TextField::new()
                .text(st.new_number.get())
                .placeholder(t!("Номер телефона"))
                .prefix_icon("\u{E0CD}")
                .autofocus(true)
                .input_filter(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')'))
                .on_change(move |t| st.new_number.set(t.to_string()))
                .class("input"),
        )]
    });
    Box::new(
        Column::new()
            .child(bar)
            .child(network_note(st))
            .child(DecoratedBox::new().child(number).class("to-row"))
            .child(DecoratedBox::new().class("grow"))
            .child(composer(st, move || st.new_number.get_untracked())),
    )
}
