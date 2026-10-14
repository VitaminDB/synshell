//! Общие детали: кнопки, карточки, строки настроек, графики, единицы.

use syngui::core::Color;
use syngui::mss::StyleValue;
use syngui::prelude::*;
use syngui::GestureDetector;
use syngui::CursorIcon;

use crate::model::OsKind;
use crate::state::icons;

pub type W = Box<dyn Widget>;

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

pub fn spacer() -> W {
    boxed(DecoratedBox::new().class("grow"))
}

// ─── Кнопки ─────────────────────────────────────────────────────────────

/// Кнопка-«пилюля»: значок и подпись. Вид — `kind`: `primary`, `danger`, `soft`, `ghost`.
pub fn btn(icon: &str, label: String, kind: &str, f: impl Fn() + Send + Sync + 'static) -> W {
    let cls = if label.is_empty() { format!("btn round {kind}") } else { format!("btn {kind}") };
    let mut row = Row::new().gap(7.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !icon.is_empty() {
        row = row.child(Icon::new(icon).class("btn-icon"));
    }
    if !label.is_empty() {
        row = row.child(Text::new(label).max_lines(1).class("btn-label"));
    }
    boxed(GestureDetector::new().cursor(CursorIcon::Pointer).on_click(f).child(DecoratedBox::new().child(row).class(cls)))
}

/// Неактивная кнопка того же вида.
pub fn btn_disabled(icon: &str, label: String, kind: &str) -> W {
    let mut row = Row::new().gap(7.0).cross_axis_alignment(CrossAxisAlignment::Center);
    if !icon.is_empty() {
        row = row.child(Icon::new(icon).class("btn-icon"));
    }
    if !label.is_empty() {
        row = row.child(Text::new(label).max_lines(1).class("btn-label"));
    }
    boxed(DecoratedBox::new().child(row).class(format!("btn {kind} disabled")))
}

/// Круглая кнопка-значок с подсказкой.
pub fn icon_btn(icon: &str, tip: String, f: impl Fn() + Send + Sync + 'static) -> W {
    let b = GestureDetector::new()
        .cursor(CursorIcon::Pointer)
        .on_click(f)
        .child(DecoratedBox::new().child(Icon::new(icon).class("ibtn-icon")).class("ibtn"));
    if tip.is_empty() {
        boxed(b)
    } else {
        boxed(Tooltip::new(b, tip).delay_ms(500))
    }
}

/// Сегменты выбора: подписи, текущий индекс.
pub fn segments(items: Vec<(String, &'static str)>, cur: usize, f: impl Fn(usize) + Send + Sync + Clone + 'static) -> W {
    let mut row = Row::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Center);
    for (i, (label, icon)) in items.into_iter().enumerate() {
        let f = f.clone();
        let mut inner = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        if !icon.is_empty() {
            inner = inner.child(Icon::new(icon).class("seg-icon"));
        }
        if !label.is_empty() {
            inner = inner.child(Text::new(label).max_lines(1).class("seg-label"));
        }
        row = row.child(
            GestureDetector::new()
                .cursor(CursorIcon::Pointer)
                .on_click(move || f(i))
                .child(DecoratedBox::new().child(inner).class(if i == cur { "seg-item active" } else { "seg-item" })),
        );
    }
    boxed(DecoratedBox::new().child(row).class("seg"))
}

// ─── Карточки ───────────────────────────────────────────────────────────

pub fn card(child: impl Widget + 'static) -> W {
    boxed(DecoratedBox::new().child(child).class("card"))
}

/// Карточка с заголовком (значок, название, подпись справа) и строками.
pub fn section(icon: &str, title: String, rows: Vec<W>) -> W {
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(badge(icon, None, 30.0))
            .child(Text::new(title).max_lines(1).class("card-title grow"))
            .class("section-head"),
    );
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            col = col.child(DecoratedBox::new().class("divider"));
        }
        col = col.child(r);
    }
    card(col)
}

/// Строка настройки: название, пояснение, справа — элемент управления.
pub fn row(title: String, hint: String, control: impl Widget + 'static) -> W {
    if viewport_size().get_untracked().width < 620.0 {
        return row_wide(title, hint, Row::new().child(control));
    }
    let mut text = Column::new().gap(2.0).child(Text::new(title).max_lines(2).class("row-title"));
    if !hint.is_empty() {
        text = text.child(Text::new(hint).max_lines(3).class("row-hint"));
    }
    boxed(Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center).child(text.class("grow")).child(control).class("row"))
}

