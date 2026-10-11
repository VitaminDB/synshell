//! Общие детали страниц: карточки, плитки, графики истории, кольца, полосы, единицы.

use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

// ─── Единицы ────────────────────────────────────────────────────────────────

fn num(v: f64) -> String {
    if v >= 100.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

pub fn bytes(b: u64) -> String {
    let b = b as f64;
    const K: f64 = 1024.0;
    if b >= K * K * K * K {
        t!("{v} ТБ", v = num(b / (K * K * K * K)))
    } else if b >= K * K * K {
        t!("{v} ГБ", v = num(b / (K * K * K)))
    } else if b >= K * K {
        t!("{v} МБ", v = num(b / (K * K)))
    } else if b >= K {
        t!("{v} КБ", v = format!("{:.0}", b / K))
    } else {
        t!("{v} Б", v = format!("{b:.0}"))
    }
}

pub fn kb(k: u64) -> String {
    bytes(k * 1024)
}

pub fn rate(bps: f64) -> String {
    if bps < 1.0 {
        return t!("0 Б/с");
    }
    t!("{v}/с", v = bytes(bps as u64))
}

pub fn mhz(m: u32) -> String {
    if m >= 1000 {
        t!("{v} ГГц", v = format!("{:.2}", m as f32 / 1000.0))
    } else {
        t!("{v} МГц", v = m)
    }
}

pub fn duration(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs / 3600 % 24, secs / 60 % 60);
    if d > 0 {
        t!("{d} д {h} ч", d = d, h = h)
    } else if h > 0 {
        t!("{h} ч {m} мин", h = h, m = m)
    } else if m > 0 {
        t!("{m} мин", m = m)
    } else {
        t!("{s} с", s = secs)
    }
}

pub fn pct(v: f32) -> String {
    format!("{v:.0}%")
}

pub fn celsius(c: f32) -> String {
    format!("{c:.0} °C")
}

/// Цвет температуры: холодно — зелёный, горячо — красный.
pub fn temp_color(c: f32) -> Color {
    Color::from_hex(if c >= 80.0 {
        "#ef5350"
    } else if c >= 65.0 {
        "#ff9f43"
    } else if c >= 45.0 {
        "#ffd54f"
    } else {
        "#5fd068"
    })
}

// ─── Карточки ───────────────────────────────────────────────────────────────

pub fn card(child: impl Widget + 'static) -> impl Widget {
    DecoratedBox::new().child(child).class("card")
}

/// Заголовок карточки: значок в цветном кружке, название, справа — подпись.
pub fn card_head(icon: &str, title: String, right: Option<String>, color: Color) -> Row {
    let mut r = Row::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .child(badge_icon(icon, color, 30.0))
        .child(Text::new(title).max_lines(1).class("card-title grow"));
    if let Some(r2) = right {
        r = r.child(Text::new(r2).max_lines(1).class("muted small"));
    }
    r
}

/// Значок в полупрозрачном кружке цвета ресурса.
pub fn badge_icon(icon: &str, color: Color, size: f32) -> impl Widget {
    DecoratedBox::new()
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Icon::new(icon).style("icon-color", color).style("icon-size", StyleValue::px(size * 0.58))),
        )
        .class("badge-icon")
        .style("background-color", color.with_alpha(0.16))
        .style("width", StyleValue::px(size))
        .style("height", StyleValue::px(size))
        .style("border-radius", StyleValue::px(size * 0.32))
}

/// Плитка: подпись, крупное значение, мелкая подпись.
pub fn tile(label: String, value: String, sub: Option<String>) -> W {
    let mut c = Column::new().gap(2.0).child(Text::new(label).max_lines(1).class("tile-label")).child(Text::new(value).max_lines(1).class("tile-value"));
    if let Some(s) = sub {
        c = c.child(Text::new(s).max_lines(1).class("tile-sub"));
    }
    boxed(DecoratedBox::new().child(c).class("tile"))
}

/// Плитки сеткой: столбцов по ширине.
pub fn tiles(items: Vec<W>, width: f32) -> W {
    let cols = ((width / 150.0) as usize).clamp(2, 6).min(items.len().max(1));
    boxed(Grid::new(cols).gap(8.0).children(items))
}

/// Строка «ключ — значение».
pub fn kv(k: String, v: String) -> W {
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(k).max_lines(1).class("kv-k"))
            .child(Text::new(v).max_lines(2).selectable(true).class("kv-v grow")),
    )
}

/// Несколько `kv` столбиком.
pub fn kv_list(rows: Vec<(String, String)>) -> W {
    let mut c = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (k, v) in rows {
        c = c.child(kv(k, v));
    }
    boxed(c)
}

pub fn empty(text: String) -> W {
    boxed(Text::new(text).max_lines(4).class("muted empty"))
}

/// Точка-цвет с подписью и значением (легенда).
pub fn legend(color: Color, label: String, value: String) -> W {
    boxed(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("dot").style("background-color", color))
            .child(Text::new(label).max_lines(1).class("muted grow"))
            .child(Text::new(value).max_lines(1).class("legend-v")),
    )
}

// ─── Графики ────────────────────────────────────────────────────────────────

/// Серия графика: значения (старые первыми), цвет, заливка.
pub struct Line {
    pub values: Vec<f32>,
    pub color: Color,
    pub fill: bool,
}

impl Line {
    pub fn new(values: Vec<f32>, color: Color) -> Self {
        Line { values, color, fill: true }
    }

    pub fn no_fill(mut self) -> Self {
        self.fill = false;
        self
    }
}

