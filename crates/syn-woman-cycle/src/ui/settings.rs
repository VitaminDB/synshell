//! «Настройки»: параметры цикла, масштаб и тема, окно, данные.

use std::sync::atomic::{AtomicU64, Ordering};

use syngui::prelude::*;
use syngui::GestureDetector;

use super::{boxed, icon, scroll_page, page_header, rise, W};
use crate::icons;
use crate::model::{Settings, ThemeMode};
use crate::state::St;

static SCALE_SAVE: AtomicU64 = AtomicU64::new(0);

pub fn section(title: String, rows: Vec<W>) -> W {
    let mut col = Column::new().gap(0.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        col = col.child(r);
        if i + 1 < n {
            col = col.child(DecoratedBox::new().class("sep-wrap").child(DecoratedBox::new().class("sep")));
        }
    }
    boxed(Column::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(Text::new(title).class("section")).child(DecoratedBox::new().class("group").child(col)))
}

fn label(glyph: &str, title: String, hint: String) -> W {
    let mut texts = Column::new().gap(2.0).child(Text::new(title).max_lines(2).class("row-title"));
    if !hint.is_empty() {
        texts = texts.child(Text::new(hint).max_lines(3).class("hint"));
    }
    boxed(
        DecoratedBox::new()
            .class("grow")
            .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(DecoratedBox::new().class("set-badge").child(icon(glyph).class("ic set-ic"))).child(DecoratedBox::new().class("grow").child(texts))),
    )
}

fn toggle_row(st: St, glyph: &str, title: String, hint: String, on: bool, set: impl Fn(&mut Settings, bool) + Send + Sync + 'static) -> W {
    let set = std::sync::Arc::new(set);
    let s2 = set.clone();
    boxed(
        GestureDetector::new().on_click(move || {
            let s = s2.clone();
            st.set_settings(move |x| s(x, !on));
        }).child(
            DecoratedBox::new().class("set-row").child(
                Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(glyph, title, hint)).child(Toggle::new().on(on).on_change(move |v| {
                    let s = set.clone();
                    st.set_settings(move |x| s(x, v));
                })),
            ),
        ),
    )
}

pub fn stepper(st: St, glyph: &str, title: String, hint: String, value: i32, min: i32, max: i32, unit: String, set: fn(&mut Settings, i32)) -> W {
    let dec = Button::new("").icon(icons::REMOVE).class("btn round step").disabled(value <= min).on_click(move || st.set_settings(|s| set(s, (value - 1).max(min))));
    let inc = Button::new("").icon(icons::ADD).class("btn round step").disabled(value >= max).on_click(move || st.set_settings(|s| set(s, (value + 1).min(max))));
    let value = DecoratedBox::new().class("step-value").child(Column::new().cross_axis_alignment(CrossAxisAlignment::Center).child(Text::new(value.to_string()).class("step-num")).child(Text::new(unit).class("step-unit")));
    let controls = Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(dec).child(value).child(inc);
    // на узком окне (или крупном масштабе) кнопки — под подписью
    let row: W = if viewport_size().get_untracked().width < 400.0 {
        boxed(Column::new().gap(10.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(label(glyph, title, hint)).child(Row::new().main_axis_alignment(MainAxisAlignment::End).child(controls)))
    } else {
        boxed(Row::new().gap(8.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(glyph, title, hint)).child(controls))
    };
    boxed(DecoratedBox::new().class("set-row").child(row))
}

/// Масштаб интерфейса: применяется сразу, сохраняется после паузы.
fn set_scale(st: St, v: f32) {
    let v = (v * 20.0).round() / 20.0;
    let v = v.clamp(0.7, 2.0);
    syngui::scale::set_ui_scale(v);
    st.scale.set(v);
    let id = SCALE_SAVE.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        run_on_main_thread(move || {
            if SCALE_SAVE.load(Ordering::Relaxed) == id {
                st.set_settings(|s| s.scale = v);
            }
        });
    });
}

