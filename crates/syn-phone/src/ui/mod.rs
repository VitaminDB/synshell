//! Интерфейс: заголовок окна (сеть, настройки, кнопки окна), вкладки «Набор / Журнал / Контакты», страницы
//! поверх вкладок (настройки, карточка и редактор контакта, записи), экран звонка поверх всего.

mod call;
mod common;
mod contacts;
mod dial;
mod log;
mod settings;

use syngui::overlay::{ControlsSide, SystemWindowControls, WindowDragRegion, WindowResizeRegion};
use syngui::prelude::*;
use syngui::widgets::{EventHook, KeyReply};
use syngui::{GestureDetector, ShowIf};

use crate::state::{dial as call_number, dtmf, Page, St, Tab};
use common::*;

/// Телефон: окно на весь экран, без рамки и изменения размера.
fn is_phone() -> bool {
    std::env::var("SYNSHELL_FORM_FACTOR").is_ok_and(|v| v.trim().eq_ignore_ascii_case("phone"))
}

fn has_titlebar() -> bool {
    synshell_common::window_controls(false)
}

/// Открыли редактор — черновик из контакта (один раз на открытие).
pub fn effects(st: St) {
    let last = std::cell::RefCell::new(Page::None);
    create_effect(move || {
        let p = st.page.get();
        if *last.borrow() == p {
            return;
        }
        if let Page::Edit(uid, number) = &p {
            contacts::start_edit(st, uid.as_deref(), number);
        }
        *last.borrow_mut() = p;
    });
}

pub fn root(st: St) -> W {
    let pages = Stack::new()
        .fit(StackFit::Expand)
        .child(ShowIf::new(1, st.page_idx).child(DecoratedBox::new().class("page").child(settings::view(st))))
        .child(ShowIf::new(2, st.page_idx).child(DecoratedBox::new().class("page").child(contacts::card_view(st))))
        .child(ShowIf::new(3, st.page_idx).child(DecoratedBox::new().class("page").child(contacts::edit_view(st))))
        .child(ShowIf::new(4, st.page_idx).child(DecoratedBox::new().class("page").child(settings::recordings_view(st))));
    let normal = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(call::return_bar(st))
        .child(DecoratedBox::new().class("grow").child(Stack::new().fit(StackFit::Expand).child(main_view(st)).child(pages)));
    let body = Stack::new().fit(StackFit::Expand).child(ShowIf::new(0, st.mode).child(normal)).child(ShowIf::new(1, st.mode).child(call::view(st)));
    let toast = Column::new()
        .main_axis_alignment(MainAxisAlignment::End)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Reactive::new(move || -> Vec<W> {
            let t = st.toast.get();
            if t.is_empty() {
                return vec![];
            }
            vec![boxed(DecoratedBox::new().child(Text::new(t).max_lines(3).class("toast-text")).class("toast"))]
        }))
        .class("toast-place");
    let chrome = !is_phone();
    let content = Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().child(body).class("root")).child(toast);
    let inner: W = if has_titlebar() {
        boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).child(titlebar(st)).child(DecoratedBox::new().child(content).class("grow")))
    } else {
        boxed(content)
    };
    let frame = DecoratedBox::new().class(if chrome { "window-frame" } else { "window-frame flat" }).clip(true).child(inner);
    boxed(
        EventHook::new()
            .on_key_down(move |k, _| {
                use syngui::input::Key;
                let in_call = st.mode.get_untracked() == 1;
                // поля ввода (поиск, редактор) — свои
                if st.page.get_untracked() != Page::None || (!in_call && st.tab.get_untracked() == Tab::Contacts) {
                    if k == Key::Escape && st.page.get_untracked() != Page::None {
                        st.open(Page::None);
                        return KeyReply::Handled;
                    }
                    return KeyReply::Ignore;
                }
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
                        call_number(st, st.number.get_untracked());
                        KeyReply::Handled
                    }
                    _ => KeyReply::Ignore,
                }
            })
            .child(WindowResizeRegion::new().over_content(true).inset(5.0).enabled(chrome).child(frame)),
    )
}

