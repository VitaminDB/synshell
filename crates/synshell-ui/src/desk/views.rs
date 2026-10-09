//! Как виджеты выглядят: для каждого типа — виды (`view`) поверх данных
//! сборщика ([`super::data`]) и графиков syngui (`LineChart`, `BarChart`,
//! `GaugeChart`, `PieChart`, `RadarChart`). Размер виджета известен заранее
//! (клетки сетки) — графики получают его явно.

use std::path::PathBuf;

use synshell_common::config::DeskWidget;
use synshell_common::ipc::WindowOp;
use synshell_common::xdg;
use syngui::containers::Keyed;
use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::widgets::charts::{AxisConfig, BarChart, BarSeries, GaugeChart, GaugeSegment, LegendPosition, LineChart, PieChart, PieLabelPosition, PieSlice, RadarChart, RadarIndicator, RadarSeries, Series};
use syngui::widgets::{PanAxis, SwipeDirection};
use syngui::GestureDetector;

use super::data::{self, AppStat, Snap};
use super::Slot;
use crate::launchers::{self, Launchable};
use crate::ui::{icon, mi, rx};
use crate::ShellCtx;

/// Внутренний отступ карточки, px.
const PAD: f32 = 12.0;
/// Высота строки заголовка карточки (с зазором), px.
const TITLE_H: f32 = 28.0;

/// Что рисовать: размер содержимого, цвет, вид.
#[derive(Clone)]
struct Cx {
    w: f32,
    h: f32,
    color: Color,
    view: String,
    history: usize,
}

fn accent(ctx: &ShellCtx) -> Color {
    Color::from_hex(&ctx.cfg().appearance.palette().accent.hex())
}

/// Цвет виджета: `color` или акцент темы.
fn color_of(ctx: &ShellCtx, w: &DeskWidget) -> Color {
    match w.str("color").filter(|c| c.starts_with('#')) {
        Some(c) => Color::from_hex(c),
        None => accent(ctx),
    }
}

/// Виджет `w` размером `iw`×`ih` (без зазоров клетки). Вне режима правки —
/// со своими касаниями: удержание или правый щелчок — меню виджета.
pub fn build(ctx: ShellCtx, slot: &Slot, w: &DeskWidget, iw: f32, ih: f32, editing: bool) -> Box<dyn Widget> {
    let view = super::view_of(w).to_string();
    let icon_like = super::is_icon(&w.kind);
    // Подложка-карточка: у метрик — по умолчанию, у часов, значков, сетки — нет.
    let card_default = !matches!(w.kind.as_str(), "clock" | "launcher" | "app" | "file");
    let card = w.bool_or("card", card_default);
    let title = card && w.bool_or("title", !matches!(view.as_str(), "text") && w.kind != "note");
    let pad = if card { PAD } else { 0.0 };
    let cw = (iw - 2.0 * pad).max(8.0);
    let chh = (ih - 2.0 * pad - if title { TITLE_H } else { 0.0 }).max(8.0);
    let cx = Cx { w: cw, h: chh, color: color_of(&ctx, w), view, history: w.int_or("history", 60).clamp(10, data::HISTORY as i64) as usize };
    let body: Box<dyn Widget> = match w.kind.as_str() {
        "clock" => clock(ctx, w, &cx),
        "cpu" | "memory" | "gpu" | "battery" => metric(ctx, w.kind.clone(), cx.clone()),
        "network" => network(ctx, cx.clone()),
        "temps" => temps(cx.clone()),
        "apps" => apps_rail(),
        "launcher" => launcher_grid(ctx, w, &cx),
        "note" => Box::new(Text::new(w.str_or("text", "Новая заметка — коснитесь в режиме правки, чтобы изменить.").to_string()).class("desk-note")),
        "app" => app_icon(ctx, slot, w, iw, ih, editing),
        "file" => file_icon(slot, w, iw, ih, editing),
        other => Box::new(Text::new(format!("Неизвестный виджет «{other}»")).class("desk-k")),
    };
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    if title {
        let (label, glyph) = super::kind_info(&w.kind).map(|k| (k.label, k.glyph)).unwrap_or(("", mi::INFO));
        col = col.child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(icon(glyph).class("desk-title-icon"))
                .child(Text::new(w.str_or("name", label).to_string()).max_lines(1).class("desk-title grow"))
                .style("height", StyleValue::px(TITLE_H)),
        );
    }
    col = col.child(body);
    let mut frame = DecoratedBox::new().child(col).style("width", StyleValue::px(iw)).style("height", StyleValue::px(ih));
    if card {
        frame = frame.style("padding", StyleValue::px(PAD)).class(format!("desk-card desk-{}", w.kind));
    } else {
        frame = frame.class(format!("desk-plain desk-{}", w.kind));
    }
    if editing || icon_like {
        return Box::new(frame);
    }
    let slot = slot.clone();
    let slot2 = slot.clone();
    let clock_tap = w.kind == "clock";
    let mut g = GestureDetector::new().on_long_press(move |_| super::open_menu(&slot)).on_secondary_click(move |_| super::open_menu(&slot2));
    if clock_tap {
        // Тап по часам — календарь и быстрые настройки времени.
        g = g.on_click(move || {
            let ctx = ShellCtx::get();
            ctx.open_popup(crate::ctx::PopupKind::Calendar, crate::commands::centered());
        });
    }
    Box::new(g.child(frame))
}

