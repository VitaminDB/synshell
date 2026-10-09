//! Экран «Спорт и здоровье»: шаги за сегодня (кольцо цели), расстояние и калории, шаги по
//! часам, неделя, настройки (цель, длина шага, вес).

use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::charts::{AxisConfig, BarChart, BarSeries, LegendPosition};

use crate::store::{self, Settings, Steps};

#[derive(Clone, Copy)]
pub struct St {
    pub steps: RwSignal<Steps>,
    pub settings: RwSignal<Settings>,
    pub settings_open: RwSignal<bool>,
    /// Шагомер есть (источник платформы).
    pub sensor: bool,
}

impl St {
    pub fn new() -> Self {
        St {
            steps: use_signal(Steps::load()),
            settings: use_signal(Settings::load()),
            settings_open: use_signal(false),
            sensor: std::path::Path::new(store::SOURCE).exists() || std::env::var_os("SYN_HEALTH_DEMO").is_some(),
        }
    }
}

/// Перечитывать файл шагов (его пишет счётчик в фоне).
pub fn start(st: St) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let s = Steps::load();
        run_on_main_thread(move || {
            if st.steps.get_untracked() != s {
                st.steps.set(s);
            }
        });
    });
}

fn accent() -> Color {
    Color::from_hex("#ff7043")
}

fn ring(value: f32, size: f32, big: String, sub: String) -> impl Widget {
    let v = value.clamp(0.0, 1.0);
    let done = value >= 1.0;
    let canvas = Canvas::new(move |c, _| {
        let stroke = size / 11.0;
        let r = size / 2.0 - stroke / 2.0 - 2.0;
        let (cx, cy) = (size / 2.0, size / 2.0);
        c.set_stroke_width(stroke);
        c.set_color(accent().with_alpha(0.18));
        c.stroke_circle(cx, cy, r);
        if v > 0.001 {
            c.set_color(if done { Color::from_hex("#66bb6a") } else { accent() });
            let a0 = -std::f32::consts::FRAC_PI_2;
            c.draw_arc(cx, cy, r, a0, a0 + v * std::f32::consts::TAU);
        }
    })
    .size(size, size);
    Stack::new().child(canvas).child(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(big).class("ring-value"))
            .child(Text::new(sub).class("muted"))
            .style("width", StyleValue::px(size))
            .style("height", StyleValue::px(size)),
    )
}

fn stat(value: String, label: String) -> impl Widget {
    Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(value).class("stat-value")).child(Text::new(label).class("muted small")).class("stat grow")
}

