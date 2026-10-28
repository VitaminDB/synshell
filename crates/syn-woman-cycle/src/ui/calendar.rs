//! «Календарь» — главный экран: сверху день и фаза цикла с полосой фаз, ниже лента месяцев.
//! Даты подсвечены целиком: месячные — заливкой, прогноз — бледной заливкой, фертильное окно и
//! овуляция — жёлтым разной плотности, близость — обводкой «ручкой». Нажатие на дату — меню отметок.

use std::f32::consts::TAU;

use syngui::prelude::*;
use syngui::widgets::{scroll_to_named, Named, SwipeDirection};
use syngui::GestureDetector;
use syngui::StyleValue;
use syngui::{PopupAnchor, PopupPanel};

use super::{boxed, icon, icon_button, W};
use crate::date::{self, Day};
use crate::icons;
use crate::model::{Forecast, Kind, Phase, Phases};
use crate::state::St;

/// Режим сетки: календарь с меню отметок или выбор даты при знакомстве.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Main,
    Pick,
}

/// Сколько месяцев ленты после текущего.
const AHEAD: i32 = 9;
/// Не раньше стольких месяцев до текущего, даже если отметки старше.
const BACK_MAX: i32 = 36;

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

fn cell_class(fc: &Forecast, d: Day, mode: Mode, sel: Option<Day>) -> String {
    let mut c = String::from("cell");
    let k = fc.kind(d);
    c.push_str(match k {
        Kind::Period => " period",
        Kind::Predicted => " predicted",
        Kind::Ovulation => " ovul",
        Kind::Fertile => " fertile",
        Kind::None => "",
    });
    if k == Kind::Ovulation && fc.ovulation_marked(d) {
        c.push_str(" marked");
    }
    // выбор даты: будущие дни без прогноза приглушены (их выбрать нельзя)
    if mode == Mode::Pick && k == Kind::None && d > fc.today {
        c.push_str(" off");
    }
    if d == fc.today {
        c.push_str(" today");
    }
    if Some(d) == sel {
        c.push_str(" sel");
    }
    c
}

fn text_class(fc: &Forecast, d: Day, mode: Mode) -> &'static str {
    match fc.kind(d) {
        Kind::Period => "cell-num strong",
        Kind::Predicted => "cell-num rose",
        Kind::Ovulation | Kind::Fertile => "cell-num fert",
        Kind::None if mode == Mode::Pick && d > fc.today => "cell-num off",
        Kind::None if d == fc.today => "cell-num today",
        Kind::None => "cell-num",
    }
}

/// Обводка «ручкой»: чуть неровный овал с нахлёстом концов, у каждой даты свой.
fn pen_ring(d: Day, size: f32, ink: Color) -> Canvas {
    let seed = (d as u32).wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32;
    Canvas::new(move |c, _| {
        let (cx, cy) = (size / 2.0, size / 2.0);
        let tilt = (seed - 0.5) * 0.5;
        let a0 = seed * TAU;
        let n = 56;
        let mut pts = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let t = i as f32 / n as f32;
            // чуть больше оборота, радиус слегка растёт — концы расходятся, как у росчерка
            let a = a0 + t * TAU * 1.1;
            let grow = 1.0 + 0.06 * t;
            let rx = size * 0.45 * grow * (1.0 + 0.035 * (2.0 * a + seed * 9.0).sin());
            let ry = size * 0.39 * grow * (1.0 + 0.04 * (3.0 * a + seed * 5.0).cos());
            let (x, y) = (rx * a.cos(), ry * a.sin());
            pts.push((cx + x * tilt.cos() - y * tilt.sin(), cy + x * tilt.sin() + y * tilt.cos()));
        }
        c.set_color(ink);
        c.set_stroke_width(1.8);
        c.set_line_cap(LineCap::Round);
        c.set_line_join(LineJoin::Round);
        c.stroke_path(&pts, false);
    })
    .size(size, size)
}