// ─── Часы ───────────────────────────────────────────────────────────────────

fn clock(ctx: ShellCtx, w: &DeskWidget, cx: &Cx) -> Box<dyn Widget> {
    let seconds = w.bool_or("seconds", false);
    let fmt = if seconds { "%H:%M:%S" } else { "%H:%M" };
    let (cw, ch, color) = (cx.w, cx.h, cx.color);
    match cx.view.as_str() {
        "analog" => Box::new(rx(move || {
            let now = ctx.now.get();
            let size = cw.min(ch);
            let (h, m, s) = {
                let t = crate::clock::format(now, "%H %M %S");
                let v: Vec<f32> = t.split(' ').filter_map(|x| x.parse().ok()).collect();
                (v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0))
            };
            let face = Canvas::new(move |c, _| {
                use std::f32::consts::{FRAC_PI_2, TAU};
                let r = size / 2.0 - 3.0;
                let (x0, y0) = (size / 2.0, size / 2.0);
                c.set_color(Color::from_hex("#00000050"));
                c.fill_circle(x0, y0, r);
                c.set_color(Color::from_hex("#ffffffc0"));
                c.set_stroke_width(2.0);
                c.stroke_circle(x0, y0, r);
                for i in 0..12 {
                    let a = i as f32 / 12.0 * TAU;
                    let (s1, s2) = (r * 0.82, r * 0.94);
                    c.draw_line(x0 + a.cos() * s1, y0 + a.sin() * s1, x0 + a.cos() * s2, y0 + a.sin() * s2);
                }
                let hand = |c: &mut syngui::core::canvas::CanvasContext, frac: f32, len: f32, width: f32| {
                    let a = frac * TAU - FRAC_PI_2;
                    c.set_stroke_width(width);
                    c.draw_line(x0, y0, x0 + a.cos() * len, y0 + a.sin() * len);
                };
                c.set_color(Color::from_hex("#ffffff"));
                hand(c, (h % 12.0 + m / 60.0) / 12.0, r * 0.5, 4.0);
                hand(c, (m + s / 60.0) / 60.0, r * 0.75, 3.0);
                if seconds {
                    c.set_color(color);
                    hand(c, s / 60.0, r * 0.85, 1.5);
                }
                c.set_color(color);
                c.fill_circle(x0, y0, 4.0);
            })
            .size(size, size);
            Box::new(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(face))
        })),
        "big" => Box::new(rx(move || {
            let now = ctx.now.get();
            let size = (ch * 0.62).clamp(18.0, 120.0);
            Box::new(
                Column::new()
                    .gap(0.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new(crate::clock::format(now, fmt)).class("desk-clock-big").style("font-size", StyleValue::px(size)))
                    .child(Text::new(crate::clock::format(now, "%A, %d %B")).class("desk-date"))
                    .style("height", StyleValue::px(ch)),
            )
        })),
        _ => Box::new(rx(move || {
            let now = ctx.now.get();
            Box::new(
                Row::new()
                    .cross_axis_alignment(CrossAxisAlignment::End)
                    .child(Text::new(crate::clock::format(now, fmt)).class("res-clock"))
                    .child(Text::new(crate::clock::format(now, "%a, %d %b")).class("res-date grow")),
            )
        })),
    }
}

// ─── Метрики ────────────────────────────────────────────────────────────────

/// Значение 0…100, крупная подпись и мелкая.
fn value_of(kind: &str, s: &Snap) -> (f32, String, String) {
    match kind {
        "cpu" => (s.cpu, format!("{:.0}%", s.cpu), s.cpu_temp.map(|t| format!("{t:.0} °C")).unwrap_or_else(|| "процессор".into())),
        "memory" => (s.mem_percent, format!("{:.0}%", s.mem_percent), synsystem::memory::human_kb(s.mem_used_kb)),
        "gpu" => match &s.gpu {
            Some((busy, mhz, _)) => (busy.unwrap_or(0.0), busy.map(|b| format!("{b:.0}%")).unwrap_or("—".into()), mhz.map(|m| format!("{m} МГц")).unwrap_or_default()),
            None => (0.0, "—".into(), "нет данных".into()),
        },
        "battery" => match &s.battery {
            Some(b) => (b.percent as f32, format!("{}%", b.percent), if b.charging { "заряжается" } else if b.full { "заряжена" } else { "батарея" }.into()),
            None => (0.0, s.adc_volt.map(|v| format!("{v:.2} В")).unwrap_or("—".into()), "нет драйвера".into()),
        },
        _ => (0.0, String::new(), String::new()),
    }
}

fn history_of<'a>(kind: &str, s: &'a Snap) -> &'a [f32] {
    match kind {
        "cpu" => &s.history.cpu,
        "memory" => &s.history.mem,
        "gpu" => &s.history.gpu,
        "battery" => &s.history.battery,
        "temps" => &s.history.temp,
        _ => &[],
    }
}