fn scale_row(st: St) -> W {
    let value = Reactive::new(move || -> Vec<W> { vec![boxed(Text::new(format!("{}%", (st.scale.get() * 100.0).round() as i32)).class("scale-value"))] });
    let start = st.scale.get_untracked();
    let presets: [f32; 5] = [0.85, 1.0, 1.15, 1.3, 1.5];
    let chips = Reactive::new(move || -> Vec<W> {
        let cur = st.scale.get();
        let mut row = Flex::new().direction(FlexDirection::Row).wrap().gap(8.0);
        for p in presets {
            let on = (cur - p).abs() < 0.01;
            row = row.child(Button::new(format!("{}%", (p * 100.0).round() as i32)).class(if on { "btn pick plum on" } else { "btn pick plum" }).on_click(move || set_scale(st, p)));
        }
        vec![boxed(row)]
    });
    boxed(
        DecoratedBox::new().class("set-row").child(
            Column::new()
                .gap(12.0)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .child(Row::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Center).child(label(icons::ZOOM, t!("Масштаб"), t!("Размер текста и элементов"))).child(value))
                .child(
                    Row::new()
                        .gap(8.0)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .child(Button::new("").icon(icons::REMOVE).class("btn round step").on_click(move || set_scale(st, st.scale.get_untracked() - 0.05)))
                        .child(DecoratedBox::new().class("grow").child(Slider::new().range(0.7, 2.0).step(0.05).value(start).on_change(move |v| set_scale(st, v))))
                        .child(Button::new("").icon(icons::ADD).class("btn round step").on_click(move || set_scale(st, st.scale.get_untracked() + 0.05))),
                )
                .child(chips),
        ),
    )
}

fn theme_row(st: St, cur: ThemeMode) -> W {
    let mut row = Row::new().gap(6.0).cross_axis_alignment(CrossAxisAlignment::Stretch);
    for (m, g, l) in [(ThemeMode::System, icons::AUTO, t!("Как в системе")), (ThemeMode::Light, icons::LIGHT, t!("Светлая")), (ThemeMode::Dark, icons::DARK, t!("Тёмная"))] {
        let on = m == cur;
        row = row.child(
            GestureDetector::new()
                .class("grow")
                .on_click(move || {
                    st.set_settings(|s| s.theme = m);
                    crate::apply_theme(st);
                })
                .child(
                    DecoratedBox::new().class(if on { "seg on" } else { "seg" }).child(
                        Column::new().gap(4.0).cross_axis_alignment(CrossAxisAlignment::Center).child(icon(g).class(if on { "ic seg-ic on" } else { "ic seg-ic" })).child(Text::new(l).max_lines(1).class(if on { "seg-text on" } else { "seg-text" })),
                    ),
                ),
        );
    }
    boxed(
        DecoratedBox::new()
            .class("set-row")
            .child(Column::new().gap(12.0).cross_axis_alignment(CrossAxisAlignment::Stretch).child(label(icons::PALETTE, t!("Тема"), String::new())).child(DecoratedBox::new().class("segs").child(row))),
    )
}