/// Недели месяца `idx` шириной `w` (без заголовка дней недели).
pub fn weeks(st: St, fc: &Forecast, idx: i32, w: f32, mode: Mode) -> W {
    let monday = st.data.get_untracked().settings.monday;
    let ink = crate::pal(st.dark.get_untracked()).ink;
    let gap = 4.0;
    let cell = ((w - gap * 6.0) / 7.0).floor();
    let h = cell.min(48.0);
    let first = date::month_start(idx);
    let n = date::days_in_month(idx);
    let lead = if monday { date::weekday(first) } else { (date::weekday(first) + 1) % 7 } as i32;
    let sel = match mode {
        Mode::Pick => st.intro_day.get_untracked(),
        Mode::Main => None,
    };
    let mut col = Column::new().gap(gap);
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
            let num = Center::new().child(Text::new((k + 1).to_string()).class(text_class(fc, d, mode)));
            let mut face = Stack::new().child(DecoratedBox::new().style("width", StyleValue::px(h)).style("height", StyleValue::px(h)).child(num));
            if mode == Mode::Main && fc.show_intimacy && !fc.intimacy(d).is_empty() {
                face = face.child(pen_ring(d, h, ink));
            }
            let disabled = mode == Mode::Pick && d > fc.today;
            let body = DecoratedBox::new()
                .class("cell-slot")
                .style("width", StyleValue::px(cell))
                .style("height", StyleValue::px(h))
                .child(DecoratedBox::new().class(&cell_class(fc, d, mode, sel)).style("width", StyleValue::px(h)).style("height", StyleValue::px(h)).child(face));
            let tap = match mode {
                Mode::Main => GestureDetector::new().on_click_with_bounds(move |_, r| st.open_menu(d, r)),
                Mode::Pick => GestureDetector::new().on_click(move || {
                    if !disabled {
                        st.intro_day.set(Some(d));
                    }
                }),
            };
            row = row.child(tap.child(body));
        }
        col = col.child(row);
    }
    boxed(col)
}

/// Строка дней недели.
pub fn weekdays(st: St, w: f32) -> W {
    let monday = st.data.get_untracked().settings.monday;
    let gap = 4.0;
    let cell = ((w - gap * 6.0) / 7.0).floor();
    let mut head = Row::new().gap(gap);
    for i in 0..7u32 {
        let wd = if monday { i } else { (i + 6) % 7 };
        head = head.child(DecoratedBox::new().class("wd").style("width", StyleValue::px(cell)).child(Text::new(date::weekday_short(wd)).class(if wd >= 5 { "wd-text weekend" } else { "wd-text" })));
    }
    boxed(head)
}

/// Месяц с днями недели (выбор даты в знакомстве).
pub fn grid(st: St, fc: &Forecast, idx: i32, w: f32, mode: Mode) -> W {
    boxed(Column::new().gap(4.0).child(weekdays(st, w)).child(weeks(st, fc, idx, w, mode)))
}

/// Заголовок месяца с перелистыванием (знакомство).
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

/// Месяц, перетекающий вбок при перелистывании; смахивание листает (знакомство).
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
                .child(AnimatedSwitcher::new((idx + 1_000_000) as u64, move || b(month.get_untracked())).version(ver).slide(60.0, 0.0).directional(true).duration_ms(300).exit_duration_ms(150)),
        )]
    }))
}

// ── Шапка: день и фаза, полоса фаз ────────────────────────────────────────

/// Отрезки полосы фаз: (класс цвета, подпись, первый и последний день).
fn phase_segments(p: &Phases) -> Vec<(&'static str, String, Day, Day)> {
    let mut v = vec![("period", t!("Менструация"), p.start, p.period_end)];
    if p.fertile.0 > p.period_end + 1 {
        v.push(("folli", t!("Фолликулярная"), p.period_end + 1, p.fertile.0 - 1));
    }
    v.push(("fert", t!("Овуляторная"), p.fertile.0.max(p.period_end + 1), p.fertile.1.min(p.next - 1)));
    if p.next - 1 > p.fertile.1 {
        v.push(("lute", t!("Лютеиновая"), p.fertile.1 + 1, p.next - 1));
    }
    v
}