fn tail<T: Copy>(v: &[T], n: usize) -> &[T] {
    &v[v.len().saturating_sub(n)..]
}

fn metric(ctx: ShellCtx, kind: String, cx: Cx) -> Box<dyn Widget> {
    let sig = data::snap();
    Box::new(rx(move || {
        let s = sig.get();
        let (v, big, sub) = value_of(&kind, &s);
        let size = cx.w.min(cx.h);
        match cx.view.as_str() {
            "ring" => Box::new(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(ring(v, big, &sub, size, cx.color))) as Box<dyn Widget>,
            "gauge" => Box::new(gauge(v, 100.0, "%", &cx)),
            "line" => Box::new(line_view(&big, &sub, &[("", tail(history_of(&kind, &s), cx.history).to_vec(), cx.color)], Some(100.0), &cx, |v| format!("{v:.0}%"))),
            "text" => Box::new(big_text(&big, &sub, &cx)),
            "bars" if kind == "cpu" => Box::new(core_bars(&s, &cx)),
            "radar" if kind == "cpu" => Box::new(core_radar(&s, &cx)),
            "pie" if kind == "memory" => Box::new(memory_pie(&s, &cx)),
            _ => card(ctx, &kind, &s, &cx),
        }
    }))
}

/// Крупное число с подписью, по центру.
fn big_text(big: &str, sub: &str, cx: &Cx) -> impl Widget {
    let size = (cx.h * 0.5).clamp(18.0, 64.0);
    Column::new()
        .gap(2.0)
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(Text::new(big.to_string()).class("desk-big").style("font-size", StyleValue::px(size)))
        .child(Text::new(sub.to_string()).max_lines(1).class("desk-k"))
        .style("height", StyleValue::px(cx.h))
}

fn gauge(v: f32, max: f64, unit: &'static str, cx: &Cx) -> impl Widget {
    GaugeChart::new()
        .value(v as f64)
        .min(0.0)
        .max(max)
        .format(move |x| format!("{x:.0}{unit}"))
        .ticks(false)
        .labels(false)
        .needle(false)
        .show_value(true)
        .segment(GaugeSegment::new(0.0, (v as f64).clamp(0.0, max), cx.color))
        .track_width(0.18)
        .class("desk-gauge")
        .animate(false)
        .size(cx.w, cx.h)
}

/// График истории: значение сверху, линии снизу.
fn line_view(big: &str, sub: &str, series: &[(&str, Vec<f32>, Color)], max: Option<f64>, cx: &Cx, fmt: impl Fn(f64) -> String + Send + Sync + 'static) -> impl Widget {
    let head = 26.0;
    let chart_h = (cx.h - head).max(20.0);
    let n = cx.history;
    let mut chart = LineChart::new()
        .x_axis(AxisConfig::new().min(0.0).max(n as f64 - 1.0).labels(false).grid(false).axis_line(false))
        .legend(LegendPosition::None)
        .tooltip(false)
        .animate(false)
        .size(cx.w, chart_h);
    let mut y = AxisConfig::new().min(0.0).tick_count(3).format(fmt).axis_line(false);
    if let Some(m) = max {
        y = y.max(m);
    }
    if cx.h < 90.0 {
        y = y.labels(false).grid(false);
    }
    chart = chart.y_axis(y);
    for (name, values, color) in series {
        // Последняя точка — у правого края.
        let off = n.saturating_sub(values.len());
        let pts: Vec<(f64, f64)> = values.iter().enumerate().map(|(i, v)| ((off + i) as f64, *v as f64)).collect();
        chart = chart.series(Series::new(*name).data(pts).color(*color).line_width(2.0).area_fill(0.18).show_points(false));
    }
    Column::new()
        .gap(0.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::End)
                .child(Text::new(big.to_string()).class("desk-value"))
                .child(Text::new(sub.to_string()).max_lines(1).class("desk-k grow"))
                .style("height", StyleValue::px(head)),
        )
        .child(chart)
}

/// Кольцо загрузки с подписью внутри.
fn ring(value: f32, label: String, sub: &str, size: f32, color: Color) -> impl Widget {
    let v = (value / 100.0).clamp(0.0, 1.0);
    let stroke = (size / 11.0).clamp(4.0, 12.0);
    let canvas = Canvas::new(move |c, _| {
        let r = size / 2.0 - stroke / 2.0 - 2.0;
        let (cx, cy) = (size / 2.0, size / 2.0);
        c.set_stroke_width(stroke);
        c.set_color(color.with_alpha(0.18));
        c.stroke_circle(cx, cy, r);
        if v > 0.001 {
            c.set_color(color);
            let start = -std::f32::consts::FRAC_PI_2;
            c.draw_arc(cx, cy, r, start, start + v * std::f32::consts::TAU);
        }
    })
    .size(size, size);
    Stack::new().child(canvas).child(
        Column::new()
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .gap(0.0)
            .child(Text::new(label).class("res-ring-value"))
            .child(Text::new(sub.to_string()).max_lines(1).class("res-ring-unit"))
            .style("width", StyleValue::px(size))
            .style("height", StyleValue::px(size)),
    )
}

