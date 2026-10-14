//! Панель настроек: справа во всю высоту поверх терминала, как в «Видео».

use syngui::prelude::*;
use syngui::widgets::*;

use crate::schemes::{self, Scheme};
use crate::settings::{FONT_MAX, FONT_MIN, SCROLLBACK_STEPS};
use crate::state::St;
use crate::ui::{boxed, icons, W};

/// Значение списка шрифтов «системный моноширинный».
const DEFAULT_FONT: &str = "__default__";

pub fn layer(st: St) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        if st.settings_open.get() {
            vec![panel(st)]
        } else {
            vec![]
        }
    }))
}

fn panel(st: St) -> W {
    let close = move || st.settings_open.set(false);
    let body = Column::new()
        .gap(10.0)
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(font_section(st))
        .child(colors_section(st))
        .child(tabs_section(st))
        .child(shell_section(st))
        .child(keys_section())
        .class("set-body");
    let panel = Column::new()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .child(
            Row::new()
                .gap(8.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(Text::new(t!("Настройки")).class("set-title"))
                .child(DecoratedBox::new().class("grow"))
                .child(ToolButton::new(icons::CLOSE).tooltip(t!("Закрыть")).on_click(close).class("set-close"))
                .class("set-head"),
        )
        .child(ScrollView::new().vertical().child(body).class("grow"))
        .class("set-panel");
    // Затемнение закрывает по щелчку; панель — справа во всю высоту.
    boxed(
        Stack::new()
            .fit(StackFit::Expand)
            .child(GestureDetector::new().on_click(close).child(DecoratedBox::new().class("set-scrim")))
            .child(
                Row::new()
                    .main_axis_alignment(MainAxisAlignment::End)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .child(GestureDetector::new().on_click(|| {}).child(panel)),
            ),
    )
}

fn section(icon: &'static str, title: &str, rows: Vec<W>) -> W {
    let mut c = Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Icon::new(icon).class("set-sec-icon"))
            .child(Text::new(title).class("set-sec-title")),
    );
    for r in rows {
        c = c.child(r);
    }
    boxed(DecoratedBox::new().child(c).class("set-card"))
}

fn hint(text: &str) -> W {
    boxed(Text::new(text).max_lines(4).class("set-hint"))
}