/// Полоса текущего цикла: фазы цветом, сегодня — метка.
fn phase_bar(st: St, p: Phases, today: Day, w: f32) -> W {
    let pal = crate::pal(st.dark.get_untracked());
    let segs: Vec<(Color, Day, Day)> = phase_segments(&p)
        .into_iter()
        .map(|(k, _, a, b)| {
            let c = match k {
                "period" => pal.rose,
                "folli" => pal.folli,
                "fert" => pal.fert,
                _ => pal.lute,
            };
            (c, a, b)
        })
        .collect();
    let len = (p.next - p.start).max(1) as f32;
    let ov = p.ovulation;
    let start = p.start;
    let h = 22.0;
    let canvas = Canvas::new(move |c, t| {
        let e = 1.0 - (1.0 - (t / 0.6).clamp(0.0, 1.0)).powi(3);
        let bar_y = 8.0;
        let bh = 8.0;
        let x = |d: Day| (d - start) as f32 / len * w;
        for &(col, a, b) in &segs {
            let (x0, x1) = (x(a) + 1.0, (x(b + 1) - 1.0).min(w * e));
            if x1 > x0 {
                c.set_color(col);
                c.fill_rounded_rect(x0, bar_y, x1 - x0, bh, bh / 2.0);
            }
        }
        // овуляция — точка, сегодня — вертикальная метка
        let ox = x(ov) + (x(ov + 1) - x(ov)) / 2.0;
        if ox <= w * e {
            c.set_color(pal.fert_strong);
            c.fill_circle(ox, bar_y + bh / 2.0, bh * 0.75);
        }
        let tx = (x(today) + (x(today + 1) - x(today)) / 2.0).clamp(2.0, w - 2.0);
        c.set_color(pal.fg);
        c.fill_rounded_rect(tx - 1.5, 2.0, 3.0, h - 4.0, 1.5);
    })
    .size(w, h)
    .animated_for(0.7)
    .replay((p.start as u64) << 20 ^ p.next as u64);
    let mut legend = Flex::new().direction(FlexDirection::Row).wrap().gap(14.0);
    for (k, label, a, b) in phase_segments(&p) {
        let days = if a == b { format!("{}", a - p.start + 1) } else { format!("{}–{}", a - p.start + 1, b - p.start + 1) };
        legend = legend.child(
            Row::new()
                .gap(6.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class(&format!("ph-dot {k}")))
                .child(Text::new(label).class("ph-label"))
                .child(Text::new(days).class("ph-days")),
        );
    }
    boxed(Column::new().gap(8.0).child(canvas).child(legend))
}

fn header(st: St, wide: bool, w: f32) -> W {
    let fc = st.fc.get_untracked();
    let today = fc.today;
    let (title, phase, sub) = match (fc.phase(), fc.cycle_day(today)) {
        (Some(ph), Some(cd)) => {
            let mut sub = Vec::new();
            if fc.late > 0 {
                sub.push(tn!(fc.late, "задержка {n} день", "задержка {n} дня", "задержка {n} дней"));
            } else if let Some(&n) = fc.next.first() {
                sub.push(t!("месячные {when}", when = date::relative(n, today)));
            }
            if let Some(ov) = fc.next_ovulation() {
                sub.push(t!("овуляция {when}", when = date::relative(ov, today)));
            }
            let tone = match ph {
                Phase::Period | Phase::Late => "period",
                Phase::Follicular => "folli",
                Phase::Fertile | Phase::Ovulation => "fert",
                Phase::Luteal => "lute",
            };
            (t!("День {n}", n = cd), Some((phase_name(ph), tone)), sub.join(" · "))
        }
        _ => (t!("Цикл"), None, t!("Нажмите на первый день месячных и выберите «Начало месячных»")),
    };
    let mut title_row = Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(title).max_lines(1).class("hd-title"));
    if let Some((name, tone)) = phase {
        title_row = title_row.child(DecoratedBox::new().class(&format!("ph-pill {tone}")).child(Text::new(name).max_lines(1).class("ph-pill-text")));
    }
    let mut top = Row::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center);
    top = top.child(DecoratedBox::new().class("grow").child(Column::new().gap(4.0).child(title_row).child(Text::new(sub).max_lines(2).class("hint"))));
    top = top.child(Button::new(t!("Сегодня")).class("btn chip").on_click(move || scroll_to_named(format!("m{}", date::month_index(date::today())))));
    if !wide || st.window.get_untracked().fullscreen {
        top = top.child(super::fullscreen_button(st));
    }
    let mut col = Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(syngui::overlay::WindowDragRegion::new().child(top));
    if let Some(p) = fc.phases(today) {
        col = col.child(phase_bar(st, p, today, w));
    }
    col = col.child(weekdays(st, w));
    boxed(DecoratedBox::new().class("cal-head").child(col))
}

/// Лента месяцев: от первой отметки (не больше трёх лет назад) до девяти месяцев вперёд.
fn ribbon(st: St, w: f32) -> W {
    let fc = st.fc.get_untracked();
    let cur = date::month_index(fc.today);
    let first = fc.cycles.first().map(|c| date::month_index(c.start)).unwrap_or(cur).max(cur - BACK_MAX).min(cur - 1);
    let (cur_y, _, _) = date::ymd(fc.today);
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for idx in first..=cur + AHEAD {
        let (y, m, _) = date::ymd(date::month_start(idx));
        let title = if y == cur_y { date::month_name(m) } else { format!("{} {y}", date::month_name(m)) };
        let head = Text::new(title).class(if idx == cur { "month-title cur" } else { "month-title" });
        col = col.child(Named::new(format!("m{idx}"), DecoratedBox::new().class("month-head").child(head)));
        col = col.child(weeks(st, &fc, idx, w, Mode::Main));
    }
    col = col.child(legend(st));
    boxed(col)
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
        f = f.child(legend_item("fertile", t!("Фертильное окно")));
    }
    if s.show_ovulation {
        f = f.child(legend_item("ovul", t!("Овуляция")));
    }
    if s.show_intimacy {
        f = f.child(legend_item("love", t!("Близость")));
    }
    boxed(DecoratedBox::new().class("legend").child(f))
}