/// Строка во всю ширину (элемент под подписью) — для узких окон и длинных полей.
pub fn row_wide(title: String, hint: String, control: impl Widget + 'static) -> W {
    let mut col = Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).max_lines(2).class("row-title"));
    if !hint.is_empty() {
        col = col.child(Text::new(hint).max_lines(3).class("row-hint"));
    }
    boxed(col.child(control).class("row"))
}

/// Значок в скруглённом квадрате; цвет — свой или акцент.
pub fn badge(icon: &str, color: Option<Color>, size: f32) -> W {
    let mut b = DecoratedBox::new()
        .child(
            Column::new()
                .main_axis_alignment(MainAxisAlignment::Center)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(match color {
                    Some(c) => Icon::new(icon).style("icon-color", c).style("icon-size", StyleValue::px(size * 0.56)),
                    None => Icon::new(icon).class("badge-icon").style("icon-size", StyleValue::px(size * 0.56)),
                }),
        )
        .class("badge")
        .style("width", StyleValue::px(size))
        .style("height", StyleValue::px(size))
        .style("border-radius", StyleValue::px(size * 0.32));
    if let Some(c) = color {
        b = b.style("background-color", c.with_alpha(0.16));
    }
    boxed(b)
}

/// Значок системы.
pub fn os_icon(os: OsKind) -> &'static str {
    match os {
        OsKind::Linux => icons::TERMINAL,
        OsKind::Windows11 | OsKind::Windows10 => icons::WINDOW,
        OsKind::Bsd => icons::DNS,
        OsKind::Android => icons::ANDROID,
        OsKind::Other => icons::COMPUTER,
    }
}

/// Точка-индикатор состояния и подпись.
pub fn status_chip(power: crate::state::Power, label: Option<String>) -> W {
    let text = label.unwrap_or_else(|| syngui::i18n::t(power.title()));
    boxed(
        DecoratedBox::new()
            .child(
                Row::new()
                    .gap(6.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(DecoratedBox::new().class(format!("dot {}", power.class())))
                    .child(Text::new(text).max_lines(1).class("chip-label")),
            )
            .class(format!("status {}", power.class())),
    )
}

pub fn kv(k: String, v: String) -> W {
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .child(DecoratedBox::new().child(Text::new(k).max_lines(1).class("kv-k")).class("kv-key"))
            .child(Text::new(v).max_lines(3).selectable(true).class("kv-v grow")),
    )
}

pub fn empty(icon: &str, title: String, hint: String) -> W {
    boxed(
        Column::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().child(Icon::new(icon).class("empty-icon")).class("empty-badge"))
            .child(Text::new(title).max_lines(2).class("empty-title"))
            .child(Text::new(hint).max_lines(4).class("empty-hint"))
            .class("empty"),
    )
}

// ─── Графики ────────────────────────────────────────────────────────────

/// История 0…`max` во всю ширину: `n` точек, последняя у правого края.
pub fn area_chart(values: Vec<f32>, n: usize, max: f32, color: Color, height: f32) -> impl Widget {
    let max = max.max(1e-6);
    Canvas::new(move |c, _| {
        let (w, h) = (c.width(), c.height());
        let grid = c.mss_color().unwrap_or(Color::from_hex("#ffffff1a"));
        c.set_color(grid.with_alpha(grid.a * 0.5));
        c.set_stroke_width(1.0);
        if h >= 60.0 {
            for i in 1..4 {
                let y = (h * i as f32 / 4.0).round() + 0.5;
                c.draw_line(0.0, y, w, y);
            }
        }
        c.draw_line(0.0, h - 0.5, w, h - 0.5);
        if values.len() < 2 {
            return;
        }
        let n = n.max(2);
        let step = w / (n - 1) as f32;
        let off = n.saturating_sub(values.len());
        let pts: Vec<(f32, f32)> = values.iter().enumerate().map(|(i, v)| ((off + i) as f32 * step, h - (v / max).clamp(0.0, 1.0) * (h - 3.0) - 1.5)).collect();
        c.set_color(color.with_alpha(0.18));
        c.fill_area_strip(&pts, h);
        c.set_color(color);
        c.set_stroke_width(2.0);
        c.draw_polyline(&pts);
    })
    .height(height)
    .class("chart")
}

