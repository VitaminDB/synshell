//! «Календарь» — главный экран: сверху день и фаза цикла с полосой фаз, ниже лента месяцев.
//! Даты подсвечены целиком: месячные — заливкой, прогноз — бледной заливкой, фертильное окно и
//! овуляция — жёлтым разной плотности, близость — обводкой «ручкой». Нажатие на дату — меню отметок.

use syngui::prelude::*;
use syngui::widgets::{scroll_to_named, Named, SwipeDirection};
use syngui::GestureDetector;
use syngui::StyleValue;
use syngui::{PopupAnchor, PopupPanel};

use super::phase::{self, PhaseKey};
use super::{boxed, icon, icon_button, W};
use crate::date::{self, Day};
use crate::decor::{self, Decor, Ink, OvulMark};
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

fn cell_class(fc: &Forecast, d: Day, mode: Mode, sel: Option<Day>, shape: bool) -> String {
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
    // отметка нарисована фигурой (капля, цветок) — без заливки круга
    if shape {
        c.push_str(" shape");
    }
    c
}

/// Фигура отметки под цифрой: капля месячных (бледная — прогноз), знак овуляции.
fn shape_canvas(decor: Decor, kind: Kind, size: f32, ink: Ink) -> Canvas {
    Canvas::new(move |c, _| {
        let (cx, cy) = (size / 2.0, size / 2.0);
        match kind {
            Kind::Period => decor::paint_drop(c, cx, cy, size * 0.98, ink.blood, true),
            Kind::Predicted => decor::paint_drop(c, cx, cy, size * 0.92, ink.blood_soft, false),
            Kind::Ovulation => decor::paint_ovul(c, decor.ovul_mark(), size, &ink),
            _ => {}
        }
    })
    .size(size, size)
}

/// Сердечко близости в правом верхнем углу даты, с обводкой цветом фона.
fn heart_badge(size: f32, fill: Color, edge: Color) -> Canvas {
    Canvas::new(move |c, _| decor::paint_heart(c, size * 0.82, size * 0.2, size * 0.36, fill, Some(edge))).size(size, size)
}

fn text_class(fc: &Forecast, d: Day, mode: Mode) -> &'static str {
    // сегодня — белая цифра на залитом кружке
    if d == fc.today {
        return "cell-num today";
    }
    match fc.kind(d) {
        Kind::Period => "cell-num strong",
        Kind::Predicted => "cell-num rose",
        Kind::Ovulation | Kind::Fertile => "cell-num fert",
        Kind::None if mode == Mode::Pick && d > fc.today => "cell-num off",
        Kind::None => "cell-num",
    }
}

/// Недели месяца `idx` шириной `w` (без заголовка дней недели).
pub fn weeks(st: St, fc: &Forecast, idx: i32, w: f32, mode: Mode) -> W {
    let monday = st.data.get_untracked().settings.monday;
    let decor = st.data.get_untracked().settings.decor;
    let dark = st.dark.get_untracked();
    let dink = decor.ink(dark);
    let bg = crate::bg_color(dark, decor);
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
            let kind = fc.kind(d);
            let shape = decor.shapes()
                && d != fc.today
                && match kind {
                    Kind::Period | Kind::Predicted => true,
                    Kind::Ovulation => decor.ovul_mark() != OvulMark::Circle,
                    _ => false,
                };
            // цифра на значке овуляции — тёмная на светлом лепестке, белая на тёмном
            let tc = if shape && kind == Kind::Ovulation {
                if dink.petal.relative_luminance() > 0.4 { "cell-num on-mark-dark" } else { "cell-num on-mark-light" }
            } else {
                text_class(fc, d, mode)
            };
            let num = Center::new().child(Text::new((k + 1).to_string()).class(tc));
            let mut face = Stack::new();
            if shape {
                face = face.child(shape_canvas(decor, kind, h, dink));
            }
            face = face.child(DecoratedBox::new().style("width", StyleValue::px(h)).style("height", StyleValue::px(h)).child(num));
            if mode == Mode::Main && fc.show_intimacy && !fc.intimacy(d).is_empty() {
                face = face.child(heart_badge(h, dink.heart, bg));
            }
            let disabled = mode == Mode::Pick && d > fc.today;
            let body = DecoratedBox::new()
                .class("cell-slot")
                .style("width", StyleValue::px(cell))
                .style("height", StyleValue::px(h))
                .child(DecoratedBox::new().class(&cell_class(fc, d, mode, sel, shape)).style("width", StyleValue::px(h)).style("height", StyleValue::px(h)).child(face));
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
    row = row.child(DecoratedBox::new().class("grow").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(date::month_year(m, y)).class("month-title"))));
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