fn toggle_row(title: &str, sub: &str, on: bool, f: impl FnMut(bool) + Send + 'static) -> W {
    boxed(
        Row::new()
            .gap(12.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(
                Column::new()
                    .gap(2.0)
                    .child(Text::new(title).max_lines(2).class("set-row-title"))
                    .child(Text::new(sub).max_lines(3).class("set-hint"))
                    .class("grow"),
            )
            .child(Toggle::with_state(on).on_change(f)),
    )
}

/// Подпись и значение над ползунком.
fn slider_head(title: String, value: String) -> W {
    boxed(
        Row::new()
            .gap(8.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(DecoratedBox::new().class("grow").child(Text::new(title).class("set-row-title")))
            .child(Text::new(value).class("set-value")),
    )
}

// ─── разделы ───────────────────────────────────────────────────────────────

fn font_section(st: St) -> W {
    let s = st.settings.get_untracked();
    let family = {
        let mut items = vec![DropdownItem::new(DEFAULT_FONT, t!("Системный моноширинный"))];
        for name in syngui::text::list_monospace_families() {
            items.push(DropdownItem::new(name.clone(), name.clone()));
        }
        let selected = if s.font_family.trim().is_empty() { DEFAULT_FONT.to_string() } else { s.font_family.clone() };
        boxed(
            Dropdown::with_items(items)
                .selected(selected)
                .on_change(move |v| {
                    let v = if v == DEFAULT_FONT { String::new() } else { v.to_string() };
                    st.update(|s| s.font_family = v);
                })
                .class("set-dropdown"),
        )
    };
    // Значения у ползунков — реактивные (меняются и от Ctrl+= / Ctrl+−),
    // сам ползунок пересобирается только при смене размера снаружи.
    let size_head = Reactive::new(move || -> Vec<W> {
        let v = st.settings.get().font_size;
        vec![slider_head(t!("Размер"), t!("{n} px", n = v as i32))]
    });
    let size = Reactive::new(move || -> Vec<W> {
        let _ = st.font_rev.get();
        let v = st.settings.get_untracked().font_size;
        vec![boxed(
            Slider::new()
                .range(FONT_MIN, FONT_MAX)
                .step(1.0)
                .value(v)
                .on_change(move |v| st.update(|s| s.font_size = v.round().clamp(FONT_MIN, FONT_MAX))),
        )]
    });
    let lh_head = Reactive::new(move || -> Vec<W> {
        let v = st.settings.get().line_height;
        vec![slider_head(t!("Межстрочный интервал"), format!("{v:.2}"))]
    });
    let lh = Slider::new()
        .range(1.0, 1.6)
        .step(0.05)
        .value(s.line_height)
        .on_change(move |v| st.update(|s| s.line_height = ((v * 20.0).round() / 20.0).clamp(1.0, 2.0)));
    section(
        icons::TEXT,
        &t!("Шрифт"),
        vec![
            family,
            boxed(size_head),
            boxed(size),
            boxed(lh_head),
            boxed(lh),
            hint(&t!("Ctrl+= и Ctrl+− — крупнее и мельче, Ctrl+0 — как было.")),
        ],
    )
}

/// Образец схемы: фон, «Aa» цветом текста и полоска цветов ANSI.
fn swatch(st: St, id: &'static str, name: String, sc: Scheme, selected: bool) -> W {
    let mut dots = Row::new().gap(3.0);
    for i in 1..7 {
        dots = dots.child(DecoratedBox::new().class("sw-dot").style("background-color", schemes::color(sc.ansi[i])));
    }
    let preview = DecoratedBox::new().class("sw-preview").style("background-color", schemes::color(sc.bg)).child(
        Column::new()
            .gap(6.0)
            .child(
                Row::new()
                    .gap(4.0)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .child(Text::new("$").class("sw-text").style("color", schemes::color(sc.cursor)))
                    .child(Text::new("ls -la").class("sw-text").style("color", schemes::color(sc.fg))),
            )
            .child(dots),
    );
    let mut col = Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(preview);
    col = col.child(
        Row::new()
            .gap(4.0)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .child(Text::new(name).max_lines(1).elide(Elide::End).class("sw-name grow"))
            .child(if selected { boxed(Icon::new(icons::CHECK).class("sw-check")) } else { boxed(DecoratedBox::new()) }),
    );
    boxed(
        GestureDetector::new()
            .on_click(move || st.update(|s| s.scheme = id.to_string()))
            .cursor(syngui::CursorIcon::Pointer)
            .child(DecoratedBox::new().class(if selected { "swatch selected" } else { "swatch" }).child(col)),
    )
}

fn colors_section(st: St) -> W {
    let grid = Reactive::new(move || -> Vec<W> {
        let cur = st.settings.get().scheme;
        let _ = st.theme.get();
        let system = {
            let id = "system";
            let sc = st.scheme_of(id);
            swatch(st, id, t!("Как в системе"), sc, !schemes::BUILTIN.iter().any(|b| b.id == cur))
        };
        let mut items = vec![system];
        for b in schemes::BUILTIN {
            items.push(swatch(st, b.id, b.name.to_string(), schemes::builtin(b), b.id == cur));
        }
        vec![boxed(Grid::new(2).gap(8.0).children(items))]
    });
    let op_head = Reactive::new(move || -> Vec<W> {
        let v = st.settings.get().opacity;
        vec![slider_head(t!("Непрозрачность фона"), format!("{}%", (v * 100.0).round() as i32))]
    });
    let op = Slider::new()
        .range(0.5, 1.0)
        .step(0.05)
        .value(st.settings.get_untracked().opacity)
        .on_change(move |v| st.update(|s| s.opacity = ((v * 20.0).round() / 20.0).clamp(0.5, 1.0)));
    section(icons::PALETTE, &t!("Цвета"), vec![boxed(grid), boxed(op_head), boxed(op)])
}

fn tabs_section(st: St) -> W {
    let s = st.settings.get_untracked();
    section(
        icons::TAB,
        &t!("Вкладки"),
        vec![
            toggle_row(&t!("Новая вкладка — в той же папке"), &t!("Иначе — в домашней."), s.new_tab_here, move |on| st.update(|s| s.new_tab_here = on)),
            toggle_row(
                &t!("Спрашивать перед закрытием"),
                &t!("Если во вкладке или окне работает программа."),
                s.confirm_close,
                move |on| st.update(|s| s.confirm_close = on),
            ),
        ],
    )
}

fn shell_section(st: St) -> W {
    let s = st.settings.get_untracked();
    let default = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let field = TextField::with_text(s.shell.clone())
        .placeholder(default)
        .submit_on_focus_lost(true)
        .on_submit(move |v| {
            let v = v.trim().to_string();
            st.update(|s| s.shell = v);
        })
        .class("set-field");
    let history = SegmentedButton::new(
        SCROLLBACK_STEPS
            .iter()
            .map(|n| if *n >= 1000 { t!("{n} тыс.", n = n / 1000) } else { n.to_string() })
            .collect::<Vec<_>>(),
    )
    .selected(SCROLLBACK_STEPS.iter().position(|v| *v == s.scrollback).unwrap_or(1))
    .on_change(move |i| st.update(|s| s.scrollback = SCROLLBACK_STEPS[i.min(SCROLLBACK_STEPS.len() - 1)]));
    section(
        icons::HISTORY,
        &t!("Оболочка и история"),
        vec![
            boxed(Text::new(t!("Оболочка")).class("set-row-title")),
            boxed(field),
            hint(&t!("Пусто — оболочка пользователя. Действует на новые вкладки.")),
            boxed(Text::new(t!("Строк истории")).class("set-row-title")),
            boxed(history),
        ],
    )
}

fn keys_section() -> W {
    let rows: Vec<(&str, String)> = vec![
        ("Ctrl+Shift+T", t!("новая вкладка")),
        ("Ctrl+Shift+W", t!("закрыть вкладку")),
        ("Ctrl+Shift+N", t!("новое окно")),
        ("Ctrl+Shift+C / V", t!("копировать / вставить")),
        ("Ctrl+PgUp / PgDn", t!("соседняя вкладка")),
        ("Ctrl+Shift+PgUp / PgDn", t!("переставить вкладку")),
        ("Ctrl+1…9", t!("вкладка по номеру")),
        ("Ctrl+= / − / 0", t!("размер шрифта")),
    ];
    let mut items: Vec<W> = Vec::new();
    for (k, what) in rows {
        items.push(boxed(
            Row::new()
                .gap(10.0)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .child(DecoratedBox::new().class("kbd").child(Text::new(k).class("kbd-text")))
                .child(Text::new(what).max_lines(2).class("set-hint grow")),
        ));
    }
    section(icons::KEYBOARD, &t!("Клавиши"), items)
}
