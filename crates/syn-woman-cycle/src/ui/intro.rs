//! Знакомство при первом запуске: приветствие, дата последних месячных, длины цикла.

use syngui::prelude::*;

use super::calendar::{self, Mode};
use super::settings::{section, stepper};
use super::{boxed, icon, rise, scroll_page, W};
use crate::icons;
use crate::state::St;

fn dots(step: u64) -> W {
    let mut row = Row::new().gap(6.0).main_axis_alignment(MainAxisAlignment::Center);
    for i in 0..3 {
        row = row.child(DecoratedBox::new().class(if i == step { "step-dot on" } else { "step-dot" }));
    }
    boxed(row)
}

fn feature(glyph: &str, tone: &str, title: String, text: String) -> W {
    boxed(
        Row::new()
            .gap(14.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class(&format!("tile-badge {tone}")).child(icon(glyph).class("ic tile-ic")))
            .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).child(Text::new(title).class("row-title")).child(Text::new(text).max_lines(3).class("hint")))),
    )
}

fn welcome(st: St, wide: bool) -> W {
    let hero = Animated::new(DecoratedBox::new().class("intro-hero").child(icon(icons::DROP).class("ic intro-hero-ic")))
        .scale(Animation::spring().from(0.6).to(1.0).stiffness(220.0).damping(14.0).into())
        .opacity(Animation::tween(Easing::EaseOutCubic).from(0.0).to(1.0).duration_ms(400).into());
    scroll_page(
        wide,
        vec![
            boxed(DecoratedBox::new().class("intro-top")),
            boxed(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(hero)),
            rise(
                1,
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(t!("Цикл")).class("intro-title"))
                    .child(Text::new(t!("Ваш календарь месячных, овуляции и фертильных дней")).max_lines(3).class("intro-text center")),
            ),
            rise(
                2,
                DecoratedBox::new().class("group pad").child(
                    Column::new()
                        .gap(16.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(feature(icons::EVENT, "rose", t!("Прогноз месячных"), t!("Подстраивается под ваши циклы")))
                        .child(feature(icons::FLOWER, "teal", t!("Овуляция и фертильное окно"), t!("Когда шанс зачатия выше всего")))
                        .child(feature(icons::LOCK, "plum", t!("Приватно"), t!("Данные хранятся только на этом устройстве"))),
                ),
            ),
            rise(3, Button::new(t!("Начать")).class("btn big primary").on_click(move || st.intro_step.set(1))),
            (dots(0)),
        ],
    )
}

fn last_period(st: St, wide: bool) -> W {
    let w = super::content_width(wide) - 28.0;
    let picked = st.intro_day.get_untracked();
    let month = st.intro_month;
    let fc = st.fc.get_untracked();
    let card = DecoratedBox::new().class("cal-card").child(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Reactive::new(move || -> Vec<W> {
                let _ = month.get();
                vec![calendar::month_bar(month, fc.today, true)]
            }))
            .child(calendar::swiper(
                month,
                move || st.intro_day.get().unwrap_or(0) as u64,
                move |i| {
                    let fc = st.fc.get_untracked();
                    calendar::grid(st, &fc, i, w, Mode::Pick)
                },
            )),
    );
    // кнопка без Reactive (иначе не растягивается): шаг перестраивается при выборе даты (`version`)
    let next = Button::new(t!("Далее")).class("btn big primary").disabled(picked.is_none()).on_click(move || st.intro_step.set(2));
    scroll_page(
        wide,
        vec![
            boxed(DecoratedBox::new().class("intro-top")),
            rise(0, Column::new().gap(6.0).child(Text::new(t!("Когда начались последние месячные?")).max_lines(3).class("h1")).child(Text::new(t!("Выберите первый день — от него строится прогноз")).max_lines(2).class("hint"))),
            rise(1, card),
            rise(
                2,
                Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(next).child(Button::new(t!("Не помню")).class("btn ghost").on_click(move || {
                    st.intro_day.set(None);
                    st.intro_step.set(2);
                })),
            ),
            (dots(1)),
        ],
    )
}

fn lengths(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let s = st.data.get().settings;
        vec![scroll_page(
            wide,
            vec![
                boxed(DecoratedBox::new().class("intro-top")),
                rise(0, Column::new().gap(6.0).child(Text::new(t!("Ваш цикл")).class("h1")).child(Text::new(t!("Если не знаете точно — оставьте как есть: прогноз уточнится по вашим отметкам")).max_lines(3).class("hint"))),
                rise(
                    1,
                    section(
                        t!("Обычно"),
                        vec![
                            stepper(st, icons::EVENT, t!("Длина цикла"), t!("От начала одних месячных до следующих"), s.cycle_len, 18, 45, t!("дн."), |x, v| x.cycle_len = v),
                            stepper(st, icons::DROP, t!("Месячные длятся"), String::new(), s.period_len, 1, 10, t!("дн."), |x, v| x.period_len = v),
                        ],
                    ),
                ),
                rise(
                    2,
                    Column::new()
                        .gap(10.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(Button::new(t!("Готово")).icon(icons::CHECK).class("btn big primary").on_click(move || st.finish_intro()))
                        .child(Button::new(t!("Назад")).class("btn ghost").on_click(move || st.intro_step.set(1))),
                ),
                (dots(2)),
            ],
        )]
    }))
}

pub fn page_root(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let step = st.intro_step.get();
        let ver = if step == 1 { st.intro_day.get().is_some() as u64 } else { 0 };
        vec![boxed(
            AnimatedSwitcher::new(step, move || -> W {
                syngui::i18n::subscribe();
                match st.intro_step.get_untracked() {
                    0 => welcome(st, wide),
                    1 => last_period(st, wide),
                    _ => lengths(st, wide),
                }
            })
            .version(ver)
            .slide(48.0, 0.0)
            .directional(true)
            .duration_ms(340)
            .animate_size(false)
            .class("grow"),
        )]
    }))
}