fn phase_color(pal: &crate::Pal, k: PhaseKey) -> Color {
    match k {
        PhaseKey::Period => pal.rose,
        PhaseKey::Follicular => pal.folli,
        PhaseKey::Fertile => pal.fert,
        PhaseKey::Luteal => pal.lute,
        PhaseKey::Late => pal.track,
    }
}

/// Полоса текущего цикла: фазы цветными отрезками (прошедшие дни ярче), сегодня — кольцо,
/// овуляция — точка. Нажатие на отрезок открывает описание фазы.
fn phase_bar(st: St, p: Phases, today: Day, w: f32) -> W {
    let pal = crate::pal_st(st);
    let segs: Vec<(Color, Day, Day)> = phase::segments(&p).into_iter().map(|(k, a, b)| (phase_color(&pal, k), a, b)).collect();
    let len = (p.next - p.start).max(1) as f32;
    let ov = p.ovulation;
    let start = p.start;
    let surface = pal.surface;
    let h = 26.0;
    let canvas = Canvas::new(move |c, t| {
        let e = 1.0 - (1.0 - (t / 0.6).clamp(0.0, 1.0)).powi(3);
        let bh = 12.0;
        let cy = h / 2.0;
        let x = |d: Day| (d - start) as f32 / len * w;
        let lim = w * e;
        for &(col, a, b) in &segs {
            let (x0, x1) = (x(a) + 1.5, (x(b + 1) - 1.5).min(lim));
            if x1 <= x0 {
                continue;
            }
            c.set_color(col);
            c.fill_rounded_rect(x0, cy - bh / 2.0, x1 - x0, bh, bh / 2.0);
        }
        let ox = x(ov) + (x(ov + 1) - x(ov)) / 2.0;
        if ox <= lim {
            c.set_color(surface);
            c.fill_circle(ox, cy, 4.0);
            c.set_color(pal.fert_strong);
            c.fill_circle(ox, cy, 2.4);
        }
        let tx = (x(today) + (x(today + 1) - x(today)) / 2.0).clamp(h / 2.0, w - h / 2.0);
        if tx <= lim + 1.0 {
            c.set_color(pal.fg);
            c.fill_circle(tx, cy, 11.0);
            c.set_color(surface);
            c.fill_circle(tx, cy, 7.5);
            c.set_color(pal.fg);
            c.fill_circle(tx, cy, 4.0);
        }
    })
    .size(w, h)
    .animated_for(0.7)
    .replay((p.start as u64) << 20 ^ p.next as u64);
    let pc = p;
    let bar = GestureDetector::new()
        .on_click_with_bounds(move |pt, r| {
            let f = ((pt.x - r.origin.x) / r.size.width.max(1.0)).clamp(0.0, 0.999);
            let d = pc.start + (f * (pc.next - pc.start) as f32) as i32;
            st.open_phase(phase::key_at(&pc, d));
        })
        .child(canvas);
    // подписи под полосой: начало, сегодня, конец цикла
    let ticks = Row::new()
        .child(Text::new(date::short(p.start)).class("ph-tick"))
        .child(DecoratedBox::new().class("grow"))
        .child(Text::new(t!("до {d}", d = date::short(p.next - 1))).class("ph-tick"));
    let cur = phase::key_at(&p, today);
    let mut chips = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (k, a, b) in phase::segments(&p) {
        let days = if a == b { format!("{}", a - p.start + 1) } else { format!("{}–{}", a - p.start + 1, b - p.start + 1) };
        let on = k == cur;
        chips = chips.child(
            GestureDetector::new().class("grow").on_click(move || st.open_phase(k)).child(
                DecoratedBox::new().class(&if on { format!("ph-chip on {}", k.tone()) } else { format!("ph-chip {}", k.tone()) }).child(
                    Column::new()
                        .gap(3.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .child(Row::new().gap(5.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(&format!("ph-dot {}", k.tone()))).child(Text::new(days).max_lines(1).class("ph-days")))
                        .child(Text::new(k.tab()).max_lines(1).class(if on { "ph-label on" } else { "ph-label" })),
                ),
            ),
        );
    }
    boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(bar).child(ticks)).child(chips))
}

/// Ближайшее событие: точка, подпись и когда.
fn kv(dot: &str, label: String, value: String) -> W {
    boxed(
        Row::new()
            .gap(6.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class(&format!("ph-dot {dot}")))
            .child(Text::new(label).max_lines(1).class("kv-label"))
            .child(Text::new(value).max_lines(1).class("kv-value")),
    )
}