fn sparkline(values: Vec<f32>, n: usize, color: Color, w: f32, h: f32) -> impl Widget {
    Canvas::new(move |c, _| {
        if values.len() < 2 {
            return;
        }
        let n = n.max(values.len()).max(2);
        let step = w / (n - 1) as f32;
        let off = n - values.len();
        let pts: Vec<(f32, f32)> = values.iter().enumerate().map(|(i, v)| ((off + i) as f32 * step, h - (v / 100.0).clamp(0.0, 1.0) * (h - 2.0) - 1.0)).collect();
        c.set_color(color.with_alpha(0.16));
        c.fill_area_strip(&pts, h);
        c.set_color(color);
        c.set_stroke_width(2.0);
        c.draw_polyline(&pts);
    })
    .size(w, h)
}

/// Ядра столбиками: загрузка — высота, класс ядра — цвет, внизу частота.
fn cores(list: &[(f32, Option<u32>, usize)], h: f32, max: usize) -> impl Widget {
    let h = h.max(16.0);
    let mut row = Row::new().gap(4.0).main_axis_alignment(MainAxisAlignment::SpaceBetween).cross_axis_alignment(CrossAxisAlignment::End);
    // Много ядер (24 на десктопе) — только самые загруженные.
    let mut items: Vec<(usize, &(f32, Option<u32>, usize))> = list.iter().enumerate().collect();
    if items.len() > max {
        items.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0));
        items.truncate(max);
        items.sort_by_key(|x| x.0);
    }
    for (_, (usage, mhz, tier)) in items {
        let fill = (usage / 100.0).clamp(0.0, 1.0) * h;
        let bar = Stack::new()
            .child(DecoratedBox::new().class("res-core-bg").style("height", StyleValue::px(h)))
            .child(
                Column::new()
                    .main_axis_alignment(MainAxisAlignment::End)
                    .child(DecoratedBox::new().class(format!("res-core-fill res-tier-{}", tier.min(&3))).style("height", StyleValue::px(fill.max(2.0))))
                    .style("height", StyleValue::px(h)),
            );
        let label = mhz.map(|m| if m >= 1000 { format!("{:.1}", m as f32 / 1000.0) } else { format!("{:.2}", m as f32 / 1000.0) }).unwrap_or_default();
        row = row.child(Column::new().gap(3.0).cross_axis_alignment(CrossAxisAlignment::Center).child(bar).child(Text::new(label).class("res-core-mhz")));
    }
    row
}

fn core_bars(s: &Snap, cx: &Cx) -> impl Widget {
    let mut list: Vec<(usize, f32)> = s.cores.iter().map(|c| c.0).enumerate().collect();
    let max = ((cx.w / 22.0) as usize).clamp(2, 32);
    if list.len() > max {
        list.sort_by(|a, b| b.1.total_cmp(&a.1));
        list.truncate(max);
        list.sort_by_key(|x| x.0);
    }
    BarChart::new()
        .categories(list.iter().map(|(i, _)| i.to_string()).collect())
        .bar_series(BarSeries::new("Загрузка", list.iter().map(|x| x.1 as f64).collect()).color(cx.color))
        .y_axis(AxisConfig::new().min(0.0).max(100.0).tick_count(3).labels(cx.h >= 90.0).axis_line(false))
        .x_axis(AxisConfig::new().labels(cx.w / list.len().max(1) as f32 >= 14.0).grid(false))
        .legend(LegendPosition::None)
        .tooltip(false)
        .animate(false)
        .bar_radius(3.0)
        .size(cx.w, cx.h)
}

fn core_radar(s: &Snap, cx: &Cx) -> impl Widget {
    let n = s.cores.len().clamp(3, 24);
    let vals: Vec<f64> = (0..n).map(|i| s.cores.get(i).map(|c| c.0 as f64).unwrap_or(0.0)).collect();
    RadarChart::new()
        .indicators((0..n).map(|i| RadarIndicator::new(i.to_string(), 100.0)).collect())
        .radar_series(RadarSeries::new("Ядра", vals).color(cx.color).area_opacity(0.3).show_points(false))
        .grid_levels(3)
        .legend(LegendPosition::None)
        .tooltip(false)
        .animate(false)
        .size(cx.w, cx.h)
}

fn memory_pie(s: &Snap, cx: &Cx) -> impl Widget {
    let used = s.mem_used_kb as f64;
    let free = s.mem_total_kb.saturating_sub(s.mem_used_kb) as f64;
    let head = 22.0;
    Column::new()
        .gap(0.0)
        .child(
            Text::new(format!("{} из {}", synsystem::memory::human_kb(s.mem_used_kb), synsystem::memory::human_kb(s.mem_total_kb)))
                .max_lines(1)
                .class("desk-k")
                .style("height", StyleValue::px(head)),
        )
        .child(
            PieChart::new()
                .slice(PieSlice::new("Занято", used).color(cx.color))
                .slice(PieSlice::new("Свободно", free).color(cx.color.with_alpha(0.22)))
                .donut(0.6)
                .label_position(PieLabelPosition::None)
                .show_percentage(false)
                .legend(LegendPosition::None)
                .tooltip(false)
                .animate(false)
                .size(cx.w, (cx.h - head).max(20.0)),
        )
}