/// Сеть: оператор (и роуминг) или причина, почему её нет; `true` — сеть есть.
fn net_text(st: St) -> (String, bool) {
    if st.demo && st.status.get().is_none() {
        return (String::new(), true);
    }
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

fn titlebar(st: St) -> W {
    let controls = Reactive::new(move || -> Vec<W> {
        let w = st.window.get();
        vec![boxed(SystemWindowControls::new(ControlsSide::Right).maximized(w.maximized).active(w.focused))]
    });
    let net = Reactive::new(move || -> Vec<W> {
        let (text, ok) = net_text(st);
        vec![boxed(Text::new(text).max_lines(1).elide(Elide::End).class(if ok { "net-ok" } else { "net-bad" }))]
    });
    let brand = Row::new()
        .gap(8.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().child(Icon::new(ic::CALL).class("brand-icon")).class("brand"))
        .child(Text::new(t!("Телефон")).max_lines(1).class("brand-title"));
    boxed(
        DecoratedBox::new()
            .child(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(WindowDragRegion::new().child(DecoratedBox::new().child(brand).class("tb-left")))
                    .child(DecoratedBox::new().class("grow tb-drag").child(WindowDragRegion::new().child(DecoratedBox::new().child(net).class("tb-drag-space"))))
                    .child(DecoratedBox::new().class("tb-btn").child(icon_btn(ic::SETTINGS, "", move || st.open(Page::Settings))))
                    .child(DecoratedBox::new().child(controls).class("window-controls")),
            )
            .class("titlebar"),
    )
}

/// Строка сети и настройки без своего заголовка (Android, iPhone).
fn top_bar(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if has_titlebar() {
            return vec![];
        }
        let (text, ok) = net_text(st);
        vec![boxed(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(text).class(if ok { "net-ok grow" } else { "net-bad grow" }))
                .child(icon_btn(ic::SETTINGS, "", move || st.open(Page::Settings)))
                .class("net"),
        )]
    }))
}

fn main_view(st: St) -> W {
    let content = Stack::new()
        .fit(StackFit::Expand)
        .child(ShowIf::new(0, st.tab_idx).child(dial::view(st)))
        .child(ShowIf::new(1, st.tab_idx).child(log::view(st)))
        .child(ShowIf::new(2, st.tab_idx).child(contacts::list_view(st)));
    let navbar = Reactive::new(move || -> Vec<W> {
        let cur = st.tab.get();
        let missed = st.log.get().iter().filter(|c| c.new).count();
        let mut row = Row::new().main_axis_alignment(MainAxisAlignment::SpaceAround);
        for (t, glyph, label) in [(Tab::Dial, ic::DIALPAD, t!("Набор")), (Tab::Log, ic::HISTORY, t!("Журнал")), (Tab::Contacts, ic::CONTACTS, t!("Контакты"))] {
            let mut inner = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(glyph).class("nb-icon"));
            if t == Tab::Log && missed > 0 {
                inner = inner.child(DecoratedBox::new().class("nb-badge").child(Text::new(missed.min(99).to_string()).class("nb-badge-text")));
            }
            let pill = DecoratedBox::new().child(inner).class(if cur == t { "nb-pill nb-pill-on" } else { "nb-pill" });
            let item = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(pill).child(Text::new(label).class(if cur == t { "nb-label nb-label-on" } else { "nb-label" }));
            row = row.child(GestureDetector::new().on_click(move || st.tab.set(t)).child(DecoratedBox::new().child(item).class("nb-item")));
        }
        vec![boxed(row.class("navbar"))]
    });
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(top_bar(st))
            .child(DecoratedBox::new().child(content).class("grow"))
            .child(navbar),
    )
}
