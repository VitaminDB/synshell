//! Кирпичики страниц: карточки-группы, строки «подпись — элемент» и
//! элементы управления, привязанные к пути в `config.toml`.

use syngui::prelude::*;
use syngui::IntoWidget;

use crate::store::{self, Seg};

/// Владеющий сегмент пути — живёт в замыканиях обработчиков.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum O {
    K(String),
    I(usize),
}

impl From<&str> for O {
    fn from(s: &str) -> Self {
        O::K(s.to_string())
    }
}

impl From<String> for O {
    fn from(s: String) -> Self {
        O::K(s)
    }
}

impl From<&String> for O {
    fn from(s: &String) -> Self {
        O::K(s.clone())
    }
}

impl From<usize> for O {
    fn from(i: usize) -> Self {
        O::I(i)
    }
}

pub type P = Vec<O>;

/// `op!["panel", 0, "size"]` — владеющий путь.
#[macro_export]
macro_rules! op {
    ($($s:expr),* $(,)?) => { vec![$($crate::ui::O::from($s)),*] };
}

pub fn segs(p: &[O]) -> Vec<Seg<'_>> {
    p.iter()
        .map(|o| match o {
            O::K(k) => Seg::K(k.as_str()),
            O::I(i) => Seg::I(*i),
        })
        .collect()
}

pub fn set(p: &[O], v: impl Into<toml_edit::Value>) {
    store::set(&segs(p), v);
}

pub fn unset(p: &[O]) {
    store::remove(&segs(p));
}

pub type W = Box<dyn Widget>;