pub fn page(st: St, wide: bool) -> W {
    boxed(Reactive::new(move || -> Vec<W> {
        let data = st.data.get();
        let fc = st.fc.get_untracked();
        let s = data.settings.clone();
        let mut blocks: Vec<W> = vec![page_header(st, wide, t!("Цикл, вид и данные"), t!("Настройки"), vec![])];
        let auto_hint = if s.auto && fc.cycles.iter().filter(|c| c.len.is_some()).count() >= 1 { t!("Сейчас по истории: цикл {c} дн., месячные {p} дн.", c = fc.cycle, p = fc.period) } else { t!("Пока истории мало — берутся значения ниже") };
        blocks.push(rise(
            0,
            section(
                t!("Цикл"),
                vec![
                    toggle_row(st, icons::SPARK, t!("Считать по истории"), auto_hint, s.auto, |x, v| x.auto = v),
                    stepper(st, icons::EVENT, t!("Длина цикла"), t!("Обычно 21–35 дней"), s.cycle_len, 18, 45, t!("дн."), |x, v| x.cycle_len = v),
                    stepper(st, icons::DROP, t!("Длительность месячных"), t!("Обычно 3–7 дней"), s.period_len, 1, 10, t!("дн."), |x, v| x.period_len = v),
                    stepper(st, icons::EGG, t!("Лютеиновая фаза"), t!("От овуляции до месячных, обычно 14"), s.luteal, 10, 16, t!("дн."), |x, v| x.luteal = v),
                ],
            ),
        ));
        blocks.push(rise(
            1,
            section(
                t!("Вид"),
                vec![
                    scale_row(st),
                    theme_row(st, s.theme),
                    toggle_row(st, icons::CALENDAR, t!("Неделя с понедельника"), String::new(), s.monday, |x, v| x.monday = v),
                    toggle_row(st, icons::FLOWER, t!("Показывать фертильное окно"), String::new(), s.show_fertile, |x, v| x.show_fertile = v),
                    toggle_row(st, icons::EGG, t!("Показывать овуляцию"), String::new(), s.show_ovulation, |x, v| x.show_ovulation = v),
                ],
            ),
        ));
        blocks.push(rise(
            2,
            section(
                t!("Окно"),
                vec![toggle_row(st, icons::FULLSCREEN, t!("Открывать во весь экран"), t!("Выйти из полноэкранного режима — кнопкой вверху или F11"), s.fullscreen, |x, v| {
                    x.fullscreen = v;
                    syngui::signal::set_fullscreen(v);
                })],
            ),
        ));
        let n = data.days.len();
        blocks.push(rise(
            3,
            section(
                t!("Данные"),
                vec![
                    boxed(DecoratedBox::new().class("set-row").child(label(icons::LOCK, t!("Только на этом устройстве"), t!("Отметки никуда не отправляются: {n} дней в файле ~/.local/share/synshell/woman-cycle", n = n)))),
                    boxed(DecoratedBox::new().class("set-row").child(Button::new(t!("Удалить все отметки")).icon(icons::DELETE).class("btn danger").on_click(move || st.confirm_reset.set(true)))),
                ],
            ),
        ));
        blocks.push(Box::new(Text::new(format!("{} {}", t!("Цикл"), env!("CARGO_PKG_VERSION"))).class("hint center disclaimer")));
        vec![scroll_page(wide, blocks)]
    }))
}

pub fn reset_dialog(st: St) -> W {
    let scrim = Reactive::new(move || -> Vec<W> {
        let shown = st.confirm_reset.get();
        let vp = viewport_size().get();
        vec![boxed(
            Presence::new(shown, GestureDetector::new().on_click(move || st.confirm_reset.set(false)).child(DecoratedBox::new().class("scrim").style("width", vp.width).style("height", vp.height)))
                .enter(Motion::fade())
                .exit(Motion::fade())
                .duration_ms(200)
                .initial(false),
        )]
    });
    let card = move || -> W {
        boxed(
            GestureDetector::new().on_click(|| {}).child(
                DecoratedBox::new().class("dialog").child(
                    Column::new()
                        .gap(14.0)
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .child(Row::new().main_axis_alignment(MainAxisAlignment::Center).child(DecoratedBox::new().class("hero-badge danger").child(icon(icons::DELETE).class("ic hero-ic"))))
                        .child(Text::new(t!("Удалить все отметки?")).class("h2 center"))
                        .child(Text::new(t!("История циклов, симптомы и настроение будут удалены без возможности восстановления. Настройки сохранятся.")).max_lines(5).class("hint center"))
                        .child(
                            Row::new()
                                .gap(10.0)
                                .child(Button::new(t!("Отмена")).class("btn ghost grow").on_click(move || st.confirm_reset.set(false)))
                                .child(Button::new(t!("Удалить")).class("btn danger-solid grow").on_click(move || {
                                    st.reset();
                                    st.toast(t!("Все отметки удалены"));
                                })),
                        ),
                ),
            ),
        )
    };
    boxed(
        Stack::new().fit(StackFit::Expand).child(scrim).child(
            Column::new().main_axis_alignment(MainAxisAlignment::Center).cross_axis_alignment(CrossAxisAlignment::Center).child(
                Presence::signal(st.confirm_reset, card).enter(Motion::fade().scale(0.9)).exit(Motion::fade().scale(0.95)).duration_ms(260).exit_duration_ms(160),
            ),
        ),
    )
}