pub fn root(st: St) -> impl Widget {
    let body = move || -> Box<dyn Widget> {
        syngui::i18n::subscribe();
        let vp = viewport_size().get();
        if st.settings_open.get() {
            return Box::new(ScrollView::new().vertical().child(settings(st)));
        }
        let steps = st.steps.get();
        let set = st.settings.get();
        let (today, hour) = store::local_date(0);
        let day = steps.day(&today);
        let w = (vp.width - 32.0).clamp(260.0, 720.0);
        let ring_size = (w * 0.6).clamp(180.0, 280.0);
        let mut col = Column::new().gap(18.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if !st.sensor {
            col = col.child(Text::new(t!("Шагомера нет: шаги считаются на телефоне с датчиками SLPI. Здесь — сохранённая история.")).max_lines(3).class("muted center note"));
        }
        let pct = day.total as f32 / set.goal.max(1) as f32;
        col = col
            .child(ring(pct, ring_size, day.total.to_string(), t!("из {goal} шагов", goal = set.goal)))
            .child(
                Row::new()
                    .gap(8.0)
                    .child(stat(synshell_common::decimal(set.km(day.total) as f64, 2), t!("км")))
                    .child(stat(format!("{:.0}", set.kcal(day.total)), t!("ккал")))
                    .child(stat(format!("{}%", (pct * 100.0).round() as i32), t!("цели")))
                    .style("width", StyleValue::px(w)),
            );
        // по часам до текущего
        let hours: Vec<f64> = day.hours.iter().take(hour + 1).map(|v| *v as f64).collect();
        col = col.child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .child(Text::new(t!("Сегодня по часам")).class("card-title"))
                        .child(
                            BarChart::new()
                                .categories((0..hours.len()).map(|h| if h % 3 == 0 { h.to_string() } else { String::new() }).collect())
                                .bar_series(BarSeries::new(t!("Шаги"), hours).color(accent()))
                                .y_axis(AxisConfig::new().min(0.0).tick_count(3).axis_line(false))
                                .x_axis(AxisConfig::new().grid(false))
                                .legend(LegendPosition::None)
                                .tooltip(false)
                                .animate(false)
                                .bar_radius(3.0)
                                .size(w - 28.0, 150.0),
                        ),
                )
                .class("card"),
        );
        // неделя
        const DAYS: [&str; 7] = [n_!("Пн"), n_!("Вт"), n_!("Ср"), n_!("Чт"), n_!("Пт"), n_!("Сб"), n_!("Вс")];
        let mut cats = vec![];
        let mut vals = vec![];
        let mut sum = 0u32;
        for ago in (0..7).rev() {
            let (date, _) = store::local_date(ago);
            let total = steps.day(&date).total;
            sum += total;
            cats.push(syngui::i18n::t(DAYS[store::weekday(ago)]));
            vals.push(total as f64);
        }
        col = col.child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(6.0)
                        .child(Row::new().child(Text::new(t!("Неделя")).class("card-title grow")).child(Text::new(t!("в среднем {n} в день", n = sum / 7)).class("muted small")))
                        .child(
                            BarChart::new()
                                .categories(cats)
                                .bar_series(BarSeries::new(t!("Шаги"), vals).color(accent()))
                                .y_axis(AxisConfig::new().min(0.0).tick_count(3).axis_line(false))
                                .x_axis(AxisConfig::new().grid(false))
                                .legend(LegendPosition::None)
                                .tooltip(false)
                                .animate(false)
                                .bar_radius(4.0)
                                .size(w - 28.0, 150.0),
                        ),
                )
                .class("card"),
        );
        col = col.child(Button::new(t!("Настройки")).icon("\u{E8B8}").on_click(move || st.settings_open.set(true)));
        Box::new(ScrollView::new().vertical().child(col.class("page")))
    };
    DecoratedBox::new().child(syngui::widgets::Reactive::new(move || vec![body()])).class("root")
}

fn stepper(label: String, value: String, minus: impl Fn() + Send + Sync + 'static, plus: impl Fn() + Send + Sync + 'static) -> impl Widget {
    Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(label).class("grow"))
        .child(Button::new("−").on_click(move || minus()))
        .child(Text::new(value).class("stat-value step-value"))
        .child(Button::new("+").on_click(move || plus()))
        .class("set-row")
}

fn settings(st: St) -> impl Widget {
    let s = st.settings.get();
    let upd = move |f: fn(&mut Settings)| {
        let mut s = st.settings.get_untracked();
        f(&mut s);
        s.save();
        st.settings.set(s);
    };
    Column::new()
        .gap(14.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(Text::new(t!("Настройки")).class("title"))
        .child(stepper(t!("Цель, шагов в день"), s.goal.to_string(), move || upd(|s| s.goal = s.goal.saturating_sub(1000).max(1000)), move || upd(|s| s.goal = (s.goal + 1000).min(50000))))
        .child(stepper(t!("Длина шага, см"), format!("{:.0}", s.step_m * 100.0), move || upd(|s| s.step_m = (s.step_m - 0.05).max(0.3)), move || upd(|s| s.step_m = (s.step_m + 0.05).min(1.5))))
        .child(stepper(t!("Вес, кг"), format!("{:.0}", s.weight_kg), move || upd(|s| s.weight_kg = (s.weight_kg - 1.0).max(30.0)), move || upd(|s| s.weight_kg = (s.weight_kg + 1.0).min(200.0))))
        .child(Text::new(t!("Расстояние — шаги × длина шага; калории — примерно 0,5 ккал на килограмм веса на километр ходьбы.")).max_lines(3).class("muted small"))
        .child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(Button::new(t!("Готово")).class("primary").on_click(move || st.settings_open.set(false))))
        .class("page")
}
