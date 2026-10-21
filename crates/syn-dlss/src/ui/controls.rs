//! Строки настроек, привязанные к полям общей памяти: значение читается при построении страницы,
//! изменение пишется сразу (`St::write`), с `tuningSeq` для полей, которые модель читает при создании
//! своей функции.

use syngui::prelude::*;
use syngui::IntoWidget;

use super::{boxed, W};
use crate::shm::{b2f, f2b, Field};
use crate::state::{header, St};

/// Читается ли поле моделью при создании функции (изменение стоит пересборки у помощника).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Latch {
    Live,
    AtCreate,
}

fn latched(l: Latch) -> bool {
    l == Latch::AtCreate
}

fn raw(f: Field) -> u32 {
    header().map(|h| h.get(f)).unwrap_or(0)
}

/// «Подпись (подсказка) — элемент». Длинная подсказка уходит во всплывающую.
pub fn row<M>(label: impl Into<String>, hint: impl Into<String>, control: impl IntoWidget<M>) -> W {
    let (label, hint) = (label.into(), hint.into());
    let mut left = Column::new().gap(2.0).class("row-text").child(Text::new(label).class("row-label"));
    if !hint.is_empty() {
        left = left.child(Text::new(hint).max_lines(3).class("row-hint"));
    }
    boxed(Row::new().gap(16.0).cross_axis_alignment(CrossAxisAlignment::Center).class("setting-row").child(left).child(control))
}

/// Группа: заголовок и карточка со строками через разделители.
pub fn group(title: impl Into<String>, rows: Vec<W>) -> W {
    let mut card = Column::new().cross_axis_alignment(CrossAxisAlignment::Stretch).class("group-card");
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        card = card.child(r);
        if i + 1 < n {
            card = card.child(DecoratedBox::new().class("row-sep"));
        }
    }
    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title.into()).class("group-title")).child(card))
}

pub fn switch(st: St, f: Field, latch: Latch, invert: bool) -> impl Widget {
    let on = (raw(f) != 0) != invert;
    Toggle::with_state(on).on_change(move |v| st.write(f, u32::from(v != invert), latched(latch)))
}

pub fn bool_row(st: St, label: impl Into<String>, hint: impl Into<String>, f: Field, latch: Latch) -> W {
    row(label, hint, switch(st, f, latch, false))
}

/// Ползунок дробного поля с числом справа.
pub fn slider(st: St, f: Field, min: f32, max: f32, step: f32, decimals: u8, latch: Latch) -> impl Widget {
    Slider::new()
        .range(min, max)
        .step(step)
        .value(b2f(raw(f)).clamp(min, max))
        .show_value(decimals)
        .width(240.0)
        .on_change(move |v| {
            let k = 10f32.powi(decimals.max(2) as i32);
            st.write(f, f2b((v * k).round() / k), latched(latch))
        })
}

pub fn slider_row(st: St, label: impl Into<String>, hint: impl Into<String>, f: Field, range: (f32, f32, f32), decimals: u8, latch: Latch) -> W {
    row(label, hint, slider(st, f, range.0, range.1, range.2, decimals, latch))
}

/// Ползунок дроби, показанной процентами (разрешение модели).
pub fn percent_row(st: St, label: impl Into<String>, hint: impl Into<String>, f: Field, min: f32, max: f32, latch: Latch) -> W {
    let w = Slider::new()
        .range(min, max)
        .step(5.0)
        .value((b2f(raw(f)) * 100.0).round().clamp(min, max))
        .show_value(0)
        .width(240.0)
        .on_change(move |v| st.write(f, f2b(v.round() / 100.0), latched(latch)));
    row(label, hint, w)
}

/// Ползунок целого поля (проценты, миллисекунды).
pub fn int_slider_row(st: St, label: impl Into<String>, hint: impl Into<String>, f: Field, min: u32, max: u32, step: u32, latch: Latch) -> W {
    let w = Slider::new()
        .range(min as f32, max as f32)
        .step(step as f32)
        .value(raw(f).clamp(min, max) as f32)
        .show_value(0)
        .width(240.0)
        .on_change(move |v| st.write(f, v.round().max(0.0) as u32, latched(latch)));
    row(label, hint, w)
}

pub fn int_spin(st: St, f: Field, min: u32, max: u32, latch: Latch) -> impl Widget {
    SpinBox::new()
        .range(min as f64, max as f64)
        .step(1.0)
        .value(raw(f).clamp(min, max) as f64)
        .width(140.0)
        .on_change(move |v| st.write(f, v.round().max(0.0) as u32, latched(latch)))
}

pub fn int_row(st: St, label: impl Into<String>, hint: impl Into<String>, f: Field, min: u32, max: u32, latch: Latch) -> W {
    row(label, hint, int_spin(st, f, min, max, latch))
}

/// Дробное поле с широким диапазоном (белая точка до 2000) — числом, не ползунком.
pub fn float_row(st: St, label: impl Into<String>, hint: impl Into<String>, f: Field, range: (f64, f64, f64), latch: Latch) -> W {
    let (min, max, step) = range;
    let w = SpinBox::new()
        .range(min, max)
        .step(step)
        .decimal_places(if step < 0.01 { 3 } else { 2 })
        .value((b2f(raw(f)) as f64).clamp(min, max))
        .width(140.0)
        .on_change(move |v| st.write(f, f2b(v as f32), latched(latch)));
    row(label, hint, w)
}

/// Выпадающий список: значение поля — индекс варианта.
pub fn choice(st: St, f: Field, options: &[String], latch: Latch) -> impl Widget {
    let mut dd = Dropdown::new().width(240.0).max_height(360.0);
    for (i, o) in options.iter().enumerate() {
        dd = dd.item(DropdownItem::new(i.to_string(), o.clone()));
    }
    let cur = raw(f) as usize;
    dd.selected(if cur < options.len() { cur } else { 0 }.to_string()).on_change(move |v: &str| {
        if let Ok(i) = v.parse::<u32>() {
            st.write(f, i, latched(latch))
        }
    })
}

pub fn choice_row(st: St, label: impl Into<String>, hint: impl Into<String>, f: Field, options: &[String], latch: Latch) -> W {
    row(label, hint, choice(st, f, options, latch))
}
