//! Карточка дня: выезжает снизу поверх затемнения — месячные и их обильность,
//! симптомы, настроение.

use syngui::prelude::*;
use syngui::StyleValue;
use syngui::GestureDetector;

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

fn body(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let fc = st.fc.get();
        let Some(d) = st.sheet.get() else { return vec![] };
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
        col = col.child(Row::new().child(DecoratedBox::new().class(&format!("status {tone}")).child(Text::new(stat).max_lines(1).class("status-text"))));

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
                            .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).child(Text::new(t!("Месячные")).class("row-title")).child(Text::new(if future { t!("Будущий день отметить нельзя") } else if on { t!("Отмечено") } else { t!("Нажмите, чтобы отметить") }).class("hint"))))
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
    let w = if wide { vp.width.min(540.0) } else { vp.width };
    let maxh = (vp.height * 0.88).max(320.0);
    boxed(
        GestureDetector::new().on_click(|| {}).child(
            DecoratedBox::new()
                .class(if wide { "sheet wide" } else { "sheet" })
                .style("width", StyleValue::px(w))
                .style("max-height", StyleValue::px(maxh))
                .child(ScrollView::new().vertical().class("sheet-scroll").child(DecoratedBox::new().class("sheet-inner").child(body(st)))),
        ),
    )
}

fn scrim(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let shown = st.sheet_open.get();
        let vp = viewport_size().get();
        vec![boxed(
            Presence::new(
                shown,
                GestureDetector::new().on_click(move || st.sheet_open.set(false)).child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)),
            )
            .enter(Motion::fade())
            .exit(Motion::fade())
            .duration_ms(240)
            .initial(false),
        )]
    }))
}

pub fn layer(st: St, wide: bool) -> W {
    boxed(
        Stack::new().fit(StackFit::Expand).child(scrim(st)).child(
            Column::new().main_axis_alignment(if wide { MainAxisAlignment::Center } else { MainAxisAlignment::End }).cross_axis_alignment(CrossAxisAlignment::Center).child(
                Presence::signal(st.sheet_open, move || panel(st, wide))
                    .enter(if wide { Motion::fade().scale(0.94).slide(0.0, 24.0) } else { Motion::fade().slide(0.0, 260.0) })
                    .exit(if wide { Motion::fade().scale(0.96) } else { Motion::fade().slide(0.0, 260.0) })
                    .duration_ms(340)
                    .exit_duration_ms(200),
            ),
        ),
    )
}
