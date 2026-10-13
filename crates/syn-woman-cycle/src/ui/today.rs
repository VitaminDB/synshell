//! «Сегодня»: кольцо цикла с фазами, главное действие и ближайшие даты.

use std::f32::consts::{FRAC_PI_2, TAU};

use syngui::prelude::*;
use syngui::StyleValue;
use syngui::GestureDetector;

use super::{boxed, icon, scroll_page, page_header, rise, W};
use crate::date;
use crate::icons;
use crate::model::{Forecast, Phase};
use crate::state::{St, Tab};

pub fn phase_name(p: Phase) -> String {
    match p {
        Phase::Period => t!("Менструация"),
        Phase::Follicular => t!("Фолликулярная фаза"),
        Phase::Fertile => t!("Фертильное окно"),
        Phase::Ovulation => t!("Овуляция"),
        Phase::Luteal => t!("Лютеиновая фаза"),
        Phase::Late => t!("Задержка"),
    }
}

fn phase_tip(p: Phase) -> String {
    match p {
        Phase::Period => t!("Больше отдыхайте, пейте тёплое и добавьте в рацион продукты с железом."),
        Phase::Follicular => t!("Энергии становится больше — хорошее время для новых дел и тренировок."),
        Phase::Fertile => t!("Высокая вероятность зачатия. Если беременность не планируется — позаботьтесь о защите."),
        Phase::Ovulation => t!("Пик фертильности: яйцеклетка готова к оплодотворению около суток."),
        Phase::Luteal => t!("Возможны ПМС, тяга к сладкому и усталость. Будьте к себе бережнее."),
        Phase::Late => t!("Задержка бывает из-за стресса, болезни или перелётов. Если она затянулась — сделайте тест или обратитесь к врачу."),
    }
}

fn phase_icon(p: Phase) -> &'static str {
    match p {
        Phase::Period => icons::DROP,
        Phase::Follicular => icons::SPA,
        Phase::Fertile => icons::FLOWER,
        Phase::Ovulation => icons::EGG,
        Phase::Luteal => icons::MOOD,
        Phase::Late => icons::EVENT,
    }
}