pub fn page(st: St, wide: bool) -> W {
    let w = super::content_width(wide);
    let cur = date::month_index(st.today.get_untracked());
    // при первом показе — к текущему месяцу (после раскладки ленты: запрос к ещё не построенному месяцу пропал бы)
    if !SCROLLED.with(|s| s.replace(true)) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            run_on_main_thread(move || scroll_to_named(format!("m{cur}")));
        });
    }
    // Шапка и содержимое ленты перестраиваются при правках, а сама прокрутка — нет: позиция сохраняется.
    let head = Reactive::new(move || -> Vec<W> {
        let _ = st.rev.get();
        let _ = st.dark.get();
        vec![header(st, wide, w)]
    });
    let months = Reactive::new(move || -> Vec<W> {
        let _ = st.rev.get();
        let _ = st.dark.get();
        vec![ribbon(st, w)]
    });
    let body = ScrollView::new().vertical().class("ribbon").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("ribbon-inner").style("width", StyleValue::px(w + 32.0)).child(months)));
    boxed(
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("cal-head-wrap").style("width", StyleValue::px(w + 32.0)).child(head)))
            .child(DecoratedBox::new().class("grow").child(body)),
    )
}

thread_local! {
    static SCROLLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// ── Меню отметок у даты ─────────────────────────────────────────────────────

fn menu_row(glyph: &str, tone: &str, label: String, on: bool, enabled: bool, f: impl FnMut() + Send + 'static) -> W {
    let mut f = f;
    let mut row = Row::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(DecoratedBox::new().class(&format!("mi-badge {tone}")).child(icon(glyph).class("ic mi-ic")))
        .child(DecoratedBox::new().class("grow").child(Text::new(label).class(if enabled { "mi-text" } else { "mi-text off" })));
    if on {
        row = row.child(icon(icons::CHECK).class("ic mi-check"));
    }
    boxed(
        GestureDetector::new()
            .on_click(move || {
                if enabled {
                    f();
                }
            })
            .child(DecoratedBox::new().class("mi").child(row)),
    )
}

fn menu_body(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let _ = st.rev.get();
        let Some(d) = st.menu_day.get() else { return vec![] };
        let fc = st.fc.get_untracked();
        let today = fc.today;
        let future = d > today;
        let close = move || st.menu_open.set(false);
        let mut sub = date::weekday_name(date::weekday(d));
        if let Some(cd) = fc.cycle_day(d) {
            sub = format!("{sub} · {}", t!("день цикла {n}", n = cd));
        }
        let mut col = Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
        col = col.child(DecoratedBox::new().class("menu-head").child(Column::new().gap(1.0).child(Text::new(date::long(d)).class("menu-title")).child(Text::new(sub).max_lines(1).class("hint"))));
        let is_period = fc.log(d).flow > 0;
        col = col.child(menu_row(icons::DROP, "rose", if future { t!("Месячные — будущий день") } else { t!("Месячные") }, is_period, !future, move || {
            st.toggle_period(d);
            close();
        }));
        if !is_period && !future {
            let n = st.data.get_untracked().settings.period_len;
            col = col.child(menu_row(icons::EVENT, "rose", tn!(n, "Начало месячных · {n} день", "Начало месячных · {n} дня", "Начало месячных · {n} дней"), false, true, move || {
                st.start_period(d);
                close();
            }));
        }
        col = col.child(menu_row(icons::EGG, "fert", t!("Овуляция"), fc.ovulation_marked(d), true, move || {
            st.toggle_ovulation(d);
            close();
        }));
        let love = !fc.intimacy(d).is_empty();
        col = col.child(menu_row(icons::HEART, "ink", t!("Близость"), love, !future, move || {
            st.toggle_intimacy(d);
            close();
        }));
        col = col.child(DecoratedBox::new().class("menu-sep"));
        col = col.child(menu_row(icons::TUNE, "plain", t!("Подробнее: время, симптомы, настроение"), false, true, move || st.open_day(d)));
        vec![boxed(col)]
    }))
}

/// Меню отметок — всплывает у нажатой даты.
pub fn menu_layer(st: St) -> W {
    boxed(PopupPanel::new().is_open(st.menu_open).anchor_rect(st.menu_rect).anchor(PopupAnchor::BottomCenter).min_width(260.0).max_width(320.0).class("day-menu").child(menu_body(st)))
}
