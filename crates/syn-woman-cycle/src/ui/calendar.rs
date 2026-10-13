//! «Календарь»: месяц с месячными, прогнозом, фертильным окном и овуляцией;
//! листается кнопками, смахиванием и стрелками, день открывается карточкой.

use syngui::prelude::*;
use syngui::StyleValue;
use syngui::widgets::SwipeDirection;
use syngui::GestureDetector;

use super::{boxed, icon, icon_button, scroll_page, page_header, rise, W};
use crate::date::{self, Day};
use crate::icons;
use crate::model::{Forecast, Kind};
use crate::state::St;

/// Режим сетки: обычный календарь или выбор даты при знакомстве.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Main,
    Pick,
}

fn cell_class(fc: &Forecast, d: Day, mode: Mode, picked: Option<Day>, sel: Option<Day>) -> String {
    let mut c = String::from("cell");
    if mode == Mode::Pick {
        if Some(d) == picked {
            c.push_str(" period");
        } else if picked.is_some_and(|p| d > p && d < p + 5) {
            c.push_str(" predicted");
        } else if d > fc.today {
            c.push_str(" off");
        }
        return c;
    }
    c.push_str(match fc.kind(d) {
        Kind::Period => " period",
        Kind::Predicted => " predicted",
        Kind::Ovulation => " ovul",
        Kind::Fertile => " fertile",
        Kind::None => "",
    });
    if Some(d) == sel {
        c.push_str(" sel");
    }
    c
}

fn text_class(fc: &Forecast, d: Day, mode: Mode, picked: Option<Day>) -> &'static str {
    let k = if mode == Mode::Pick {
        if Some(d) == picked {
            Kind::Period
        } else if d > fc.today {
            return "cell-num off";
        } else {
            Kind::None
        }
    } else {
        fc.kind(d)
    };
    match k {
        Kind::Period | Kind::Ovulation => "cell-num strong",
        Kind::Predicted => "cell-num rose",
        Kind::Fertile => "cell-num teal",
        Kind::None if d == fc.today => "cell-num today",
        Kind::None => "cell-num",
    }
}

/// Сетка месяца `idx` шириной `w`.
pub fn grid(st: St, fc: &Forecast, idx: i32, w: f32, mode: Mode) -> W {
    let monday = st.data.get_untracked().settings.monday;
    let gap = 4.0;
    let cell = ((w - gap * 6.0) / 7.0).floor();
    let h = cell.min(52.0);
    let first = date::month_start(idx);
    let n = date::days_in_month(idx);
    let lead = if monday { date::weekday(first) } else { (date::weekday(first) + 1) % 7 } as i32;
    let picked = st.intro_day.get_untracked();
    let sel = if st.sheet_open.get_untracked() { st.sheet.get_untracked() } else { None };
    let mut col = Column::new().gap(gap);
    let mut head = Row::new().gap(gap);
    for i in 0..7u32 {
        let wd = if monday { i } else { (i + 6) % 7 };
        head = head.child(DecoratedBox::new().class("wd").style("width", StyleValue::px(cell)).child(Text::new(date::weekday_short(wd)).class(if wd >= 5 { "wd-text weekend" } else { "wd-text" })));
    }
    col = col.child(head);
    let rows = (lead + n + 6) / 7;
    for r in 0..rows {
        let mut row = Row::new().gap(gap);
        for c in 0..7 {
            let k = r * 7 + c - lead;
            if k < 0 || k >= n {
                row = row.child(DecoratedBox::new().style("width", StyleValue::px(cell)).style("height", StyleValue::px(h)));
                continue;
            }
            let d = first + k;
            let log = fc.log(d);
            let mut inner = Column::new().gap(2.0).main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new((k + 1).to_string()).class(text_class(fc, d, mode, picked)));
            let dot = if mode == Mode::Main && d == fc.today {
                "dot today"
            } else if mode == Mode::Main && (log.sym != 0 || log.mood != 0) {
                "dot note"
            } else {
                "dot none"
            };
            inner = inner.child(DecoratedBox::new().class(dot));
            let disabled = mode == Mode::Pick && d > fc.today;
            row = row.child(
                GestureDetector::new()
                    .on_click(move || match mode {
                        Mode::Main => st.open_day(d),
                        Mode::Pick if !disabled => st.intro_day.set(Some(d)),
                        Mode::Pick => {}
                    })
                    .child(
                        DecoratedBox::new()
                            .class("cell-slot")
                            .style("width", StyleValue::px(cell))
                            .style("height", StyleValue::px(h))
                            .child(DecoratedBox::new().class(&cell_class(fc, d, mode, picked, sel)).style("width", StyleValue::px(h)).style("height", StyleValue::px(h)).child(inner)),
                    ),
            );
        }
        col = col.child(row);
    }
    boxed(col)
}

/// Заголовок месяца с перелистыванием.
pub fn month_bar(month: RwSignal<i32>, today: Day, show_today: bool) -> W {
    let idx = month.get_untracked();
    let (y, m, _) = date::ymd(date::month_start(idx));
    let mut row = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    row = row.child(icon_button(icons::PREV, "nav-btn", move || month.set(month.get_untracked() - 1)));
    row = row.child(DecoratedBox::new().class("grow").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(format!("{} {y}", date::month_name(m))).class("month-title"))));
    let cur = date::month_index(today);
    if show_today && idx != cur {
        row = row.child(Button::new(t!("Сегодня")).class("btn chip").on_click(move || month.set(cur)));
    }
    row = row.child(icon_button(icons::NEXT, "nav-btn", move || month.set(month.get_untracked() + 1)));
    boxed(row)
}