thread_local! {
    static NARROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Узкое окно (телефон): строки складываются в столбец.
pub fn narrow() -> bool {
    NARROW.with(|n| n.get())
}

pub fn set_narrow(v: bool) {
    NARROW.with(|n| n.set(v));
}

/// Путь с `~` вместо домашнего каталога.
pub fn home_short(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

pub fn boxed(w: impl Widget + 'static) -> W {
    Box::new(w)
}

// ─── Раскладка страницы ─────────────────────────────────────────────────────

/// Страница: заголовок, подзаголовок и группы, с прокруткой.
pub fn page(title: &str, subtitle: &str, body: Vec<W>) -> W {
    let mut col = Column::new()
        .gap(18.0)
        .class(if narrow() { "page-body page-body-narrow" } else { "page-body" })
        .child(
            Column::new()
                .gap(4.0)
                .child(Text::new(title).class("page-title"))
                .child(Text::new(subtitle).class("page-subtitle")),
        );
    for w in body {
        col = col.child(w);
    }
    boxed(ScrollView::new().vertical().class("page-scroll").child(col))
}

/// Карточка с заголовком и строками, разделёнными тонкими линиями.
pub fn group(title: &str, rows: Vec<W>) -> W {
    let mut inner = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("group-card");
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        inner = inner.child(r);
        if i + 1 < n {
            inner = inner.child(DecoratedBox::new().class("row-sep"));
        }
    }
    let mut col = Column::new().gap(8.0);
    if !title.is_empty() {
        col = col.child(Text::new(title).class("group-title"));
    }
    boxed(col.child(inner))
}

/// Пояснение под группой или внутри страницы.
pub fn note(text: &str) -> W {
    boxed(Text::new(text).class("note"))
}

/// Строка: подпись (+ подсказка) слева, элемент управления справа. В узком
/// окне — элемент под подписью.
pub fn row<M>(label: &str, hint: &str, control: impl IntoWidget<M>) -> W {
    if narrow() {
        return row_wide(label, hint, control);
    }
    row_inline(label, hint, control)
}

/// Строка «подпись — элемент» и в узком окне (переключатели, кнопки).
pub fn row_inline<M>(label: &str, hint: &str, control: impl IntoWidget<M>) -> W {
    let mut left = Column::new().gap(2.0).class("row-text").child(Text::new(label).class("row-label"));
    if !hint.is_empty() {
        left = left.child(Text::new(hint).class("row-hint"));
    }
    boxed(
        Row::new()
            .gap(16.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("setting-row")
            .child(left)
            .child(control),
    )
}

/// Строка, у которой элемент управления под подписью (длинные поля).
pub fn row_wide<M>(label: &str, hint: &str, control: impl IntoWidget<M>) -> W {
    let mut col = Column::new().gap(6.0).class("setting-row-wide").child(Text::new(label).class("row-label"));
    if !hint.is_empty() {
        col = col.child(Text::new(hint).class("row-hint"));
    }
    boxed(col.child(control))
}

// ─── Привязанные элементы управления ────────────────────────────────────────

pub fn switch(p: P, on: bool) -> impl Widget {
    Toggle::with_state(on).on_change(move |v| set(&p, v))
}

pub fn switch_row(label: &str, hint: &str, p: P, on: bool) -> W {
    row_inline(label, hint, switch(p, on))
}

pub fn text(p: P, value: &str, placeholder: &str, width: f32) -> impl Widget {
    TextField::with_text(value)
        .placeholder(placeholder)
        .width(width)
        .on_change(move |s| set(&p, s.to_string()))
}

pub fn text_row(label: &str, hint: &str, p: P, value: &str, placeholder: &str) -> W {
    row(label, hint, text(p, value, placeholder, 260.0))
}

/// Текстовое поле, где пустое значение удаляет ключ (необязательные поля правил).
pub fn opt_text(p: P, value: &str, placeholder: &str, width: f32) -> impl Widget {
    TextField::with_text(value).placeholder(placeholder).width(width).on_change(move |s| {
        if s.is_empty() {
            unset(&p)
        } else {
            set(&p, s.to_string())
        }
    })
}

/// Выпадающий список строковых значений: `(значение, подпись)`.
pub fn choice(p: P, current: &str, options: &[(&str, &str)], width: f32) -> impl Widget {
    let mut dd = Dropdown::new().width(width);
    for (v, l) in options {
        dd = dd.item(DropdownItem::new(*v, *l));
    }
    dd.selected(current).on_change(move |v: &str| set(&p, v.to_string()))
}

pub fn choice_row(label: &str, hint: &str, p: P, current: &str, options: &[(&str, &str)]) -> W {
    row(label, hint, choice(p, current, options, 220.0))
}

/// Выпадающий список из динамических значений.
pub fn choice_owned(p: P, current: &str, options: Vec<(String, String)>, width: f32) -> impl Widget {
    let mut dd = Dropdown::new().width(width).max_height(320.0);
    for (v, l) in options {
        dd = dd.item(DropdownItem::new(v, l));
    }
    dd.selected(current).on_change(move |v: &str| set(&p, v.to_string()))
}

pub fn int_spin(p: P, value: i64, min: i64, max: i64, step: i64) -> impl Widget {
    SpinBox::new()
        .range(min as f64, max as f64)
        .step(step as f64)
        .value(value as f64)
        .width(140.0)
        .on_change(move |v| set(&p, v.round() as i64))
}

pub fn int_row(label: &str, hint: &str, p: P, value: i64, min: i64, max: i64, step: i64) -> W {
    row(label, hint, int_spin(p, value, min, max, step))
}

pub fn float_spin(p: P, value: f64, min: f64, max: f64, step: f64, decimals: u8) -> impl Widget {
    SpinBox::new()
        .range(min, max)
        .step(step)
        .decimal_places(decimals)
        .value(value)
        .width(140.0)
        .on_change(move |v| set(&p, round_to(v, decimals)))
}

pub fn round_to(v: f64, decimals: u8) -> f64 {
    let k = 10f64.powi(decimals as i32);
    (v * k).round() / k
}

/// Ползунок с числом справа; значение пишется дробным.
pub fn slider(p: P, value: f64, min: f64, max: f64, step: f64, decimals: u8) -> impl Widget {
    Slider::new()
        .range(min as f32, max as f32)
        .step(step as f32)
        .value(value as f32)
        .show_value(decimals)
        .width(240.0)
        .on_change(move |v| set(&p, round_to(v as f64, decimals.max(2))))
}

pub fn slider_row(label: &str, hint: &str, p: P, value: f64, min: f64, max: f64, step: f64, decimals: u8) -> W {
    row(label, hint, slider(p, value, min, max, step, decimals))
}

/// Три состояния для необязательного bool (правила окон): не задано / да / нет.
pub fn tri(p: P, value: Option<bool>) -> impl Widget {
    let cur = match value {
        None => "-",
        Some(true) => "yes",
        Some(false) => "no",
    };
    Dropdown::new()
        .width(150.0)
        .item(DropdownItem::new("-", "Не менять"))
        .item(DropdownItem::new("yes", "Да"))
        .item(DropdownItem::new("no", "Нет"))
        .selected(cur)
        .on_change(move |v: &str| match v {
            "yes" => set(&p, true),
            "no" => set(&p, false),
            _ => unset(&p),
        })
}

/// Цвет `#rrggbb`: образец + поле ввода + палитра по кнопке.
pub fn color_field(p: P, value: &str, placeholder: &str) -> impl Widget {
    let shown = use_signal(false);
    let current = use_signal(value.to_string());
    let p2 = p.clone();
    Column::new()
        .gap(8.0)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(move || {
                    let hex = current.get();
                    let c = synshell_common::config::Rgba::parse(&hex)
                        .map(|c| Color::from_srgb(c.r, c.g, c.b, c.a as f32 / 255.0))
                        .unwrap_or(Color::transparent());
                    DecoratedBox::new().class("swatch").style("background", c)
                })
                .child(
                    TextField::with_text(value)
                        .placeholder(placeholder)
                        .width(130.0)
                        .on_change(move |s| {
                            current.set(s.to_string());
                            if s.is_empty() {
                                unset(&p);
                            } else if synshell_common::config::Rgba::parse(s).is_some() {
                                set(&p, s.to_string());
                            }
                        }),
                )
                .child(
                    Button::new("")
                        .icon(icons::PALETTE)
                        .class("icon-btn")
                        .on_click(move || shown.set(!shown.get_untracked())),
                ),
        )
        .child(Reactive::new(move || -> Vec<W> {
            if !shown.get() {
                return vec![];
            }
            let init = ColorValue::from_hex(&current.get_untracked()).unwrap_or(ColorValue::new(61, 139, 253));
            let p = p2.clone();
            vec![boxed(ColorPicker::new().color(init).width(260.0).on_change(move |c| {
                let hex = format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b);
                set(&p, hex);
            }))]
        }))
}