fn ease_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// Кольцо цикла: месячные, фертильное окно, овуляция и сегодняшний день; при появлении
/// дуги «прорисовываются», а отметка дня проезжает по кругу к своему месту.
fn ring(st: St, fc: &Forecast, size: f32) -> W {
    let p = crate::pal(st.dark.get_untracked());
    let today = fc.today;
    let span = fc.cycle_of(today);
    let len = span.map(|(s, n)| (n - s).max(today - s + 1)).unwrap_or(fc.cycle).max(1) as f32;
    let day = span.map(|(s, _)| (today - s) as f32).unwrap_or(0.0);
    let period = span.map(|(s, _)| fc.cycles.iter().find(|c| c.start == s).map(|c| c.period).unwrap_or(fc.period).max(fc.ongoing_end.map(|e| e - s + 1).unwrap_or(0))).unwrap_or(0) as f32;
    let ov = span.map(|(s, n)| (fc.ovulation(n) - s) as f32);
    let late = fc.late > 0;
    let has = fc.has_data();
    let key = (today as u64) << 8 ^ (len as u64) ^ ((period as u64) << 20);
    let canvas = Canvas::new(move |c, t| {
        let e = ease_out(t / 1.25);
        let stroke = size * 0.074;
        let r = size / 2.0 - stroke * 0.9;
        let (cx, cy) = (size / 2.0, size / 2.0);
        let a0 = -FRAC_PI_2;
        let ang = |d: f32| a0 + d / len * TAU;
        let lim = a0 + e * TAU;
        c.set_line_cap(LineCap::Round);
        c.set_stroke_width(stroke);
        c.set_color(p.track);
        c.stroke_circle(cx, cy, r);
        if !has {
            return;
        }
        let gap = 0.035;
        let arc = |c: &mut CanvasContext, from: f32, to: f32, col: Color| {
            let (a, b) = (ang(from) + gap, (ang(to) - gap).min(lim));
            if b > a {
                c.set_color(col);
                c.draw_arc(cx, cy, r, a, b);
            }
        };
        if late {
            arc(c, 0.0, len, p.plum.lerp(&p.track, 0.55));
        }
        if period > 0.0 {
            arc(c, 0.0, period, p.rose);
        }
        if let Some(o) = ov {
            if o > period {
                arc(c, (o - 5.0).max(period), o + 2.0, p.teal.lerp(&p.track, 0.5));
                if lim >= ang(o + 0.5) {
                    c.set_color(p.teal);
                    let (x, y) = (cx + r * ang(o + 0.5).cos(), cy + r * ang(o + 0.5).sin());
                    c.fill_circle(x, y, stroke * 0.62);
                }
            }
        }
        // сегодняшний день: точка с ободком
        let at = ang((day + 0.5) * e);
        let (x, y) = (cx + r * at.cos(), cy + r * at.sin());
        c.set_color(p.track.lerp(&p.plum, 0.25));
        c.fill_circle(x, y, stroke * 1.05);
        c.set_color(p.surface);
        c.fill_circle(x, y, stroke * 0.82);
        c.set_color(p.plum);
        c.fill_circle(x, y, stroke * 0.42);
    })
    .size(size, size)
    .animated_for(1.3)
    .replay(key);

    let phase = fc.phase();
    let mut center = Column::new().gap(4.0).main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center);
    if let (Some(ph), Some(cd)) = (phase, fc.cycle_day(today)) {
        center = center
            .child(Text::new(phase_name(ph)).max_lines(1).class(match ph {
                Phase::Period => "ring-over rose",
                Phase::Fertile | Phase::Ovulation => "ring-over teal",
                Phase::Late => "ring-over plum",
                _ => "ring-over",
            }))
            .child(Text::new(t!("День {n}", n = cd)).max_lines(1).class("ring-big"));
        let sub = if late {
            tn!(fc.late, "задержка {n} день", "задержка {n} дня", "задержка {n} дней")
        } else if fc.ongoing || fc.log(today).flow > 0 {
            let left = fc.ongoing_end.map(|e| e - today).unwrap_or(0);
            if left > 0 {
                tn!(left, "ещё примерно {n} день", "ещё примерно {n} дня", "ещё примерно {n} дней")
            } else {
                t!("последний день по прогнозу")
            }
        } else if let Some(&n) = fc.next.first() {
            t!("месячные {when}", when = date::relative(n, today))
        } else {
            String::new()
        };
        center = center.child(Text::new(sub).max_lines(2).class("ring-sub"));
    } else {
        center = center.child(icon(icons::DROP).class("ic ring-empty-ic")).child(Text::new(t!("Отметьте начало\nмесячных")).max_lines(3).class("ring-sub center"));
    }
    let inner = size * 0.68;
    boxed(
        Stack::new()
            .child(canvas)
            .child(Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().style("width", StyleValue::px(inner)).child(center)).style("width", StyleValue::px(size)).style("height", StyleValue::px(size))),
    )
}

fn info_tile(glyph: &str, tone: &str, label: String, value: String, sub: String) -> W {
    boxed(
        DecoratedBox::new().class("tile grow").child(
            Column::new()
                .gap(6.0)
                .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(&format!("tile-badge {tone}")).child(icon(glyph).class("ic tile-ic"))).child(Text::new(label).max_lines(1).class("tile-label")))
                .child(Text::new(value).max_lines(1).class("tile-value"))
                .child(Text::new(sub).max_lines(1).class("hint")),
        ),
    )
}

fn chance_text(c: u8) -> (String, &'static str) {
    match c {
        2 => (t!("Высокий"), "teal"),
        1 => (t!("Средний"), "gold"),
        _ => (t!("Низкий"), "plum"),
    }
}