/// Месяц, перетекающий вбок при перелистывании; смахивание листает.
/// `version` — отметки и выбор: при их смене месяц перерисовывается без перелистывания.
pub fn swiper(month: RwSignal<i32>, version: impl Fn() -> u64 + Send + Sync + 'static, build: impl Fn(i32) -> W + Send + Sync + 'static) -> W {
    let build = std::sync::Arc::new(build);
    boxed(Reactive::new(move || -> Vec<W> {
        let idx = month.get();
        let ver = version();
        let b = build.clone();
        vec![boxed(
            GestureDetector::new()
                .on_swipe(move |dir, _| match dir {
                    SwipeDirection::Left => month.set(month.get_untracked() + 1),
                    SwipeDirection::Right => month.set(month.get_untracked() - 1),
                    _ => {}
                })
                .child(AnimatedSwitcher::new((idx + 1_000_000) as u64, move || b(month.get_untracked()))
                .version(ver)
                .slide(60.0, 0.0)
                .directional(true)
                .duration_ms(300)
                .exit_duration_ms(150)),
        )]
    }))
}

fn legend_item(class: &str, label: String) -> W {
    boxed(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(&format!("legend-dot {class}"))).child(Text::new(label).class("legend-text")))
}

fn legend(st: St) -> W {
    let s = st.data.get_untracked().settings;
    let mut f = Flex::new().direction(FlexDirection::Row).wrap().gap(12.0);
    f = f.child(legend_item("period", t!("Месячные")));
    f = f.child(legend_item("predicted", t!("Прогноз")));
    if s.show_fertile {
        f = f.child(legend_item("fertile", t!("Фертильные дни")));
    }
    if s.show_ovulation {
        f = f.child(legend_item("ovul", t!("Овуляция")));
    }
    boxed(DecoratedBox::new().class("legend").child(f))
}

/// События месяца: месячные (факт и прогноз), фертильные окна, овуляции.
fn events(fc: &Forecast, idx: i32) -> W {
    let first = date::month_start(idx);
    let last = first + date::days_in_month(idx) - 1;
    let mut list: Vec<(Day, &'static str, &'static str, String, String)> = Vec::new();
    // подряд идущие дни одного вида; овуляция — внутри фертильного окна и отдельной строкой
    let group = |k: Kind| match k {
        Kind::Ovulation => Kind::Fertile,
        k => k,
    };
    let mut d = first;
    while d <= last {
        let k = group(fc.kind(d));
        let mut e = d;
        while e < last && group(fc.kind(e + 1)) == k {
            e += 1;
        }
        let item = match k {
            Kind::Period => Some((icons::DROP, "rose", t!("Месячные"))),
            Kind::Predicted => Some((icons::DROP, "rose-o", t!("Ожидаются месячные"))),
            Kind::Fertile => Some((icons::FLOWER, "teal", t!("Фертильное окно"))),
            _ => None,
        };
        if let Some((g, tone, title)) = item {
            let when = if d == e { date::long(d) } else { date::range(d, e) };
            list.push((d, g, tone, title, when));
        }
        if k == Kind::Fertile {
            if let Some(o) = (d..=e).find(|&x| fc.kind(x) == Kind::Ovulation) {
                list.push((o, icons::EGG, "teal", t!("Овуляция"), date::long(o)));
            }
        }
        d = e + 1;
    }
    list.sort_by_key(|x| x.0);
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    col = col.child(Text::new(t!("В этом месяце")).class("section"));
    if list.is_empty() {
        col = col.child(Text::new(t!("Нет отметок и прогнозов")).class("hint"));
    }
    for (_, g, tone, title, when) in list {
        col = col.child(
            DecoratedBox::new().class("event").child(
                Row::new()
                    .gap(12.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class(&format!("tile-badge {tone}")).child(icon(g).class("ic tile-ic")))
                    .child(DecoratedBox::new().class("grow").child(Text::new(title).class("row-title")))
                    .child(Text::new(when).class("event-when")),
            ),
        );
    }
    boxed(col)
}

pub fn page(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let fc = st.fc.get_untracked();
        let _ = st.rev.get();
        let idx = st.month.get();
        let w = super::content_width(wide);
        let gw = w - 28.0;
        let mut blocks: Vec<W> = vec![page_header(st, wide, t!("Нажмите на день, чтобы отметить"), t!("Календарь"), vec![])];
        let month = st.month;
        let card = DecoratedBox::new().class("cal-card").child(
            Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(month_bar(month, fc.today, true)).child(swiper(month, move || {
                let sel = if st.sheet_open.get() { st.sheet.get().unwrap_or(0) as u64 } else { 0 };
                st.rev.get() << 24 ^ sel
            }, move |i| {
                let fc = st.fc.get_untracked();
                grid(st, &fc, i, gw, Mode::Main)
            })),
        );
        blocks.push(rise(0, card));
        blocks.push(rise(1, legend(st)));
        blocks.push(rise(2, events(&fc, idx)));
        vec![scroll_page(wide, blocks)]
    }))
}