pub fn button(label: &str, on_click: impl FnMut() + Send + 'static) -> impl Widget {
    Button::new(label).class("btn").on_click(on_click)
}

pub fn primary_button(label: &str, on_click: impl FnMut() + Send + 'static) -> impl Widget {
    Button::new(label).class("btn primary").on_click(on_click)
}

pub fn icon_button(icon: &str, on_click: impl FnMut() + Send + 'static) -> impl Widget {
    Button::new("").icon(icon).class("icon-btn").on_click(on_click)
}

pub fn danger_icon_button(icon: &str, on_click: impl FnMut() + Send + 'static) -> impl Widget {
    Button::new("").icon(icon).class("icon-btn danger").on_click(on_click)
}

/// Кодовые точки Material Icons.
pub mod icons {
    pub const PALETTE: &str = "\u{e40a}";
    pub const WALLPAPER: &str = "\u{e1bc}";
    pub const PANEL: &str = "\u{e06c}";
    pub const WINDOW: &str = "\u{e069}";
    pub const STYLE: &str = "\u{e41d}";
    pub const ANIMATION: &str = "\u{e65f}";
    pub const WORKSPACES: &str = "\u{e8f0}";
    pub const RULES: &str = "\u{e85d}";
    pub const KEYBOARD: &str = "\u{e312}";
    pub const SHORTCUTS: &str = "\u{e3e7}";
    pub const MOUSE: &str = "\u{e323}";
    pub const DISPLAY: &str = "\u{e30c}";
    pub const APPS: &str = "\u{e5c3}";
    pub const NOTIFICATIONS: &str = "\u{e7f4}";
    pub const LOCK: &str = "\u{e897}";
    pub const AUTOSTART: &str = "\u{e037}";
    pub const SETTINGS: &str = "\u{e8b8}";
    pub const INFO: &str = "\u{e88e}";
    pub const SEARCH: &str = "\u{e8b6}";
    pub const CHEVRON_RIGHT: &str = "\u{e5cc}";
    pub const PHONE: &str = "\u{e325}";
    pub const WIFI: &str = "\u{e63e}";
    pub const BLUETOOTH: &str = "\u{e1a7}";
    pub const GESTURE: &str = "\u{e155}";
    pub const HARDWARE: &str = "\u{e30d}";
    pub const BATTERY: &str = "\u{e1a4}";
    pub const COPY: &str = "\u{e14d}";
    pub const BACK: &str = "\u{e5c4}";
    pub const ADD: &str = "\u{e145}";
    pub const DELETE: &str = "\u{e872}";
    pub const UP: &str = "\u{e5d8}";
    pub const DOWN: &str = "\u{e5db}";
    pub const UNDO: &str = "\u{e166}";
    pub const FOLDER: &str = "\u{e2c7}";
    pub const FILE: &str = "\u{e24d}";
    pub const OPEN: &str = "\u{e89e}";
    pub const REFRESH: &str = "\u{e5d5}";
    pub const TUNE: &str = "\u{e429}";
    pub const WARNING: &str = "\u{e002}";
    pub const CLOSE: &str = "\u{e5cd}";
    pub const CHECK: &str = "\u{e86c}";
    pub const THEMES: &str = "\u{e3b7}";
}
