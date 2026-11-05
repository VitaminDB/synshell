//! Экран статистики с графиками.

use syngui::mgui;
use syngui::prelude::*;
use syngui::widgets::*;

use std::collections::HashMap;

use syngui::GestureDetector;

use crate::app_state::AppState;
use crate::model::log::CigaretteLog;
use crate::model::profile::QuitMode;
use crate::model::stats;
use crate::util::{civil_from_days, local_day_index, local_hm, local_hour, weekday_mon0};
use crate::widgets::components::{card, screen_title, section_title};

pub fn build(state: AppState) -> impl Widget {
    // Выбранный день календаря — вне реактивного body, чтобы сохранялся.
    let selected_day = use_signal(None::<i64>);
    Column::new().child(move || body(state, selected_day))
}

fn body(state: AppState, selected_day: RwSignal<Option<i64>>) -> impl Widget {
    let now = state.now_tick.get();
    let p = state.profile.get();
    let log = state.log.get();
    let s = stats::compute(&p, &log, now);

    // Круговая: избегнуто vs выкурено.
    let pie = PieChart::new()
        .slice(PieSlice::new(state.tr("stats.avoided"), s.cigs_avoided.max(0.0)).color(Color::from_hex("#2fbf71")))
        .slice(PieSlice::new(state.tr("stats.smoked"), s.logged_count.max(1) as f64).color(Color::from_hex("#f4a259")))
        .donut(0.62)
        .legend(LegendPosition::Bottom)
        .animate(true)
        .size(360.0, 300.0)
        .class("chart");

    // Столбцы: экономия по неделям (последние 4).
    let per_cig = if p.cigs_per_pack > 0 {
        p.price_per_pack / p.cigs_per_pack as f64
    } else {
        0.0
    };
    let baseline_week = p.cigs_per_day as f64 * 7.0;
    let week_ms: u64 = 7 * 86_400_000;
    let mut cats = Vec::new();
    let mut vals = Vec::new();
    for w in (0..4).rev() {
        let end = now.saturating_sub(w * week_ms);
        let start = end.saturating_sub(week_ms).max(p.quit_started_at);
        let logged = log.count_in(start, end) as f64;
        let elapsed_frac = ((end.saturating_sub(start)) as f64 / week_ms as f64).clamp(0.0, 1.0);
        let avoided = (baseline_week * elapsed_frac - logged).max(0.0);
        vals.push(avoided * per_cig);
        cats.push(if w == 0 {
            state.tr("stats.week").to_string()
        } else {
            format!("−{w}")
        });
    }
    let bar = BarChart::new()
        .categories(cats)
        .bar_series(BarSeries::new(state.tr("stats.money"), vals).color(Color::from_hex("#f4a259")))
        .bar_radius(6.0)
        .value_labels(true)
        .animate(true)
        .size(320.0, 240.0)
        .class("chart");

    // Индикатор: прогресс программы / стрика.
    let gauge_val = match p.mode {
        QuitMode::Gradual => {
            let d = state.taper().day_fraction(now);
            (d / p.taper_days.max(1) as f64 * 100.0).clamp(0.0, 100.0)
        }
        QuitMode::Immediate => {
            let days = s.streak_secs as f64 / 86_400.0;
            (days / 30.0 * 100.0).clamp(0.0, 100.0)
        }
    };
    let gauge = GaugeChart::new()
        .value(gauge_val)
        .min(0.0)
        .max(100.0)
        .segment(GaugeSegment::new(0.0, 40.0, Color::from_hex("#f4a259")))
        .segment(GaugeSegment::new(40.0, 75.0, Color::from_hex("#8bd450")))
        .segment(GaugeSegment::new(75.0, 100.0, Color::from_hex("#2fbf71")))
        .show_value(true)
        .animate(true)
        .size(300.0, 240.0)
        .class("chart");

    // Календарь-хитмап выкуренных и график по времени суток (все режимы).
    let heatmap = cig_heatmap(state, &log, now, selected_day);
    let by_hour = if log.count() > 0 {
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(centered(time_of_day(state, &log)))
    } else {
        Column::new()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(Text::new(state.tr("stats.no_smoke_data")).class("stats-empty"))
    };

    mgui! {
        Column::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Stretch) => [
            screen_title(state.tr("stats.title")),
            section_title(state.tr("stats.smoked_vs_avoided")),
            card("", centered(pie)),
            section_title(state.tr("stats.calendar")),
            card("", heatmap),
            section_title(state.tr("stats.by_hour")),
            card("", by_hour),
            section_title(state.tr("stats.money")),
            card("", centered(bar)),
            section_title(state.tr("stats.progress")),
            card("", centered(gauge)),
        ]
    }
}

