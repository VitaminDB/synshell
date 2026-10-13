//! Карточка дня: выезжает снизу поверх затемнения — месячные и их обильность,
//! симптомы, настроение.

use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::StyleValue;

use super::{boxed, icon_button, W};
use crate::date::{self, Day};
use crate::icons;
use crate::model::{Kind, FLOWS, MOODS, SYMPTOMS};
use crate::state::St;

fn status(st: St, d: Day) -> (String, &'static str) {
    let fc = st.fc.get_untracked();
    match fc.kind(d) {
        Kind::Period => (t!("Месячные"), "rose"),
        Kind::Predicted => (t!("Ожидаются месячные"), "rose-o"),
        Kind::Ovulation => (t!("Овуляция · высокий шанс зачатия"), "teal"),
        Kind::Fertile => (
            match fc.chance(d) {
                2 => t!("Фертильное окно · высокий шанс зачатия"),
                _ => t!("Фертильное окно · средний шанс зачатия"),
            },
            "teal",
        ),
        Kind::None if fc.has_data() => (t!("Низкий шанс зачатия"), "plain"),
        Kind::None => (t!("Нет данных для прогноза"), "plain"),
    }
}

fn chip(label: String, glyph: Option<&str>, on: bool, tone: &str, on_click: impl FnMut() + Send + 'static) -> W {
    let mut b = Button::new(label);
    if let Some(g) = glyph {
        b = b.icon(g);
    }
    boxed(b.class(&format!("btn pick {tone}{}", if on { " on" } else { "" })).on_click(on_click))
}

/// Время: часы и минуты кнопками (минуты шагом 5), «без времени».
fn time_editor(st: St, id: u64, time: Option<u16>) -> W {
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    row = row.child(super::icon(icons::TIME).class("ic love-ic-muted"));
    match time {
        None => {
            row = row.child(DecoratedBox::new().class("grow").child(Text::new(t!("Время не указано")).class("hint")));
            row = row.child(Button::new(t!("Указать")).class("btn chip").on_click(move || st.update_intimacy(id, |i| i.time = Some(if i.day == crate::date::today() { crate::date::now_minutes() } else { 21 * 60 }))));
        }
        Some(m) => {
            let set = move |d: i32| move || st.update_intimacy(id, |i| i.time = Some(((i.time.unwrap_or(0) as i32 + d).rem_euclid(24 * 60)) as u16));
            row = row
                .child(Button::new("").icon(icons::REMOVE).class("btn round step").on_click(set(-60)))
                .child(Text::new(format!("{:02}", m / 60)).class("time-num"))
                .child(Button::new("").icon(icons::ADD).class("btn round step").on_click(set(60)))
                .child(Text::new(":").class("time-num"))
                .child(Button::new("").icon(icons::REMOVE).class("btn round step").on_click(set(-5)))
                .child(Text::new(format!("{:02}", m % 60)).class("time-num"))
                .child(Button::new("").icon(icons::ADD).class("btn round step").on_click(set(5)))
                .child(DecoratedBox::new().class("grow"))
                .child(icon_button(icons::CLOSE, "nav-btn", move || st.update_intimacy(id, |i| i.time = None)));
        }
    }
    boxed(row)
}

/// Близость в этот день: список отметок, правка выбранной, добавление.
fn intimacy(st: St, d: Day) -> W {
    let fc = st.fc.get_untracked();
    let list = fc.intimacy(d);
    let editing = st.love_edit.get_untracked();
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    col = col.child(Text::new(t!("Близость")).class("section"));
    for it in list {
        let id = it.id;
        let open = editing == Some(id);
        let title = it.time_text().unwrap_or_else(|| t!("Без времени"));
        let sub = match it.protected {
            Some(true) => t!("С защитой"),
            Some(false) => t!("Без защиты"),
            None if it.time.is_some() => t!("Защита не указана"),
            None => t!("Нажмите, чтобы указать время и защиту"),
        };
        let head = GestureDetector::new().on_click(move || st.love_edit.set(if open { None } else { Some(id) })).child(
            Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("tile-badge love").child(super::icon(icons::HEART).class("ic tile-ic")))
                .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).child(Text::new(title).class("row-title")).child(Text::new(sub).max_lines(2).class("hint"))))
                .child(icon_button(icons::DELETE, "nav-btn danger-ic", move || st.remove_intimacy(id))),
        );
        let mut card = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(head);
        if open {
            let p = it.protected;
            let prot = Flex::new()
                .direction(FlexDirection::Row)
                .wrap()
                .gap(8.0)
                .child(chip(t!("С защитой"), Some(icons::SHIELD), p == Some(true), "plum", move || st.update_intimacy(id, |i| i.protected = if i.protected == Some(true) { None } else { Some(true) })))
                .child(chip(t!("Без защиты"), Some(icons::NO_SHIELD), p == Some(false), "rose", move || st.update_intimacy(id, |i| i.protected = if i.protected == Some(false) { None } else { Some(false) })));
            card = card.child(time_editor(st, id, it.time)).child(prot);
        }
        col = col.child(DecoratedBox::new().class(if open { "love-row on" } else { "love-row" }).child(card));
    }
    col = col.child(Button::new(t!("Отметить близость")).icon(icons::HEART_OUTLINE).class("btn love").on_click(move || {
        let id = st.add_intimacy(d);
        st.love_edit.set(Some(id));
    }));
    boxed(col)
}