fn kv(k: &str, v: String) -> impl Widget {
    Row::new().child(Text::new(k.to_string()).class("res-k grow")).child(Text::new(v).class("res-v"))
}

/// Подробная карточка: как на прежнем экране ресурсов.
fn card(ctx: ShellCtx, kind: &str, s: &Snap, cx: &Cx) -> Box<dyn Widget> {
    let _ = ctx;
    let color = cx.color;
    match kind {
        "cpu" => {
            let temp = s.cpu_temp.map(|t| format!("{t:.0} °C")).unwrap_or_default();
            let max_mhz = s.cores.iter().filter_map(|c| c.1).max();
            let ring_size = (cx.h * 0.5).clamp(56.0, 110.0).min(cx.w * 0.4);
            let top = Row::new()
                .gap(14.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(ring(s.cpu, format!("{:.0}%", s.cpu), "процессор", ring_size, color))
                .child(
                    Column::new()
                        .gap(6.0)
                        .child(kv("Ядер", s.cores.len().to_string()))
                        .child(kv("Частота", max_mhz.map(|m| format!("до {m} МГц")).unwrap_or_default()))
                        .child(kv("Температура", temp))
                        .child(sparkline(tail(&s.history.cpu, cx.history).to_vec(), cx.history, color, (cx.w - ring_size - 14.0).max(40.0), 30.0))
                        .class("grow"),
                );
            let bars_h = cx.h - ring_size - 12.0 - 18.0;
            let mut col = Column::new().gap(12.0).child(top);
            if bars_h >= 20.0 {
                col = col.child(cores(&s.cores, bars_h.min(80.0), ((cx.w / 20.0) as usize).clamp(4, 16)));
            }
            Box::new(col)
        }
        "memory" | "gpu" => {
            let (v, big, sub) = value_of(kind, s);
            let size = (cx.h - 18.0).min(cx.w).max(40.0);
            let foot = if kind == "memory" {
                format!("из {}", synsystem::memory::human_kb(s.mem_total_kb))
            } else {
                s.gpu.as_ref().map(|g| format!("{}{}", g.2.clone().unwrap_or_default(), s.gpu_temp.map(|t| format!(" · {t:.0} °C")).unwrap_or_default())).unwrap_or_default()
            };
            Box::new(
                Column::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(ring(v, big, &sub, size, color))
                    .child(Text::new(foot).max_lines(1).class("res-ring-sub")),
            )
        }
        "battery" => power_body(s),
        _ => Box::new(DecoratedBox::new()),
    }
}

fn power_body(s: &Snap) -> Box<dyn Widget> {
    match (&s.battery, &s.battery_info) {
        (Some(b), info) => {
            let state = if b.charging { "Заряжается" } else if b.full { "Заряжена" } else { "Разряжается" };
            let mut col = Column::new()
                .gap(6.0)
                .child(Row::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(format!("{}%", b.percent)).class("res-big")).child(Text::new(state).class("res-v grow")))
                .child(crate::ui::meter(b.percent));
            if let Some(m) = b.minutes {
                col = col.child(kv(if b.charging { "До полного" } else { "Осталось" }, format!("{}:{:02}", m / 60, m % 60)));
            }
            if let Some(w) = b.power_w.filter(|w| *w > 0.05) {
                col = col.child(kv("Мощность", format!("{w:.2} Вт")));
            }
            if let Some(i) = info {
                if let Some(v) = i.voltage_v {
                    col = col.child(kv("Напряжение", format!("{v:.2} В")));
                }
                if let Some(t) = i.temp_c {
                    col = col.child(kv("Температура", format!("{t:.1} °C")));
                }
            }
            Box::new(col)
        }
        (None, _) => match s.adc_volt {
            // Драйвера батареи нет (телефон без ADSP) — напряжение с АЦП.
            Some(v) => Box::new(Column::new().gap(4.0).child(Text::new(format!("{v:.2} В")).class("res-big")).child(Text::new("напряжение аккумулятора").class("res-k"))),
            None => Box::new(Text::new("Аккумулятора нет").class("res-v")),
        },
    }
}

// ─── Сеть ───────────────────────────────────────────────────────────────────

fn human_rate(b: f64) -> String {
    if b >= 1e6 {
        format!("{:.1} МБ/с", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} КБ/с", b / 1e3)
    } else {
        format!("{b:.0} Б/с")
    }
}

/// Строка подключения: значок или полоски слева, имя и подробности.
fn net_line(lead: Box<dyn Widget>, title: String, sub: String) -> impl Widget {
    let mut col = Column::new().gap(0.0).child(Text::new(title).max_lines(1).class("res-v"));
    if !sub.is_empty() {
        col = col.child(Text::new(sub).max_lines(1).class("res-k"));
    }
    Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().child(lead).class("res-net-lead")).child(col.class("grow"))
}