/// Полоса заполнения 0…1.
pub fn meter(frac: f32, color: Color, height: f32) -> impl Widget {
    let frac = frac.clamp(0.0, 1.0);
    Canvas::new(move |c, _| {
        let (w, h) = (c.width(), c.height());
        let track = c.mss_color().unwrap_or(Color::from_hex("#ffffff1a"));
        c.set_color(track);
        c.fill_rounded_rect(0.0, 0.0, w, h, h / 2.0);
        if frac > 0.0 {
            c.set_color(color);
            c.fill_rounded_rect(0.0, 0.0, (w * frac).max(h), h, h / 2.0);
        }
    })
    .height(height)
    .class("track")
}

// ─── Единицы ────────────────────────────────────────────────────────────

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

/// Русское склонение по числу: 1 ядро, 2 ядра, 5 ядер.
fn plural(n: u64) -> usize {
    let (m10, m100) = (n % 10, n % 100);
    if m10 == 1 && m100 != 11 {
        0
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        1
    } else {
        2
    }
}

pub fn cores(n: impl Into<u64>) -> String {
    let n = n.into();
    match plural(n) {
        0 => t!("{n} ядро", n = n),
        1 => t!("{n} ядра", n = n),
        _ => t!("{n} ядер", n = n),
    }
}

pub fn disks(n: usize) -> String {
    match plural(n as u64) {
        0 => t!("{n} диск", n = n),
        1 => t!("{n} диска", n = n),
        _ => t!("{n} дисков", n = n),
    }
}

pub fn mem_mb(mb: u32) -> String {
    if mb >= 1024 && mb % 1024 == 0 {
        t!("{v} ГБ", v = mb / 1024)
    } else if mb >= 1024 {
        t!("{v} ГБ", v = format!("{:.1}", mb as f32 / 1024.0))
    } else {
        t!("{v} МБ", v = mb)
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

/// «сегодня», «вчера», «5 дней назад».
pub fn ago(t: u64) -> String {
    if t == 0 {
        return t!("не запускалась");
    }
    let d = crate::model::now().saturating_sub(t);
    if d < 60 {
        t!("только что")
    } else if d < 3600 {
        t!("{n} мин назад", n = d / 60)
    } else if d < 86400 {
        t!("{n} ч назад", n = d / 3600)
    } else if d < 2 * 86400 {
        t!("вчера")
    } else {
        t!("{n} дн. назад", n = d / 86400)
    }
}

pub fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// Окно выбора файла Проводника (`synfiles --choose`): путь или `None` при отмене.
/// Вызывать из фонового потока — ждёт, пока пользователь выберет.
pub fn choose_file(title: &str, filter_name: &str, patterns: &[&str], directory: bool) -> Option<String> {
    let filters: Vec<serde_json::Value> = if patterns.is_empty() {
        Vec::new()
    } else {
        vec![serde_json::json!({ "name": filter_name, "patterns": patterns.iter().map(|p| serde_json::json!([0, p])).collect::<Vec<_>>() })]
    };
    let folder = synshell_common::paths::user_dir_or_default("DOWNLOAD");
    let req = serde_json::json!({
        "mode": "open",
        "title": title,
        "directory": directory,
        "filters": filters,
        "current_folder": folder.to_string_lossy(),
    });
    let exe = synshell_common::paths::self_exe("synfiles").with_file_name("synfiles");
    let exe = if exe.exists() { exe } else { std::path::PathBuf::from("synfiles") };
    let out = std::process::Command::new(exe).arg("--choose").arg(req.to_string()).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = line.lines().rev().find_map(|l| serde_json::from_str(l).ok())?;
    if v.get("response")?.as_u64()? != 0 {
        return None;
    }
    let uri = v.get("uris")?.as_array()?.first()?.as_str()?.to_string();
    Some(uri_to_path(&uri))
}

fn uri_to_path(uri: &str) -> String {
    let Some(p) = uri.strip_prefix("file://") else { return uri.to_string() };
    let b = p.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&p[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn uri() {
        assert_eq!(super::uri_to_path("file:///home/u/%D0%9E%D0%B1%D1%80%D0%B0%D0%B7%20x.iso"), "/home/u/Образ x.iso");
    }
}