fn body(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let fc = st.fc.get();
        let _ = st.love_edit.get();
        let Some(d) = st.sheet.get() else {
            return vec![];
        };
        let log = fc.log(d);
        let today = fc.today;
        let (_, m, dd) = date::ymd(d);
        let mut sub = date::weekday_name(date::weekday(d));
        if let Some(cd) = fc.cycle_day(d) {
            sub = format!("{sub} · {}", t!("день цикла {n}", n = cd));
        }
        if d == today {
            sub = format!("{} · {sub}", t!("сегодня"));
        }
        let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("grabber")));
        col = col.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).child(Text::new(format!("{dd} {}", date::month_gen(m))).class("sheet-title")).child(Text::new(sub).max_lines(1).class("hint"))))
                .child(icon_button(icons::PREV, "nav-btn", move || st.sheet.set(Some(d - 1))))
                .child(icon_button(icons::NEXT, "nav-btn", move || st.sheet.set(Some(d + 1))))
                .child(icon_button(icons::CLOSE, "nav-btn", move || st.sheet_open.set(false))),
        );
        let (stat, tone) = status(st, d);
        col = col.child(DecoratedBox::new().class(&format!("status {tone}")).child(Text::new(stat).max_lines(2).class("status-text")));

        // месячные
        let future = d > today;
        let on = log.flow > 0;
        col = col.child(
            GestureDetector::new()
                .on_click(move || {
                    if !future {
                        st.toggle_period(d);
                    }
                })
                .child(
                    DecoratedBox::new().class(if on { "period-row on" } else { "period-row" }).child(
                        Row::new()
                            .gap(12.0)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .child(DecoratedBox::new().class(if on { "tile-badge rose-solid" } else { "tile-badge rose" }).child(super::icon(icons::DROP).class("ic tile-ic")))
                            .child(
                                DecoratedBox::new().class("grow").child(
                                    Column::new().gap(2.0).child(Text::new(t!("Месячные")).class("row-title")).child(
                                        Text::new(if future {
                                            t!("Будущий день отметить нельзя")
                                        } else if on {
                                            t!("Отмечено")
                                        } else {
                                            t!("Нажмите, чтобы отметить")
                                        })
                                        .class("hint"),
                                    ),
                                ),
                            )
                            .child(Toggle::new().on(on).disabled(future).on_change(move |_| st.toggle_period(d))),
                    ),
                ),
        );
        if on {
            let mut f = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
            for &(v, label) in FLOWS {
                f = f.child(chip(t!(label), None, log.flow == v, "rose", move || st.set_log(d, |l| l.flow = v)));
            }
            col = col.child(Column::new().gap(8.0).child(Text::new(t!("Обильность")).class("section")).child(f));
        }
        if !future {
            col = col.child(intimacy(st, d));
            let mut f = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
            for &(bit, label, g) in SYMPTOMS {
                let on = log.sym & bit != 0;
                f = f.child(chip(t!(label), Some(g), on, "plum", move || st.set_log(d, |l| l.sym ^= bit)));
            }
            col = col.child(Column::new().gap(8.0).child(Text::new(t!("Симптомы")).class("section")).child(f));
            let mut f = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
            for &(v, label, g) in MOODS {
                let on = log.mood == v;
                f = f.child(chip(t!(label), Some(g), on, "gold", move || st.set_log(d, |l| l.mood = if l.mood == v { 0 } else { v })));
            }
            col = col.child(Column::new().gap(8.0).child(Text::new(t!("Настроение")).class("section")).child(f));
        }
        col = col.child(Button::new(t!("Готово")).class("btn big primary").on_click(move || st.sheet_open.set(false)));
        vec![boxed(col)]
    }))
}

fn panel(st: St, wide: bool) -> W {
    let vp = viewport_size().get_untracked();
    let maxh = (vp.height * 0.88).max(320.0);
    let mut sheet = DecoratedBox::new().class(if wide { "sheet wide" } else { "sheet" });
    if wide {
        sheet = sheet.style("width", StyleValue::px(vp.width.min(540.0)));
    }
    boxed(GestureDetector::new().on_click(|| {}).child(sheet.style("max-height", StyleValue::px(maxh)).child(ScrollView::new().vertical().class("sheet-scroll").child(DecoratedBox::new().class("sheet-inner").child(body(st))))))
}

fn scrim(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let shown = st.sheet_open.get();
        let vp = viewport_size().get();
        vec![boxed(
            Presence::new(shown, GestureDetector::new().on_click(move || st.sheet_open.set(false)).child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)))
                .enter(Motion::fade())
                .exit(Motion::fade())
                .duration_ms(240)
                .initial(false),
        )]
    }))
}

pub fn layer(st: St, wide: bool) -> W {
    boxed(Stack::new().fit(StackFit::Expand).child(scrim(st)).child(
        Column::new().main_axis_alignment(if wide { MainAxisAlignment::Center } else { MainAxisAlignment::End }).cross_axis_alignment(if wide { CrossAxisAlignment::Center } else { CrossAxisAlignment::Stretch }).child(
            // лист строится при открытии: его ширина зависит от окна (Presence::signal построил бы его
            // один раз при запуске, когда размер окна ещё не известен)
            Reactive::new(move || -> Vec<W> {
                let shown = st.sheet_open.get();
                vec![boxed(
                    Presence::new(shown, DecoratedBox::new().child(panel(st, wide)))
                        .enter(if wide { Motion::fade().scale(0.94).slide(0.0, 24.0) } else { Motion::fade().slide(0.0, 260.0) })
                        .exit(if wide { Motion::fade().scale(0.96) } else { Motion::fade().slide(0.0, 260.0) })
                        .duration_ms(340)
                        .exit_duration_ms(200)
                        .initial(false),
                )]
            }),
        ),
    ))
}