fn network(ctx: ShellCtx, cx: Cx) -> Box<dyn Widget> {
    let sig = data::snap();
    Box::new(rx(move || {
        let s = sig.get();
        let rx_c = cx.color;
        let tx_c = Color::from_hex("#ffb74d");
        match cx.view.as_str() {
            "line" => {
                let to_k = |v: &[f64]| tail(v, cx.history).iter().map(|b| (*b / 1024.0) as f32).collect::<Vec<f32>>();
                Box::new(line_view(
                    &format!("↓ {}", human_rate(s.net.0)),
                    &format!("↑ {}", human_rate(s.net.1)),
                    &[("Приём", to_k(&s.history.rx), rx_c), ("Передача", to_k(&s.history.tx), tx_c)],
                    None,
                    &cx,
                    |v| if v >= 1024.0 { format!("{:.0}M", v / 1024.0) } else { format!("{v:.0}K") },
                )) as Box<dyn Widget>
            }
            "bars" => {
                let n = ((cx.w / 10.0) as usize).clamp(6, 40).min(cx.history);
                let rx_v: Vec<f64> = tail(&s.history.rx, n).iter().map(|b| b / 1024.0).collect();
                let tx_v: Vec<f64> = tail(&s.history.tx, n).iter().map(|b| b / 1024.0).collect();
                Box::new(
                    BarChart::new()
                        .categories((0..rx_v.len()).map(|i| i.to_string()).collect())
                        .bar_series(BarSeries::new("Приём", rx_v).color(rx_c))
                        .bar_series(BarSeries::new("Передача", tx_v).color(tx_c))
                        .x_axis(AxisConfig::new().labels(false).grid(false))
                        .y_axis(AxisConfig::new().min(0.0).tick_count(3).labels(cx.h >= 90.0).axis_line(false).format(|v| format!("{v:.0}K")))
                        .legend(LegendPosition::None)
                        .tooltip(false)
                        .animate(false)
                        .size(cx.w, cx.h),
                )
            }
            "text" => Box::new(
                Column::new()
                    .gap(4.0)
                    .main_axis_alignment(MainAxisAlignment::Center)
                    .child(Text::new(format!("↓ {}", human_rate(s.net.0))).class("desk-value"))
                    .child(Text::new(format!("↑ {}", human_rate(s.net.1))).class("desk-value"))
                    .style("height", StyleValue::px(cx.h)),
            ),
            _ => Box::new(net_card(&ctx.network.get(), ctx.modem.get().as_ref(), s.net)),
        }
    }))
}

/// Карточка «Сеть»: Wi-Fi, мобильная связь (если есть модем) и скорость.
fn net_card(n: &synsystem::network::Network, modem: Option<&synmodem::api::Status>, rate: (f64, f64)) -> impl Widget {
    let (glyph, title, sub) = match n.kind.as_str() {
        "wifi" if n.online => (mi::WIFI, n.connection.clone(), n.signal.map(|v| format!("Wi-Fi · {v} %")).unwrap_or_else(|| "Wi-Fi".into())),
        "ethernet" if n.online => (mi::ETHERNET, n.connection.clone(), "Проводная сеть".into()),
        _ => (mi::WIFI_OFF, "Нет Wi-Fi".into(), String::new()),
    };
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(net_line(Box::new(icon(glyph).class("res-net-icon")), title, sub));
    if let Some(m) = modem.filter(|m| m.present) {
        let (title, sub) = crate::modem::summary(m);
        let bars = if m.radio { m.bars } else { None };
        col = col.child(net_line(Box::new(crate::modem::signal_bars(bars)), title, sub));
    }
    col.child(Row::new().gap(10.0).child(Text::new(format!("↓ {}", human_rate(rate.0))).class("res-k grow")).child(Text::new(format!("↑ {}", human_rate(rate.1))).class("res-k")))
}

// ─── Температуры ────────────────────────────────────────────────────────────

fn temps(cx: Cx) -> Box<dyn Widget> {
    let sig = data::snap();
    Box::new(rx(move || {
        let s = sig.get();
        let hottest = s.temps.first().map(|t| t.1).unwrap_or(0.0);
        match cx.view.as_str() {
            "gauge" => Box::new(gauge(hottest, 110.0, "°", &cx)) as Box<dyn Widget>,
            "line" => Box::new(line_view(
                &s.cpu_temp.map(|t| format!("{t:.0} °C")).unwrap_or("—".into()),
                "процессор",
                &[("", tail(&s.history.temp, cx.history).to_vec(), cx.color)],
                None,
                &cx,
                |v| format!("{v:.0}°"),
            )),
            "text" => {
                let rows = ((cx.h / 18.0) as usize).max(1);
                let mut col = Column::new().gap(2.0);
                for (name, t) in s.temps.iter().take(rows) {
                    col = col.child(kv(name, format!("{t:.0} °C")));
                }
                Box::new(col)
            }
            _ => {
                let n = ((cx.w / 34.0) as usize).clamp(2, 16);
                let list: Vec<&(String, f32)> = s.temps.iter().take(n).collect();
                Box::new(
                    BarChart::new()
                        .categories(list.iter().map(|t| t.0.chars().take(8).collect()).collect())
                        .x_axis(AxisConfig::new().labels(cx.w / list.len().max(1) as f32 >= 52.0).grid(false))
                        .bar_series(BarSeries::new("°C", list.iter().map(|t| t.1 as f64).collect()).color(cx.color))
                        .y_axis(AxisConfig::new().min(0.0).tick_count(3).labels(cx.h >= 90.0).axis_line(false))
                        .legend(LegendPosition::None)
                        .tooltip(false)
                        .animate(false)
                        .value_labels(cx.h >= 70.0)
                        .bar_radius(3.0)
                        .size(cx.w, cx.h),
                )
            }
        }
    }))
}