fn actions(st: St, fc: &Forecast) -> W {
    let today = fc.today;
    let marked = fc.log(today).flow > 0;
    let mut col = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if marked {
        col = col.child(Button::new(t!("Месячные закончились")).icon(icons::CHECK).class("btn big soft").on_click(move || {
            st.end_period(st.today.get_untracked());
            st.toast(t!("Отмечено: месячные закончились"));
        }));
    } else if fc.ongoing {
        col = col.child(
            Row::new()
                .gap(10.0)
                .child(Button::new(t!("Идут сегодня")).icon(icons::DROP).class("btn big primary grow").on_click(move || st.toggle_period(st.today.get_untracked())))
                .child(Button::new(t!("Закончились")).icon(icons::CHECK).class("btn big soft grow").on_click(move || {
                    st.end_period(st.today.get_untracked() - 1);
                    st.toast(t!("Отмечено: месячные закончились"));
                })),
        );
    } else {
        col = col.child(Button::new(t!("Начались месячные")).icon(icons::DROP).class("btn big primary").on_click(move || {
            st.toggle_period(st.today.get_untracked());
            st.toast(t!("Отмечено начало месячных"));
        }));
        col = col.child(Button::new(t!("Начались в другой день")).icon(icons::EDIT_CAL).class("btn ghost").on_click(move || st.go(Tab::Calendar)));
    }
    boxed(col)
}

pub fn page(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let fc = st.fc.get();
        let _ = st.dark.get();
        let today = fc.today;
        let (_, m, d) = date::ymd(today);
        let over = format!("{}, {d} {}", date::weekday_name(date::weekday(today)), date::month_gen(m));
        let w = super::content_width(wide);
        let size = (w * 0.8).clamp(220.0, 320.0);
        let mut blocks: Vec<W> = vec![page_header(st, wide, over, t!("Сегодня"), vec![])];
        blocks.push(rise(0, DecoratedBox::new().class("hero").child(Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center).child(ring(st, &fc, size)).child(DecoratedBox::new().style("width", StyleValue::px(w - 36.0)).child(actions(st, &fc))))));
        if fc.has_data() {
            let next = fc.next.first().copied();
            let ov = fc.next_ovulation();
            let row1 = Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(info_tile(icons::DROP, "rose", t!("Месячные"), next.map(date::long).unwrap_or_default(), next.map(|n| date::relative(n, today)).unwrap_or_default()))
                .child(info_tile(icons::EGG, "teal", t!("Овуляция"), ov.map(date::long).unwrap_or_default(), ov.map(|o| date::relative(o, today)).unwrap_or_default()));
            let (ch, tone) = chance_text(fc.chance(today));
            let win = ov.map(|o| date::range(o - 5, o + 1)).unwrap_or_default();
            let row2 = Row::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(info_tile(icons::FLOWER, "teal", t!("Фертильное окно"), win, t!("6–7 дней вокруг овуляции")))
                .child(info_tile(icons::SPARK, tone, t!("Шанс зачатия"), ch, t!("сегодня")));
            blocks.push(rise(1, Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(row1).child(row2)));
            if let Some(ph) = fc.phase() {
                blocks.push(rise(
                    2,
                    DecoratedBox::new().class("tip").child(
                        Row::new()
                            .gap(12.0)
                            .child(DecoratedBox::new().class("tip-badge").child(icon(phase_icon(ph)).class("ic tip-ic")))
                            .child(DecoratedBox::new().class("grow").child(Column::new().gap(3.0).child(Text::new(phase_name(ph)).class("tip-title")).child(Text::new(phase_tip(ph)).max_lines(5).class("tip-text")))),
                    ),
                ));
            }
        }
        let log = fc.log(today);
        let n = log.sym.count_ones() + (log.mood > 0) as u32;
        let feel = if n == 0 { t!("Отметьте симптомы и настроение") } else { tn!(n, "Отмечена {n} запись", "Отмечено {n} записи", "Отмечено {n} записей") };
        blocks.push(rise(
            3,
            GestureDetector::new().on_click(move || st.open_day(st.today.get_untracked())).child(
                DecoratedBox::new().class("row-card").child(
                    Row::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("tile-badge plum").child(icon(icons::HEART).class("ic tile-ic")))
                        .child(DecoratedBox::new().class("grow").child(Column::new().gap(2.0).child(Text::new(t!("Самочувствие")).class("row-title")).child(Text::new(feel).max_lines(1).class("hint"))))
                        .child(icon(icons::NEXT).class("ic chevron")),
                ),
            ),
        ));
        blocks.push(Box::new(Text::new(t!("Прогноз приблизительный и не заменяет контрацепцию и консультацию врача.")).max_lines(3).class("hint center disclaimer")));
        vec![scroll_page(wide, blocks)]
    }))
}