fn header(st: St, wide: bool, w: f32) -> W {
    let fc = st.fc.get_untracked();
    let today = fc.today;
    let cw = w - 40.0;
    let over = format!("{}, {}", date::weekday_name(date::weekday(today)), date::long(today));
    let mut top = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
    top = top.child(DecoratedBox::new().class("grow").child(Text::new(over).max_lines(1).class("hd-over")));
    top = top.child(Button::new(t!("Сегодня")).class("btn chip").on_click(move || scroll_to_named(format!("m{}", date::month_index(date::today())))));
    // iPhone: окно всегда на весь экран, переключать нечего.
    if !cfg!(target_os = "ios") && (!wide || st.window.get_untracked().fullscreen) {
        top = top.child(super::fullscreen_button(st));
    }
    let mut col = Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(syngui::overlay::WindowDragRegion::new().child(top));
    match (fc.phase(), fc.cycle_day(today), fc.phases(today)) {
        (Some(ph), Some(cd), Some(p)) => {
            let k = PhaseKey::of(ph);
            let name = if fc.late > 0 { phase_name(Phase::Late) } else { phase_name(ph) };
            let mut big = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::End).child(Text::new(t!("День {n}", n = cd)).max_lines(1).class("hd-title"));
            if fc.late == 0 {
                big = big.child(Text::new(t!("из {n}", n = p.next - p.start)).max_lines(1).class("hd-of"));
            }
            let pill = GestureDetector::new().on_click(move || st.open_phase(k)).child(
                DecoratedBox::new().class(&format!("ph-pill {}", k.tone())).child(
                    Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class(&format!("ph-dot {}", k.tone()))).child(Text::new(name).max_lines(1).class("ph-pill-text")).child(icon(icons::NEXT).class("ic ph-pill-ic")),
                ),
            );
            col = col.child(Column::new().gap(10.0).child(big).child(Row::new().child(pill)));
            let mut kvs = Row::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center);
            if fc.late > 0 {
                kvs = kvs.child(kv("period", t!("Месячные"), tn!(fc.late, "задержка {n} день", "задержка {n} дня", "задержка {n} дней")));
            } else if let Some(&n) = fc.next.first() {
                kvs = kvs.child(kv("period", t!("Месячные"), date::relative(n, today)));
            }
            // при задержке срок овуляции неизвестен, пока не начнутся месячные
            if let Some(ov) = fc.next_ovulation().filter(|_| fc.late == 0) {
                kvs = kvs.child(kv("fert", t!("Овуляция"), date::relative(ov, today)));
            }
            col = col.child(kvs);
            col = col.child(phase_bar(st, p, today, cw));
        }
        _ => {
            col = col.child(Text::new(t!("Цикл")).class("hd-title")).child(Text::new(t!("Нажмите на первый день месячных и выберите «Начало месячных»")).max_lines(3).class("hint"));
        }
    }
    boxed(Column::new().gap(14.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(DecoratedBox::new().class("head-card").child(col)).child(weekdays(st, w)))
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
        let title = if y == cur_y { date::month_name(m) } else { date::month_year(m, y) };
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

/// Пункт легенды с фигурой оформления (капля, цветок, сердечко).
fn legend_shape(canvas: Canvas, label: String) -> W {
    boxed(Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center).child(canvas).child(Text::new(label).class("legend-text")))
}

fn legend(st: St) -> W {
    let s = st.data.get_untracked().settings;
    let decor = s.decor;
    let ink = decor.ink(st.dark.get_untracked());
    let shapes = decor.shapes();
    let mut f = Flex::new().direction(FlexDirection::Row).wrap().gap(12.0);
    f = f.child(legend_item("today", t!("Сегодня")));
    if shapes {
        f = f.child(legend_shape(shape_canvas(decor, Kind::Period, 18.0, ink), t!("Месячные")));
        f = f.child(legend_shape(shape_canvas(decor, Kind::Predicted, 18.0, ink), t!("Прогноз")));
    } else {
        f = f.child(legend_item("period", t!("Месячные")));
        f = f.child(legend_item("predicted", t!("Прогноз")));
    }
    if s.show_fertile {
        f = f.child(legend_item("fertile", t!("Фертильное окно")));
    }
    if s.show_ovulation {
        f = f.child(if shapes && decor.ovul_mark() != OvulMark::Circle { legend_shape(shape_canvas(decor, Kind::Ovulation, 18.0, ink), t!("Овуляция")) } else { legend_item("ovul", t!("Овуляция")) });
    }
    if s.show_intimacy {
        f = f.child(legend_shape(Canvas::new(move |c, _| decor::paint_heart(c, 9.0, 9.0, 14.0, ink.heart, None)).size(18.0, 18.0), t!("Близость")));
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