// ─── Запущенные приложения ──────────────────────────────────────────────────

fn apps_rail() -> Box<dyn Widget> {
    let sig = data::snap();
    Box::new(rx(move || {
        let s = sig.get();
        if s.apps.is_empty() {
            return Box::new(Text::new("Нет запущенных приложений").class("res-empty")) as Box<dyn Widget>;
        }
        let mut row = Row::new().gap(12.0);
        for a in &s.apps {
            row = row.child(Keyed::new(a.window, 0, {
                let a = a.clone();
                move || Box::new(app_card(&a))
            }));
        }
        Box::new(ScrollView::new().horizontal().child(row.class("res-rail-row")).class("res-rail"))
    }))
}

/// Карточка приложения в ленте: значок, имя и подпись (с переносом), внизу — бейджи памяти и
/// процессора; тап — к окну, «×» в углу или свайп вверх — закрыть.
fn app_card(a: &AppStat) -> impl Widget {
    let entry = xdg::app_for_window(&a.app_id);
    let icon_path = entry.as_ref().and_then(|e| xdg::lookup_icon(&e.icon)).or_else(|| xdg::window_icon(&a.app_id));
    let name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| a.app_id.clone());
    // У Android «Android» и экземпляр — разными строками: имя образа не рвётся посередине
    let mut subtitle = xdg::window_subtitle(entry.as_ref(), &a.title);
    if entry.as_ref().is_some_and(|e| e.android.is_some()) {
        subtitle = subtitle.replacen(" · ", "\n", 1);
    }
    let mem = if a.mem_kb >= 1024 * 1024 { format!("{:.1}G", a.mem_kb as f64 / 1048576.0) } else { format!("{}M", a.mem_kb / 1024) };
    let cpu_class = if a.cpu >= 25.0 { "res-badge res-badge-cpu res-badge-hot" } else { "res-badge res-badge-cpu" };
    let id = a.window;
    let close = GestureDetector::new()
        .on_click(move || crate::actions::window_op(id, WindowOp::Close))
        .child(DecoratedBox::new().child(icon(mi::CLOSE).class("res-app-close-icon")).class("res-app-close"));
    let card = GestureDetector::new()
        .pan_axis(PanAxis::Vertical)
        .on_click(move || crate::actions::window_op(id, WindowOp::Activate))
        .on_swipe(move |d, _| {
            if d == SwipeDirection::Up {
                crate::actions::window_op(id, WindowOp::Close);
            }
        })
        .child(
            DecoratedBox::new()
                .child(
                    Column::new()
                        .gap(4.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(launchers::icon_widget(&icon_path, &None, "res-app-icon", 44.0))
                        .child(Text::new(name).max_lines(2).class("res-app-name"))
                        .child(Text::new(subtitle).max_lines(3).class("res-app-title"))
                        .child(
                            Row::new()
                                .gap(4.0)
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .child(DecoratedBox::new().child(Text::new(mem).class("res-badge-text")).class("res-badge res-badge-mem"))
                                .child(DecoratedBox::new().child(Text::new(format!("{:.0}%", a.cpu)).class("res-badge-text")).class(cpu_class))
                                .class("res-app-badges"),
                        ),
                )
                .class("res-app"),
        );
    Stack::new().child(card).child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(close).class("res-app-close-row"))
}

// ─── Значки приложений ──────────────────────────────────────────────────────

/// Сетка значков: свои (`apps`), закреплённые на домашнем экране (`[mobile]
/// home_apps`) или все программы по алфавиту.
fn launcher_grid(ctx: ShellCtx, w: &DeskWidget, cx: &Cx) -> Box<dyn Widget> {
    let own = w.strings("apps");
    let cols_opt = w.int_or("columns", 0);
    let (cw, ch) = (cx.w, cx.h);
    Box::new(rx(move || {
        let _ = ctx.apps_rev.get();
        let cfg = ctx.cfg();
        let ids = if own.is_empty() { cfg.mobile.home_apps.clone() } else { own.clone() };
        let entries: Vec<xdg::DesktopEntry> = if ids.is_empty() {
            // Программы Linux; приложения Android — в меню, своим разделом.
            let mut v: Vec<_> = xdg::apps().iter().filter(|a| !a.no_display && a.android.is_none()).cloned().collect();
            v.sort_by_key(|a| a.name.to_lowercase());
            v
        } else {
            ids.iter().filter_map(|id| xdg::app_by_id(id)).collect()
        };
        let cols = if cols_opt > 0 { cols_opt as usize } else if ctx.is_phone() { cfg.mobile.home_columns.clamp(2, 8) as usize } else { ((cw / 96.0) as usize).max(1) };
        let mut grid = Grid::new(cols.clamp(1, 12)).gap(4.0);
        for e in &entries {
            grid = grid.child(app_tile(ctx, e));
        }
        Box::new(DecoratedBox::new().child(ScrollView::new().vertical().child(grid)).style("height", StyleValue::px(ch)))
    }))
}

