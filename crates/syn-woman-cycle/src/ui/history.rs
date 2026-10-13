//! «Мой цикл»: средние длины, регулярность и история циклов полосами.

use syngui::prelude::*;

use super::{boxed, icon, page, page_header, rise, W};
use crate::date;
use crate::icons;
use crate::model::{Cycle, Forecast};
use crate::state::{St, Tab};

fn stat(value: String, unit: String, label: String, tone: &str) -> W {
    boxed(
        DecoratedBox::new().class(&format!("stat grow {tone}")).child(
            Column::new()
                .gap(2.0)
                .child(Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::End).child(Text::new(value).class("stat-value")).child(Text::new(unit).class("stat-unit")))
                .child(Text::new(label).max_lines(2).class("hint")),
        ),
    )
}

/// Полоса цикла: месячные, фертильное окно, овуляция; растёт слева направо.
fn bar(st: St, fc: &Forecast, c: Cycle, i: usize, max_len: i32, w: f32) -> W {
    let p = crate::pal(st.dark.get_untracked());
    let len = c.len.unwrap_or_else(|| (fc.today - c.start + 1).max(fc.cycle)) as f32;
    let next = c.start + len as i32;
    let ov = (fc.ovulation(next) - c.start) as f32;
    let elapsed = c.len.map(|_| len).unwrap_or((fc.today - c.start + 1) as f32);
    let period = c.period as f32;
    let full = w * (len / max_len as f32).min(1.0);
    let h = 12.0;
    let delay = i as f32 * 0.06;
    let canvas = Canvas::new(move |cv, t| {
        let e = 1.0 - (1.0 - ((t - delay) / 0.7).clamp(0.0, 1.0)).powi(3);
        let px = |d: f32| full * d / len;
        let lim = full * e;
        let seg = |cv: &mut CanvasContext, a: f32, b: f32, col: Color| {
            let (x0, x1) = (px(a), px(b).min(lim));
            if x1 > x0 {
                cv.set_color(col);
                cv.fill_rounded_rect(x0, 0.0, x1 - x0, h, h / 2.0);
            }
        };
        seg(cv, 0.0, len, p.track);
        if c.len.is_none() {
            seg(cv, 0.0, elapsed, p.plum.with_alpha(0.22));
        }
        seg(cv, (ov - 5.0).max(period), ov + 2.0, p.teal.with_alpha(0.4));
        seg(cv, 0.0, period, p.rose);
        if px(ov + 0.5) <= lim {
            cv.set_color(p.teal);
            cv.fill_circle(px(ov + 0.5), h / 2.0, h / 2.0 + 1.0);
        }
    })
    .size(w, h)
    .animated_for(0.7 + delay + 0.05);
    let title = match c.len {
        Some(l) => date::range(c.start, c.start + l - 1),
        None => t!("Текущий цикл"),
    };
    let sub = match c.len {
        Some(l) => format!("{} · {}", tn!(l, "{n} день", "{n} дня", "{n} дней"), t!("месячные {n} дн.", n = c.period)),
        None => format!("{} · {}", t!("с {d}", d = date::short(c.start)), t!("день {n}", n = fc.today - c.start + 1)),
    };
    boxed(
        DecoratedBox::new().class("cycle-row").child(
            Column::new()
                .gap(8.0)
                .child(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("grow").child(Text::new(title).class("row-title"))).child(Text::new(sub).max_lines(1).class("hint")))
                .child(canvas),
        ),
    )
}

pub fn page(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let fc = st.fc.get();
        let _ = st.dark.get();
        let w = super::content_width(wide);
        let mut blocks: Vec<W> = vec![page_header(st, wide, t!("Статистика и история"), t!("Мой цикл"), vec![])];
        if !fc.has_data() {
            blocks.push(rise(
                0,
                DecoratedBox::new().class("empty").child(
                    Column::new()
                        .gap(12.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(DecoratedBox::new().class("hero-badge").child(icon(icons::TIMELINE).class("ic hero-ic")))
                        .child(Text::new(t!("Пока нет циклов")).class("h2"))
                        .child(Text::new(t!("Отмечайте месячные в календаре — здесь появятся средняя длина цикла и история.")).max_lines(4).class("hint center"))
                        .child(Button::new(t!("Открыть календарь")).icon(icons::CALENDAR).class("btn primary").on_click(move || st.go(Tab::Calendar))),
                ),
            ));
            return vec![page(wide, blocks)];
        }
        let lens: Vec<i32> = fc.cycles.iter().filter_map(|c| c.len).collect();
        let regular = match (lens.iter().min(), lens.iter().max()) {
            (Some(a), Some(b)) if lens.len() >= 2 => Some(b - a <= 7),
            _ => None,
        };
        let row1 = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(stat(fc.cycle.to_string(), t!("дн."), t!("Средний цикл"), "plum"))
            .child(stat(fc.period.to_string(), t!("дн."), t!("Месячные"), "rose"));
        let row2 = Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(stat(fc.cycles.len().to_string(), String::new(), t!("Циклов отмечено"), "gold"))
            .child(stat(
                match regular {
                    Some(true) => t!("Да"),
                    Some(false) => t!("Нет"),
                    None => "—".into(),
                },
                String::new(),
                match regular {
                    Some(_) => t!("Регулярный (разброс до 7 дней)"),
                    None => t!("Регулярность — после 2 циклов"),
                },
                "teal",
            ));
        blocks.push(rise(0, Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(row1).child(row2)));
        let max_len = fc.cycles.iter().map(|c| c.len.unwrap_or_else(|| (fc.today - c.start + 1).max(fc.cycle))).max().unwrap_or(fc.cycle).max(fc.cycle);
        let mut list = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        list = list.child(Text::new(t!("История циклов")).class("section"));
        for (i, c) in fc.cycles.iter().rev().take(24).enumerate() {
            list = list.child(bar(st, &fc, *c, i, max_len, w - 32.0));
        }
        blocks.push(rise(1, list));
        blocks.push(Box::new(Text::new(t!("Средние считаются по последним 6 циклам длиной 18–60 дней.")).max_lines(2).class("hint center disclaimer")));
        vec![page(wide, blocks)]
    }))
}
