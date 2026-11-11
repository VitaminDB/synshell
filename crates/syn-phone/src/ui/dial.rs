//! Набор номера: дисплей, подсказки из контактов по набранным цифрам, клавиатура, вызов.

use syngui::prelude::*;
use syngui::GestureDetector;
use synshell_common::contacts;

use super::common::*;
use crate::state::{dial, Page, St};

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

pub fn keypad(on_key: impl Fn(char) + Send + Sync + Clone + 'static, plus: bool, class: &str) -> W {
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
                        .class(format!("key {class}")),
                ),
            );
        }
        grid = grid.child(r);
    }
    boxed(grid)
}

pub fn view(st: St) -> W {
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
                    .child(DecoratedBox::new().child(Icon::new(ic::BACKSPACE).class("bs-icon")).class("display-side")),
            );
        }
        vec![boxed(row.class("display-row"))]
    });
    // подсказки: контакты, в номерах которых есть набранные цифры (или имя совпало по началу)
    let hints = Reactive::new(move || -> Vec<W> {
        let n = st.number.get();
        let d = contacts::digits(&n);
        if d.len() < 2 {
            return vec![boxed(DecoratedBox::new().class("hints-empty"))];
        }
        let all = st.contacts.get();
        let exact = all.iter().find(|c| c.has_number(&n)).cloned();
        let mut found: Vec<(String, String, synshell_common::contacts::Contact)> = Vec::new();
        for c in &all {
            for p in &c.phones {
                if contacts::digits(&p.number).contains(&d) {
                    found.push((c.title(), p.number.clone(), c.clone()));
                    break;
                }
            }
            if found.len() >= 3 {
                break;
            }
        }
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        for (name, number, c) in found {
            let num2 = number.clone();
            col = col.child(
                GestureDetector::new().on_click(move || st.number.set(num2.clone())).child(
                    DecoratedBox::new().class("hint-row").child(
                        Row::new()
                            .gap(10.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(avatar(Some(&c), "av-s"))
                            .child(Column::new().child(Text::new(name).max_lines(1).class("hint-name")).child(Text::new(number).max_lines(1).class("hint-num")).class("grow")),
                    ),
                ),
            );
        }
        if exact.is_none() {
            let num = n.clone();
            col = col.child(GestureDetector::new().on_click(move || st.open(Page::Edit(None, num.clone()))).child(
                DecoratedBox::new().class("hint-row").child(
                    Row::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("av av-s av-add").child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Icon::new(ic::PERSON_ADD).class("av-icon")).class("grow")))
                        .child(Text::new(t!("Добавить в контакты")).class("hint-add grow")),
                ),
            ));
        }
        vec![boxed(DecoratedBox::new().class("hints").child(col))]
    });
    let call_btn = GestureDetector::new().on_click(move || dial(st, st.number.get_untracked())).child(DecoratedBox::new().child(Icon::new(ic::CALL).class("round-icon")).class("round round-green"));
    boxed(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::End)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow"))
            .child(hints)
            .child(display)
            .child(keypad(move |c| st.number.update(|n| n.push(c)), true, ""))
            .child(DecoratedBox::new().child(call_btn).class("call-row"))
            .class("grow dial"),
    )
}