/// График истории во всю ширину: `n` точек по горизонтали (последняя у правого края),
/// шкала 0…`max`, сетка в четверти. Сетка — цветом `color` класса `chart`.
pub fn area_chart(lines: Vec<Line>, n: usize, max: f32, height: f32) -> impl Widget {
    let max = max.max(1e-6);
    Canvas::new(move |c, _| {
        let (w, h) = (c.width(), c.height());
        let grid = c.mss_color().unwrap_or(Color::from_hex("#ffffff1a"));
        c.set_color(grid.with_alpha(grid.a * 0.45));
        c.set_stroke_width(1.0);
        // Мелкий график — только линия основания.
        if h >= 60.0 {
            for i in 1..4 {
                let y = (h * i as f32 / 4.0).round() + 0.5;
                c.draw_line(0.0, y, w, y);
            }
        }
        c.draw_line(0.0, h - 0.5, w, h - 0.5);
        let n = n.max(2);
        let step = w / (n - 1) as f32;
        for l in &lines {
            if l.values.len() < 2 {
                continue;
            }
            let off = n.saturating_sub(l.values.len());
            let pts: Vec<(f32, f32)> =
                l.values.iter().enumerate().map(|(i, v)| ((off + i) as f32 * step, h - (v / max).clamp(0.0, 1.0) * (h - 3.0) - 1.5)).collect();
            if l.fill {
                c.set_color(l.color.with_alpha(0.18));
                c.fill_area_strip(&pts, h);
            }
            c.set_color(l.color);
            c.set_stroke_width(2.0);
            c.draw_polyline(&pts);
        }
    })
    .height(height)
    .class("chart")
}

/// Мини-график фиксированного размера (без сетки).
pub fn spark(values: Vec<f32>, n: usize, max: f32, color: Color, w: f32, h: f32) -> impl Widget {
    let max = max.max(1e-6);
    Canvas::new(move |c, _| {
        if values.len() < 2 {
            return;
        }
        let n = n.max(values.len()).max(2);
        let step = w / (n - 1) as f32;
        let off = n - values.len();
        let pts: Vec<(f32, f32)> = values.iter().enumerate().map(|(i, v)| ((off + i) as f32 * step, h - (v / max).clamp(0.0, 1.0) * (h - 2.0) - 1.0)).collect();
        c.set_color(color.with_alpha(0.2));
        c.fill_area_strip(&pts, h);
        c.set_color(color);
        c.set_stroke_width(1.5);
        c.draw_polyline(&pts);
    })
    .size(w, h)
}

/// Кольцо 0…100 с крупной подписью внутри.
pub fn ring(value: f32, size: f32, color: Color, big: String, sub: String) -> impl Widget {
    let v = (value / 100.0).clamp(0.0, 1.0);
    let stroke = (size / 10.0).clamp(5.0, 14.0);
    let canvas = Canvas::new(move |c, _| {
        let r = size / 2.0 - stroke / 2.0 - 1.0;
        let (cx, cy) = (size / 2.0, size / 2.0);
        c.set_stroke_width(stroke);
        c.set_color(color.with_alpha(0.16));
        c.stroke_circle(cx, cy, r);
        if v > 0.002 {
            c.set_color(color);
            let a0 = -std::f32::consts::FRAC_PI_2;
            c.draw_arc(cx, cy, r, a0, a0 + v * std::f32::consts::TAU);
        }
    })
    .size(size, size);
    let big_size = (size * 0.24).clamp(13.0, 34.0);
    Stack::new().child(canvas).child(
        Column::new()
            .gap(0.0)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(big).class("ring-value").style("font-size", StyleValue::px(big_size)))
            .child(Text::new(sub).max_lines(1).class("ring-sub"))
            .style("width", StyleValue::px(size))
            .style("height", StyleValue::px(size)),
    )
}

/// Полоса из частей (доли 0…1) на фоне дорожки: части — скруглённые отрезки с зазором.
pub fn stacked(parts: Vec<(f32, Color)>, height: f32) -> impl Widget {
    Canvas::new(move |c, _| {
        let (w, h) = (c.width(), c.height());
        let track = c.mss_color().unwrap_or(Color::from_hex("#ffffff1a"));
        let r = h / 2.0;
        c.set_color(track);
        c.fill_rounded_rect(0.0, 0.0, w, h, r);
        let total: f32 = parts.iter().map(|p| p.0.max(0.0)).sum::<f32>().max(1.0);
        let mut x = 0.0;
        for (f, col) in &parts {
            let pw = (f.max(0.0) / total * w).min(w - x);
            if pw >= 1.0 && col.a > 0.0 {
                let gap = if x > 0.0 { 2.0f32.min(pw / 2.0) } else { 0.0 };
                c.set_color(*col);
                c.fill_rounded_rect(x + gap, 0.0, pw - gap, h, r.min((pw - gap) / 2.0));
            }
            x += pw;
        }
    })
    .height(height)
    .class("track")
}

/// Полоса заполнения 0…100.
pub fn meter(value: f32, color: Color, height: f32) -> impl Widget {
    stacked(vec![((value / 100.0).clamp(0.0, 1.0), color), ((1.0 - value / 100.0).clamp(0.0, 1.0), Color::from_hex("#00000000"))], height)
}

/// Красивый потолок шкалы для скоростей: 1, 2, 5 × 10ⁿ.
pub fn nice_max(v: f32, min: f32) -> f32 {
    let v = v.max(min);
    let p = 10f32.powf(v.log10().floor());
    for k in [1.0, 2.0, 5.0, 10.0] {
        if v <= k * p {
            return k * p;
        }
    }
    10.0 * p
}