/// Интерактивный календарь-хитмап (последние ~15 недель, стиль «вкладов»).
/// Подписи дней недели слева; клик по дню открывает детали (дата, число, часы).
fn cig_heatmap(
    state: AppState,
    log: &CigaretteLog,
    now: u64,
    selected_day: RwSignal<Option<i64>>,
) -> impl Widget {
    let mut counts: HashMap<i64, u32> = HashMap::new();
    for e in &log.entries {
        *counts.entry(local_day_index(e.at)).or_insert(0) += 1;
    }
    let today = local_day_index(now);
    const WEEKS: i64 = 15;
    let this_monday = today - weekday_mon0(today) as i64;
    let start_monday = this_monday - (WEEKS - 1) * 7;
    let sel = selected_day.get();

    // Подписи дней недели слева (через одну: Пн / Ср / Пт / Вс).
    let wd_raw = state.tr("stats.weekdays");
    let wd: Vec<&str> = wd_raw.split_whitespace().collect();
    let mut wd_col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::End);
    for row in 0..7usize {
        let label = if row % 2 == 0 { wd.get(row).copied().unwrap_or("") } else { "" };
        wd_col = wd_col.child(Text::new(label).class("heat-wd"));
    }

    let mut grid = Row::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .child(wd_col);
    for col in 0..WEEKS {
        let mut week = Column::new().gap(4.0);
        for row in 0..7 {
            let day = start_monday + col * 7 + row;
            let base = if day > today {
                "heat-cell heat-future"
            } else {
                match counts.get(&day).copied().unwrap_or(0) {
                    0 => "heat-cell heat-0",
                    1 => "heat-cell heat-1",
                    2 => "heat-cell heat-2",
                    3..=4 => "heat-cell heat-3",
                    _ => "heat-cell heat-4",
                }
            };
            let cls = if sel == Some(day) { format!("{base} heat-sel") } else { base.to_string() };
            let cell = GestureDetector::new()
                .child(DecoratedBox::new().class(cls))
                .on_click(move || selected_day.set(Some(day)));
            week = week.child(cell);
        }
        grid = grid.child(week);
    }

    let legend = Row::new()
        .gap(5.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(state.tr("stats.less")).class("heat-legend"))
        .child(DecoratedBox::new().class("heat-cell heat-0"))
        .child(DecoratedBox::new().class("heat-cell heat-1"))
        .child(DecoratedBox::new().class("heat-cell heat-2"))
        .child(DecoratedBox::new().class("heat-cell heat-3"))
        .child(DecoratedBox::new().class("heat-cell heat-4"))
        .child(Text::new(state.tr("stats.more")).class("heat-legend"));

    Column::new()
        .gap(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(centered(grid))
        .child(legend)
        .child(heat_detail(state, log, &counts, sel))
}

/// Панель деталей выбранного дня календаря: дата, число сигарет и время каждой.
fn heat_detail(
    state: AppState,
    log: &CigaretteLog,
    counts: &HashMap<i64, u32>,
    sel: Option<i64>,
) -> impl Widget {
    let mut col = Column::new()
        .gap(4.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .class("heat-detail");
    match sel {
        None => {
            col = col.child(Text::new(state.tr("stats.tap_day")).class("heat-detail-hint"));
        }
        Some(day) => {
            let (y, m, d) = civil_from_days(day);
            let cnt = counts.get(&day).copied().unwrap_or(0);
            col = col.child(Text::new(format!("{:02}.{:02}.{}", d, m, y)).class("heat-detail-date"));
            if cnt > 0 {
                let mut times: Vec<(u32, u32)> = log
                    .entries
                    .iter()
                    .filter(|e| local_day_index(e.at) == day)
                    .map(|e| local_hm(e.at))
                    .collect();
                times.sort();
                let times_str = times
                    .iter()
                    .map(|(h, mn)| format!("{:02}:{:02}", h, mn))
                    .collect::<Vec<_>>()
                    .join("   ");
                col = col.child(Text::new(format!("{cnt}×   {times_str}")).class("heat-detail-times"));
            } else {
                col = col.child(Text::new(state.tr("stats.clean_day")).class("heat-detail-clean"));
            }
        }
    }
    col
}

/// Распределение выкуренных по времени суток (24 часа) — для анализа привычки.
fn time_of_day(state: AppState, log: &CigaretteLog) -> impl Widget {
    let mut buckets = [0u32; 24];
    for e in &log.entries {
        buckets[local_hour(e.at) as usize] += 1;
    }
    let cats: Vec<String> = (0..24)
        .map(|h| if h % 3 == 0 { format!("{h}") } else { String::new() })
        .collect();
    let vals: Vec<f64> = buckets.iter().map(|&c| c as f64).collect();
    BarChart::new()
        .categories(cats)
        .bar_series(BarSeries::new(state.tr("stats.cigs"), vals).color(Color::from_hex("#ef6f6c")))
        .bar_radius(3.0)
        .animate(true)
        .size(340.0, 220.0)
        .class("chart")
}

/// Горизонтальное центрирование виджета фиксированной ширины внутри карточки,
/// которая растянута на всю ширину экрана.
fn centered(child: impl Widget + 'static) -> impl Widget {
    Row::new()
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(child)
}