fn app_tile(ctx: ShellCtx, e: &xdg::DesktopEntry) -> impl Widget {
    let l = Launchable::from_entry(e);
    let key = l.key();
    let launch = l.clone();
    let id = e.id.clone();
    let icon = launchers::icon_widget(&l.icon, &None, "home-app-icon", 56.0);
    let tile = rx(move || {
        let launching = launchers::is_launching(&ctx, &key);
        let class = if launching { "home-app home-app-launching" } else { "home-app" };
        Box::new(DecoratedBox::new().class(class))
    });
    GestureDetector::new()
        .on_click(move || launchers::launch(ShellCtx::get(), &launch))
        .on_long_press(move |_| {
            let ctx = ShellCtx::get();
            crate::edit::open_at_press(&ctx, crate::ctx::PopupKind::HomeAppMenu(id.clone()));
        })
        .child(
            Stack::new().child(tile).child(
                Column::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(icon)
                    .child(Text::new(e.name.clone()).max_lines(2).class("home-app-name"))
                    .class("home-app-body"),
            ),
        )
}

/// Значок с подписью по размеру клетки.
fn icon_cell(icon: Box<dyn Widget>, label: String, ih: f32) -> impl Widget {
    let _ = ih;
    Column::new()
        .gap(4.0)
        .main_axis_alignment(MainAxisAlignment::Center)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(icon)
        .child(Text::new(label).max_lines(2).class("desk-label"))
        .class("desk-item")
}

fn icon_size(iw: f32, ih: f32) -> f32 {
    (iw.min(ih) * 0.5).clamp(24.0, 96.0)
}

fn app_icon(ctx: ShellCtx, slot: &Slot, w: &DeskWidget, iw: f32, ih: f32, editing: bool) -> Box<dyn Widget> {
    let id = w.str("app").unwrap_or_default().to_string();
    let Some(e) = xdg::app_by_id(&id) else {
        return Box::new(icon_cell(Box::new(icon(mi::APPS).class("desk-missing")), format!("{id}?"), ih));
    };
    let l = Launchable::from_entry(&e);
    let size = icon_size(iw, ih);
    let cell = icon_cell(launchers::icon_widget(&l.icon, &None, "desk-icon", size), e.name.clone(), ih);
    if editing {
        return Box::new(cell);
    }
    let key = l.key();
    let tile = rx(move || {
        let class = if launchers::is_launching(&ctx, &key) { "desk-item-bg desk-item-launching" } else { "desk-item-bg" };
        Box::new(DecoratedBox::new().class(class))
    });
    Box::new(icon_gestures(Stack::new().fit(StackFit::Expand).child(tile).child(cell), slot.clone(), move || launchers::launch(ShellCtx::get(), &l)))
}

/// Значок файла или папки (`.desktop` — как значок запуска).
fn file_icon(slot: &Slot, w: &DeskWidget, iw: f32, ih: f32, editing: bool) -> Box<dyn Widget> {
    let path: PathBuf = synshell_common::paths::expand_tilde(w.str("path").unwrap_or_default());
    let size = icon_size(iw, ih);
    let (label, icon_path, cmd) = if path.extension().is_some_and(|e| e == "desktop") {
        match crate::xdg::parse_desktop_file(&path, String::new(), &[]) {
            Some(e) => (e.name.clone(), crate::xdg::lookup_icon(&e.icon), e.command()),
            None => (super::file_name(&path), None, String::new()),
        }
    } else {
        let name = if path.is_dir() { "folder" } else if synshell_common::wallpaper::is_image(&path) { "image-x-generic" } else { "text-x-generic" };
        (super::file_name(&path), crate::xdg::lookup_icon(name), crate::manager::open_command(&path))
    };
    let cell = icon_cell(launchers::icon_widget(&icon_path, &None, "desk-icon", size), label, ih);
    if editing {
        return Box::new(cell);
    }
    Box::new(icon_gestures(Stack::new().fit(StackFit::Expand).child(DecoratedBox::new().class("desk-item-bg")).child(cell), slot.clone(), move || {
        if !cmd.is_empty() {
            crate::actions::spawn(&cmd);
        }
    }))
}

/// Касания значка: тап — открыть, удержание и правый щелчок — меню.
fn icon_gestures(child: impl Widget + 'static, slot: Slot, open: impl Fn() + Send + Sync + 'static) -> impl Widget {
    let slot2 = slot.clone();
    GestureDetector::new()
        .on_click(open)
        .on_long_press(move |_| super::open_menu(&slot))
        .on_secondary_click(move |_| super::open_menu(&slot2))
        .child(child)
}
