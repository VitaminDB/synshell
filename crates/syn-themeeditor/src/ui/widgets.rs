//! Строки инспектора: цвет (пикер, hex, контраст, сброс), ползунок, поле.

use syngui::prelude::*;
use syngui::widgets::input::color_picker::ColorValue;
use synshell_common::config::Rgba;
use synshell_common::t;

use super::{boxed, tip, W};
use crate::color;
use crate::icons;
use crate::state::{values, with_draft, St};

/// Что правит строка цвета.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Поле варианта темы (`bg`, `accent`, `titlebar`, …).
    Field(&'static str),
    /// Своя переменная варианта (`[dark.vars]`), имя без `--`.
    Var(String),
}

impl Target {
    fn key(&self) -> String {
        match self {
            Target::Field(k) => format!("f:{k}"),
            Target::Var(v) => format!("v:{v}"),
        }
    }

    /// Значение, заданное в теме явно.
    pub fn explicit(&self, st: &St) -> Option<String> {
        let v = st.variant.get_untracked();
        with_draft(|d| match self {
            Target::Field(k) => d.get(v, k),
            Target::Var(n) => d.get_var(v, n),
        })
    }

    pub fn set(&self, st: &St, value: Option<&str>, quiet: bool) {
        let v = st.variant.get_untracked();
        let t = self.clone();
        let value = value.map(str::to_string);
        let f = move |d: &mut crate::draft::Draft| match &t {
            Target::Field(k) => d.set(v, k, value.as_deref()),
            Target::Var(n) => d.set_var(v, n, value.as_deref()),
        };
        if quiet {
            st.edit_quiet(&self.key(), f);
        } else {
            st.edit(&self.key(), f);
        }
    }
}

/// Цвет из значения темы: `#hex` или имя цвета палитры (`accent`, `surface`, …).
pub fn resolve_color(st: &St, s: &str) -> Option<Rgba> {
    Rgba::parse(s).or_else(|| {
        let vals = values(st);
        let name = s.trim().replace('_', "-");
        vals.get(&name).and_then(|v| Rgba::parse(v))
    })
}

fn to_cv(c: Rgba) -> ColorValue {
    ColorValue::new(c.r, c.g, c.b).with_alpha(c.a)
}

fn hex_of(c: ColorValue) -> String {
    Rgba { r: c.r, g: c.g, b: c.b, a: c.a }.hex()
}

/// Строка цвета.
///
/// `effective` — имя токена в [`values`], из которого брать действующее
/// значение, когда в теме его нет; `contrast_on` — токен фона для оценки
/// контраста текста.
pub fn color_row(st: St, target: Target, label: String, sub: String, effective: Option<String>, contrast_on: Option<&'static str>) -> W {
    let t_pick = target.clone();
    let eff_pick = effective.clone();
    // Пикер — перестраивается только при внешних изменениях (см. `St::ext`).
    let picker = Reactive::new(move || -> Vec<W> {
        let _ = st.ext.get();
        let _ = st.variant.get();
        let cur = t_pick.explicit(&st).or_else(|| eff_pick.as_ref().and_then(|e| values(&st).get(e).cloned()));
        let c = cur.as_deref().and_then(|s| resolve_color(&st, s)).unwrap_or(Rgba::rgb(0x80, 0x80, 0x80));
        let t2 = t_pick.clone();
        vec![boxed(DecoratedBox::new().class("te-checker").child(
            ColorPicker::new().color(to_cv(c)).show_alpha(true).width(40.0).on_change(move |cv| {
                t2.set(&st, Some(&hex_of(cv)), true);
            }),
        ))]
    });
    let t_info = target.clone();
    let info = Reactive::new(move || -> Vec<W> {
        syngui::i18n::subscribe();
        let _ = st.rev.get();
        let explicit = t_info.explicit(&st);
        let vals = values(&st);
        let shown = explicit.clone().or_else(|| effective.as_ref().and_then(|e| vals.get(e).cloned())).unwrap_or_default();
        let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Center);
        let t_hex = t_info.clone();
        row = row.child(
            TextField::new()
                .text(shown.clone())
                .placeholder(t!("по умолчанию"))
                .width(108.0)
                .submit_on_focus_lost(true)
                .on_submit(move |s| {
                    let s = s.trim();
                    if s.is_empty() {
                        t_hex.set(&st, None, false);
                    } else if Rgba::parse(s).is_some() || values(&st).contains_key(&s.replace('_', "-")) {
                        t_hex.set(&st, Some(s), false);
                    } else {
                        st.toast.set(t!("Цвет пишется как #rrggbb, #rrggbbaa или имя цвета палитры"));
                    }
                })
                .class(if explicit.is_some() { "te-hex" } else { "te-hex te-inherited" }),
        );
        if let (Some(bg), Some(fg)) = (contrast_on, resolve_color(&st, &shown)) {
            if let Some(bgc) = vals.get(bg).and_then(|v| Rgba::parse(v)) {
                let r = color::contrast(fg, bgc);
                let (grade, class) = color::contrast_grade(r);
                row = row.child(tip(
                    DecoratedBox::new().class(&format!("te-grade {class}")).child(Text::new(format!("{} {:.1}", grade, r)).class("te-grade-text")),
                    t!("Контраст с «{bg}»: {r}:1. Для текста нужно не меньше 4.5, для крупного и значков — 3", bg = bg, r = format!("{r:.1}")),
                ));
            }
        }
        if explicit.is_some() {
            let t_reset = t_info.clone();
            row = row.child(super::icon_button(icons::RESET, t!("Сбросить: значение по умолчанию"), "te-mini", move || t_reset.set(&st, None, false)));
        } else {
            row = row.child(DecoratedBox::new().class("te-mini-space"));
        }
        vec![boxed(row)]
    });
    let name = match &target {
        Target::Field(k) => k.replace('_', "-"),
        Target::Var(v) => v.clone(),
    };
    boxed(
        Row::new()
            .gap(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .class("te-row")
            .child(picker)
            .child(
                DecoratedBox::new().class("grow").child(
                    Column::new()
                        .gap(1.0)
                        .child(Reactive::new(move || -> Vec<W> {
                            let hit = st.pick.get().is_some_and(|p| p.tokens.iter().any(|t| *t == name));
                            vec![boxed(Text::new(label.clone()).max_lines(1).elide(Elide::End).class(if hit { "te-row-label te-row-hit" } else { "te-row-label" }))]
                        }))
                        .child(Text::new(sub).max_lines(1).elide(Elide::End).class("te-row-sub")),
                ),
            )
            .child(info),
    )
}

/// Ползунок с подписью и значением.
pub fn slider_row(label: String, value: f32, range: (f32, f32), step: f32, decimals: u8, on: impl FnMut(f32) + Send + 'static) -> W {
    boxed(
        Column::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .class("te-slider-row")
            .child(Text::new(label).class("te-row-label"))
            .child(Slider::new().range(range.0, range.1).step(step).value(value).show_value(decimals).on_change(on).class("te-slider")),
    )
}

/// Заголовок группы.
pub fn group_title(text: String) -> W {
    boxed(Text::new(text).class("te-group-title"))
}

/// Карточка группы.
pub fn card(children: Vec<W>) -> W {
    boxed(Column::new().gap(2.0).cross_axis_alignment(CrossAxisAlignment::Stretch).class("te-card").children(children))
}
