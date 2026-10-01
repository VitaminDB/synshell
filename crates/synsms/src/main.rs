//! synsms — «Сообщения»: переписка по SMS через демон модема `synmodemd`. Телефон — стек «переписки →
//! чат», рабочий стол — две колонки. `synsms --chat НОМЕР` открывает переписку, `--new` — новое сообщение.

use std::time::Duration;

use synmodem::api::{self, Event, Request, Sms, SmsStatus, Status};
use synmodem::time as tm;
use synshell_common::Config;
use syngui::async_runtime::run_on_main_thread;
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let chat = args.iter().position(|a| a == "--chat").and_then(|i| args.get(i + 1)).cloned();
    let new = args.iter().any(|a| a == "--new");
    let (cfg, _) = Config::load();
    let mss = theme(&cfg);
    App::new()
        .title("Сообщения")
        .app_id("synsms")
        .size(960, 680)
        .min_size(340, 480)
        .with_icon_font(syngui::text::icon_fonts::material::FONT_DATA)
        .with_styles_str(&mss)
        .run(move |_| {
            let st = St {
                msgs: use_signal(Vec::new()),
                chat: use_signal(chat.clone()),
                composing: use_signal(new),
                draft: use_signal(String::new()),
                new_number: use_signal(String::new()),
                toast: use_signal(String::new()),
                status: use_signal(None),
                confirm_delete: use_signal(false),
                offline: use_signal(None),
            };
            watch(st);
            mark_read_effect(st);
            root(st)
        });
}

fn theme(cfg: &Config) -> String {
    let a = &cfg.appearance;
    let mut s = a.mss_variables();
    let p = a.palette();
    let dark = a.is_dark();
    s.push_str(&format!(
        ":root {{\n  --input-bg: {};\n  --bubble-in: {};\n}}\n",
        if dark { p.bg.mix(p.surface, 0.35) } else { p.surface }.hex(),
        if dark { p.surface.mix(p.fg, 0.08) } else { p.surface.mix(p.fg, 0.06) }.hex(),
    ));
    s.push_str(&a.theme_mss_variables());
    s.push_str(include_str!("../styles/synsms.mss"));
    if !a.font.trim().is_empty() {
        s.push_str(&format!("Text, Button, TextField {{ font-family: \"{}\"; }}\n", a.font.trim()));
    }
    s
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
                Event::CallLog => {}
            }
            true
        });
        let msg = match r {
            Ok(()) => "synmodemd остановлен".to_string(),
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
        st.toast.set("Укажите номер".into());
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
            let m = format!("Не отправлено: {e:#}");
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
            let m = format!("Не удалено: {e:#}");
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
                    .child(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().child(body).class("root")).child(toast)),
            ),
    )
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
                    list_view(st)
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
            Box::new(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .gap(8.0)
                    .child(Icon::new("\u{E0B7}").class("empty-icon"))
                    .child(Text::new("Выберите переписку").class("muted"))
                    .class("grow"),
            )
        };
        vec![v]
    });
    Box::new(
        Row::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(DecoratedBox::new().child(list_view(st)).class("side"))
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
            Some(format!("Модем недоступен: {e}"))
        } else {
            st.status.get().and_then(|s| {
                if !s.present {
                    Some("Модем не запущен".into())
                } else if !s.radio {
                    Some("Режим полёта: отправка недоступна".into())
                } else if !s.registration.registered() {
                    Some("Нет сети: сообщения уйдут, когда она появится".into())
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

fn list_view(st: St) -> W {
    let list = Reactive::new(move || -> Vec<W> {
        let convs = conversations(&st.msgs.get());
        let open = st.chat.get().map(|c| key(&c));
        if convs.is_empty() {
            return vec![Box::new(
                Column::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Icon::new("\u{E0B7}").class("empty-icon"))
                    .child(Text::new("Сообщений пока нет").class("muted"))
                    .class("empty"),
            )];
        }
        convs
            .into_iter()
            .map(|c| -> W {
                let number = c.number.clone();
                let on = open.as_deref() == Some(key(&c.number).as_str());
                let prefix = if c.last.incoming { "" } else { "Вы: " };
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
                                                .child(Text::new(format!("{prefix}{}", c.last.text.replace('\n', " "))).max_lines(1).class("conv-last"))
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
                    .child(Row::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new("Сообщения").class("title grow")).class("bar"))
                    .child(network_note(st))
                    .child(ScrollView::new().vertical().child(Column::new().gap(2.0).child(list).class("list")).class("grow")),
            )
            .child(fab_place),
    )
}

fn composer(st: St, number: impl Fn() -> String + Send + Sync + Clone + 'static) -> impl Widget {
    let n2 = number.clone();
    Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let number = number.clone();
            vec![Box::new(
                TextField::new()
                    .text(st.draft.get())
                    .placeholder("Сообщение")
                    .on_change(move |t| st.draft.set(t.to_string()))
                    .on_submit(move |_| send(st, number()))
                    .class("input grow"),
            )]
        }).class("grow"))
        .child(Reactive::new(move || -> Vec<W> {
            let n2 = n2.clone();
            let can = !st.draft.get().trim().is_empty();
            vec![Box::new(
                GestureDetector::new()
                    .on_click(move || send(st, n2()))
                    .child(DecoratedBox::new().child(Icon::new("\u{E163}").class("send-icon")).class(if can { "send send-on" } else { "send" })),
            )]
        }))
        .class("composer")
}

fn chat_view(st: St, number: String, phone: bool) -> W {
    let k = key(&number);
    let title = number.clone();
    let n_del = number.clone();
    let confirm_bar = Reactive::new(move || -> Vec<W> {
        if st.confirm_delete.get() {
            let n = n_del.clone();
            return vec![Box::new(
                Row::new()
                    .gap(8.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new("Удалить переписку?").class("bar-title grow"))
                    .child(GestureDetector::new().on_click(move || st.confirm_delete.set(false)).child(DecoratedBox::new().child(Text::new("Отмена").class("btn-text")).class("btn")))
                    .child(GestureDetector::new().on_click(move || delete_chat(st, &n)).child(DecoratedBox::new().child(Text::new("Удалить").class("btn-text btn-danger-text")).class("btn btn-danger")))
                    .class("bar"),
            )];
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
            vec![Box::new(
                row.child(avatar(&title))
                    .child(Text::new(title.clone()).max_lines(1).class("bar-title grow"))
                    .child(icon_btn("\u{E872}", move || st.confirm_delete.set(true)))
                    .class("bar"),
            )]
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
        SmsStatus::Sending => " · отправка…",
        SmsStatus::Failed => " · не отправлено",
        _ => "",
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
    let bar = bar.child(Text::new("Новое сообщение").class("bar-title grow")).class("bar");
    let number = Reactive::new(move || -> Vec<W> {
        vec![Box::new(
            TextField::new()
                .text(st.new_number.get())
                .placeholder("Номер телефона")
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
