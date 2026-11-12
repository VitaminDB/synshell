//! Окно USSD поверх приложения: код, ответ сети, ожидание; меню (сеть ждёт ответа) — поле и «Отправить»;
//! «Отмена» прерывает сеанс в сети, «Закрыть» — после его конца. `*#06#` — IMEI без сети.

use syngui::prelude::*;
use syngui::{GestureDetector, ScrollView};

use super::common::*;
use crate::state::{ussd_close, ussd_reply, St};

pub fn view(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let Some(u) = st.ussd.get() else { return vec![] };
        let title = if u.code.is_empty() { t!("Сообщение сети").to_string() } else { u.code.clone() };
        let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).max_lines(1).class("ussd-title"));
        if u.waiting {
            col = col.child(Text::new(t!("Запрос…")).class("muted"));
        }
        if !u.text.is_empty() {
            col = col.child(ScrollView::new().vertical().child(Text::new(u.text.clone()).class("ussd-text")).class("ussd-scroll"));
        }
        if u.reply {
            // Ответ — своими клавишами: экранная клавиатура над окном «Телефона» сама не выезжает, а набор
            // номера закрыт затемнением. Строка ответа следит только за своим сигналом.
            let answer = Reactive::new(move || -> Vec<W> {
                let a = st.ussd_input.get();
                let (text, class) = if a.is_empty() { (t!("Ответ").to_string(), "ussd-answer ussd-answer-empty") } else { (a, "ussd-answer") };
                vec![boxed(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Text::new(text).max_lines(1).class(class))
                        .child(DecoratedBox::new().class("grow"))
                        .child(
                            GestureDetector::new()
                                .on_click(move || {
                                    st.ussd_input.update(|s| {
                                        s.pop();
                                    })
                                })
                                .on_long_press(move |_| st.ussd_input.set(String::new()))
                                .child(Icon::new(ic::BACKSPACE).class("bs-icon")),
                        )
                        .class("ussd-answer-row"),
                )]
            });
            col = col.child(answer).child(super::dial::keypad(move |c| st.ussd_input.update(|s| s.push(c)), false, "key-ussd"));
        }
        let mut buttons = Row::new().gap(8.0).main_axis_alignment(MainAxisAlignment::End);
        if u.reply {
            // текстовый ответ — экранная клавиатура (буквы идут в окно, его поле — строка ответа выше)
            buttons = buttons.child(boxed(
                GestureDetector::new()
                    .on_click(move || keyboard(st))
                    .child(DecoratedBox::new().class("ussd-btn").child(Icon::new("\u{E312}").class("ussd-kbd-icon"))),
            ));
            buttons = buttons.child(DecoratedBox::new().class("grow"));
        }
        buttons = buttons.child(button(if u.done { t!("Закрыть") } else { t!("Отмена") }, "ussd-btn", move || ussd_close(st)));
        if u.reply {
            buttons = buttons.child(button(t!("Отправить"), "ussd-btn ussd-btn-main", move || ussd_reply(st)));
        }
        col = col.child(buttons);
        vec![boxed(
            DecoratedBox::new()
                .class("ussd-dim")
                .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("ussd-card").child(col)).class("grow")),
        )]
    }))
}

fn button(label: impl Into<String>, class: &'static str, f: impl Fn() + Send + Sync + 'static) -> W {
    boxed(GestureDetector::new().on_click(move || f()).child(DecoratedBox::new().class(class).child(Text::new(label.into()).class("ussd-btn-text"))))
}

/// Экранная клавиатура для текстового ответа: synkeyboard шлёт буквы нам в сокет (`show float for synphone-ussd.sock`),
/// фокус окна не нужен.
fn keyboard(st: St) {
    crate::ipc::ussd_keys(st);
    if let Ok(mut c) = std::process::Command::new("synkeyboard").args(["show", "float", "for", "synphone-ussd.sock"]).spawn() {
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
}
